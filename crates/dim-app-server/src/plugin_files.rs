//! Generic, host-owned file transactions for an explicitly authorized plugin root.
//! No account formats or provider policy belong at this boundary.
use dim_agent_capabilities::file_storage::{FileLease, FilePath, FileStorage, FileStorageError};
use dim_agent_runtime::Names;
use rutis::Ctx;
use rutis::host::{BoxFuture, CtxHost, DynPlugin, Effect, Key, TypeKey};
use rutis_bridge::session::{Error, HostDispatch, Reply, Value, host_key};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    fs::Permissions,
    io,
    sync::{Arc, Mutex},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

const FORMAT_VERSION: u32 = 1;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredFile {
    version: u32,
    text: String,
}

fn failure(error: FileStorageError) -> Value {
    let code = match error {
        FileStorageError::Closed | FileStorageError::Cancelled => "closed",
        FileStorageError::InvalidPath(_) => "invalid_path",
        FileStorageError::Worker(_) => "internal",
        FileStorageError::Io(ref error) if error.kind() == io::ErrorKind::AlreadyExists => "exists",
        FileStorageError::Io(ref error) if error.kind() == io::ErrorKind::InvalidData => {
            "invalid_data"
        }
        _ => "io",
    };
    json!({"ok":false,"error":code}).into()
}
fn success(value: Value) -> Value {
    Value::Record([("ok".into(), json!(true).into()), ("value".into(), value)].into())
}
fn invalid() -> Error {
    Error::Value("invalid file operation".into())
}
fn private_permissions() -> Option<Permissions> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Some(Permissions::from_mode(0o600))
    }
    #[cfg(not(unix))]
    {
        None
    }
}
async fn resolve(value: Value) -> Reply {
    match value {
        Value::Reference(reference) if reference.is_future() => reference.wait_async().await,
        value => Ok(value),
    }
}

struct Files {
    storage: Arc<FileStorage>,
    root: usize,
    admission: Mutex<()>,
    stopping: CancellationToken,
    tasks: TaskTracker,
}
struct Scope {
    files: Arc<Files>,
    stopping: CancellationToken,
    tasks: TaskTracker,
}
impl Scope {
    fn quiesce(&self) {
        let _admission = self.files.admission.lock().unwrap();
        self.stopping.cancel();
        self.tasks.close();
    }
    fn value(self: Arc<Self>) -> Value {
        let transaction = self.clone();
        let quiesce = self.clone();
        Value::Record(
            [
                (
                    "withLock".into(),
                    Value::callback(move |args| transaction.with_lock(args)),
                ),
                (
                    "quiesce".into(),
                    Value::callback(move |_| {
                        quiesce.quiesce();
                        Ok(json!(null).into())
                    }),
                ),
                (
                    "close".into(),
                    Value::callback(move |_| {
                        self.quiesce();
                        let scope = self.clone();
                        Ok(Value::future(async move {
                            scope.tasks.wait().await;
                            Ok(json!(null).into())
                        }))
                    }),
                ),
            ]
            .into(),
        )
    }
    fn with_lock(self: &Arc<Self>, args: Value) -> Reply {
        let mut args = args.list()?.into_iter();
        let name: String = serde_json::from_value(args.next().ok_or_else(invalid)?.json()?)
            .map_err(|_| invalid())?;
        let callback = args.next().ok_or_else(invalid)?.reference()?;
        let cancelled = args.next().map(Value::reference).transpose()?;
        if cancelled
            .as_ref()
            .is_some_and(|reference| !reference.is_future())
            || args.next().is_some()
        {
            return Err(invalid());
        }
        let path = match self.files.storage.path(self.files.root, name) {
            Ok(path) => path,
            Err(error) => return Ok(Value::future(async move { Ok(failure(error)) })),
        };
        let _admission = self.files.admission.lock().unwrap();
        if self.stopping.is_cancelled() {
            return Ok(Value::future(async {
                Ok(failure(FileStorageError::Closed))
            }));
        }
        let scope = self.clone();
        let acquisition = self.stopping.child_token();
        let cancel_waiter = acquisition.clone().drop_guard();
        let owner = self.files.tasks.token();
        let task = self.tasks.spawn(async move {
            let _owner = owner;
            let acquire = async {
                loop {
                    match scope.files.storage.lock(&path, &acquisition).await {
                        Ok(lease) => return Ok(lease),
                        Err(FileStorageError::Locked) => tokio::select! {
                            _ = acquisition.cancelled() => return Err(FileStorageError::Closed),
                            _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {},
                        },
                        Err(error) => return Err(error),
                    }
                }
            };
            let cancelled = async {
                match cancelled {
                    Some(reference) => {
                        let _ = reference.wait_async().await;
                    }
                    None => std::future::pending::<()>().await,
                }
            };
            let lease = tokio::select! {
                _ = cancelled => {
                    acquisition.cancel();
                    return Ok(failure(FileStorageError::Closed));
                }
                result = acquire => match result {
                    Ok(lease) => lease,
                    Err(error) => return Ok(failure(error)),
                }
            };
            let lease = Arc::new(tokio::sync::Mutex::new(Some(lease)));
            let result = match callback
                .call_async(Value::List(vec![lease_value(
                    lease.clone(),
                    scope.tasks.clone(),
                )]))
                .await
            {
                Ok(value) => resolve(value).await,
                Err(error) => Err(error),
            };
            lease.lock().await.take();
            result.map(success)
        });
        Ok(Value::future(async move {
            let _cancel_waiter = cancel_waiter;
            task.await
                .unwrap_or_else(|error| Ok(failure(FileStorageError::Worker(error))))
        }))
    }
}

