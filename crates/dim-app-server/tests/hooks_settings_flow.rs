//! Hooks configuration: the `hooks` and `hooks-projects` Settings sources, the
//! project file `.dimcode/hooks.json` and the `hooks.*` management methods
//! (Hooks design A.1, A.2, A.7, appendix D item 8).
use dim_agent_capabilities::{file_storage::FileStoragePlugin, methods::Owner};
use dim_agent_runtime::Names;
use dim_agent_runtime::wait_active;
use dim_app_server::{
    hooks_settings::HooksSettingsPlugin,
    methods::{ManagementMethods, ManagementMethodsPlugin},
    settings::SettingsPlugin,
};
use rutis::Ctx;
use rutis::host::CtxHost;
use rutis_loader::resolve::Build;
use serde_json::{Value, json};
use std::{fs, path::PathBuf, time::Duration};
use tokio_util::sync::CancellationToken;

struct Fixture {
    root: Ctx,
    home: PathBuf,
    project: PathBuf,
    cancel: CancellationToken,
}
impl Fixture {
    async fn new() -> Self {
        let home =
            std::env::temp_dir().join(format!("dim-hooks-settings-{}", uuid::Uuid::new_v4()));
        let project = home.join("project");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(home.join(".dimcode")).unwrap();
        let home = fs::canonicalize(home).unwrap();
        let project = home.join("project");
        let root = Ctx::root().unwrap();
        let catalog = dim_app_server::catalog();
        let build = Build::default();
        let views = vec![
            root.plugin(FileStoragePlugin::new(vec![home.clone(), project.clone()])),
            root.plugin(SettingsPlugin::new(&Names::copy(&catalog, &build), 0, vec![1]).unwrap()),
            root.plugin(
                HooksSettingsPlugin::new(&Names::copy(&catalog, &build), Some(home.clone()))
                    .unwrap(),
            ),
            root.plugin(ManagementMethodsPlugin::typed(&catalog).unwrap()),
        ];
        let cancel = CancellationToken::new();
        wait_active(&root, &views, Duration::from_secs(5), &cancel)
            .await
            .unwrap();
        Self {
            root,
            home,
            project,
            cancel,
        }
    }

    /// One `hooks.*` call, as the RPC layer makes it.
    async fn call(
        &self,
        method: &str,
        params: Value,
    ) -> Result<Value, dim_agent_capabilities::methods::RpcError> {
        let table = self.root.get::<ManagementMethods>().unwrap();
        let provider = match table.owner(method) {
            Owner::Active(provider) => provider,
            _ => panic!("{method} has no active provider"),
        };
        provider
            .accept(method.to_owned(), params, self.cancel.clone(), |_| {})
            .unwrap()
            .reply
            .await
            .unwrap()
    }

    fn write_project(&self, document: Value) {
        fs::create_dir_all(self.project.join(".dimcode")).unwrap();
        fs::write(
            self.project.join(".dimcode/hooks.json"),
            serde_json::to_vec_pretty(&document).unwrap(),
        )
        .unwrap();
    }
}

fn handler(command: &str) -> Value {
    json!({"type": "command", "command": command})
}

fn documents(hooks: Value) -> Value {
    json!({"hooks": hooks})
}

#[tokio::test]
async fn catalog_reports_sources_states_and_never_reads_foreign_hook_files() {
    let fixture = Fixture::new().await;
    let catalog = fixture.call("hooks.catalog", json!({})).await.unwrap();
    assert_eq!(catalog["sources"][0]["source"], "user");
    assert_eq!(catalog["sources"][0]["state"], "missing");
    assert_eq!(catalog["hooks"], json!([]));

    // A Claude or Codex file in the workspace and in the home is not a source.
    for path in [
        fixture.project.join(".claude/settings.json"),
        fixture.home.join(".claude/settings.json"),
        fixture.project.join(".codex/hooks.json"),
        fixture.home.join(".codex/hooks.json"),
    ] {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            serde_json::to_vec(&documents(json!({
                "Stop": [{"hooks": [handler("foreign")]}],
            })))
            .unwrap(),
        )
        .unwrap();
    }
    let catalog = fixture
        .call(
            "hooks.catalog",
            json!({"cwd": fixture.project.to_str().unwrap()}),
        )
        .await
        .unwrap();
    assert_eq!(catalog["hooks"], json!([]));
    let sources = catalog["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 2, "{sources:?}");
    assert_eq!(sources[1]["source"], "project");
    assert_eq!(sources[1]["state"], "missing");

    fixture.write_project(documents(json!({
        "Stop": [{"hooks": [handler("notify")]}],
        "PreToolUse": [{"matcher": "read", "hooks": [handler("guard")]}],
        "UnknownEvent": [{"hooks": [handler("ignored")]}],
    })));
    let catalog = fixture
        .call(
            "hooks.catalog",
            json!({"cwd": fixture.project.to_str().unwrap()}),
        )
        .await
        .unwrap();
    assert_eq!(catalog["sources"][1]["state"], "ok");
    assert_eq!(catalog["sources"][1]["enabled"], false);
    let hooks = catalog["hooks"].as_array().unwrap();
    assert_eq!(hooks.len(), 3, "{hooks:?}");
    let by_event = |event: &str| {
        hooks
            .iter()
            .find(|entry| entry["event"] == event)
            .unwrap_or_else(|| panic!("{event} is listed"))
    };
    assert_eq!(by_event("Stop")["state"], "ok");
    assert_eq!(by_event("Stop")["source"], "project");
    // Listed, and not run until its owner lands (#758).
    assert_eq!(by_event("PreToolUse")["state"], "unsupported");
    assert_eq!(by_event("UnknownEvent")["state"], "unsupported");

    // An invalid document is a source state, not a failed call.
    fs::write(fixture.project.join(".dimcode/hooks.json"), b"{ nope").unwrap();
    let catalog = fixture
        .call(
            "hooks.catalog",
            json!({"cwd": fixture.project.to_str().unwrap()}),
        )
        .await
        .unwrap();
    assert_eq!(catalog["sources"][1]["state"], "invalid");
    assert!(catalog["sources"][1]["message"].is_string());
    assert_eq!(catalog["hooks"], json!([]));
    fixture.root.shutdown().await.unwrap();
    fs::remove_dir_all(&fixture.home).unwrap();
}

