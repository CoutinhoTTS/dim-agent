use super::*;
use dim_app_server::session_typescript::SessionTypeScriptConfig;
use std::path::PathBuf;

fn parts() -> SessionTypeScriptConfig {
    let mut parts = session_typescript::fixture();
    parts.process_rows = vec![json!({
        "id":"process-example", "name":parts.anchor.parent().unwrap().join("fixtures/process-part.ts"),
        "provides":serde_json::from_str::<Value>(include_str!("../../../../runtime/fixtures/process-part.json")).unwrap(),
        "settings":serde_json::from_str::<Value>(include_str!("../../../../runtime/fixtures/settings-owner.settings.json")).unwrap(),
        "config":{},
    })];
    parts
}

fn config(home: &std::path::Path, parts: SessionTypeScriptConfig) -> ServerConfig {
    ServerConfig {
        product_home: Some(home.to_owned()),
        target_home: Some(home.join("rust")),
        session_typescript: Some(parts),
        ..Default::default()
    }
}

fn home() -> PathBuf {
    PathBuf::from("/tmp").join(format!("dm-proc-{}", uuid::Uuid::new_v4()))
}

async fn available(client: &mut Client, server: &RunningServer) -> Value {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let reply = client
                .rpc("example.status", json!({"original":[1,true]}))
                .await;
            if reply.get("result").is_some() {
                return reply["result"].clone();
            }
            assert_eq!(
                reply["error"]["data"]["appError"],
                "service_reloading",
                "{reply}: {:?}",
                server
                    .diagnostics()
                    .plugins
                    .iter()
                    .filter(|plugin| plugin.error.is_some())
                    .collect::<Vec<_>>()
            );
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}

async fn select_model(client: &mut Client) {
    assert_ok(&client.rpc("provider.save", json!({"provider":{
        "providerId":"process-fixture", "displayName":"Fixture", "driverKind":"openai-compatible",
        "baseUrl":"http://localhost/v1", "auth":{"type":"anonymous"}, "models":[{
            "modelId":"fixture", "displayName":"Fixture", "status":"active",
            "capabilities":dim_agent_types::ModelCapabilities::default()
        }], "enabled":true
    }})).await);
    assert_ok(
        &client
            .rpc(
                "provider.select",
                json!({"providerId":"process-fixture","modelId":"fixture"}),
            )
            .await,
    );
}

