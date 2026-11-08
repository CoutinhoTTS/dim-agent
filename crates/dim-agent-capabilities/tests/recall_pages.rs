//! The work behind a `recall` read: a long history with no match must not
//! re-fold the pages it already holds (design §3.1, issue #871).
//!
//! The fold counter is process-wide, so this file holds one test: it is the
//! only test in its binary and no other test can fold while it runs.
use dim_agent_capabilities::{
    adapters::EchoModel,
    defaults,
    history::HistoryRenderer,
    recall::{
        self, RECALL_TOOL,
        testing::{folds, reset_folds},
    },
};
use dim_agent_runtime::testing::MinimalHost;
use dim_agent_runtime::testing::chain_events;
use dim_agent_runtime::*;
use dim_agent_types::*;
use std::{sync::Arc, time::Duration};

async fn agent() -> MinimalHost {
    let mut builder = AgentBuilder::new(AgentConfig::default());
    defaults::mock_hosts_with_model(
        &mut builder,
        HistoryRenderer::default(),
        Arc::new(EchoModel),
    );
    defaults::install(&mut builder);
    recall::install(&mut builder);
    MinimalHost::start(&builder).await.unwrap()
}

/// Runs one no-match search after `turns` completed turns and returns the
/// folds the call built.
async fn no_match_folds(agent: &MinimalHost, turns: usize) -> usize {
    let session = agent
        .create_session(SessionConfig {
            tools: None,
            ..SessionConfig::default()
        })
        .await
        .unwrap();
    for task in 0..turns {
        session
            .submit(format!("seed {task}"), InputMode::Queue, "main")
            .await
            .unwrap();
        session.idle("main").await.unwrap();
    }
    reset_folds();
    session
        .submit(
            format!(
                "tool:{RECALL_TOOL} {{\"about\":\"missing_unique_qzxy\",\"tool\":\"missing_tool\"}}"
            ),
            InputMode::Queue,
            "main",
        )
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    let folds = folds();
    let result = chain_events(&session, "main", ..)
        .await
        .unwrap()
        .iter()
        .rev()
        .find_map(|event| match &event.kind {
            EventKind::ToolResult(data) => data.results.last().map(|item| item.result.clone()),
            _ => None,
        })
        .expect("the recall result");
    assert_eq!(
        result.content,
        ToolContent::Text(
            "Nothing matches about \"missing_unique_qzxy\" and tool missing_tool.".into()
        )
    );
    folds
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_no_match_search_does_not_fold_once_per_page() {
    tokio::time::timeout(Duration::from_secs(120), async {
        let agent = agent().await;
        let short = no_match_folds(&agent, 40).await;
        let long = no_match_folds(&agent, 120).await;
        assert!(
            (1..=2).contains(&short),
            "a no-match search folds {short} times"
        );
        assert_eq!(
            short, long,
            "the folds grew with the pages read: {short} then {long}"
        );
        agent.close().await.unwrap();
    })
    .await
    .expect("the fold regression flow");
}
