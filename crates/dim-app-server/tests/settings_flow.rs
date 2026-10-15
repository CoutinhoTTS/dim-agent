use dim_agent_capabilities::file_storage::{FileStorage, FileStoragePlugin};
use dim_agent_runtime::{capability_plugin, wait_active};
use dim_app_server::{
    settings::{
        BusinessDocument, ProjectAuthorized, Result, SettingsChanged, SettingsError,
        SettingsPlugin, SettingsScope, SettingsService, project_authorizations, settings_changes,
    },
    storage::SqliteStoragePlugin,
};
use rutis::Ctx;
use serde::{Deserialize, Serialize};
use std::{fs, time::Duration};
use tokio_util::sync::CancellationToken;

struct ProcessSettings(
    tokio::sync::watch::Sender<
        Option<(
            std::sync::Arc<SettingsService>,
            std::sync::Arc<rutis_loader::Loader>,
        )>,
    >,
);
impl rutis::TypedPlugin for ProcessSettings {
    type Deps = (
        std::sync::Arc<SettingsService>,
        std::sync::Arc<rutis_loader::Loader>,
    );
    fn name(&self) -> &str {
        "test.process-settings"
    }
    fn apply<'a>(
        &'a self,
        _: &'a Ctx,
        services: Self::Deps,
    ) -> rutis::BoxFuture<'a, std::result::Result<rutis::Effect, rutis::CordisError>> {
        Box::pin(async move {
            self.0.send_replace(Some(services));
            Ok(rutis::Effect::Done)
        })
    }
}
impl dim_agent_runtime::Capability for ProcessSettings {
    fn keys(&self) -> dim_agent_runtime::KeysOf<Self> {
        ((), ())
    }
}

/// Records every event of one rutis channel, in delivery order.
struct Log<E>(std::sync::Arc<tokio::sync::watch::Sender<Vec<E>>>);
impl<E> Clone for Log<E> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<E: rutis::Event<Value = ()> + Clone> rutis::Listener<E> for Log<E> {
    fn call<'a>(
        &'a self,
        _: &'a Ctx,
        event: &'a E,
    ) -> rutis::BoxFuture<'a, std::result::Result<Option<()>, rutis::CordisError>> {
        self.0.send_modify(|log| log.push(event.clone()));
        Box::pin(async { Ok(None) })
    }
}
fn listen<E: rutis::Event<Value = ()> + Clone>(
    root: &Ctx,
    key: &rutis::EventKey<E>,
) -> tokio::sync::watch::Receiver<Vec<E>> {
    let log = Log(std::sync::Arc::new(tokio::sync::watch::Sender::new(
        Vec::new(),
    )));
    root.events().on(root, key, log.clone()).unwrap();
    log.0.subscribe()
}
/// Waits until the channel has delivered an event matching `seen`.
async fn saw<E: Clone>(
    changes: &mut tokio::sync::watch::Receiver<Vec<E>>,
    seen: impl Fn(&E) -> bool,
) {
    tokio::time::timeout(
        Duration::from_secs(5),
        changes.wait_for(|log| log.iter().any(&seen)),
    )
    .await
    .unwrap()
    .unwrap();
}
fn change(scope: SettingsScope, revision: &str) -> impl Fn(&SettingsChanged) -> bool + '_ {
    move |c| c.source == "example" && c.scope == scope && c.revision == revision
}

