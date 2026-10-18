//! Branch TypeScript parts on the exported branch services
//! (branch-typescript.md): a row that injects `dispatcher` runs in every
//! branch, follows its committed events in order and writes input through
//! `treeCommands`.
use super::*;
use dim_app_server::session_typescript::SessionTypeScriptConfig;
use std::path::PathBuf;

fn branch_part() -> SessionTypeScriptConfig {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    SessionTypeScriptConfig {
        node: std::env::var_os("DIM_TEST_NODE")
            .expect("DIM_TEST_NODE")
            .into(),
        js_runtime: std::env::var_os("DIM_TEST_JS_RUNTIME")
            .expect("DIM_TEST_JS_RUNTIME")
            .into(),
        anchor: root.join("runtime/package.json"),
        rows: vec![json!({
            "id": "branch-part",
            "name": root.join("runtime/fixtures/branch-part.ts"),
            "provides": {},
            "inject": ["dispatcher", "timelineIdentity", "treeCommands"],
            "config": {},
        })],
    }
}

/// The first `main` input whose raw block starts with `prefix`, once it is
/// committed.
async fn marked(agent: &dim_agent_runtime::Agent, session: &str, prefix: &str) -> String {
    let session = agent.get_session(session).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            for event in session.history("main", None).await.unwrap() {
                if let EventKind::InputMessage(data) = &event.kind {
                    for block in &data.blocks {
                        let dim_agent_types::InputBlock::Raw { text, .. } = block;
                        if text.starts_with(prefix) {
                            return text.clone();
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the branch part writes its marker")
}
/// How many `main` inputs carry a raw block starting with `prefix`.
async fn count(agent: &dim_agent_runtime::Agent, session: &str, prefix: &str) -> usize {
    agent
        .get_session(session)
        .unwrap()
        .history("main", None)
        .await
        .unwrap()
        .iter()
        .filter(|event| {
            matches!(&event.kind, EventKind::InputMessage(data)
            if data.blocks.iter().any(|block| matches!(block,
                dim_agent_types::InputBlock::Raw { text, .. } if text.starts_with(prefix))))
        })
        .count()
}

/// A server on persistent storage, so a restart restores the Sessions.
async fn launch_stored(config: ServerConfig) -> (RunningServer, dim_agent_runtime::Agent) {
    let mut builder = AgentBuilder::new(AgentConfig::default());
    defaults::install(&mut builder);
    defaults::mock_model_hosts(&mut builder);
    dim_agent_capabilities::defaults::tool_host(&mut builder, Arc::new(MockTools));
    let server = serve(builder, config).await.unwrap();
    let agent = server.agent().unwrap();
    (server, agent)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_branch_typescript_part_follows_events_and_writes_input() {
    let home = std::env::temp_dir().join(format!(
        "dm-c5-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    ));
    let config = ServerConfig {
        unix_path: Some(home.join("server.sock")),
        database_path: Some(home.join("sessions.sqlite")),
        trace_directory: home.join("trace"),
        session_typescript: Some(branch_part()),
        ..Default::default()
    };
    let (server, agent) = launch_stored(config.clone()).await;
    let mut client = Client::new_version(&server, false, 3).await;
    let mut sessions = Vec::new();
    for text in ["first", "second"] {
        let created = client.rpc("session.create", create(json!([]))).await;
        assert_ok(&created);
        let session = created["result"]["sessionId"].as_str().unwrap().to_owned();
        assert_ok(
            &client
                .rpc(
                    "session.submit",
                    json!({"sessionId":session,"branchId":"main","text":text}),
                )
                .await,
        );
        sessions.push(session);
    }
    for session in &sessions {
        let marker = marked(&agent, session, "<c5-marker").await;
        // Its own branch identity, and the events in commit order.
        assert!(marker.contains(r#"branch="main""#), "{marker}");
        let seen = marker
            .split(r#"seen=""#)
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .unwrap();
        let input = seen.find("input/message").expect(seen);
        let start = seen.find("turn/start").expect(seen);
        let end = seen.find("turn/end").expect(seen);
        assert!(input < start && start < end, "{seen}");
        // One marker per branch: a later turn/end does not write again.
        assert_ok(
            &client
                .rpc(
                    "session.idle",
                    json!({"sessionId":session,"branchId":"main"}),
                )
                .await,
        );
        assert_eq!(count(&agent, session, "<c5-marker").await, 1);
    }

    drop(client);
    server.shutdown();
    server.join().await.unwrap();

    // Restored Sessions: each new generation rebuilds from committed history,
    // so it does not write the first marker again, and reports its rebuild
    // on its first live turn end.
    let (server, agent) = launch_stored(config).await;
    let mut client = Client::new_version(&server, false, 3).await;
    for session in &sessions {
        assert_ok(
            &client
                .rpc(
                    "session.submit",
                    json!({"sessionId":session,"branchId":"main","text":"again"}),
                )
                .await,
        );
        let rebuilt = marked(&agent, session, "<c5-rebuilt").await;
        assert!(!rebuilt.contains(r#"from="0""#), "{rebuilt}");
        assert_eq!(count(&agent, session, "<c5-marker").await, 1);
        assert_eq!(count(&agent, session, "<c5-rebuilt").await, 1);
    }
    server.shutdown();
    server.join().await.unwrap();
    let _ = std::fs::remove_dir_all(&home);
}
