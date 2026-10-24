//! Rows of other languages declare their group (plugin contract P4, rutis
//! usage §2, §3.5): nothing infers it from what they inject. A Python
//! Session row and branch row, and an unmodified Cordis 4 plugin in a branch
//! row, each read their own instance's services by plain names.
use super::branch_typescript::{launch_stored, marked};
use super::session_typescript::{fixture, submit};
use super::*;
use dim_app_server::session_typescript::PythonConfig;
use std::path::PathBuf;

/// Waits until every copy of the rows named `name` is Active, `expected`
/// copies in all.
async fn active(server: &RunningServer, name: &str, expected: usize) {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let rows: Vec<_> = server
                .diagnostics()
                .plugins
                .into_iter()
                .filter(|row| row.name.ends_with(name))
                .collect();
            if rows.len() == expected
                && rows
                    .iter()
                    .all(|row| row.state == rutis::FiberState::Active)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{expected} active copies of {name}"));
}

fn home(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "{prefix}-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    ))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE, DIM_TEST_JS_RUNTIME and DIM_TEST_PYTHON"]
async fn node_python_rows_mount_in_session_and_branch() {
    let mut parts = fixture();
    parts.python = Some(PythonConfig {
        interpreter: std::env::var_os("DIM_TEST_PYTHON")
            .expect("DIM_TEST_PYTHON")
            .into(),
        project: parts.anchor.parent().unwrap().join("fixtures/python"),
    });
    parts.rows = vec![
        json!({
            "id": "python-session", "name": "py:python_session_part", "group": "session",
            "inject": ["sessionIdentity"],
            "provides": {"toolContribution:py_echo": {"declaration": "sync", "execute": "async"}},
            "config": {},
        }),
        json!({
            "id": "python-branch", "name": "py:python_branch_part", "group": "branch",
            "inject": ["dispatcher", "timelineIdentity", "treeCommands"],
            "provides": {},
            "config": {},
        }),
    ];
    let home = home("dm-py");
    let (server, agent) = launch_stored(ServerConfig {
        unix_path: Some(home.join("server.sock")),
        database_path: Some(home.join("sessions.sqlite")),
        session_typescript: Some(parts),
        ..Default::default()
    })
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    let mut sessions = Vec::new();
    for _ in 0..2 {
        let mut params = create(json!(["py_echo"]));
        params["permissionPolicy"]["allowedTools"] = json!(["py_echo"]);
        let created = client.rpc("session.create", params).await;
        assert_ok(&created);
        sessions.push(created["result"]["sessionId"].as_str().unwrap().to_owned());
        active(&server, "py:python_session_part", sessions.len()).await;
        active(&server, "py:python_branch_part", sessions.len()).await;
    }
    for session in &sessions {
        submit(&mut client, session, r#"tool:py_echo {"text":"hello"}"#).await;
        // The Session row's tool ran with its own Session's identity.
        let history = agent
            .get_session(session)
            .unwrap()
            .history("main", None)
            .await
            .unwrap();
        let content = history
            .iter()
            .find_map(|event| match &event.kind {
                EventKind::ToolResult(batch) => batch
                    .results
                    .first()
                    .map(|result| serde_json::to_value(&result.result.content).unwrap()),
                _ => None,
            })
            .expect("the py_echo result");
        let echoed: Value = serde_json::from_str(content.as_str().unwrap()).unwrap();
        assert_eq!(
            echoed,
            json!({"session":session, "callSession":session, "branch":"main", "text":"hello"})
        );
        // The branch row folded its own branch's facts in order and wrote
        // its input there.
        let marker = marked(&agent, session, "main", "<py-marker").await;
        assert!(
            marker.contains(&format!(r#"session="{session}" branch="main""#)),
            "{marker}"
        );
        let seen = marker
            .split(r#"seen=""#)
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .unwrap();
        let input = seen.find("input/message").expect(seen);
        let start = seen.find("turn/start").expect(seen);
        let end = seen.find("turn/end").expect(seen);
        assert!(input < start && start < end, "{seen}");
    }
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_cordis_plugin_in_a_branch_row_gets_its_own_branch() {
    let mut parts = fixture();
    let root = parts.anchor.parent().unwrap().join("fixtures");
    parts.rows = vec![json!({
        "id": "cordis-branch", "name": root.join("cordis-branch.ts"), "group": "branch",
        "inject": ["dispatcher", "timelineIdentity", "treeCommands"],
        "provides": {},
        "config": {},
    })];
    let home = home("dm-cordis");
    let (server, agent) = launch_stored(ServerConfig {
        unix_path: Some(home.join("server.sock")),
        database_path: Some(home.join("sessions.sqlite")),
        session_typescript: Some(parts),
        ..Default::default()
    })
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    let mut sessions = Vec::new();
    for _ in 0..2 {
        let created = client.rpc("session.create", create(json!([]))).await;
        assert_ok(&created);
        let session = created["result"]["sessionId"].as_str().unwrap().to_owned();
        active(&server, "cordis-branch.ts", sessions.len() + 1).await;
        submit(&mut client, &session, "first").await;
        sessions.push(session);
    }
    for session in &sessions {
        let marker = marked(&agent, session, "main", "<cordis-marker").await;
        assert_eq!(
            marker,
            format!(r#"<cordis-marker session="{session}" branch="main"/>"#)
        );
    }
    // A child branch is an instance of its own: its copy writes its own id.
    let parent = agent.get_session(&sessions[0]).unwrap();
    let child = parent
        .branch(
            "main".into(),
            parent.sequence("main").unwrap(),
            dim_agent_runtime::BranchOptions {
                prompt: "child branch input".into(),
                source: dim_agent_types::BranchSource::Host {
                    operation_id: "cordis-child".into(),
                },
            },
        )
        .await
        .unwrap();
    active(&server, "cordis-branch.ts", 3).await;
    assert_ok(
        &client
            .rpc(
                "session.submit",
                json!({"sessionId":sessions[0],"branchId":child,"text":"child input"}),
            )
            .await,
    );
    let marker = marked(&agent, &sessions[0], &child, "<cordis-marker").await;
    assert_eq!(
        marker,
        format!(
            r#"<cordis-marker session="{}" branch="{child}"/>"#,
            sessions[0]
        )
    );
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}