fn lease_value(lease: Arc<tokio::sync::Mutex<Option<FileLease>>>, tasks: TaskTracker) -> Value {
    Value::Record(
        ["read", "replace", "backup"]
            .into_iter()
            .map(|method| {
                let lease = lease.clone();
                let tasks = tasks.clone();
                (
                    method.into(),
                    Value::callback(move |args| {
                        let args = args.json()?;
                        let args = args.as_array().ok_or_else(invalid)?;
                        let argument = match (method, args.as_slice()) {
                            ("read", []) => None,
                            ("replace" | "backup", [serde_json::Value::String(value)]) => {
                                Some(value.clone())
                            }
                            _ => return Err(invalid()),
                        };
                        let lease = lease.clone();
                        let task = tasks.spawn(async move {
                            let mut guard = lease.lock().await;
                            let Some(lease) = guard.as_mut() else {
                                return Ok(failure(FileStorageError::Closed));
                            };
                            let cancel = CancellationToken::new();
                            let result = match method {
                                "read" => match lease.read(&cancel).await {
                                    Ok(bytes) => match serde_json::from_slice::<StoredFile>(&bytes)
                                    {
                                        Ok(StoredFile {
                                            version: FORMAT_VERSION,
                                            text,
                                        }) => Ok(json!(text).into()),
                                        _ => Err(FileStorageError::Io(
                                            io::ErrorKind::InvalidData.into(),
                                        )),
                                    },
                                    Err(FileStorageError::Io(error))
                                        if error.kind() == io::ErrorKind::NotFound =>
                                    {
                                        Ok(json!(null).into())
                                    }
                                    Err(error) => Err(error),
                                },
                                "replace" => lease
                                    .replace(
                                        serde_json::to_vec(&StoredFile {
                                            version: FORMAT_VERSION,
                                            text: argument.unwrap(),
                                        })
                                        .expect("text records contain only serializable fields"),
                                        private_permissions(),
                                        &cancel,
                                    )
                                    .await
                                    .map(|_| json!(null).into()),
                                "backup" => lease
                                    .backup(&argument.unwrap(), private_permissions(), &cancel)
                                    .await
                                    .map(|_| json!(null).into()),
                                _ => unreachable!(),
                            };
                            Ok(match result {
                                Ok(value) => success(value),
                                Err(error) => failure(error),
                            })
                        });
                        Ok(Value::future(async move {
                            task.await.unwrap_or_else(|error| {
                                Ok(failure(FileStorageError::Worker(error)))
                            })
                        }))
                    }),
                )
            })
            .collect(),
    )
}

/// Why a typed file transaction did not run or did not commit.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PluginFileError {
    #[error("the file store is closed")]
    Closed,
    #[error("the file path is outside the authorized root")]
    InvalidPath,
    #[error("the stored file is not a version 1 text envelope; it is left unchanged")]
    InvalidData,
    #[error("file operation failed: {0}")]
    Io(String),
}

fn file_error(error: FileStorageError) -> PluginFileError {
    match error {
        FileStorageError::Closed | FileStorageError::Cancelled => PluginFileError::Closed,
        FileStorageError::InvalidPath(_) => PluginFileError::InvalidPath,
        FileStorageError::Io(ref error) if error.kind() == io::ErrorKind::InvalidData => {
            PluginFileError::InvalidData
        }
        error => PluginFileError::Io(error.to_string()),
    }
}

