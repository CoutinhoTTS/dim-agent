use super::*;

/// The four `session.grants.*` methods resolve paths against the Session
/// workspace, and a stored write grant lets the next write run without an
/// approval prompt.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grants_are_managed_through_session_methods_and_allow_the_next_write() {
    use dim_agent_capabilities::{
        model_provider::ModelProviderPlugin, workspace_tools::WorkspaceToolsConfig,
    };
    use dim_agent_runtime::SessionConfig;
    use dim_agent_types::{PermissionPolicy, PermissionPreset};
    use std::os::unix::fs::DirBuilderExt;

    let home = std::path::PathBuf::from("/tmp").join(format!("dg-{}", uuid::Uuid::new_v4()));
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&home)
        .unwrap();
    let home = home.canonicalize().unwrap();
    let native = home.join("rust");
    let work = home.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let outside = home.join("outside.txt");
    let mut builder = AgentBuilder::new(AgentConfig::default());
    defaults::install_local_execution(
        &mut builder,
        ModelProviderPlugin::new,
        WorkspaceToolsConfig {
            output_root: native.join("outputs"),
        },
        dim_agent_capabilities::timeline_tree::BranchToolConfig::default(),
    );
    dim_app_server::permission_rules::install(&mut builder);
    dim_app_server::session_grants::install(&mut builder);
    defaults::mock_model_hosts(&mut builder);
    let server = serve(
        builder,
        ServerConfig {
            product_home: Some(home.clone()),
            target_home: Some(native.clone()),
            unix_path: Some(native.join("daemon.sock")),
            websocket_address: Some(([127, 0, 0, 1], 0).into()),
            session_defaults: SessionConfig {
                tools: Some(vec!["write".into()]),
                permission_policy: PermissionPolicy {
                    preset: PermissionPreset::WorkspaceWrite,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let mut client = Client::new_version(&server, false, 3).await;
    let caps = dim_agent_types::CatalogCapabilities::with_limits(32000, 2048);
    let entry =
        json!({"modelId":"alpha","displayName":"alpha","status":"active","capabilities":caps});
    assert_ok(&client.rpc("provider.create",json!({"providerId":"018ec73f-0000-7000-8000-000000000001","displayName":"Fixture","connection":{"driverKind":"openai-compatible","baseUrl":"https://model.example/v1","auth":{"type":"bearer"},"credential":{"action":"set","value":"fixture-access"}},"models":[entry],"enabled":true})).await);
    assert_ok(
        &client
            .rpc(
                "provider.select",
                json!({"providerId":"018ec73f-0000-7000-8000-000000000001","modelId":"alpha"}),
            )
            .await,
    );
    let created = client.rpc("session.create", json!({"cwd":work})).await;
    assert_ok(&created);
    let sid = created["result"]["sessionId"].as_str().unwrap().to_owned();
    let session = server.agent().unwrap().get_session(&sid).unwrap();
    let branch = json!({"sessionId":sid,"branchId":"main"});
    assert_ok(&client.rpc("session.attach", json!({"sessionId":sid})).await);

    let list = client
        .rpc("session.grants.list", json!({"sessionId":sid}))
        .await;
    assert_ok(&list);
    assert_eq!(list["result"]["grants"], json!([]));

    // `notes.md` is relative to the Session workspace, which the daemon
    // resolves; `sub/../notes.md` is the same canonical path.
    assert_ok(
        &client
            .rpc(
                "session.grants.add",
                json!({"sessionId":sid,"path":"notes.md","kind":"file","mode":"write"}),
            )
            .await,
    );
    assert_ok(
        &client
            .rpc(
                "session.grants.add",
                json!({"sessionId":sid,"path":"sub/../notes.md","kind":"file","mode":"read"}),
            )
            .await,
    );
    let list = client
        .rpc("session.grants.list", json!({"sessionId":sid}))
        .await;
    assert_ok(&list);
    assert_eq!(
        list["result"]["grants"],
        json!([{"path":work.join("notes.md"),"kind":"file","mode":"write"}])
    );
    let lookup = client
        .rpc(
            "session.grants.lookup",
            json!({"sessionId":sid,"path":"notes.md","mode":"read"}),
        )
        .await;
    assert_ok(&lookup);
    assert_eq!(
        lookup["result"]["grant"],
        json!({"path":work.join("notes.md"),"kind":"file","mode":"write"})
    );
    let lookup = client
        .rpc(
            "session.grants.lookup",
            json!({"sessionId":sid,"path":"notes.md","mode":"write"}),
        )
        .await;
    assert_ok(&lookup);
    assert_eq!(
        lookup["result"]["grant"],
        json!({"path":work.join("notes.md"),"kind":"file","mode":"write"})
    );

    // A read grant answers a read lookup, not a write lookup.
    assert_ok(
        &client
            .rpc(
                "session.grants.add",
                json!({"sessionId":sid,"path":"only-read.txt","kind":"file","mode":"read"}),
            )
            .await,
    );
    let lookup = client
        .rpc(
            "session.grants.lookup",
            json!({"sessionId":sid,"path":"only-read.txt","mode":"write"}),
        )
        .await;
    assert_ok(&lookup);
    assert_eq!(lookup["result"]["grant"], json!(null));
    let lookup = client
        .rpc(
            "session.grants.lookup",
            json!({"sessionId":sid,"path":"only-read.txt","mode":"read"}),
        )
        .await;
    assert_ok(&lookup);
    assert_eq!(
        lookup["result"]["grant"],
        json!({"path":work.join("only-read.txt"),"kind":"file","mode":"read"})
    );
    assert_ok(
        &client
            .rpc(
                "session.grants.remove",
                json!({"sessionId":sid,"path":"only-read.txt"}),
            )
            .await,
    );

    // A directory grant matches a child by component.
    assert_ok(
        &client
            .rpc(
                "session.grants.add",
                json!({"sessionId":sid,"path":"notes","kind":"dir","mode":"write"}),
            )
            .await,
    );
    let lookup = client
        .rpc(
            "session.grants.lookup",
            json!({"sessionId":sid,"path":"notes/child.txt","mode":"write"}),
        )
        .await;
    assert_ok(&lookup);
    assert_eq!(
        lookup["result"]["grant"],
        json!({"path":work.join("notes"),"kind":"dir","mode":"write"})
    );
    assert_ok(
        &client
            .rpc(
                "session.grants.remove",
                json!({"sessionId":sid,"path":"notes"}),
            )
            .await,
    );

    // A new file outside the workspace is stored by its parent and name.
    assert_ok(
        &client
            .rpc(
                "session.grants.add",
                json!({"sessionId":sid,"path":outside,"kind":"file","mode":"write"}),
            )
            .await,
    );
    // Remove takes the same path the client typed.
    assert_ok(
        &client
            .rpc(
                "session.grants.remove",
                json!({"sessionId":sid,"path":"notes.md"}),
            )
            .await,
    );
    let list = client
        .rpc("session.grants.list", json!({"sessionId":sid}))
        .await;
    assert_ok(&list);
    assert_eq!(
        list["result"]["grants"],
        json!([{"path":outside,"kind":"file","mode":"write"}])
    );

    let write = json!({
        "sessionId":sid,"branchId":"main",
        "text":format!("tool:write {}", json!({"path":outside,"content":"granted"}))
    });
    assert_ok(&client.rpc("session.submit", write).await);
    assert_ok(&client.rpc("session.idle", branch).await);
    let result = session
        .history("main", None)
        .await
        .unwrap()
        .iter()
        .rev()
        .find_map(|event| match &event.kind {
            EventKind::ToolResult(data) => Some(data.results[0].result.clone()),
            _ => None,
        })
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "granted");
    let approvals = session
        .local("main", ..)
        .await
        .unwrap()
        .iter()
        .filter(|event| matches!(event.kind, EventKind::ApprovalRequest(_)))
        .count();
    assert_eq!(approvals, 0);

    server.shutdown();
    server.join().await.unwrap();
    let _ = std::fs::remove_dir_all(&home);
}
