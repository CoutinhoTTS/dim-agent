use dim_agent_capabilities::file_storage::{FileStorage, FileStoragePlugin};
use dim_agent_runtime::{Capability, KeysOf, capability_plugin, wait_active};
use dim_app_server::{
    mcp::{
        McpChange, McpError, McpPlugin, McpService,
        config::{McpServerSettings, McpSettings},
    },
    settings::{SettingsError, SettingsPlugin, SettingsScope, SettingsService},
};
use rutis::{BoxFuture, CordisError, Ctx, Effect, FiberView, Gate, TypedPlugin};
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

#[path = "fixtures/mcp_host.rs"]
mod mcp_host;

type Services = (Arc<McpService>, Arc<SettingsService>, Arc<FileStorage>);
struct Capture(Arc<OnceLock<Services>>);
impl TypedPlugin for Capture {
    type Deps = Services;
    fn name(&self) -> &str {
        "McpTest"
    }
    fn apply<'a>(
        &'a self,
        _: &'a Ctx,
        services: Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            assert!(self.0.set(services).is_ok());
            Ok(Effect::Done)
        })
    }
}
impl Capability for Capture {
    fn keys(&self) -> KeysOf<Self> {
        ((), (), ())
    }
}

struct Fixture {
    root: Ctx,
    publication: mcp_host::Host,
    loader: rutis_loader::Loader,
    plugin: FiberView,
    mcp: mcp_host::Directory,
    settings: Arc<SettingsService>,
    files: Arc<FileStorage>,
    home: PathBuf,
}
impl Fixture {
    async fn new() -> Self {
        let home = std::env::temp_dir().join(format!("dim-mcp-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(home.join("project")).unwrap();
        let home = fs::canonicalize(home).unwrap();
        fs::write(home.join("mcp.json"), serde_json::to_vec(&json!({
            "mcpServers": {
                "shared": {"command":"user-server", "env":{"TOKEN":"user-secret"}},
                "user-only": {"type":"http", "url":"https://example.test/mcp", "auth":{"type":"bearer", "token":"private"}, "future":{"keep":true}}
            },
            "extension": {"preserve":17}
        })).unwrap()).unwrap();
        fs::write(home.join("project/.mcp.json"), serde_json::to_vec(&json!({
            "mcpServers": {
                "shared": {"type":"sse", "url":"https://project.test/sse", "enabled":false, "headers":{"X-Key":"project-secret"}, "future":[1,2]}
            },
            "projectExtension": "keep"
        })).unwrap()).unwrap();
        Self::open(home, false).await
    }
    async fn open(home: PathBuf, reversed: bool) -> Self {
        let root = Ctx::root().unwrap();
        let (publication, loader) = mcp_host::Host::start(&root, usize::from(reversed)).await;
        let capture = Arc::new(OnceLock::new());
        let plugin = root.plugin(dim_agent_runtime::capability_plugin(McpPlugin));
        let mut roots = vec![home.clone(), home.join("project")];
        if reversed {
            roots.reverse();
        }
        let views = vec![
            root.plugin(capability_plugin(FileStoragePlugin::new(roots))),
            root.plugin(capability_plugin(SettingsPlugin::new(
                usize::from(reversed),
                vec![usize::from(!reversed)],
            ))),
            plugin.clone(),
            root.plugin(capability_plugin(Capture(capture.clone()))),
        ];
        wait_active(
            &root,
            &views,
            Duration::from_secs(5),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let (mcp, settings, files) = capture.get().unwrap().clone();
        let mcp = mcp_host::Directory::new(mcp, &root, loader.clone());
        Self {
            root,
            publication,
            loader,
            plugin,
            mcp,
            settings,
            files,
            home,
        }
    }
    async fn close(&self) {
        self.publication.close().await;
        self.root.shutdown().await.unwrap();
        fs::remove_dir_all(&self.home).unwrap();
    }
}

#[tokio::test]
async fn loader_publication_is_scoped_secret_free_and_restart_stable() {
    use dim_app_server::mcp::server::McpServerState;
    let fixture = Fixture::new().await;
    let cancel = CancellationToken::new();
    let user = fixture
        .mcp
        .server(None, "shared".into(), &cancel)
        .await
        .unwrap();
    assert!(matches!(
        fixture.mcp.server(Some(1), "shared".into(), &cancel).await,
        Err(McpError::Disabled)
    ));
    let catalog = fixture.mcp.catalog(Some(1), &cancel).await.unwrap();
    assert_eq!(
        fixture
            .mcp
            .application(SettingsScope::User)
            .applied_revision,
        Some(catalog.user_revision)
    );
    assert_eq!(
        fixture
            .mcp
            .application(SettingsScope::Project(1))
            .applied_revision,
        catalog.project_revision
    );
    let rows = fixture.loader.layers()[0].patches[0]
        .insert
        .clone()
        .unwrap();
    assert_eq!(
        rows.len(),
        2,
        "only the authority and immutable template are root rows"
    );
    assert_eq!(
        fixture.loader.layers().len(),
        1,
        "MCP must not replace root layers"
    );
    let public = format!("{:?}", fixture.loader.layers());
    for secret in [
        "user-secret",
        "project-secret",
        "private",
        "TOKEN",
        "X-Key",
        "user-server",
        "project.test",
    ] {
        assert!(
            !public.contains(secret),
            "generic loader state leaked {secret}"
        );
    }

    // Saving a project source is not execution approval, even if its transport
    // is invalid. The disabled loader row cannot execute that configuration.
    let revision = fixture
        .mcp
        .update(
            SettingsScope::Project(1),
            fixture
                .mcp
                .catalog(Some(1), &cancel)
                .await
                .unwrap()
                .project_revision
                .unwrap(),
            "shared".into(),
            McpChange::Put(
                serde_json::from_value(json!({"type":"http", "url":"ftp://invalid.test/mcp"}))
                    .unwrap(),
            ),
            &cancel,
        )
        .await
        .unwrap();
    assert!(
        fixture
            .mcp
            .server(Some(1), "shared".into(), &cancel)
            .await
            .is_err()
    );
    let application = fixture.mcp.application(SettingsScope::Project(1));
    assert_eq!(application.applied_revision, Some(revision.clone()));
    let entry = fixture
        .mcp
        .catalog(Some(1), &cancel)
        .await
        .unwrap()
        .servers
        .remove("shared")
        .unwrap();
    assert_eq!(entry.approval, dim_app_server::mcp::McpApproval::Required);
    assert!(matches!(
        fixture
            .mcp
            .approve(1, "shared".into(), entry.approval_token.unwrap(), &cancel)
            .await,
        Err(McpError::Server(
            dim_app_server::mcp::server::McpServerError::Configuration(_)
        ))
    ));
    assert_eq!(
        fixture
            .mcp
            .catalog(Some(1), &cancel)
            .await
            .unwrap()
            .project_revision,
        Some(revision)
    );
    assert_eq!(user.state(), McpServerState::Disconnected);

    // Restart with different FileStorage indices. Templates stay immutable;
    // execution authorization still belongs to the canonical workspace.
    fixture.publication.close().await;
    fixture.root.shutdown().await.unwrap();
    assert_eq!(user.state(), McpServerState::Closed);
    let restarted = Fixture::open(fixture.home.clone(), true).await;
    restarted
        .mcp
        .server(None, "shared".into(), &cancel)
        .await
        .unwrap();
    let new_rows = restarted.loader.layers()[0].patches[0]
        .insert
        .clone()
        .unwrap();
    assert_eq!(rows, new_rows);
    assert!(
        restarted
            .mcp
            .server(Some(0), "shared".into(), &cancel)
            .await
            .is_err()
    );
    restarted.close().await;
}

#[tokio::test]
async fn project_approval_is_persisted_scoped_and_checked_on_retained_connections() {
    use dim_app_server::mcp::{
        McpApproval,
        server::{McpServerError, McpServerState},
    };
    use sha2::{Digest, Sha256};
    let fixture = Fixture::new().await;
    let cancel = CancellationToken::new();
    let config =
        json!({"command":"approved-server", "args":["--first"], "env":{"SECRET":"do-not-export"}});
    let write = |value: &Value| {
        fs::write(
            fixture.home.join("project/.mcp.json"),
            json!({"mcpServers":{"shared":value}}).to_string(),
        )
        .unwrap()
    };
    write(&config);
    let entry = fixture
        .mcp
        .catalog(Some(1), &cancel)
        .await
        .unwrap()
        .servers
        .remove("shared")
        .unwrap();
    let token = entry.approval_token.unwrap();
    assert_eq!(entry.approval, McpApproval::Required);
    assert!(matches!(
        fixture.mcp.server(Some(1), "shared".into(), &cancel).await,
        Err(McpError::ApprovalRequired)
    ));
    // Freeze the original execution identity and typed serialization, without
    // using the new owner's hashing helpers to construct the legacy grant.
    let workspace = fixture.home.join("project");
    let key = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(&workspace, "shared")).unwrap())
    );
    let effective = format!(
        r#"{{"command":"approved-server","args":["--first"],"env":{{"SECRET":"do-not-export"}},"cwd":{}}}"#,
        serde_json::to_string(&workspace).unwrap(),
    );
    let fingerprint = format!(
        "{:x}",
        Sha256::digest(format!(r#"["dim/mcp-execution/v1","{key}",{effective}]"#))
    );
    assert_eq!(token, fingerprint);
    let legacy_grants = format!(r#"{{"version":1,"approved":{{"{key}":"{fingerprint}"}}}}"#);
    fs::write(fixture.home.join("mcp-approvals.json"), &legacy_grants).unwrap();
    assert_eq!(
        fixture.mcp.catalog(Some(1), &cancel).await.unwrap().servers["shared"].approval,
        McpApproval::Approved
    );
    let mut disabled = config.clone();
    disabled["enabled"] = json!(false);
    write(&disabled);
    let disabled = fixture.mcp.catalog(Some(1), &cancel).await.unwrap();
    assert_eq!(
        disabled.servers["shared"].approval_token.as_ref(),
        Some(&token)
    );
    assert!(!disabled.servers["shared"].executable());
    write(&config);
    fixture
        .mcp
        .approve(1, "shared".into(), token.clone(), &cancel)
        .await
        .unwrap();
    let first = fixture
        .mcp
        .server(Some(1), "shared".into(), &cancel)
        .await
        .unwrap();
    let approved = fs::read_to_string(fixture.home.join("mcp-approvals.json")).unwrap();
    assert_eq!(approved, legacy_grants);
    assert!(
        !approved.contains("SECRET")
            && !approved.contains("do-not-export")
            && !approved.contains("approved-server")
    );

    let other = fixture.home.join("other");
    fs::create_dir(&other).unwrap();
    fs::write(
        other.join(".mcp.json"),
        json!({"mcpServers":{"shared":config}}).to_string(),
    )
    .unwrap();
    let other = fixture
        .settings
        .authorize_project(other, &cancel)
        .await
        .unwrap();
    assert!(matches!(
        fixture
            .mcp
            .server(Some(other), "shared".into(), &cancel)
            .await,
        Err(McpError::ApprovalRequired)
    ));
    assert!(matches!(
        fixture
            .mcp
            .approve(other, "shared".into(), token.clone(), &cancel)
            .await,
        Err(McpError::Settings(SettingsError::Conflict))
    ));

    let mut changed = config.clone();
    changed["args"] = json!(["--second"]);
    write(&changed);
    // No catalog refresh is necessary: a retained executable handle rechecks
    // the actual project source before any connection or tool request.
    assert!(matches!(
        first.tools(&cancel).await,
        Err(McpServerError::ApprovalRequired)
    ));
    assert_eq!(first.state(), McpServerState::Closed);
    assert!(matches!(
        fixture
            .mcp
            .approve(1, "shared".into(), token.clone(), &cancel)
            .await,
        Err(McpError::Settings(SettingsError::Conflict))
    ));
    let token = fixture.mcp.catalog(Some(1), &cancel).await.unwrap().servers["shared"]
        .approval_token
        .clone()
        .unwrap();

    // A persistence error must not grant execution or overwrite the old store.
    fs::rename(
        fixture.home.join("mcp-approvals.json"),
        fixture.home.join("approval-backup"),
    )
    .unwrap();
    fs::create_dir(fixture.home.join("mcp-approvals.json")).unwrap();
    assert!(
        fixture
            .mcp
            .approve(1, "shared".into(), token.clone(), &cancel)
            .await
            .is_err()
    );
    assert!(
        fixture
            .mcp
            .server(Some(1), "shared".into(), &cancel)
            .await
            .is_err()
    );
    fs::remove_dir(fixture.home.join("mcp-approvals.json")).unwrap();
    fs::rename(
        fixture.home.join("approval-backup"),
        fixture.home.join("mcp-approvals.json"),
    )
    .unwrap();
    fixture
        .mcp
        .approve(1, "shared".into(), token.clone(), &cancel)
        .await
        .unwrap();

    fixture.publication.close().await;
    fixture.root.shutdown().await.unwrap();
    let restarted = Fixture::open(fixture.home.clone(), true).await;
    let entry = restarted
        .mcp
        .catalog(Some(0), &cancel)
        .await
        .unwrap()
        .servers
        .remove("shared")
        .unwrap();
    assert_eq!(entry.approval, McpApproval::Approved);
    assert_eq!(entry.approval_token, Some(token));
    let connection = restarted
        .mcp
        .server(Some(0), "shared".into(), &cancel)
        .await
        .unwrap();
    let grants_path = restarted.home.join("mcp-approvals.json");
    let grants = fs::read(&grants_path).unwrap();
    fs::write(&grants_path, b"{invalid").unwrap();
    assert!(matches!(
        connection.tools(&cancel).await,
        Err(McpServerError::ApprovalRequired)
    ));
    assert_eq!(connection.state(), McpServerState::Closed);
    fs::write(&grants_path, grants).unwrap();
    let connection = restarted
        .mcp
        .server(Some(0), "shared".into(), &cancel)
        .await
        .unwrap();
    restarted
        .mcp
        .revoke(0, "shared".into(), &cancel)
        .await
        .unwrap();
    assert_eq!(connection.state(), McpServerState::Closed);
    assert!(matches!(
        connection.tools(&cancel).await,
        Err(McpServerError::ApprovalRequired)
    ));
    assert!(matches!(
        connection
            .call("anything".into(), Default::default(), &cancel)
            .await,
        Err(McpServerError::ApprovalRequired)
    ));
    assert!(matches!(
        restarted
            .mcp
            .server(Some(0), "shared".into(), &cancel)
            .await,
        Err(McpError::ApprovalRequired)
    ));
    assert_eq!(
        restarted.mcp.catalog(None, &cancel).await.unwrap().servers["shared"].approval,
        McpApproval::NotRequired
    );
    restarted.close().await;
}

#[tokio::test]
async fn directory_preserves_scope_and_cas_through_plugin_management() {
    let fixture = Fixture::new().await;
    let mcp = &fixture.mcp;
    let cancel = CancellationToken::new();
    let user = mcp.catalog(None, &cancel).await.unwrap();
    assert!(user.project_revision.is_none());
    assert!(user.servers["shared"].enabled());
    let project = mcp.catalog(Some(1), &cancel).await.unwrap();
    assert_eq!(project.servers.len(), 2);
    assert_eq!(project.servers["shared"].source, SettingsScope::Project(1));
    assert!(!project.servers["shared"].enabled());
    assert_eq!(project.servers["user-only"].source, SettingsScope::User);
    assert!(matches!(
        project.servers["shared"].configuration,
        McpServerSettings::Remote { .. }
    ));

    let original_user = fs::read(fixture.home.join("mcp.json")).unwrap();
    let enabled_revision = mcp
        .update(
            SettingsScope::Project(1),
            project.project_revision.unwrap(),
            "shared".into(),
            McpChange::SetEnabled(true),
            &cancel,
        )
        .await
        .unwrap();
    let persisted: Value =
        serde_json::from_slice(&fs::read(fixture.home.join("project/.mcp.json")).unwrap()).unwrap();
    assert_eq!(
        persisted["mcpServers"]["shared"],
        json!({
            "type":"sse", "url":"https://project.test/sse", "enabled":true,
            "headers":{"X-Key":"project-secret"}, "future":[1,2]
        })
    );
    assert_eq!(persisted["projectExtension"], "keep");
    assert_eq!(
        fs::read(fixture.home.join("mcp.json")).unwrap(),
        original_user
    );

    // Removing only the project override reveals, rather than deletes, user config.
    mcp.update(
        SettingsScope::Project(1),
        enabled_revision,
        "shared".into(),
        McpChange::Remove,
        &cancel,
    )
    .await
    .unwrap();
    let revealed = mcp.catalog(Some(1), &cancel).await.unwrap();
    assert_eq!(revealed.servers["shared"].source, SettingsScope::User);
    assert!(revealed.servers["shared"].enabled());

    // Different edits using one source revision must not both succeed.
    let new_server =
        serde_json::from_value(json!({"command":"other", "args":["--read-only"]})).unwrap();
    let (a, b) = tokio::join!(
        mcp.update(
            SettingsScope::User,
            user.user_revision.clone(),
            "shared".into(),
            McpChange::SetEnabled(false),
            &cancel
        ),
        mcp.update(
            SettingsScope::User,
            user.user_revision,
            "new".into(),
            McpChange::Put(new_server),
            &cancel
        ),
    );
    assert!(matches!(
        (&a, &b),
        (Ok(_), Err(McpError::Settings(SettingsError::Conflict)))
            | (Err(McpError::Settings(SettingsError::Conflict)), Ok(_))
    ));
    let saved = mcp.catalog(None, &cancel).await.unwrap();
    assert_eq!(saved.servers.contains_key("new"), b.is_ok());
    assert_eq!(saved.servers["shared"].enabled(), a.is_err());
    let persisted: Value =
        serde_json::from_slice(&fs::read(fixture.home.join("mcp.json")).unwrap()).unwrap();
    assert_eq!(persisted["extension"], json!({"preserve":17}));
    assert_eq!(
        persisted["mcpServers"]["user-only"]["auth"]["token"],
        "private"
    );
    assert_eq!(
        persisted["mcpServers"]["user-only"]["future"],
        json!({"keep":true})
    );

    // Put replaces an existing entry, rather than merging old transport fields.
    let replacement = serde_json::from_value(json!({
        "type":"http", "url":"https://replacement.test/mcp"
    }))
    .unwrap();
    let revision = mcp
        .update(
            SettingsScope::User,
            saved.user_revision,
            "shared".into(),
            McpChange::Put(replacement),
            &cancel,
        )
        .await
        .unwrap();
    let persisted: Value =
        serde_json::from_slice(&fs::read(fixture.home.join("mcp.json")).unwrap()).unwrap();
    assert_eq!(
        persisted["mcpServers"]["shared"],
        json!({
            "type":"http", "url":"https://replacement.test/mcp"
        })
    );
    assert_eq!(persisted["extension"], json!({"preserve":17}));

    let invalid = serde_json::from_value(json!({"command":""})).unwrap();
    assert!(matches!(
        mcp.update(
            SettingsScope::User,
            revision.clone(),
            "shared".into(),
            McpChange::Put(invalid),
            &cancel
        )
        .await,
        Err(McpError::Settings(SettingsError::Invalid(_)))
    ));
    assert_eq!(
        mcp.catalog(None, &cancel).await.unwrap().user_revision,
        revision
    );

    assert!(matches!(
        mcp.catalog(Some(2), &cancel).await,
        Err(McpError::Settings(SettingsError::Invalid(
            "unauthorized project"
        )))
    ));
    assert!(matches!(
        mcp.update(
            SettingsScope::User,
            revision.clone(),
            "absent".into(),
            McpChange::SetEnabled(false),
            &cancel
        )
        .await,
        Err(McpError::UnknownServer)
    ));
    assert!(matches!(
        mcp.update(
            SettingsScope::User,
            revision,
            " ".into(),
            McpChange::Remove,
            &cancel
        )
        .await,
        Err(McpError::InvalidId)
    ));

    // External corruption is reported, not silently replaced with an empty source.
    fs::write(fixture.home.join("project/.mcp.json"), b"{").unwrap();
    assert!(matches!(
        mcp.catalog(Some(1), &cancel).await,
        Err(McpError::Settings(SettingsError::Invalid(_)))
    ));
    assert!(mcp.catalog(None, &cancel).await.is_ok());
    fixture.close().await;
}

#[tokio::test]
async fn framework_precancellation_closes_admission_before_dependent_cleanup_finishes() {
    struct SlowConsumer {
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    }
    impl TypedPlugin for SlowConsumer {
        type Deps = Gate<McpService>;
        fn name(&self) -> &str {
            "SlowMcpConsumer"
        }
        fn apply<'a>(
            &'a self,
            _: &'a Ctx,
            _: Self::Deps,
        ) -> BoxFuture<'a, Result<Effect, CordisError>> {
            Box::pin(async move {
                let entered = self.entered.clone();
                let release = self.release.clone();
                Ok(Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        entered.notify_one();
                        release.notified().await;
                        Ok(())
                    })
                })))
            })
        }
    }
    impl Capability for SlowConsumer {
        fn keys(&self) -> KeysOf<Self> {}
    }

    let fixture = Fixture::new().await;
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let consumer = fixture.root.plugin(capability_plugin(SlowConsumer {
        entered: entered.clone(),
        release: release.clone(),
    }));
    wait_active(
        &fixture.root,
        &[consumer],
        Duration::from_secs(5),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let closing = tokio::spawn(fixture.plugin.shutdown());
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    let result = fixture.mcp.catalog(None, &CancellationToken::new()).await;
    release.notify_one();
    closing.await.unwrap().unwrap();
    fixture.close().await;
    assert!(matches!(result, Err(McpError::Closed)));
}