/// Typed transactions on files under the authorized root, for process plugins
/// written in Rust. The host's `{version, text}` envelope is read and written
/// here; a caller sees and returns only the inner text, as the JSON
/// `pluginFiles` scope exposes it.
pub struct PluginFileTransactions(Arc<Files>);
impl PluginFileTransactions {
    /// Runs `change` on the inner text of `name` under its file lock. `change`
    /// receives `None` for a missing file. It returns the replacement text
    /// (`None` keeps the file as it is) and its own result, which the
    /// transaction returns. An envelope that is not version 1 fails with
    /// [`PluginFileError::InvalidData`] before `change` runs, and the bytes are
    /// left as they were.
    pub async fn transact<R: Send + 'static>(
        &self,
        name: &str,
        change: impl FnOnce(Option<&str>) -> (Option<String>, R) + Send + 'static,
    ) -> Result<R, PluginFileError> {
        let files = self.0.clone();
        let path = files
            .storage
            .path(files.root, name)
            .map_err(|_| PluginFileError::InvalidPath)?;
        let admission = files.admission.lock().unwrap();
        if files.stopping.is_cancelled() {
            return Err(PluginFileError::Closed);
        }
        let stopping = files.stopping.child_token();
        let storage = files.storage.clone();
        let owner = files.tasks.token();
        let task = files.tasks.spawn(async move {
            let _owner = owner;
            transact_file(&storage, &path, &stopping, change).await
        });
        drop(admission);
        task.await
            .unwrap_or_else(|error| Err(PluginFileError::Io(error.to_string())))
    }
}

async fn transact_file<R>(
    storage: &Arc<FileStorage>,
    path: &FilePath,
    stopping: &CancellationToken,
    change: impl FnOnce(Option<&str>) -> (Option<String>, R),
) -> Result<R, PluginFileError> {
    let mut lease = loop {
        match storage.lock(path, stopping).await {
            Ok(lease) => break lease,
            Err(FileStorageError::Locked) => tokio::select! {
                _ = stopping.cancelled() => return Err(PluginFileError::Closed),
                _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {},
            },
            Err(error) => return Err(file_error(error)),
        }
    };
    let current = match lease.read(stopping).await {
        Ok(bytes) => match serde_json::from_slice::<StoredFile>(&bytes) {
            Ok(StoredFile {
                version: FORMAT_VERSION,
                text,
            }) => Some(text),
            _ => return Err(PluginFileError::InvalidData),
        },
        Err(FileStorageError::Io(error)) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(file_error(error)),
    };
    let (next, result) = change(current.as_deref());
    if let Some(text) = next {
        let bytes = serde_json::to_vec(&StoredFile {
            version: FORMAT_VERSION,
            text,
        })
        .expect("text records contain only serializable fields");
        lease
            .replace(bytes, private_permissions(), stopping)
            .await
            .map_err(file_error)?;
    }
    Ok(result)
}

struct Dispatch(Arc<Files>);
impl HostDispatch for Dispatch {
    fn methods(&self) -> Option<serde_json::Value> {
        Some(json!({"open":"async"}))
    }
    fn invoke(&self, method: &str, args: Value) -> Reply {
        if method != "open" || !args.list()?.is_empty() {
            return Err(invalid());
        }
        let scope = Arc::new(Scope {
            files: self.0.clone(),
            stopping: self.0.stopping.child_token(),
            tasks: TaskTracker::new(),
        });
        Ok(Value::future(async move { Ok(scope.value()) }))
    }
}

/// The composition root chooses the authorized root; JS cannot register roots.
#[derive(Clone)]
pub struct PluginFilesPlugin {
    root: usize,
    storage: Key<FileStorage>,
}
impl PluginFilesPlugin {
    pub fn new(names: &Names<'_>, root: usize) -> rutis::Result<Self> {
        Ok(Self {
            root,
            storage: names.key()?,
        })
    }
}
impl DynPlugin for PluginFilesPlugin {
    fn name(&self) -> &str {
        "PluginFiles"
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![self.storage.erase()]
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let storage = ctx.require_at(&self.storage)?;
            let files = Arc::new(Files {
                storage,
                root: self.root,
                admission: Mutex::new(()),
                stopping: CancellationToken::new(),
                tasks: TaskTracker::new(),
            });
            let owner = files.clone();
            ctx.effect(move || async move {
                {
                    let _admission = owner.admission.lock().unwrap();
                    owner.stopping.cancel();
                    owner.tasks.close();
                }
                owner.tasks.wait().await;
                Ok(())
            })?;
            ctx.provide_at(
                host_key("pluginFiles"),
                Arc::new(Dispatch(files.clone())) as Arc<dyn HostDispatch>,
            )?;
            ctx.provide_as::<PluginFileTransactions>(Arc::new(PluginFileTransactions(files)))?;
            Ok(Effect::Done)
        })
    }
}