#[tokio::test]
async fn server_reload_preserves_project_root_identity_and_settings() {
    use dim_agent_capabilities::defaults;
    use dim_agent_runtime::{AgentBuilder, AgentConfig, SessionConfig};
    use dim_app_server::{ServerConfig, mcp::config::McpSettings, serve};

    tokio::time::timeout(Duration::from_secs(10), async {
        let home = home();
        let project_a = home.join("project-a");
        let project_b = home.join("project-b");
        for (project, marker) in [(&project_a, "a"), (&project_b, "b")] {
            fs::create_dir(project).unwrap();
            fs::write(
                project.join(".mcp.json"),
                serde_json::json!({"mcpServers":{},"project":marker}).to_string(),
            )
            .unwrap();
        }
        let (capture, mut services) = tokio::sync::watch::channel(None);
        let mut builder = AgentBuilder::new(AgentConfig::default());
        defaults::mock_hosts(&mut builder);
        defaults::install(&mut builder);
        builder.session_plugin(move |_| ProcessSettings(capture.clone()));
        let server = serve(
            builder,
            ServerConfig {
                product_home: Some(home.clone()),
                target_home: Some(home.join("runtime")),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let _session = server
            .agent()
            .unwrap()
            .create_session(SessionConfig::default())
            .await
            .unwrap();
        let (settings, loader) = services.borrow().clone().unwrap();
        let cancel = CancellationToken::new();
        let root_a = settings
            .authorize_project(project_a.clone(), &cancel)
            .await
            .unwrap();
        let expected = settings
            .read::<McpSettings>(SettingsScope::Project(root_a), &cancel)
            .await
            .unwrap();
        loader.restart("FileStorage").await.unwrap();
        let current = services
            .wait_for(|value| {
                value
                    .as_ref()
                    .is_some_and(|(current, _)| !std::sync::Arc::ptr_eq(current, &settings))
            })
            .await
            .unwrap()
            .as_ref()
            .unwrap()
            .0
            .clone();
        assert!(matches!(
            settings
                .read::<McpSettings>(SettingsScope::Project(root_a), &cancel)
                .await,
            Err(SettingsError::Closed)
        ));
        assert_eq!(current.project_root(&project_a), Some(root_a));
        let root_b = current
            .authorize_project(project_b.clone(), &cancel)
            .await
            .unwrap();
        assert_ne!(root_a, root_b);
        assert_eq!(
            current.authorize_project(project_a, &cancel).await.unwrap(),
            root_a
        );
        assert!(
            current
                .read::<McpSettings>(SettingsScope::Project(root_a), &cancel)
                .await
                .unwrap()
                == expected
        );
        assert!(
            current
                .read::<McpSettings>(SettingsScope::Project(root_b), &cancel)
                .await
                .unwrap()
                .document
                != expected.document
        );
        server.shutdown();
        server.join().await.unwrap();
        fs::remove_dir_all(home).unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn late_workspace_authorization_and_settings_survive_dependency_reload() {
    use dim_app_server::mcp::config::McpSettings;
    let home = home();
    let root = Ctx::root().unwrap();
    let views = vec![
        root.plugin(capability_plugin(FileStoragePlugin::new(vec![
            home.clone(),
        ]))),
        root.plugin(capability_plugin(SettingsPlugin::new(0, vec![]))),
    ];
    let cancel = CancellationToken::new();
    wait_active(&root, &views, Duration::from_secs(5), &cancel)
        .await
        .unwrap();
    let settings = root.get::<SettingsService>().unwrap();
    let project = home.join("later");
    fs::create_dir(&project).unwrap();
    assert_eq!(settings.project_root(&project), None);
    let mut authorized = listen(&root, &project_authorizations());
    let (first, same) = tokio::join!(
        settings.authorize_project(project.clone(), &cancel),
        settings.authorize_project(project.clone(), &cancel)
    );
    let project_root = first.unwrap();
    assert_eq!(project_root, same.unwrap());
    // Authorizing the same directory twice announces it once.
    saw(&mut authorized, |a: &ProjectAuthorized| a.cwd == project).await;
    assert_eq!(authorized.borrow().len(), 1);
    assert_eq!(authorized.borrow()[0].root, project_root);
    let scope = SettingsScope::Project(project_root);
    let snapshot = settings.read::<McpSettings>(scope, &cancel).await.unwrap();
    let document = serde_json::from_value::<McpSettings>(serde_json::json!({"mcpServers":{"late":{"command":"node","enabled":false}},"future":{"keep":7}})).unwrap();
    let saved = settings
        .update(scope, &snapshot.revision, document, &cancel)
        .await
        .unwrap();
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(
        settings.authorize_project(home.clone(), &cancelled).await,
        Err(SettingsError::Cancelled)
    ));
    assert_eq!(settings.project_root(&home), None);
    for plugin in [&views[1], &views[0]] {
        let old = root.get::<SettingsService>().unwrap();
        plugin.restart().await.unwrap();
        wait_active(&root, &views, Duration::from_secs(5), &cancel)
            .await
            .unwrap();
        let current = root.get::<SettingsService>().unwrap();
        assert!(matches!(
            old.authorize_project(project.clone(), &cancel).await,
            Err(SettingsError::Closed)
        ));
        assert_eq!(current.project_root(&project), Some(project_root));
        assert_eq!(
            current
                .authorize_project(project.clone(), &cancel)
                .await
                .unwrap(),
            project_root
        );
        assert!(current.read::<McpSettings>(scope, &cancel).await.unwrap() == saved);
    }
    root.shutdown().await.unwrap();
    fs::remove_dir_all(home).unwrap();
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Document {
    value: String,
}
impl BusinessDocument for Document {
    const NAMESPACE: &'static str = "example";
    const USER_FILE: &'static str = "example.json";
    const PROJECT_FILE: Option<&'static str> = Some(".example.json");
    fn validate(&self) -> Result<()> {
        if self.value == "invalid" {
            Err(SettingsError::Invalid("invalid example"))
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Other {
    value: String,
}
impl BusinessDocument for Other {
    const NAMESPACE: &'static str = "other";
    const USER_FILE: &'static str = "other.json";
    fn validate(&self) -> Result<()> {
        Ok(())
    }
}
fn home() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("dim-settings-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&path).unwrap();
    path.canonicalize().unwrap()
}

#[tokio::test]
async fn native_import_plain_v13_cas_failure_and_restart() {
    let home = home();
    let database = home.join("settings.sqlite");
    let original = br#"{"value":"original"}"#;
    fs::write(home.join("example.json"), original).unwrap();
    fs::write(home.join("other.json"), b"broken").unwrap();
    let cancel = CancellationToken::new();
    let mut saved = None;
    for restarted in [false, true] {
        let root = Ctx::root().unwrap();
        let views = vec![
            root.plugin(capability_plugin(FileStoragePlugin::new(vec![
                home.clone(),
            ]))),
            root.plugin(capability_plugin(SqliteStoragePlugin::new(
                database.clone(),
            ))),
            root.plugin(capability_plugin(SettingsPlugin::native(0, vec![]))),
        ];
        wait_active(&root, &views, Duration::from_secs(5), &cancel)
            .await
            .unwrap();
        let settings = root.get::<SettingsService>().unwrap();
        let mut changes = listen(&root, &settings_changes("example"));
        assert!(changes.borrow_and_update().is_empty());
        let current = settings
            .read::<Document>(SettingsScope::User, &cancel)
            .await
            .unwrap();
        assert!(!changes.has_changed().unwrap());
        let db = rusqlite::Connection::open(&database).unwrap();
        if restarted {
            assert_eq!(Some(current), saved);
        } else {
            assert_eq!(current.revision, "native:user/example:1");
            assert_eq!(current.document.value, "original");
            let (body, imported): (String, String) = db.query_row("SELECT document, imported_revision FROM settings_documents WHERE source='user/example'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
            assert_eq!(
                serde_json::from_str::<Document>(&body).unwrap(),
                current.document
            );
            assert_eq!(imported.len(), 64);
            assert_eq!(fs::read(home.join("example.json")).unwrap(), original);
            // A corrupt optional source does not block plugin assembly or another namespace.
            assert!(matches!(
                settings.read::<Other>(SettingsScope::User, &cancel).await,
                Err(SettingsError::Invalid(_))
            ));
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM settings_documents WHERE source='user/other'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
            fs::write(home.join("other.json"), br#"{"value":"repaired"}"#).unwrap();
            let other = settings
                .read::<Other>(SettingsScope::User, &cancel)
                .await
                .unwrap();
            settings
                .update(
                    SettingsScope::User,
                    &other.revision,
                    Other::default(),
                    &cancel,
                )
                .await
                .unwrap();
            assert!(!changes.has_changed().unwrap());
            db.execute_batch("CREATE TRIGGER fail_save BEFORE UPDATE ON settings_documents BEGIN SELECT RAISE(ABORT, 'save failed'); END").unwrap();
            let next = Document {
                value: "saved".into(),
            };
            assert!(matches!(
                settings
                    .update(
                        SettingsScope::User,
                        &current.revision,
                        next.clone(),
                        &cancel
                    )
                    .await,
                Err(SettingsError::Persistence)
            ));
            assert_eq!(
                settings
                    .read::<Document>(SettingsScope::User, &cancel)
                    .await
                    .unwrap(),
                current
            );
            assert!(!changes.has_changed().unwrap());
            db.execute_batch("DROP TRIGGER fail_save").unwrap();
            let (a, b) = tokio::join!(
                settings.update(
                    SettingsScope::User,
                    &current.revision,
                    next.clone(),
                    &cancel
                ),
                settings.update(SettingsScope::User, &current.revision, next, &cancel)
            );
            let winner = match (a, b) {
                (Ok(value), Err(SettingsError::Conflict))
                | (Err(SettingsError::Conflict), Ok(value)) => value,
                result => panic!("unexpected CAS: {result:?}"),
            };
            saw(&mut changes, change(SettingsScope::User, &winner.revision)).await;
            assert_eq!(winner.revision, "native:user/example:2");
            saved = Some(winner);
            fs::write(home.join("example.json"), b"ignored malformed legacy data").unwrap();
            // Explicitly seed the historical v13 payload-only contract and token.
            db.execute("INSERT INTO settings_documents(source,version,document,imported_revision) VALUES ('user/mcp',7,'{\"mcpServers\":{},\"extension\":\"v13\"}','missing')", []).unwrap();
            let compat = settings
                .read::<dim_app_server::mcp::config::McpSettings>(SettingsScope::User, &cancel)
                .await
                .unwrap();
            assert_eq!(compat.revision, "native:user/mcp:7");
            assert!(compat.document.mcp_servers.is_empty());
            assert_eq!(compat.document.extra["extension"], "v13");
            assert_eq!(
                db.query_row(
                    "SELECT imported_revision FROM settings_documents WHERE source='user/example'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                imported
            );
        }
        root.shutdown().await.unwrap();
        assert!(matches!(
            settings
                .read::<Document>(SettingsScope::User, &cancel)
                .await,
            Err(SettingsError::Closed)
        ));
    }
    fs::remove_dir_all(home).unwrap();
}

#[tokio::test]
async fn file_cas_project_isolation_notifications_and_accepted_work_ownership() {
    let home = home();
    let project = home.join("project");
    let second = home.join("second");
    fs::create_dir(&project).unwrap();
    fs::create_dir(&second).unwrap();
    let root = Ctx::root().unwrap();
    let views = vec![
        root.plugin(capability_plugin(FileStoragePlugin::new(vec![
            home.clone(),
            project.clone(),
            second.clone(),
        ]))),
        root.plugin(capability_plugin(SettingsPlugin::new(0, vec![1, 2]))),
    ];
    let cancel = CancellationToken::new();
    wait_active(&root, &views, Duration::from_secs(5), &cancel)
        .await
        .unwrap();
    let settings = root.get::<SettingsService>().unwrap();
    let files = root.get::<FileStorage>().unwrap();
    let mut changes = listen(&root, &settings_changes("example"));
    let empty = settings
        .read::<Document>(SettingsScope::User, &cancel)
        .await
        .unwrap();
    assert_eq!(empty.revision, "missing");
    let document = Document {
        value: "saved".into(),
    };
    let (a, b) = tokio::join!(
        settings.update(
            SettingsScope::User,
            &empty.revision,
            document.clone(),
            &cancel
        ),
        settings.update(
            SettingsScope::User,
            &empty.revision,
            document.clone(),
            &cancel
        )
    );
    let saved = match (a, b) {
        (Ok(value), Err(SettingsError::Conflict)) | (Err(SettingsError::Conflict), Ok(value)) => {
            value
        }
        result => panic!("unexpected file CAS: {result:?}"),
    };
    saw(&mut changes, change(SettingsScope::User, &saved.revision)).await;
    // One write, one event: the conflicting writer announced nothing.
    assert_eq!(changes.borrow().len(), 1);
    assert_eq!(settings.project_path(1).unwrap(), project);
    assert_eq!(settings.project_sources().len(), 2);
    assert!(matches!(
        settings
            .read::<Document>(SettingsScope::Project(0), &cancel)
            .await,
        Err(SettingsError::Invalid(_))
    ));
    let a = settings
        .read::<Document>(SettingsScope::Project(1), &cancel)
        .await
        .unwrap();
    let b = settings
        .read::<Document>(SettingsScope::Project(2), &cancel)
        .await
        .unwrap();
    let project_saved = settings
        .update(
            SettingsScope::Project(1),
            &a.revision,
            document.clone(),
            &cancel,
        )
        .await
        .unwrap();
    // Each write is announced with its own scope and revision.
    saw(
        &mut changes,
        change(SettingsScope::Project(1), &project_saved.revision),
    )
    .await;
    assert!(
        changes
            .borrow()
            .iter()
            .all(|c| c.scope != SettingsScope::Project(2))
    );
    assert_eq!(
        settings
            .read::<Document>(SettingsScope::Project(2), &cancel)
            .await
            .unwrap(),
        b
    );
    assert!(!changes.has_changed().unwrap());
    fs::write(project.join(".example.json"), b"corrupt").unwrap();
    assert!(matches!(
        settings
            .read::<Document>(SettingsScope::Project(1), &cancel)
            .await,
        Err(SettingsError::Invalid(_))
    ));
    fs::write(project.join(".example.json"), br#"{"value":"repaired"}"#).unwrap();
    assert_eq!(
        settings
            .read::<Document>(SettingsScope::Project(1), &cancel)
            .await
            .unwrap()
            .document
            .value,
        "repaired"
    );
    fs::write(home.join("example.json"), br#"{"value":"external"}"#).unwrap();
    assert!(matches!(
        settings
            .update(
                SettingsScope::User,
                &saved.revision,
                document.clone(),
                &cancel
            )
            .await,
        Err(SettingsError::Conflict)
    ));
    let current = settings
        .read::<Document>(SettingsScope::User, &cancel)
        .await
        .unwrap();
    let path = files.path(0, "example.json").unwrap();
    let lock = files.lock(&path, &cancel).await.unwrap();
    let stopped = CancellationToken::new();
    let update = settings.update(
        SettingsScope::User,
        &current.revision,
        document.clone(),
        &stopped,
    );
    tokio::pin!(update);
    tokio::select! { result = &mut update => panic!("unexpected {result:?}"), _ = tokio::time::sleep(Duration::from_millis(50)) => {} }
    stopped.cancel();
    assert!(matches!(update.await, Err(SettingsError::Cancelled)));
    assert!(!changes.has_changed().unwrap());
    // Dropping the caller does not discard admitted work or its commit notification.
    {
        let update = settings.update(
            SettingsScope::User,
            &current.revision,
            document.clone(),
            &cancel,
        );
        tokio::pin!(update);
        tokio::select! { result = &mut update => panic!("unexpected {result:?}"), _ = tokio::time::sleep(Duration::from_millis(50)) => {} }
    }
    drop(lock);
    let saved = loop {
        let saved = settings
            .read::<Document>(SettingsScope::User, &cancel)
            .await
            .unwrap();
        if saved.document == document {
            break saved;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    saw(&mut changes, change(SettingsScope::User, &saved.revision)).await;
    assert_eq!(saved.document, document);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(home.join("example.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    // Unload cancels lease waiters and drains them before releasing ownership.
    let lock = files.lock(&path, &cancel).await.unwrap();
    {
        let update = settings.update(
            SettingsScope::User,
            &saved.revision,
            Document::default(),
            &cancel,
        );
        tokio::pin!(update);
        tokio::select! { result = &mut update => panic!("unexpected {result:?}"), _ = tokio::time::sleep(Duration::from_millis(50)) => {} }
    }
    views[1].dispose().await.unwrap();
    drop(lock);
    assert!(!changes.has_changed().unwrap());
    assert!(matches!(
        settings
            .update(
                SettingsScope::User,
                &saved.revision,
                Document::default(),
                &cancel
            )
            .await,
        Err(SettingsError::Closed)
    ));
    let view = root.plugin(capability_plugin(SettingsPlugin::new(0, vec![1, 2])));
    wait_active(&root, &[view], Duration::from_secs(5), &cancel)
        .await
        .unwrap();
    let fresh = root.get::<SettingsService>().unwrap();
    assert_eq!(
        fresh
            .read::<Document>(SettingsScope::User, &cancel)
            .await
            .unwrap(),
        saved
    );
    root.shutdown().await.unwrap();
    fs::remove_dir_all(home).unwrap();
}

#[tokio::test]
async fn business_documents_reject_escaping_filenames() {
    let home = home();
    let root = Ctx::root().unwrap();
    let views = vec![
        root.plugin(capability_plugin(FileStoragePlugin::new(vec![
            home.clone(),
        ]))),
        root.plugin(capability_plugin(SettingsPlugin::new(0, vec![]))),
    ];
    let cancel = CancellationToken::new();
    wait_active(&root, &views, Duration::from_secs(5), &cancel)
        .await
        .unwrap();
    let settings = root.get::<SettingsService>().unwrap();
    #[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
    struct BadPath;
    impl BusinessDocument for BadPath {
        const NAMESPACE: &'static str = "bad";
        const USER_FILE: &'static str = "../escape.json";
        fn validate(&self) -> Result<()> {
            Ok(())
        }
    }
    assert!(matches!(
        settings.read::<BadPath>(SettingsScope::User, &cancel).await,
        Err(SettingsError::Invalid(_))
    ));
    // A namespace or file belongs to the first document type that used it.
    #[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
    struct SameNamespace;
    impl BusinessDocument for SameNamespace {
        const NAMESPACE: &'static str = "example";
        const USER_FILE: &'static str = "same-namespace.json";
        fn validate(&self) -> Result<()> {
            Ok(())
        }
    }
    #[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
    struct SameFile;
    impl BusinessDocument for SameFile {
        const NAMESPACE: &'static str = "same-file";
        const USER_FILE: &'static str = "example.json";
        fn validate(&self) -> Result<()> {
            Ok(())
        }
    }
    settings
        .read::<Document>(SettingsScope::User, &cancel)
        .await
        .unwrap();
    for result in [
        settings
            .read::<SameNamespace>(SettingsScope::User, &cancel)
            .await
            .map(drop),
        settings
            .read::<SameFile>(SettingsScope::User, &cancel)
            .await
            .map(drop),
    ] {
        assert!(matches!(result, Err(SettingsError::Invalid(_))));
    }
    settings
        .read::<Document>(SettingsScope::User, &cancel)
        .await
        .unwrap();
    root.shutdown().await.unwrap();
    fs::remove_dir_all(home).unwrap();
}
