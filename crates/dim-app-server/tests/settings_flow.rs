use dim_agent_capabilities::file_storage::{FileStorage, FileStoragePlugin};
use dim_agent_capabilities::skill_settings::{SkillSettingsError, SkillSettingsHost};
use dim_agent_runtime::{Capability, KeysOf, capability_plugin, wait_active};
use dim_app_server::settings::{
    SettingsDocument, SettingsError, SettingsPlugin, SettingsService, SettingsSource,
};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};
use serde_json::json;
use std::{
    fs,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

struct Capture(Arc<OnceLock<(Arc<SettingsService>, Arc<FileStorage>)>>);
impl TypedPlugin for Capture {
    type Deps = (Arc<SettingsService>, Arc<FileStorage>);
    fn name(&self) -> &str {
        "SettingsTest"
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
        ((), ())
    }
}

#[tokio::test]
async fn late_workspace_authorization_and_settings_survive_dependency_reload() {
    let home = std::env::temp_dir().join(format!("dim-settings-late-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&home).unwrap();
    let home = home.canonicalize().unwrap();
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
    let (first, same) = tokio::join!(
        settings.authorize_project(project.clone(), &cancel),
        settings.authorize_project(project.clone(), &cancel)
    );
    let project_root = first.unwrap();
    assert_eq!(project_root, same.unwrap());
    assert_eq!(settings.project_root(&project), Some(project_root));
    let source = SettingsSource::ProjectMcp(project_root);
    let snapshot = settings.read(source, &cancel).await.unwrap();
    let saved = settings
        .update(
            source,
            &snapshot.revision,
            SettingsDocument::Mcp(
                serde_json::from_value(
                    json!({"mcpServers":{"late":{"command":"node","enabled":false}}}),
                )
                .unwrap(),
            ),
            &cancel,
        )
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
        assert!(current.read(source, &cancel).await.unwrap().document == saved.document);
    }
    root.shutdown().await.unwrap();
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn cancellation_in_the_file_worker_preserves_the_settings_contract() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap()
        .block_on(async {
            let home = std::env::temp_dir().join(format!("dim-settings-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&home).unwrap();
            let home = home.canonicalize().unwrap();
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

            // Hold the only file worker so cancellation happens after admission
            // but before the actual read, not in SettingsService's entry check.
            let (release, held) = std::sync::mpsc::channel();
            let blocker = tokio::task::spawn_blocking(move || held.recv().unwrap());
            let pending = settings.read(SettingsSource::UserMcp, &cancel);
            tokio::pin!(pending);
            assert!(futures_util::poll!(&mut pending).is_pending());
            cancel.cancel();
            release.send(()).unwrap();
            blocker.await.unwrap();
            let result = pending.await;
            root.shutdown().await.unwrap();
            fs::remove_dir_all(home).unwrap();
            assert!(matches!(result, Err(SettingsError::Cancelled)));
        });
}

#[tokio::test]
async fn settings_persist_cas_cancel_and_reload_through_plugins() {
    let home = std::env::temp_dir().join(format!("dim-settings-{}", uuid::Uuid::new_v4()));
    let project = home.join("project");
    fs::create_dir_all(&project).unwrap();
    let home = fs::canonicalize(home).unwrap();
    let project = fs::canonicalize(project).unwrap();
    let root = Ctx::root().unwrap();
    let captured = Arc::new(OnceLock::new());
    let views = vec![
        root.plugin(capability_plugin(FileStoragePlugin::new(vec![
            home.clone(),
            project.clone(),
        ]))),
        root.plugin(capability_plugin(SettingsPlugin::new(0, vec![1]))),
        root.plugin(capability_plugin(Capture(captured.clone()))),
    ];
    let cancel = CancellationToken::new();
    wait_active(&root, &views, Duration::from_secs(5), &cancel)
        .await
        .unwrap();
    let (settings, files) = captured.get().unwrap();
    let mut changed = settings.subscribe();
    let empty = settings
        .read(SettingsSource::UserMcp, &cancel)
        .await
        .unwrap();
    assert_eq!(empty.revision, "missing");
    let document = SettingsDocument::Mcp(serde_json::from_value(json!({
        "mcpServers": {
            "local": { "command": "node", "args": ["server.mjs"], "enabled": false },
            "remote": { "type": "http", "url": "https://example.test/mcp", "headers": {"Authorization": "private"} },
            "events": { "type": "sse", "url": "https://example.test/sse" }
        }, "future": {"keep": null}
    })).unwrap());
    // Two consumers using the same version cannot overwrite one another.
    let (a, b) = tokio::join!(
        settings.update(
            SettingsSource::UserMcp,
            &empty.revision,
            document.clone(),
            &cancel
        ),
        settings.update(
            SettingsSource::UserMcp,
            &empty.revision,
            document.clone(),
            &cancel
        )
    );
    assert!(matches!(
        (&a, &b),
        (Ok(_), Err(SettingsError::Conflict)) | (Err(SettingsError::Conflict), Ok(_))
    ));
    assert_eq!(*changed.borrow_and_update(), 1);
    let current = settings
        .read(SettingsSource::UserMcp, &cancel)
        .await
        .unwrap();
    assert!(current.document == document);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(home.join("mcp.json")).unwrap())
            .unwrap()["future"],
        json!({"keep":null})
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(home.join("mcp.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    // External changes invalidate the disk revision, not just an in-memory counter.
    fs::write(
        home.join("mcp.json"),
        br#"{"mcpServers":{},"external":true}"#,
    )
    .unwrap();
    assert!(matches!(
        settings
            .update(
                SettingsSource::UserMcp,
                &current.revision,
                document.clone(),
                &cancel
            )
            .await,
        Err(SettingsError::Conflict)
    ));
    assert!(!changed.has_changed().unwrap());
    assert!(matches!(
        settings.read(SettingsSource::ProjectMcp(0), &cancel).await,
        Err(SettingsError::Invalid(_))
    ));
    let project_snapshot = settings
        .read(SettingsSource::ProjectMcp(1), &cancel)
        .await
        .unwrap();
    settings
        .update(
            SettingsSource::ProjectMcp(1),
            &project_snapshot.revision,
            document,
            &cancel,
        )
        .await
        .unwrap();
    assert!(project.join(".mcp.json").exists());

    let skill = settings
        .read(SettingsSource::Skills, &cancel)
        .await
        .unwrap();
    let document = SettingsDocument::Skills(
        serde_json::from_value(json!({"disabledSkillIds":["project:a","user:b"]})).unwrap(),
    );
    // Cancellation while waiting on another process's lease must not save anything.
    let lock = files
        .lock(&files.path(0, "skills.json").unwrap(), &cancel)
        .await
        .unwrap();
    let stopped = CancellationToken::new();
    let update = settings.update(
        SettingsSource::Skills,
        &skill.revision,
        document.clone(),
        &stopped,
    );
    tokio::pin!(update);
    tokio::select! {
        result = &mut update => panic!("unexpected early completion: {}", result.is_ok()),
        _ = tokio::time::sleep(Duration::from_millis(50)) => {},
    }
    stopped.cancel();
    assert!(matches!(update.await, Err(SettingsError::Cancelled)));
    assert!(!home.join("skills.json").exists());
    drop(lock);
    // Dropping a waiter does not drop the admitted update or its notification.
    changed.borrow_and_update();
    let lock = files
        .lock(&files.path(0, "skills.json").unwrap(), &cancel)
        .await
        .unwrap();
    {
        let update = settings.update(
            SettingsSource::Skills,
            &skill.revision,
            document.clone(),
            &cancel,
        );
        tokio::pin!(update);
        tokio::select! {
            _ = &mut update => panic!("update must wait for lease"),
            _ = tokio::time::sleep(Duration::from_millis(50)) => {},
        }
    }
    drop(lock);
    tokio::time::timeout(Duration::from_secs(3), changed.changed())
        .await
        .unwrap()
        .unwrap();
    assert!(
        settings
            .read(SettingsSource::Skills, &cancel)
            .await
            .unwrap()
            .document
            == document
    );
    fs::write(home.join("skills.json"), b"broken").unwrap();
    assert!(matches!(
        settings.read(SettingsSource::Skills, &cancel).await,
        Err(SettingsError::Invalid(_))
    ));
    root.shutdown().await.unwrap();
    assert!(matches!(
        settings.read(SettingsSource::UserMcp, &cancel).await,
        Err(SettingsError::Closed)
    ));
    fs::remove_dir_all(home).unwrap();
}

struct SkillConsumer(Arc<OnceLock<Arc<SkillSettingsHost>>>);
impl TypedPlugin for SkillConsumer {
    type Deps = Arc<SkillSettingsHost>;
    fn name(&self) -> &str {
        "SkillSettingsTest"
    }
    fn apply<'a>(
        &'a self,
        _: &'a Ctx,
        host: Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            assert!(self.0.set(host).is_ok());
            Ok(Effect::Done)
        })
    }
}
impl Capability for SkillConsumer {
    fn keys(&self) -> KeysOf<Self> {}
}

#[tokio::test]
async fn skill_settings_port_preserves_data_and_owns_cas_notifications_and_shutdown() {
    let home = std::env::temp_dir().join(format!("dim-skill-settings-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&home).unwrap();
    let home = fs::canonicalize(home).unwrap();
    let path = home.join("skills.json");
    fs::write(
        &path,
        br#"{"disabledSkillIds":["user:a","user:absent"],"future":{"keep":7}}"#,
    )
    .unwrap();
    let root = Ctx::root().unwrap();
    let captured = Arc::new(OnceLock::new());
    let views = vec![
        root.plugin(capability_plugin(FileStoragePlugin::new(vec![
            home.clone(),
        ]))),
        root.plugin(capability_plugin(SettingsPlugin::new(0, vec![]))),
        root.plugin(capability_plugin(SkillConsumer(captured.clone()))),
    ];
    let cancel = CancellationToken::new();
    wait_active(&root, &views, Duration::from_secs(5), &cancel)
        .await
        .unwrap();
    let port = &captured.get().unwrap().0;
    let settings = root.get::<SettingsService>().unwrap();
    let files = root.get::<FileStorage>().unwrap();
    let mut changed = port.changes();
    let initial = port.skills(cancel.clone()).await.unwrap();
    assert_eq!(*changed.borrow_and_update(), "unread");
    assert_eq!(
        initial.disabled,
        ["user:a".into(), "user:absent".into()].into()
    );

    let mcp = settings
        .read(SettingsSource::UserMcp, &cancel)
        .await
        .unwrap();
    settings
        .update(
            SettingsSource::UserMcp,
            &mcp.revision,
            mcp.document,
            &cancel,
        )
        .await
        .unwrap();
    assert!(!changed.has_changed().unwrap());

    // No-op is not a rewrite or notification, but stale no-ops still conflict.
    let original = fs::read(&path).unwrap();
    assert_eq!(
        port.clone()
            .update_skills(
                initial.version.clone(),
                "user:a".into(),
                false,
                cancel.clone()
            )
            .await
            .unwrap(),
        initial.version
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(!changed.has_changed().unwrap());
    let enabled = port
        .clone()
        .update_skills(
            initial.version.clone(),
            "user:a".into(),
            true,
            cancel.clone(),
        )
        .await
        .unwrap();
    assert_ne!(enabled, initial.version);
    assert_eq!(*changed.borrow_and_update(), enabled);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(&path).unwrap()).unwrap(),
        json!({"disabledSkillIds":["user:absent"],"future":{"keep":7}})
    );
    assert!(matches!(
        port.clone()
            .update_skills(initial.version, "user:a".into(), true, cancel.clone())
            .await,
        Err(SkillSettingsError::Conflict)
    ));

    // Two different edits based on one revision must not silently lose an ID.
    let (a, b) = tokio::join!(
        port.clone()
            .update_skills(enabled.clone(), "user:b".into(), false, cancel.clone()),
        port.clone()
            .update_skills(enabled, "user:c".into(), false, cancel.clone())
    );
    let (version, winner, loser) = match (a, b) {
        (Ok(version), Err(SkillSettingsError::Conflict)) => (version, "user:b", "user:c"),
        (Err(SkillSettingsError::Conflict), Ok(version)) => (version, "user:c", "user:b"),
        result => panic!("unexpected concurrent result: {result:?}"),
    };
    let current = port.skills(cancel.clone()).await.unwrap();
    assert_eq!(current.version, version);
    assert_eq!(*changed.borrow_and_update(), version);
    assert_eq!(
        current.disabled,
        ["user:absent".into(), winner.into()].into()
    );

    // The general Settings writer and the narrow port share the same revision.
    let mut snapshot = settings
        .read(SettingsSource::Skills, &cancel)
        .await
        .unwrap();
    let SettingsDocument::Skills(document) = &mut snapshot.document else {
        panic!("expected Skills")
    };
    document.disabled_skill_ids.push(loser.into());
    let saved = settings
        .update(SettingsSource::Skills, &version, snapshot.document, &cancel)
        .await
        .unwrap();
    assert_eq!(*changed.borrow_and_update(), saved.revision);
    assert!(matches!(
        port.clone()
            .update_skills(version, winner.into(), true, cancel.clone())
            .await,
        Err(SkillSettingsError::Conflict)
    ));

    // Cancellation while waiting for the file lease cannot commit or notify.
    let lock = files
        .lock(&files.path(0, "skills.json").unwrap(), &cancel)
        .await
        .unwrap();
    let stopped = CancellationToken::new();
    let update =
        port.clone()
            .update_skills(saved.revision.clone(), winner.into(), true, stopped.clone());
    tokio::pin!(update);
    tokio::select! {
        result = &mut update => panic!("unexpected early result: {result:?}"),
        _ = tokio::time::sleep(Duration::from_millis(50)) => {},
    }
    stopped.cancel();
    assert!(matches!(update.await, Err(SkillSettingsError::Cancelled)));
    drop(lock);
    assert_eq!(
        port.skills(cancel.clone()).await.unwrap().version,
        saved.revision
    );
    assert!(!changed.has_changed().unwrap());
    assert!(matches!(
        port.clone()
            .update_skills(saved.revision.clone(), "  ".into(), false, cancel.clone())
            .await,
        Err(SkillSettingsError::Invalid(_))
    ));

    // Disposal drains an admitted edit even when its original waiter is gone.
    let lock = files
        .lock(&files.path(0, "skills.json").unwrap(), &cancel)
        .await
        .unwrap();
    {
        let update =
            port.clone()
                .update_skills(saved.revision.clone(), winner.into(), true, cancel.clone());
        tokio::pin!(update);
        tokio::select! {
            result = &mut update => panic!("unexpected early result: {result:?}"),
            _ = tokio::time::sleep(Duration::from_millis(50)) => {},
        }
    }
    views[1].dispose().await.unwrap();
    drop(lock);
    assert!(!changed.has_changed().unwrap());
    assert!(matches!(
        port.skills(cancel.clone()).await,
        Err(SkillSettingsError::Closed)
    ));
    assert!(matches!(
        port.clone()
            .update_skills(saved.revision.clone(), winner.into(), true, cancel.clone())
            .await,
        Err(SkillSettingsError::Closed)
    ));

    let new_capture = Arc::new(OnceLock::new());
    views[2].dispose().await.unwrap();
    let reloaded = vec![
        root.plugin(capability_plugin(SettingsPlugin::new(0, vec![]))),
        root.plugin(capability_plugin(SkillConsumer(new_capture.clone()))),
    ];
    wait_active(&root, &reloaded, Duration::from_secs(5), &cancel)
        .await
        .unwrap();
    let port = &new_capture.get().unwrap().0;
    assert_eq!(*port.changes().borrow(), "unread");
    assert_eq!(
        port.skills(cancel.clone()).await.unwrap().disabled,
        ["user:absent".into(), "user:b".into(), "user:c".into()].into()
    );
    fs::write(&path, b"broken").unwrap();
    assert!(matches!(
        port.skills(cancel.clone()).await,
        Err(SkillSettingsError::Invalid(_))
    ));
    assert!(matches!(
        port.clone()
            .update_skills(saved.revision, "user:b".into(), true, cancel)
            .await,
        Err(SkillSettingsError::Invalid(_))
    ));
    assert_eq!(fs::read(&path).unwrap(), b"broken");
    root.shutdown().await.unwrap();
    fs::remove_dir_all(home).unwrap();
}
