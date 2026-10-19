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

/// The first input owned by `branch` whose raw block starts with `prefix`.
async fn marked(
    agent: &dim_agent_runtime::Agent,
    session: &str,
    branch: &str,
    prefix: &str,
) -> String {
    let session = agent.get_session(session).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            for event in session.local(branch, ..).await.unwrap() {
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
/// How many inputs owned by `branch` carry a raw block starting with `prefix`.
async fn count(
    agent: &dim_agent_runtime::Agent,
    session: &str,
    branch: &str,
    prefix: &str,
) -> usize {
    agent
        .get_session(session)
        .unwrap()
        .local(branch, ..)
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
        let marker = marked(&agent, session, "main", "<c5-marker").await;
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
        assert_eq!(count(&agent, session, "main", "<c5-marker").await, 1);
    }

    // A child shares its Session services, but has its own Dispatcher,
    // identity and local fold, despite inheriting its parent's marker.
    let parent = agent.get_session(&sessions[0]).unwrap();
    let child = parent
        .branch(
            "main".into(),
            parent.sequence("main").unwrap(),
            dim_agent_runtime::BranchOptions {
                prompt: "child branch input".into(),
                source: dim_agent_types::BranchSource::Host {
                    operation_id: "c5-child".into(),
                },
            },
        )
        .await
        .unwrap();
    let marker = marked(&agent, &sessions[0], &child, "<c5-marker").await;
    assert!(marker.contains(&format!(r#"branch="{child}""#)), "{marker}");
    parent.idle(&child).await.unwrap();
    assert_eq!(count(&agent, &sessions[0], &child, "<c5-marker").await, 1);
    assert_eq!(count(&agent, &sessions[0], "main", "<c5-marker").await, 1);

    drop(client);
    server.shutdown();
    server.join().await.unwrap();

    // Restored Sessions: each new generation rebuilds from committed history,
    // so it does not write the first marker again, and reports its rebuild
    // on its first live turn end.
    let (server, agent) = launch_stored(config).await;
    let mut client = Client::new_version(&server, false, 3).await;
    for (session, branch) in [
        (&sessions[0], "main"),
        (&sessions[1], "main"),
        (&sessions[0], child.as_str()),
    ] {
        assert_ok(
            &client
                .rpc(
                    "session.submit",
                    json!({"sessionId":session,"branchId":branch,"text":"again"}),
                )
                .await,
        );
        let rebuilt = marked(&agent, session, branch, "<c5-rebuilt").await;
        assert!(!rebuilt.contains(r#"from="0""#), "{rebuilt}");
        assert_eq!(count(&agent, session, branch, "<c5-marker").await, 1);
        assert_eq!(count(&agent, session, branch, "<c5-rebuilt").await, 1);
    }
    server.shutdown();
    server.join().await.unwrap();
    let _ = std::fs::remove_dir_all(&home);
}

/// `treeCommands.create` takes the caller's cancellation as an explicit
/// argument, as C4's `cancelled` does: one that never completes lets the
/// child be created; one that has completed ends the call `Cancelled`. A
/// creation already committed is a fact TimelineTree finishes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_branch_typescript_create_takes_explicit_cancellation() {
    let mut config = branch_part();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    config.rows = vec![json!({
        "id": "tree-create",
        "name": root.join("runtime/fixtures/tree-create.ts"),
        "provides": serde_json::from_str::<Value>(include_str!("../../../../runtime/fixtures/tree-create.json")).unwrap(),
        "inject": ["treeCommands"],
        "config": {},
    })];
    let (server, agent) = launch_with_agent(
        Arc::new(MockTools),
        ServerConfig {
            session_typescript: Some(config),
            ..Default::default()
        },
    )
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    let created = client.rpc("session.create", create(json!([]))).await;
    assert_ok(&created);
    let id = created["result"]["sessionId"].as_str().unwrap().to_owned();
    let session = agent.get_session(&id).unwrap();
    let at = session.sequence("main").unwrap();
    let probe = |cancel: bool| json!({"sessionId":id,"at":at,"cancel":cancel});

    // Session publication does not wait for optional method providers.
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let listed = client.rpc("session.methods", json!({"sessionId":id})).await;
            assert_ok(&listed);
            if listed["result"]["methods"]
                .as_array()
                .unwrap()
                .iter()
                .any(|method| method["name"] == "session.treeprobe.create")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("tree-create fixture publishes its method");

    let kept = client.rpc("session.treeprobe.create", probe(false)).await;
    let child = kept["result"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("{kept}"))
        .to_owned();
    assert!(session.branches().iter().any(|branch| branch.id == child));

    let cancelled = client.rpc("session.treeprobe.create", probe(true)).await;
    assert_eq!(cancelled["result"]["error"], "Cancelled", "{cancelled}");
    // Cancellation stops admission; a creation whose anchor already
    // committed is a fact the tree finishes. Either way every anchor on the
    // parent ends as a child branch or a recorded failure.
    session.idle("main").await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let events = session.local("main", ..).await.unwrap();
            let anchors = events
                .iter()
                .filter(|event| matches!(&event.kind, EventKind::BranchCreate(_)))
                .count();
            let failed = events
                .iter()
                .filter(|event| matches!(&event.kind, EventKind::BranchCreateFailed(_)))
                .count();
            if session.branches().len() - 1 + failed == anchors {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("every creation anchor settles");
    server.shutdown();
    server.join().await.unwrap();
}
