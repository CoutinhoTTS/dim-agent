use dim_agent_capabilities::file_storage::{FileStorage, FileStoragePlugin};
use dim_agent_runtime::{Capability, KeysOf, capability_plugin, wait_active};
use dim_app_server::{
    mcp::{McpChange, McpError, McpPlugin, McpService},
    settings::{McpServerSettings, SettingsError, SettingsPlugin, SettingsService, SettingsSource},
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
    plugin: FiberView,
    mcp: Arc<McpService>,
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
        let root = Ctx::root().unwrap();
        let capture = Arc::new(OnceLock::new());
        let plugin = root.plugin(dim_agent_runtime::capability_plugin(McpPlugin));
        let views = vec![
            root.plugin(capability_plugin(FileStoragePlugin::new(vec![
                home.clone(),
                home.join("project"),
            ]))),
            root.plugin(capability_plugin(SettingsPlugin::new(0, vec![1]))),
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
        Self {
            root,
            plugin,
            mcp,
            settings,
            files,
            home,
        }
    }
    async fn close(&self) {
        self.root.shutdown().await.unwrap();
        fs::remove_dir_all(&self.home).unwrap();
    }
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
    assert_eq!(
        project.servers["shared"].source,
        SettingsSource::ProjectMcp(1)
    );
    assert!(!project.servers["shared"].enabled());
    assert_eq!(project.servers["user-only"].source, SettingsSource::UserMcp);
    assert!(matches!(
        project.servers["shared"].configuration,
        McpServerSettings::Remote { .. }
    ));

    let original_user = fs::read(fixture.home.join("mcp.json")).unwrap();
    let enabled_revision = mcp
        .update(
            SettingsSource::ProjectMcp(1),
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
        SettingsSource::ProjectMcp(1),
        enabled_revision,
        "shared".into(),
        McpChange::Remove,
        &cancel,
    )
    .await
    .unwrap();
    let revealed = mcp.catalog(Some(1), &cancel).await.unwrap();
    assert_eq!(revealed.servers["shared"].source, SettingsSource::UserMcp);
    assert!(revealed.servers["shared"].enabled());

    // Different edits using one source revision must not both succeed.
    let new_server =
        serde_json::from_value(json!({"command":"other", "args":["--read-only"]})).unwrap();
    let (a, b) = tokio::join!(
        mcp.update(
            SettingsSource::UserMcp,
            user.user_revision.clone(),
            "shared".into(),
            McpChange::SetEnabled(false),
            &cancel
        ),
        mcp.update(
            SettingsSource::UserMcp,
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
            SettingsSource::UserMcp,
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
            SettingsSource::UserMcp,
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
            SettingsSource::Skills,
            "missing".into(),
            "x".into(),
            McpChange::Remove,
            &cancel
        )
        .await,
        Err(McpError::InvalidSource)
    ));
    assert!(matches!(
        mcp.update(
            SettingsSource::UserMcp,
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
            SettingsSource::UserMcp,
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
                SettingsSource::UserMcp,
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
        SettingsSource::UserMcp,
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
        SettingsSource::UserMcp,
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
                SettingsSource::UserMcp,
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
        .read(SettingsSource::UserMcp, &cancel)
        .await
        .unwrap();
    fixture.close().await;
}
