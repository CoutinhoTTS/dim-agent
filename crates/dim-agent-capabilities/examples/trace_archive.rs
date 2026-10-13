//! Generate an offline inspection sample using deterministic hosts, without credentials.
use dim_agent_capabilities::{
    defaults,
    host::{HostFuture, LoadPrompt},
    trace::{
        self,
        archive::{ArchiveContent, TraceArchive},
    },
};
use dim_agent_runtime::{
    AgentBuilder, AgentConfig, BranchOptions, ConfigureUpdate, PromptSourceTarget, Session,
    SessionConfig,
};
use dim_agent_types::{
    EventKind, InputMode, ModelSelection, PromptDelivery, PromptEntry, PromptScope, TurnOutcome,
};
use std::{path::Path, sync::Arc};
use tokio_util::sync::CancellationToken;

struct SamplePrompt;
impl LoadPrompt for SamplePrompt {
    fn load(
        &self,
        target: PromptSourceTarget,
        _: CancellationToken,
    ) -> HostFuture<'_, Option<PromptEntry>> {
        Box::pin(async move {
            Ok(Some(PromptEntry {
                source: target.source,
                scope: target.scope,
                version: "sample-v1".into(),
                body: "Use the echo tool to inspect the sample, then summarize the result.".into(),
                delivery: PromptDelivery::System,
                order: 0,
            }))
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args_os()
        .nth(1)
        .ok_or("usage: trace_archive <new-directory>")?;
    record_sample(Path::new(&directory)).await
}

async fn record_sample(directory: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let archive =
        TraceArchive::create_with_content(directory, ArchiveContent::SensitiveContent).await?;
    let mut builder = AgentBuilder::new(AgentConfig::default());
    defaults::mock_hosts(&mut builder);
    defaults::install_with_prompt(
        &mut builder,
        Arc::new(SamplePrompt),
        Some(ConfigureUpdate {
            sources: vec![PromptSourceTarget {
                source: "sample-rules".into(),
                scope: PromptScope::Branch {
                    timeline: "main".into(),
                },
            }],
            model: Some(ModelSelection {
                provider_id: "mock".into(),
                model_id: "deterministic".into(),
                capability_snapshot: None,
                config: None,
                metadata: None,
            }),
        }),
    );
    // The host root outlives every Session, so closing records reach the archive.
    let root = rutis::Ctx::root_with(tokio::runtime::Handle::current());
    trace::install(
        &mut builder,
        trace::TraceHost {
            ctx: root.clone(),
            recorder: archive.recorder(),
        },
    );
    let agent = builder.build_in(&root);
    let run = async {
        let session = agent.create_session(SessionConfig::default()).await?;
        session
            .submit(
                r#"tool:echo {"text":"Rust trace inspection sample"}"#.into(),
                InputMode::Queue,
                "main",
            )
            .await?;
        require_completed_turn(&session, "main").await?;
        let child = session
            .branch(
                "main".into(),
                session.sequence("main")?,
                BranchOptions {
                    prompt: "Inspect the child branch".into(),
                    source: dim_agent_types::BranchSource::Host {
                        operation_id: "sample-branch".into(),
                    },
                },
            )
            .await?;
        require_completed_turn(&session, &child).await
    }
    .await;
    let closed = agent.close().await;
    let status = archive.finish().await?;
    run?;
    closed?;
    if status.incomplete {
        return Err("archive incomplete".into());
    }
    println!(
        "Recorded {} records. Open tools/trace-viewer/rust.html and import trace.jsonl.",
        status.records
    );
    Ok(())
}

async fn require_completed_turn(
    session: &Session,
    branch: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    session.idle(branch).await?;
    // Idle means quiescent, not successful. Inspect only this branch's latest turn.
    let history = session.local(branch, ..).await?;
    let outcome = history.iter().rev().find_map(|event| match &event.kind {
        EventKind::TurnEnd(data) => Some(&data.outcome),
        _ => None,
    });
    match outcome {
        Some(TurnOutcome::Completed { .. }) => Ok(()),
        Some(outcome) => {
            Err(format!("Sample branch {branch} did not complete: {outcome:?}").into())
        }
        None => Err(format!("Sample branch {branch} has no completed turn").into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[tokio::test]
    async fn sample_archive_contains_successful_tool_continuation_and_child_turn() {
        let directory = std::env::temp_dir().join(format!("trace-sample-{}", uuid::Uuid::new_v4()));
        let result = record_sample(&directory).await;
        let contents = std::fs::read_to_string(directory.join("trace.jsonl")).unwrap();
        std::fs::remove_dir_all(directory).unwrap();
        result.unwrap();
        let records: Vec<Value> = contents
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.last().unwrap()["incomplete"], false);
        let events: Vec<&Value> = records
            .iter()
            .filter_map(|record| record.pointer("/record/payload/event"))
            .collect();
        let results: Vec<_> = events
            .iter()
            .filter(|event| event["type"] == "step" && event["name"] == "tool-result")
            .collect();
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0]["data"]["results"][0]["result"]["content"],
            "Rust trace inspection sample"
        );
        let calls: Vec<_> = events
            .iter()
            .filter(|event| event["type"] == "model" && event["name"] == "call")
            .collect();
        assert_eq!(calls.len(), 3);
        assert!(
            calls[1]["data"]["request"]["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|message| message["role"] == "tool_result"
                    && message.to_string().contains("Rust trace inspection sample"))
        );
        let ends: Vec<_> = events
            .iter()
            .filter(|event| event["type"] == "turn" && event["name"] == "end")
            .collect();
        assert_eq!(ends.len(), 2);
        assert!(
            ends.iter()
                .all(|event| event["data"]["status"] == "completed")
        );
        assert!(
            events
                .iter()
                .any(|event| event["type"] == "branch" && event["name"] == "create")
        );
    }

    #[tokio::test]
    async fn idle_failure_is_not_hidden_by_an_earlier_successful_turn() {
        let mut builder = AgentBuilder::new(AgentConfig::default());
        defaults::mock_hosts(&mut builder);
        defaults::install(&mut builder);
        let agent = builder.build();
        let session = agent
            .create_session(SessionConfig::default())
            .await
            .unwrap();
        session
            .submit("hello".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        require_completed_turn(&session, "main").await.unwrap();
        session
            .submit("tool:not-offered {}".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        let result = require_completed_turn(&session, "main").await;
        agent.close().await.unwrap();
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Tool was not offered")
        );
    }
}
