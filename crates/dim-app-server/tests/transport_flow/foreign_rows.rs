//! Rows of other languages declare their group (plugin contract P4, rutis
//! usage §2, §3.5): nothing infers it from what they inject. A Python
//! Session row and branch row, and an unmodified Cordis 4 plugin in a branch
//! row, each read their own instance's services by plain names.
use super::branch_typescript::{launch_stored, marked};
use super::session_typescript::{ControlledGate, fixture, new_session, setup, submit};
use super::*;
use dim_app_server::session_typescript::{PythonConfig, SessionTypeScriptConfig};
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
            "inject": ["sessionIdentity", "toolRegistry"],
            "provides": {},
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

/// The entry of row `id` in the Session instance `instance`.
fn entry(
    loader: &rutis_loader::Loader,
    id: &str,
    instance: rutis::InstanceId,
) -> rutis_loader::EntryInfo {
    loader
        .entries()
        .into_iter()
        .find(|entry| {
            entry.id == id
                && entry
                    .instance
                    .as_ref()
                    .is_some_and(|scope| scope.chain.iter().any(|(_, id)| *id == instance))
        })
        .unwrap_or_else(|| panic!("row {id} in the Session"))
}

/// A server whose Sessions report their instance and the loader.
async fn controlled(
    parts: SessionTypeScriptConfig,
) -> (
    RunningServer,
    tokio::sync::mpsc::UnboundedReceiver<Vec<String>>,
    tokio::sync::mpsc::UnboundedReceiver<super::session_typescript::Control>,
) {
    let (mut builder, requests, _) = setup();
    let (send, controls) = tokio::sync::mpsc::unbounded_channel();
    builder.session_part(move |names| ControlledGate::part(names, send.clone()));
    let server = serve(
        builder,
        ServerConfig {
            unix_path: Some(home("dm-rows").join("server.sock")),
            session_typescript: Some(parts),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    (server, requests, controls)
}

/// A `py:` module that cannot be imported leaves its row unresolved; the
/// Session and its native tools are unaffected.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE, DIM_TEST_JS_RUNTIME and DIM_TEST_PYTHON"]
async fn node_python_import_failure_is_local_to_its_row() {
    let mut parts = fixture();
    parts.python = Some(PythonConfig {
        interpreter: std::env::var_os("DIM_TEST_PYTHON")
            .expect("DIM_TEST_PYTHON")
            .into(),
        project: parts.anchor.parent().unwrap().join("fixtures/python"),
    });
    parts.rows = vec![json!({
        "id": "python-missing", "name": "py:no_such_fixture_module", "group": "session",
        "provides": {}, "config": {},
    })];
    let (server, mut requests, mut controls) = controlled(parts).await;
    let mut client = Client::new_version(&server, false, 3).await;
    let id = new_session(&mut client, json!(["echo"])).await;
    let (session_instance, _, loader, _) = controls.recv().await.unwrap();
    let status = entry(&loader, "python-missing", session_instance).status;
    assert!(
        matches!(&status, rutis_loader::EntryStatus::Unresolved(error)
            if error.to_string().contains("no_such_fixture_module")),
        "{status:?}"
    );
    submit(&mut client, &id, "native continues").await;
    assert!(requests.recv().await.unwrap().contains(&"echo".into()));
    server.shutdown();
    server.join().await.unwrap();
}

/// One provider per name per instance (rutis usage §3.1): of two rows that
/// provide one declared name in a Session, one provides it and the other
/// fails with rutis's duplicate-provider error; each Session has its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_a_second_provider_of_a_name_in_one_instance_fails() {
    let mut parts = fixture();
    let module = parts
        .anchor
        .parent()
        .unwrap()
        .join("fixtures/shared-provider.ts");
    parts.rows = ["first-provider", "second-provider"]
        .map(|id| {
            json!({
                "id": id, "name": module, "group": "session",
                "provides": {"exampleShared": {"ping": "sync"}}, "config": {},
            })
        })
        .to_vec();
    let (server, _requests, mut controls) = controlled(parts).await;
    let mut client = Client::new_version(&server, false, 3).await;
    for _ in 0..2 {
        new_session(&mut client, json!(["echo"])).await;
        let (session_instance, _, loader, _) = controls.recv().await.unwrap();
        let mut settled = Vec::new();
        for id in ["first-provider", "second-provider"] {
            let mut state = entry(&loader, id, session_instance).view.unwrap().watch();
            settled.push(
                tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        let snapshot = state.borrow().clone();
                        if matches!(
                            snapshot.state,
                            rutis::FiberState::Active | rutis::FiberState::Failed
                        ) {
                            break snapshot;
                        }
                        state.changed().await.unwrap();
                    }
                })
                .await
                .expect("the row settles"),
            );
        }
        let failed: Vec<_> = settled
            .iter()
            .filter(|snapshot| snapshot.state == rutis::FiberState::Failed)
            .collect();
        assert_eq!(failed.len(), 1, "{settled:?}");
        let error = failed[0].error.as_ref().unwrap().to_string();
        assert!(error.contains("exampleShared"), "{error}");
    }
    server.shutdown();
    server.join().await.unwrap();
}
