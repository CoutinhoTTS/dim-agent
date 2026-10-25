use dim_agent_capabilities::file_storage::{FileStorage, FileStoragePlugin};
use dim_agent_runtime::wait_active;
use dim_app_server::{
    mcp::{
        McpChange, McpError, McpPlugin, McpService,
        config::{McpServerSettings, McpSettings},
    },
    settings::{
        SettingsChanged, SettingsError, SettingsPlugin, SettingsScope, SettingsService,
        settings_changes,
    },
};
use rutis::{BoxFuture, CordisError, Ctx, Effect, FiberView, Gate, Keyed, Typed, TypedPlugin};
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
    type Deps = (
        Keyed<McpService>,
        Keyed<SettingsService>,
        Keyed<FileStorage>,
    );
    fn name(&self) -> &str {
        "McpTest"
    }
    fn apply<'a>(
        &'a self,
        _: &'a Ctx,
        (Keyed(mcp), Keyed(settings), Keyed(files)): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            assert!(self.0.set((mcp, settings, files)).is_ok());
            Ok(Effect::Done)
        })
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
        // The management table the process providers register with.
        root.plugin(
            dim_app_server::methods::ManagementMethodsPlugin::typed(&dim_app_server::catalog())
                .unwrap(),
        );
        let (publication, loader) = mcp_host::Host::start(&root).await;
        let capture = Arc::new(OnceLock::new());
        let plugin = root.plugin(Typed::new(McpPlugin::new(0)));
        let mut roots = vec![home.clone(), home.join("project")];
        if reversed {
            roots.reverse();
        }
        let views = vec![
            root.plugin(rutis::Typed::new(FileStoragePlugin::new(roots))),
            root.plugin(
                SettingsPlugin::new(usize::from(reversed), vec![usize::from(!reversed)])
                    .typed(&dim_app_server::catalog())
                    .unwrap(),
            ),
            plugin.clone(),
            root.plugin(Typed::new(Capture(capture.clone()))),
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

    // Saving a project source never enables it, even with a valid-looking
    // transport; enabling records only the user's choice, and an invalid
    // transport still cannot run.
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
    assert!(matches!(
        fixture.mcp.server(Some(1), "shared".into(), &cancel).await,
        Err(McpError::Disabled)
    ));
    let application = fixture.mcp.application(SettingsScope::Project(1));
    assert_eq!(application.applied_revision, Some(revision.clone()));
    fixture
        .mcp
        .update(
            SettingsScope::Project(1),
            revision.clone(),
            "shared".into(),
            McpChange::SetEnabled(true),
            &cancel,
        )
        .await
        .unwrap();
    assert!(matches!(
        fixture.mcp.server(Some(1), "shared".into(), &cancel).await,
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
    // enablement still belongs to the canonical workspace.
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
            .catalog(Some(0), &cancel)
            .await
            .unwrap()
            .servers["shared"]
            .enabled()
    );
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
async fn project_entries_start_disabled_and_only_the_users_record_enables_them() {
    use dim_app_server::mcp::server::McpServerState;
    let fixture = Fixture::new().await;
    let cancel = CancellationToken::new();
    let config = json!({"command":"project-server", "args":["--first"], "env":{"SECRET":"do-not-export"}, "enabled":true});
    let project_file = fixture.home.join("project/.mcp.json");
    let write = |value: &Value| {
        fs::write(
            &project_file,
            json!({"mcpServers":{"shared":value}}).to_string(),
        )
        .unwrap()
    };
    write(&config);
    // The file's own `enabled` cannot enable it, and a grant from the removed
    // approval store is ignored rather than migrated.
    let grants = r#"{"version":1,"approved":{}}"#;
    fs::write(fixture.home.join("mcp-approvals.json"), grants).unwrap();
    let catalog = fixture.mcp.catalog(Some(1), &cancel).await.unwrap();
    assert!(!catalog.servers["shared"].enabled());
    assert!(matches!(
        fixture.mcp.server(Some(1), "shared".into(), &cancel).await,
        Err(McpError::Disabled)
    ));

    let written = fs::read(&project_file).unwrap();
    assert!(matches!(
        fixture
            .mcp
            .update(
                SettingsScope::Project(1),
                catalog.user_revision.clone(),
                "shared".into(),
                McpChange::SetEnabled(true),
                &cancel,
            )
            .await,
        Err(McpError::Settings(SettingsError::Conflict))
    ));
    assert!(matches!(
        fixture
            .mcp
            .update(
                SettingsScope::Project(1),
                catalog.project_revision.clone().unwrap(),
                "missing".into(),
                McpChange::SetEnabled(true),
                &cancel,
            )
            .await,
        Err(McpError::UnknownServer)
    ));
    set_project(&fixture.mcp, 1, true, &cancel).await.unwrap();
    assert!(fixture.mcp.catalog(Some(1), &cancel).await.unwrap().servers["shared"].enabled());
    let first = fixture
        .mcp
        .server(Some(1), "shared".into(), &cancel)
        .await
        .unwrap();
    // Shared project files never carry the choice; the user's record has only
    // the workspace and id, no configuration.
    assert_eq!(fs::read(&project_file).unwrap(), written);
    let record = fs::read_to_string(fixture.home.join("mcp-projects.json")).unwrap();
    let workspace = fixture.home.join("project");
    assert_eq!(
        serde_json::from_str::<Value>(&record).unwrap(),
        json!({"enabled":{(workspace.to_str().unwrap()):["shared"]}})
    );
    assert_eq!(
        fs::read_to_string(fixture.home.join("mcp-approvals.json")).unwrap(),
        grants
    );

    // Enablement is keyed by workspace: the same id elsewhere stays disabled.
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
        Err(McpError::Disabled)
    ));

    // ADR 003 D4: a changed project command runs without a new decision, and
    // a retained connection keeps its generation.
    let mut changed = config.clone();
    changed["args"] = json!(["--second"]);
    write(&changed);
    assert!(fixture.mcp.catalog(Some(1), &cancel).await.unwrap().servers["shared"].enabled());
    let second = fixture
        .mcp
        .server(Some(1), "shared".into(), &cancel)
        .await
        .unwrap();
    assert_eq!(first.state(), McpServerState::Disconnected);
    drop((first, second));

    // An unreadable record disables every project entry; user entries, even
    // looked up from the workspace, are unaffected.
    let record_path = fixture.home.join("mcp-projects.json");
    fs::write(&record_path, b"{invalid").unwrap();
    let degraded = fixture.mcp.catalog(Some(1), &cancel).await.unwrap();
    assert!(!degraded.servers["shared"].enabled());
    assert!(matches!(
        fixture.mcp.server(Some(1), "shared".into(), &cancel).await,
        Err(McpError::Disabled)
    ));
    fixture
        .mcp
        .server(Some(1), "user-only".into(), &cancel)
        .await
        .unwrap();
    assert!(
        fixture
            .mcp
            .application(SettingsScope::Project(1))
            .error
            .is_some()
    );
    assert!(set_project(&fixture.mcp, 1, false, &cancel).await.is_err());
    fs::write(&record_path, &record).unwrap();

    fixture.publication.close().await;
    fixture.root.shutdown().await.unwrap();
    let restarted = Fixture::open(fixture.home.clone(), true).await;
    let mcp = restarted.mcp.clone();
    let set = |enabled: bool| {
        let mcp = mcp.clone();
        let cancel = cancel.clone();
        async move { set_project(&mcp, 0, enabled, &cancel).await }
    };
    // Disabling stops the running server in every Session before it returns,
    // even while bound.
    let connection = mcp.server(Some(0), "shared".into(), &cancel).await.unwrap();
    set(false).await.unwrap();
    assert_eq!(connection.state(), McpServerState::Closed);
    assert!(matches!(
        mcp.server(Some(0), "shared".into(), &cancel).await,
        Err(McpError::Disabled)
    ));
    assert_eq!(
        fs::read_to_string(restarted.home.join("mcp-projects.json")).unwrap(),
        "{\n  \"enabled\": {}\n}\n"
    );

    // Removing an entry withdraws its enablement: re-adding the id, by an edit
    // or a checkout, starts it disabled again.
    set(true).await.unwrap();
    let revision = mcp
        .catalog(Some(0), &cancel)
        .await
        .unwrap()
        .project_revision
        .unwrap();
    let revision = mcp
        .update(
            SettingsScope::Project(0),
            revision,
            "shared".into(),
            McpChange::Remove,
            &cancel,
        )
        .await
        .unwrap();
    mcp.update(
        SettingsScope::Project(0),
        revision,
        "shared".into(),
        McpChange::Put(serde_json::from_value(config.clone()).unwrap()),
        &cancel,
    )
    .await
    .unwrap();
    assert!(!mcp.catalog(Some(0), &cancel).await.unwrap().servers["shared"].enabled());

    // Disabling never depends on the project file being readable: a retained
    // connection closes while the publisher still cannot read the file.
    set(true).await.unwrap();
    let retained = mcp.server(Some(0), "shared".into(), &cancel).await.unwrap();
    let written = fs::read(&project_file).unwrap();
    let broken = || async {
        fs::write(&project_file, b"{broken").unwrap();
        assert!(mcp.catalog(Some(0), &cancel).await.is_err());
        tokio::time::timeout(Duration::from_secs(5), async {
            while mcp.application(SettingsScope::Project(0)).error.is_none() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the publisher reads the broken file");
    };
    broken().await;
    assert_eq!(retained.state(), McpServerState::Disconnected);
    mcp.update(
        SettingsScope::Project(0),
        "stale".into(),
        "shared".into(),
        McpChange::SetEnabled(false),
        &cancel,
    )
    .await
    .unwrap();
    assert_eq!(retained.state(), McpServerState::Closed);
    assert_eq!(fs::read(&project_file).unwrap(), b"{broken");
    fs::write(&project_file, &written).unwrap();
    assert!(!mcp.catalog(Some(0), &cancel).await.unwrap().servers["shared"].enabled());

    // With both the file and the record unreadable, it is withdrawn too.
    set(true).await.unwrap();
    let retained = mcp.server(Some(0), "shared".into(), &cancel).await.unwrap();
    broken().await;
    let record = restarted.home.join("mcp-projects.json");
    let kept = fs::read(&record).unwrap();
    fs::write(&record, b"{invalid").unwrap();
    assert!(mcp.catalog(Some(0), &cancel).await.is_err());
    tokio::time::timeout(Duration::from_secs(5), async {
        while retained.state() != McpServerState::Closed {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("an unreadable record withdraws an unreadable file's entries");
    fs::write(&record, kept).unwrap();
    fs::write(&project_file, &written).unwrap();
    assert!(mcp.catalog(Some(0), &cancel).await.unwrap().servers["shared"].enabled());
    drop((connection, retained));
    restarted.close().await;
}

async fn set_project(
    mcp: &McpService,
    root: usize,
    enabled: bool,
    cancel: &CancellationToken,
) -> dim_app_server::mcp::Result<String> {
    let revision = mcp
        .catalog(Some(root), cancel)
        .await?
        .project_revision
        .unwrap();
    mcp.update(
        SettingsScope::Project(root),
        revision,
        "shared".into(),
        McpChange::SetEnabled(enabled),
        cancel,
    )
    .await
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
    let original_project = fs::read(fixture.home.join("project/.mcp.json")).unwrap();
    let project_revision = project.project_revision.unwrap();
    mcp.update(
        SettingsScope::Project(1),
        project_revision.clone(),
        "shared".into(),
        McpChange::SetEnabled(true),
        &cancel,
    )
    .await
    .unwrap();
    // Enabling a project entry edits neither the project nor the user MCP file.
    let enabled = mcp.catalog(Some(1), &cancel).await.unwrap();
    assert!(enabled.servers["shared"].enabled());
    assert_eq!(enabled.project_revision, Some(project_revision.clone()));
    assert_eq!(
        fs::read(fixture.home.join("project/.mcp.json")).unwrap(),
        original_project
    );
    assert_eq!(
        fs::read(fixture.home.join("mcp.json")).unwrap(),
        original_user
    );

    // Removing only the project override reveals, rather than deletes, user config.
    mcp.update(
        SettingsScope::Project(1),
        project_revision,
        "shared".into(),
        McpChange::Remove,
        &cancel,
    )
    .await
    .unwrap();
    let persisted: Value =
        serde_json::from_slice(&fs::read(fixture.home.join("project/.mcp.json")).unwrap()).unwrap();
    assert_eq!(persisted["projectExtension"], "keep");
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

    let fixture = Fixture::new().await;
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let consumer = fixture.root.plugin(Typed::new(SlowConsumer {
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

/// Wakes the test once Settings announces a write.
struct Ping(Arc<tokio::sync::Notify>);
impl rutis::Listener<SettingsChanged> for Ping {
    fn call<'a>(
        &'a self,
        _: &'a Ctx,
        _: &'a SettingsChanged,
        _: rutis::Call<'a, ()>,
    ) -> BoxFuture<'a, Result<Option<()>, CordisError>> {
        self.0.notify_one();
        Box::pin(async { Ok(None) })
    }
}

#[tokio::test]
async fn admitted_writes_survive_dropped_waiters_and_stop_with_the_plugin() {
    let fixture = Fixture::new().await;
    let cancel = CancellationToken::new();
    let path = fixture.files.path(0, "mcp.json").unwrap();
    let snapshot = fixture.mcp.catalog(None, &cancel).await.unwrap();
    let changed = Arc::new(tokio::sync::Notify::new());
    fixture
        .root
        .events()
        .on(
            &fixture.root,
            &settings_changes("mcp"),
            Ping(changed.clone()),
        )
        .unwrap();
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
    tokio::time::timeout(Duration::from_secs(5), changed.notified())
        .await
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