#[tokio::test]
async fn admitted_writes_survive_dropped_waiters_and_stop_with_the_plugin() {
    let fixture = Fixture::new().await;
    let cancel = CancellationToken::new();
    let path = fixture.files.path(0, "mcp.json").unwrap();
    let snapshot = fixture.mcp.catalog(None, &cancel).await.unwrap();
    let mut changes = fixture.settings.subscribe();
    let lease = fixture.files.lock(&path, &cancel).await.unwrap();
    // Poll the operation into admission, then drop only its caller while the
    // file lease guarantees no commit has happened yet.
    assert!(
        tokio::time::timeout(
            Duration::from_millis(50),
            fixture.mcp.update(
                SettingsScope::User,
                snapshot.user_revision,
                "shared".into(),
                McpChange::SetEnabled(false),
                &cancel,
            )
        )
        .await
        .is_err()
    );
    drop(lease);
    tokio::time::timeout(Duration::from_secs(5), changes.changed())
        .await
        .unwrap()
        .unwrap();
    let snapshot = fixture.mcp.catalog(None, &cancel).await.unwrap();
    assert!(!snapshot.servers["shared"].enabled());
    let bytes = fs::read(fixture.home.join("mcp.json")).unwrap();

    // A caller cancellation before the file commit must not change the file.
    let lease = fixture.files.lock(&path, &cancel).await.unwrap();
    let cancelled = CancellationToken::new();
    let pending = fixture.mcp.update(
        SettingsScope::User,
        snapshot.user_revision.clone(),
        "shared".into(),
        McpChange::SetEnabled(true),
        &cancelled,
    );
    tokio::pin!(pending);
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut pending)
            .await
            .is_err()
    );
    cancelled.cancel();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(5), pending)
            .await
            .unwrap(),
        Err(McpError::Settings(SettingsError::Cancelled))
    ));
    drop(lease);
    assert_eq!(fs::read(fixture.home.join("mcp.json")).unwrap(), bytes);

    // Unload just Mcp while its Settings dependency remains active. Shutdown
    // drains the lock wait and an old service handle cannot start more work.
    let lease = fixture.files.lock(&path, &cancel).await.unwrap();
    let pending = fixture.mcp.update(
        SettingsScope::User,
        snapshot.user_revision,
        "shared".into(),
        McpChange::SetEnabled(true),
        &cancel,
    );
    tokio::pin!(pending);
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut pending)
            .await
            .is_err()
    );
    tokio::time::timeout(Duration::from_secs(5), fixture.plugin.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        pending.await,
        Err(McpError::Settings(SettingsError::Cancelled))
    ));
    assert!(matches!(
        fixture.mcp.catalog(None, &cancel).await,
        Err(McpError::Closed)
    ));
    assert!(matches!(
        fixture
            .mcp
            .update(
                SettingsScope::User,
                "missing".into(),
                "x".into(),
                McpChange::Remove,
                &cancel
            )
            .await,
        Err(McpError::Closed)
    ));
    drop(lease);
    assert_eq!(fs::read(fixture.home.join("mcp.json")).unwrap(), bytes);
    fixture
        .settings
        .read::<McpSettings>(SettingsScope::User, &cancel)
        .await
        .unwrap();
    fixture.close().await;
}
