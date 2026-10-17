use super::*;
use dim_app_server::settings::SourceDeclaration;

fn declaration(source: &str) -> SourceDeclaration {
    SourceDeclaration {
        owner: "example-package".into(),
        source: source.into(),
        user_file: format!("{source}.json"),
        project_file: None,
        schema: json!({
            "type": "object",
            "properties": {
                "searchService": {
                    "type": "object",
                    "properties": {"kind": {"type": "string"}, "secretRef": {"type": "string"}}
                },
                "maxResults": {"type": "integer", "minimum": 1, "default": 5}
            },
            "additionalProperties": false
        }),
        user_only: vec!["/searchService".into()],
    }
}
fn product_home() -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let home = std::path::PathBuf::from("/tmp").join(format!("ds-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
    home
}

/// The client path (settings-service §4.3): local connections read and write
/// declared sources, including user-only fields; Rust sources and paired
/// devices are not served.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clients_edit_declared_sources_through_settings_methods() {
    let home = product_home();
    let (server, _) = launch_with_agent(
        Arc::new(MockTools),
        ServerConfig {
            product_home: Some(home.clone()),
            target_home: Some(home.join("rust")),
            settings_sources: vec![declaration("example")],
            ..Default::default()
        },
    )
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    for method in ["settings.read", "settings.update"] {
        assert!(client.features.as_array().unwrap().contains(&json!(method)));
    }
    let mut remote = Client::new_version(&server, true, 3).await;
    assert_eq!(
        remote
            .rpc("settings.read", json!({"source":"example","scope":"user"}))
            .await["error"]["data"]["appError"],
        "local_transport_required"
    );

    let read = client
        .rpc("settings.read", json!({"source":"example","scope":"user"}))
        .await;
    assert_ok(&read);
    assert_eq!(read["result"]["document"], json!({"maxResults": 5}));
    let first = read["result"]["revision"].clone();
    let service = json!({"kind":"web","secretRef":"web-search"});
    let written = client
        .rpc(
            "settings.update",
            json!({"source":"example","scope":"user","expectedRevision":first,"document":{"searchService":service}}),
        )
        .await;
    assert_ok(&written);
    assert_eq!(
        written["result"]["document"],
        json!({"maxResults": 5, "searchService": service})
    );
    let second = written["result"]["revision"].clone();
    assert_ne!(second, first);
    for (params, error) in [
        (
            json!({"source":"example","scope":"user","expectedRevision":first,"document":{}}),
            "settings_conflict",
        ),
        (
            json!({"source":"example","scope":"user","expectedRevision":second,"document":{"maxResults":0}}),
            "invalid_params",
        ),
        (
            json!({"source":"example","scope":"user","expectedRevision":second,"document":{"searchService":{"secretRef":""}}}),
            "invalid_params",
        ),
        (
            json!({"source":"example","scope":{"project":0},"expectedRevision":second,"document":{}}),
            "invalid_params",
        ),
        (
            json!({"source":"example","scope":"workspace","expectedRevision":second,"document":{}}),
            "invalid_params",
        ),
        (
            json!({"source":"mcp","scope":"user","expectedRevision":second,"document":{}}),
            "settings_unknown_source",
        ),
    ] {
        assert_eq!(
            client.rpc("settings.update", params).await["error"]["data"]["appError"],
            error
        );
    }
    // Rust sources keep their owners' methods.
    assert_eq!(
        client
            .rpc("settings.read", json!({"source":"mcp","scope":"user"}))
            .await["error"]["data"]["appError"],
        "settings_unknown_source"
    );
    let read = client
        .rpc("settings.read", json!({"source":"example","scope":"user"}))
        .await;
    assert_eq!(read["result"]["revision"], second);
    server.shutdown();
    server.join().await.unwrap();

    // A declaration naming a Rust source fails assembly.
    let failed = try_launch_with(
        Arc::new(MockTools),
        ServerConfig {
            product_home: Some(home.clone()),
            target_home: Some(home.join("rust")),
            settings_sources: vec![declaration("mcp")],
            ..Default::default()
        },
        |_| {},
    )
    .await;
    assert!(failed.is_err());
    let _ = std::fs::remove_dir_all(&home);
}