async fn started(client: &mut Client, id: u64) {
    loop {
        let notice = client.notice("management.progress").await;
        if notice["params"]["id"] == id && notice["params"]["data"]["phase"] == "started" {
            return;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn process_typescript_rejects_invalid_embedding_before_runtime_start() {
    let process = json!({"id":"example", "name":"/fixture/process.ts", "provides":{
        "managementMethods:example":{"describe":"async","call":"async"}
    }});
    let mut cases = Vec::new();
    for service in [
        "managementMethods:",
        "managementMethods:hello.child",
        "managementMethods:session.child",
        "managementMethods:bad-namespace",
        "managementMethods:daemon.shutdown.child",
        "settings",
        "treeCommands",
        "toolContribution:unexpected",
        "sessionMethods:session.example",
        "sessionEvents:example",
    ] {
        let mut row = process.clone();
        row["provides"] = json!({});
        row["provides"][service] = json!({"describe":"async","call":"async"});
        cases.push((vec![row], vec![]));
    }
    for service in ["dispatcher", "treeCommands"] {
        let mut row = process.clone();
        row["inject"] = json!([service]);
        cases.push((vec![row], vec![]));
    }
    let mut row = process.clone();
    row["provides"]["managementMethods:example"]["call"] = json!("sync");
    cases.push((vec![row], vec![]));
    cases.push((vec![process.clone(), process.clone()], vec![]));
    for field in ["id", "name"] {
        let mut row = process.clone();
        row[field] = json!("Rpc");
        cases.push((vec![row], vec![]));
    }
    cases.push((
        vec![process.clone()],
        vec![json!({"id":"session", "name":"/fixture/session.ts",
        "provides":{"sessionMethods:example.child":{"describe":"async","call":"async"}}})],
    ));
    cases.push((vec![], vec![process]));
    for (process_rows, rows) in cases {
        let mut builder = AgentBuilder::new(AgentConfig::default());
        defaults::install(&mut builder);
        defaults::mock_hosts(&mut builder);
        let result = serve(
            builder,
            ServerConfig {
                unix_path: Some(
                    std::env::temp_dir()
                        .join(format!("dm-invalid-{}/server.sock", uuid::Uuid::new_v4())),
                ),
                session_typescript: Some(SessionTypeScriptConfig {
                    node: "/unstarted/node".into(),
                    js_runtime: "/unstarted/runtime".into(),
                    anchor: "/unstarted/package.json".into(),
                    rows,
                    process_rows,
                }),
                ..Default::default()
            },
        )
        .await;
        assert!(matches!(
            result,
            Err(dim_app_server::ServerError::Configuration(_))
        ));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_process_typescript_shared_owner_and_local_management_boundary() {
    let home = home();
    let (server, _, _) = session_typescript::start(config(&home, parts())).await;
    let mut client = Client::new_version(&server, false, 3).await;
    select_model(&mut client).await;
    let before = available(&mut client, &server).await;
    assert_eq!(before["params"], json!({"original":[1,true]}));
    assert!(!before["callId"].as_str().unwrap().is_empty());
    let features = Client::new_version(&server, false, 3).await.features;
    assert!(
        features
            .as_array()
            .unwrap()
            .contains(&json!("example.status"))
    );
    let mut sessions = Vec::new();
    for _ in 0..2 {
        let created = client
            .rpc("session.create", create(json!(["echo", "ts_echo"])))
            .await;
        assert_ok(&created);
        let id = created["result"]["sessionId"].as_str().unwrap().to_owned();
        let status = client
            .rpc("session.typescript.status", json!({"sessionId":id}))
            .await;
        assert_ok(&status);
        assert_eq!(
            status["result"]["pid"], before["pid"],
            "one shared Node runtime"
        );
        sessions.push(id);
    }
    for id in sessions {
        assert_ok(&client.rpc("session.close", json!({"sessionId":id})).await);
        assert_eq!(
            available(&mut client, &server).await["generation"],
            before["generation"]
        );
    }
    assert_eq!(
        client.rpc("example.read", json!({})).await["result"]["maxResults"],
        5
    );
    let error = client.rpc("example.error", json!({})).await;
    assert_eq!(error["error"]["code"], -32011);
    assert_eq!(error["error"]["data"]["appError"], "fixture_error");
    let malformed = client.rpc("example.malformed", json!({})).await;
    assert_eq!(malformed["error"]["code"], -32603);
    assert!(!malformed.to_string().contains("extra"));
    let empty = client.rpc("example.undefined", json!({})).await;
    assert_ok(&empty);
    assert_eq!(empty["result"], Value::Null);

    let mut remote = Client::new_version(&server, true, 3).await;
    let mut legacy = Client::new_version(&server, false, 2).await;
    assert_eq!(
        remote.rpc("example.status", json!({})).await["error"]["data"]["appError"],
        "local_transport_required"
    );
    assert_eq!(
        legacy.rpc("example.status", json!({})).await["error"]["data"]["appError"],
        "unknown_method"
    );
    assert!(server.restart("process-example").await.is_err());

    let report = client.send("example.report", json!({})).await;
    started(&mut client, report).await;
    let drain = client.send("example.drain", json!({})).await;
    started(&mut client, drain).await;
    server.restart("Rpc").await.unwrap();
    assert_eq!(
        client.response(report).await["error"]["data"]["appError"],
        "operation_interrupted"
    );
    let settled = client.response(drain).await;
    assert_ok(&settled);
    assert_eq!(settled["result"]["generation"], before["generation"]);
    assert_eq!(settled["result"]["alive"], true);
    assert_eq!(settled["result"]["cancelled"], true);
    client = Client::new_version(&server, false, 3).await;
    assert_eq!(
        available(&mut client, &server).await["generation"],
        before["generation"]
    );
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_process_typescript_dependency_reload_settles_calls_and_keeps_failures_local() {
    let home = home();
    let mut parts = parts();
    parts.process_rows[0]["config"] = json!({"failAfter":2});
    let (mut builder, _, _) = session_typescript::setup();
    let (send, mut controls) = tokio::sync::mpsc::unbounded_channel();
    builder.session_plugin(move |scope| session_typescript::ControlledGate(*scope, send.clone()));
    let mut config = config(&home, parts);
    config.unix_path = Some(home.join("server.sock"));
    let server = serve(builder, config).await.unwrap();
    let mut client = Client::new_version(&server, false, 3).await;
    select_model(&mut client).await;
    let before = available(&mut client, &server).await;
    let created = client.rpc("session.create", create(json!([]))).await;
    assert_ok(&created);
    let (_, _, loader, _) = controls.recv().await.unwrap();
    let report = client.send("example.report", json!({})).await;
    started(&mut client, report).await;
    let drain = client.send("example.drain", json!({})).await;
    started(&mut client, drain).await;
    let disconnected = {
        let mut disconnected = Client::new_version(&server, false, 3).await;
        let call = disconnected.send("example.drain", json!({})).await;
        started(&mut disconnected, call).await;
        disconnected
    };
    drop(disconnected);
    let settings = loader
        .entries()
        .into_iter()
        .find(|entry| entry.options["name"] == "Settings")
        .expect("Settings row");
    loader.restart(&settings.id).await.unwrap();
    assert_eq!(
        client.response(report).await["error"]["data"]["appError"],
        "operation_interrupted"
    );
    let settled = client.response(drain).await;
    assert_ok(&settled);
    assert_eq!(settled["result"]["generation"], before["generation"]);
    assert_eq!(settled["result"]["alive"], true);
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if matches!(loader.get("process-example").unwrap().status,
                rutis_loader::EntryStatus::Running(ref state) if state.state == rutis::FiberState::Failed) {
                break;
            }
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    assert_eq!(
        client.rpc("example.status", json!({})).await["error"]["data"]["appError"],
        "service_unavailable"
    );
    for plugin in ["Rpc", "Agents"] {
        tokio::time::timeout(Duration::from_secs(10), server.restart(plugin))
            .await
            .unwrap()
            .unwrap();
        client = Client::new_version(&server, false, 3).await;
        assert_eq!(
            client.rpc("example.status", json!({})).await["error"]["data"]["appError"],
            "service_unavailable"
        );
    }
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_process_typescript_optional_pending_failed_unresolved_and_disabled_owners() {
    for mode in ["pending", "failed", "unresolved", "disabled", "bad_methods"] {
        let home = home();
        let mut parts = parts();
        let row = &mut parts.process_rows[0];
        match mode {
            "pending" => row["inject"] = json!(["missingProcessDependency"]),
            "failed" => row["config"] = json!({"failAfter":1}),
            "unresolved" => row["provides"]["extra"] = json!({"read":"sync"}),
            "disabled" => row["disabled"] = json!(true),
            "bad_methods" => row["config"] = json!({"badMethods":true}),
            _ => unreachable!(),
        }
        // A disabled child must not fall through to its active parent namespace.
        parts.process_rows.push(json!({"id":"disabled-child", "name":parts.anchor.parent().unwrap().join("fixtures/disabled-child.ts"),
            "disabled":true,"provides":{"managementMethods:example.child":{"describe":"async","call":"async"}}}));
        let (server, mut requests, _) = session_typescript::start(config(&home, parts)).await;
        let mut client = Client::new_version(&server, false, 3).await;
        select_model(&mut client).await;
        let expected = match mode {
            "pending" => "service_reloading",
            "disabled" => "unknown_method",
            _ => "service_unavailable",
        };
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let reply = client.rpc("example.status", json!({})).await;
                if reply["error"]["data"]["appError"] == expected {
                    break;
                }
                assert_eq!(
                    reply["error"]["data"]["appError"], "service_reloading",
                    "{mode}: {reply}"
                );
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            client.rpc("example.child.status", json!({})).await["error"]["data"]["appError"],
            "unknown_method"
        );
        for plugin in ["Rpc", "Agents"] {
            server.restart(plugin).await.unwrap();
            client = Client::new_version(&server, false, 3).await;
            assert_eq!(
                client.rpc("example.status", json!({})).await["error"]["data"]["appError"],
                expected
            );
        }
        let created = client.rpc("session.create", create(json!(["echo"]))).await;
        assert_ok(&created);
        let session = &created["result"]["sessionId"];
        assert_ok(&client.rpc("session.submit", json!({"sessionId":session,"branchId":"main","text":"native remains available"})).await);
        assert_ok(
            &client
                .rpc(
                    "session.idle",
                    json!({"sessionId":session,"branchId":"main"}),
                )
                .await,
        );
        assert!(requests.recv().await.unwrap().contains(&"echo".into()));
        server.shutdown();
        server.join().await.unwrap();
        std::fs::remove_dir_all(home).unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_process_typescript_held_apply_and_describe_do_not_hold_native_startup() {
    for hold in ["holdApply", "hangDescribe"] {
        let home = home();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut parts = parts();
        parts.process_rows[0]["config"] = json!({});
        parts.process_rows[0]["config"][hold] =
            json!({"host":"127.0.0.1","port":listener.local_addr().unwrap().port()});
        let (server, _, _) = tokio::time::timeout(
            Duration::from_secs(10),
            session_typescript::start(config(&home, parts)),
        )
        .await
        .unwrap();
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(10), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let mut client = Client::new_version(&server, false, 3).await;
        select_model(&mut client).await;
        assert_eq!(
            client.rpc("example.status", json!({})).await["error"]["data"]["appError"],
            "service_reloading"
        );
        assert_ok(&client.rpc("session.create", create(json!(["echo"]))).await);
        socket.write_all(b"release").await.unwrap();
        assert_eq!(available(&mut client, &server).await["alive"], true);
        server.shutdown();
        server.join().await.unwrap();
        let mut byte = [0];
        assert_eq!(
            tokio::time::timeout(
                Duration::from_secs(10),
                tokio::io::AsyncReadExt::read(&mut socket, &mut byte)
            )
            .await
            .unwrap()
            .unwrap(),
            0
        );
        std::fs::remove_dir_all(home).unwrap();
    }
}
