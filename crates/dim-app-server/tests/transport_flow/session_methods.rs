use super::*;

/// Methods contract A.1: assembly fails for a recorded Session namespace that
/// could shadow a built-in or share a name with a configured management
/// namespace; one under `session` that does neither loads.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_namespaces_overlapping_builtins_or_management_fail_assembly() {
    for (namespace, reason) in [
        ("session", "invalid Session namespace session"),
        ("session.mcp", "invalid Session namespace session.mcp"),
        (
            "session.permission",
            "invalid Session namespace session.permission",
        ),
        (
            "session.create.draft",
            "invalid Session namespace session.create.draft",
        ),
        ("agent.notes", "invalid Session namespace agent.notes"),
        ("Shell", "invalid Session namespace Shell"),
    ] {
        let error = try_launch_with(Arc::new(MockTools), ServerConfig::default(), |builder| {
            builder.session_methods(namespace);
        })
        .await
        .err()
        .unwrap_or_else(|| panic!("{namespace} loaded"));
        assert!(error.to_string().contains(reason), "{namespace}: {error}");
    }
    use std::os::unix::fs::PermissionsExt;
    let home = std::path::PathBuf::from("/tmp").join(format!("ds-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
    for namespace in ["mcp", "account.codex.device"] {
        let error = try_launch_with(
            Arc::new(MockTools),
            ServerConfig {
                product_home: Some(home.clone()),
                target_home: Some(home.join("rust")),
                ..Default::default()
            },
            |builder| {
                builder.session_methods(namespace);
            },
        )
        .await
        .err()
        .unwrap_or_else(|| panic!("{namespace} loaded"));
        assert!(
            error.to_string().contains(&format!(
                "Session namespace {namespace} overlaps management namespace"
            )),
            "{namespace}: {error}"
        );
    }
    let (server, _) = launch_with(Arc::new(MockTools), ServerConfig::default(), |builder| {
        builder.session_methods("session.echo");
    })
    .await;
    server.shutdown();
    server.join().await.unwrap();
}