#[tokio::test]
async fn project_hooks_stay_disabled_until_the_user_enables_them_and_edits_apply() {
    let fixture = Fixture::new().await;
    fixture.write_project(documents(json!({
        "Stop": [{"hooks": [handler("first")]}],
    })));
    let cwd = fixture.project.to_str().unwrap().to_owned();
    let catalog = fixture
        .call("hooks.catalog", json!({"cwd": cwd}))
        .await
        .unwrap();
    assert_eq!(catalog["sources"][1]["enabled"], false);
    let revision = catalog["projectsRevision"].as_str().unwrap().to_owned();

    let enabled = fixture
        .call(
            "hooks.set_enabled",
            json!({"cwd": cwd, "enabled": true, "expectedRevision": revision}),
        )
        .await
        .unwrap();
    let revision = enabled["revision"].as_str().unwrap().to_owned();
    let catalog = fixture
        .call("hooks.catalog", json!({"cwd": cwd}))
        .await
        .unwrap();
    assert_eq!(catalog["sources"][1]["enabled"], true);

    // A stale revision is a conflict; the workspace's own document cannot
    // change its enablement.
    let stale = fixture
        .call(
            "hooks.set_enabled",
            json!({"cwd": cwd, "enabled": false, "expectedRevision": "missing"}),
        )
        .await
        .unwrap_err();
    assert_eq!(stale.data.as_ref().unwrap().app_error, "hooks_conflict");
    assert_eq!(
        fixture
            .call("hooks.catalog", json!({"cwd": cwd}))
            .await
            .unwrap()["sources"][1]["enabled"],
        true
    );

    // A later edit is read by the next catalog call: nothing is cached.
    fixture.write_project(documents(json!({
        "Stop": [{"hooks": [handler("second")]}],
    })));
    let catalog = fixture
        .call("hooks.catalog", json!({"cwd": cwd}))
        .await
        .unwrap();
    assert_eq!(catalog["hooks"][0]["command"], "second");
    assert_eq!(catalog["sources"][1]["enabled"], true);

    // Disabling again keeps the file and stops the source.
    let disabled = fixture
        .call(
            "hooks.set_enabled",
            json!({"cwd": cwd, "enabled": false, "expectedRevision": revision}),
        )
        .await
        .unwrap();
    assert!(disabled["revision"].is_string());
    assert_eq!(
        fixture
            .call("hooks.catalog", json!({"cwd": cwd}))
            .await
            .unwrap()["sources"][1]["enabled"],
        false
    );
    assert!(fixture.project.join(".dimcode/hooks.json").exists());
    fixture.root.shutdown().await.unwrap();
    fs::remove_dir_all(&fixture.home).unwrap();
}

#[tokio::test]
async fn a_user_document_is_saved_with_revisions_and_reports_disable_all_hooks() {
    let fixture = Fixture::new().await;
    let missing = fixture.call("hooks.catalog", json!({})).await.unwrap()["userRevision"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(missing, "missing");
    let saved = fixture
        .call(
            "hooks.save",
            json!({
                "expectedRevision": missing,
                "document": documents(json!({
                    "SessionStart": [{"hooks": [handler("welcome")]}],
                    "PostToolUse": [{"matcher": "write|edit", "hooks": [handler("format")]}],
                })),
            }),
        )
        .await
        .unwrap();
    let revision = saved["revision"].as_str().unwrap().to_owned();
    assert_ne!(revision, missing);
    let catalog = fixture.call("hooks.catalog", json!({})).await.unwrap();
    assert_eq!(catalog["userRevision"], revision);
    assert_eq!(catalog["sources"][0]["state"], "ok");
    assert_eq!(catalog["hooks"].as_array().unwrap().len(), 2);
    assert_eq!(catalog["disableAllHooks"], false);

    // A document that does not parse is refused without writing.
    let rejected = fixture
        .call(
            "hooks.save",
            json!({"expectedRevision": revision, "document": {"hooks": []}}),
        )
        .await
        .unwrap_err();
    assert_eq!(rejected.code, -32602);
    assert_eq!(
        fixture.call("hooks.catalog", json!({})).await.unwrap()["userRevision"],
        revision
    );

    // disableAllHooks is reported and does not change the catalog.
    let updated = fixture
        .call(
            "hooks.save",
            json!({
                "expectedRevision": revision,
                "document": {
                    "disableAllHooks": true,
                    "hooks": {"SessionStart": [{"hooks": [handler("welcome")]}]},
                },
            }),
        )
        .await
        .unwrap();
    let revision = updated["revision"].as_str().unwrap().to_owned();
    let catalog = fixture.call("hooks.catalog", json!({})).await.unwrap();
    assert_eq!(catalog["userRevision"], revision);
    assert_eq!(catalog["disableAllHooks"], true);
    fixture.root.shutdown().await.unwrap();
    fs::remove_dir_all(&fixture.home).unwrap();
}
