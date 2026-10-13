#![cfg(any(unix, windows))]
use dim_agent_capabilities::file_storage::FileStoragePlugin;
use dim_agent_runtime::{Capability, KeysOf, capability_plugin};
use dim_app_server::plugin_files::PluginFilesPlugin;
use rutis::{BoxFuture, CordisError, Ctx, DepKey, Effect, Keyed, TypedPlugin};
use rutis_bridge::session::{HostDispatch, Value};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, SystemTime},
};
use tokio::sync::Notify;

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/plugin-files-flow")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        Self(path.canonicalize().unwrap())
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
async fn resolved(value: Value) -> Value {
    match value {
        Value::Reference(reference) if reference.is_future() => {
            reference.wait_async().await.unwrap()
        }
        value => value,
    }
}
fn member(value: &Value, name: &str) -> Value {
    let Value::Record(fields) = value else {
        panic!("expected record")
    };
    fields[name].clone()
}
async fn call(value: &Value, name: &str, args: Vec<Value>) -> Value {
    resolved(
        member(value, name)
            .reference()
            .unwrap()
            .call_async(Value::List(args))
            .await
            .unwrap(),
    )
    .await
}
struct Capture(Arc<OnceLock<Arc<dyn HostDispatch>>>);
impl TypedPlugin for Capture {
    type Deps = Keyed<dyn HostDispatch>;
    fn name(&self) -> &str {
        "capture-plugin-files"
    }
    fn apply<'a>(
        &'a self,
        _: &'a Ctx,
        Keyed(files): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let _ = self.0.set(files);
            Ok(Effect::Done)
        })
    }
}
impl Capability for Capture {
    fn keys(&self) -> KeysOf<Self> {
        DepKey::dynamic("pluginFiles")
    }
}
async fn mount(path: &PathBuf) -> (Ctx, Value) {
    let root = Ctx::root().unwrap();
    (&root.plugin(capability_plugin(FileStoragePlugin::new(vec![
        path.clone(),
    ]))))
        .await
        .unwrap();
    (&root.plugin(capability_plugin(PluginFilesPlugin(0))))
        .await
        .unwrap();
    let captured = Arc::new(OnceLock::new());
    (&root.plugin(capability_plugin(Capture(captured.clone()))))
        .await
        .unwrap();
    let files = captured.get().unwrap();
    let scope = resolved(files.invoke("open", json!([]).into()).unwrap()).await;
    (root, scope)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn transactions_keep_backups_private_and_revoke_escaped_handles() {
    let directory = Directory::new();
    let (root, scope) = mount(&directory.0).await;
    let escaped = Arc::new(Mutex::new(None));
    let captured = escaped.clone();
    let result = call(
        &scope,
        "withLock",
        vec![
            json!("state.json").into(),
            Value::callback(move |args| {
                let lease = args.list()?.remove(0);
                *captured.lock().unwrap() = Some(lease.clone());
                Ok(Value::future(async move {
                    assert_eq!(
                        call(&lease, "read", vec![]).await.json().unwrap(),
                        json!({"ok":true,"value":null})
                    );
                    assert_eq!(
                        call(&lease, "replace", vec![json!("original\n").into()])
                            .await
                            .json()
                            .unwrap()["ok"],
                        true
                    );
                    assert_eq!(
                        call(&lease, "backup", vec![json!("before-reset").into()])
                            .await
                            .json()
                            .unwrap()["ok"],
                        true
                    );
                    assert_eq!(
                        call(&lease, "replace", vec![json!("replacement\n").into()])
                            .await
                            .json()
                            .unwrap()["ok"],
                        true
                    );
                    assert_eq!(
                        call(&lease, "backup", vec![json!("before-reset").into()])
                            .await
                            .json()
                            .unwrap()["error"],
                        "exists"
                    );
                    assert_eq!(
                        call(&lease, "backup", vec![json!("../escape").into()])
                            .await
                            .json()
                            .unwrap()["error"],
                        "invalid_path"
                    );
                    Ok(json!("completed").into())
                }))
            }),
        ],
    )
    .await
    .json()
    .unwrap();
    assert_eq!(result, json!({"ok":true,"value":"completed"}));
    assert_eq!(
        std::fs::read(directory.0.join("state.json")).unwrap(),
        b"replacement\n"
    );
    assert_eq!(
        std::fs::read(directory.0.join("state.json.before-reset")).unwrap(),
        b"original\n"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for name in ["state.json", "state.json.before-reset"] {
            assert_eq!(
                std::fs::metadata(directory.0.join(name))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
    let escaped = escaped.lock().unwrap().take().unwrap();
    assert_eq!(
        call(&escaped, "replace", vec![json!("late").into()])
            .await
            .json()
            .unwrap()["error"],
        "closed"
    );
    for path in [
        "../outside",
        "/absolute",
        "auth.json.lock",
        ".dim-file-storage-locks/anything",
    ] {
        let result = call(
            &scope,
            "withLock",
            vec![
                json!(path).into(),
                Value::callback(|_| panic!("invalid path admitted")),
            ],
        )
        .await;
        assert_eq!(result.json().unwrap()["error"], "invalid_path");
    }
    call(&scope, "close", vec![]).await;
    root.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quiesce_cancels_waiters_but_settles_issued_transactions_and_isolates_scopes() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let directory = Directory::new();
        let (root, scope) = mount(&directory.0).await;
        let (other_root, other) = mount(&directory.0).await;
        let entered = Arc::new(Notify::new());
        let resume = Arc::new(Notify::new());
        let (ready, proceed) = (entered.clone(), resume.clone());
        let active = scope.clone();
        let task = tokio::spawn(async move {
            call(
                &active,
                "withLock",
                vec![
                    json!("auth.json").into(),
                    Value::callback(move |args| {
                        let lease = args.list()?.remove(0);
                        let (ready, proceed) = (ready.clone(), proceed.clone());
                        Ok(Value::future(async move {
                            ready.notify_one();
                            proceed.notified().await;
                            Ok(call(&lease, "replace", vec![json!("rotated").into()]).await)
                        }))
                    }),
                ],
            )
            .await
        });
        entered.notified().await;
        let cancelled = call(
            &other,
            "withLock",
            vec![
                json!("auth.json").into(),
                Value::callback(|_| panic!("cancelled acquisition entered locked file")),
                Value::future(async { Ok(json!(null).into()) }),
            ],
        )
        .await;
        assert_eq!(cancelled.json().unwrap()["error"], "closed");
        let waiting = other.clone();
        let waiter = tokio::spawn(async move {
            call(
                &waiting,
                "withLock",
                vec![
                    json!("auth.json").into(),
                    Value::callback(|_| panic!("waiter entered locked file")),
                ],
            )
            .await
        });
        call(&other, "quiesce", vec![]).await;
        assert_eq!(waiter.await.unwrap().json().unwrap()["error"], "closed");
        call(&other, "close", vec![]).await;
        call(&scope, "quiesce", vec![]).await;
        assert!(!task.is_finished());
        resume.notify_one();
        assert_eq!(task.await.unwrap().json().unwrap()["value"]["ok"], true);
        assert_eq!(
            std::fs::read(directory.0.join("auth.json")).unwrap(),
            b"rotated"
        );
        call(&scope, "close", vec![]).await;
        root.shutdown().await.unwrap();
        other_root.shutdown().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_stale_lock_reclaimers_serialize_through_native_ownership() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let directory = Directory::new();
        let lock = directory.0.join("auth.json.lock");
        std::fs::write(&lock, b"2147483647\n0\nstale\n").unwrap();
        std::fs::File::options()
            .write(true)
            .open(lock)
            .unwrap()
            .set_times(
                std::fs::FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(60)),
            )
            .unwrap();
        std::fs::write(directory.0.join("auth.json"), "0").unwrap();
        let (root, a) = mount(&directory.0).await;
        let (other_root, b) = mount(&directory.0).await;
        let increment = |scope: Value, amount: u64| async move {
            call(
                &scope,
                "withLock",
                vec![
                    json!("auth.json").into(),
                    Value::callback(move |args| {
                        let file = args.list()?.remove(0);
                        Ok(Value::future(async move {
                            let value = call(&file, "read", vec![]).await.json().unwrap()["value"]
                                .as_str()
                                .unwrap()
                                .parse::<u64>()
                                .unwrap();
                            tokio::time::sleep(Duration::from_millis(30)).await;
                            Ok(call(
                                &file,
                                "replace",
                                vec![json!((value + amount).to_string()).into()],
                            )
                            .await)
                        }))
                    }),
                ],
            )
            .await
        };
        let (first, second) = tokio::join!(increment(a.clone(), 17), increment(b.clone(), 31));
        assert_eq!(first.json().unwrap()["value"]["ok"], true);
        assert_eq!(second.json().unwrap()["value"]["ok"], true);
        assert_eq!(
            std::fs::read_to_string(directory.0.join("auth.json")).unwrap(),
            "48"
        );
        call(&a, "close", vec![]).await;
        call(&b, "close", vec![]).await;
        root.shutdown().await.unwrap();
        other_root.shutdown().await.unwrap();
    })
    .await
    .unwrap();
}
