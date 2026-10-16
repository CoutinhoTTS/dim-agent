//! Business-owned documents with source-local persistence and CAS.
mod native;

use crate::storage::SqliteStorage;
use dim_agent_capabilities::file_storage::{FilePath, FileStorage, FileStorageError};
use dim_agent_runtime::{Capability, KeysOf, When};
use rutis::{BoxFuture, CordisError, Ctx, Effect, Event, EventKey, TypeKey, TypedPlugin};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error(transparent)]
    Storage(FileStorageError),
    #[error("invalid settings: {0}")]
    Invalid(&'static str),
    #[error("settings changed; read the current version before updating")]
    Conflict,
    #[error("settings service is closed")]
    Closed,
    #[error("settings operation cancelled")]
    Cancelled,
    #[error("settings persistence failed; read the current revision before retrying")]
    Persistence,
    #[error("settings worker failed: {0}")]
    Worker(#[from] tokio::task::JoinError),
}
impl From<FileStorageError> for SettingsError {
    fn from(error: FileStorageError) -> Self {
        match error {
            FileStorageError::Cancelled => Self::Cancelled,
            FileStorageError::Closed => Self::Closed,
            error => Self::Storage(error),
        }
    }
}
pub type Result<T> = std::result::Result<T, SettingsError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SettingsScope {
    User,
    Project(usize),
}

/// A committed write to one Settings source. It carries the revision written,
/// never the document: a listener reads the source when it needs the content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsChanged {
    pub source: &'static str,
    pub scope: SettingsScope,
    pub revision: String,
}
impl Event for SettingsChanged {
    const NAME: &'static str = "settings.changed";
    type Value = ();
}
/// The rutis channel of one source's committed writes, in commit order.
pub fn settings_changes(source: &str) -> EventKey<SettingsChanged> {
    EventKey::dynamic(format!("settings/{source}"))
}
/// A project directory newly authorized for project sources.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectAuthorized {
    pub root: usize,
    pub cwd: PathBuf,
}
impl Event for ProjectAuthorized {
    const NAME: &'static str = "settings.project_authorized";
    type Value = ();
}
/// The rutis channel of project authorizations.
pub fn project_authorizations() -> EventKey<ProjectAuthorized> {
    EventKey::dynamic("settings/projects")
}

/// Wakes a receiver on every event it listens to, for an owner whose read
/// loop waits on a watch receiver: it rereads the source after each wake.
#[derive(Clone)]
pub(crate) struct Wake(Arc<tokio::sync::watch::Sender<()>>);
impl Wake {
    pub(crate) fn new() -> Self {
        Self(Arc::new(tokio::sync::watch::Sender::new(())))
    }
    pub(crate) fn subscribe(&self) -> tokio::sync::watch::Receiver<()> {
        self.0.subscribe()
    }
}
impl<E: Event<Value = ()>> rutis::Listener<E> for Wake {
    fn call<'a>(
        &'a self,
        _: &'a Ctx,
        _: &'a E,
    ) -> BoxFuture<'a, std::result::Result<Option<()>, CordisError>> {
        self.0.send_replace(());
        Box::pin(async { Ok(None) })
    }
}

pub trait BusinessDocument: Default + Serialize + DeserializeOwned + Send + Sync + 'static {
    const NAMESPACE: &'static str;
    const USER_FILE: &'static str;
    const PROJECT_FILE: Option<&'static str> = None;
    fn validate(&self) -> Result<()>;
}
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsSnapshot<D> {
    pub revision: String,
    pub document: D,
}

pub struct SettingsService {
    files: Arc<FileStorage>,
    database: Option<Arc<SqliteStorage>>,
    home_root: usize,
    projects: Arc<Mutex<BTreeMap<PathBuf, usize>>>,
    stopping: CancellationToken,
    gate: Mutex<()>,
    updates: TaskTracker,
    /// Settings' own plugin context, which emits change events.
    events: Ctx,
    /// The document type that first used each namespace and file; another
    /// type naming one of them would share its persisted source.
    claims: Mutex<BTreeMap<String, std::any::TypeId>>,
}
impl SettingsService {
    pub async fn authorize_project(
        self: &Arc<Self>,
        cwd: PathBuf,
        cancel: &CancellationToken,
    ) -> Result<usize> {
        if cwd.to_str().is_none() {
            return Err(SettingsError::Invalid("project cwd must be UTF-8"));
        }
        let task = {
            let _guard = self.gate.lock().unwrap();
            self.check_open(cancel)?;
            let owner = self.clone();
            let cancel = cancel.clone();
            self.updates.spawn(async move {
                let root = owner.files.register_root(cwd.clone(), &cancel).await?;
                let _guard = owner.gate.lock().unwrap();
                owner.check_open(&cancel)?;
                if owner.projects.lock().unwrap().insert(cwd.clone(), root) != Some(root) {
                    owner.emit(&project_authorizations(), ProjectAuthorized { root, cwd });
                }
                Ok(root)
            })
        };
        task.await?
    }
    pub fn project_root(&self, cwd: &Path) -> Option<usize> {
        self.projects.lock().unwrap().get(cwd).copied()
    }
    pub fn project_path(&self, root: usize) -> Result<PathBuf> {
        self.projects
            .lock()
            .unwrap()
            .iter()
            .find(|(_, index)| **index == root)
            .map(|(path, _)| path.clone())
            .ok_or(SettingsError::Invalid("unauthorized project"))
    }
    pub fn project_sources(&self) -> Vec<(usize, PathBuf)> {
        self.projects
            .lock()
            .unwrap()
            .iter()
            .map(|(path, index)| (*index, path.clone()))
            .collect()
    }
    /// Announces a committed write on the source's channel. Callers hold the
    /// commit order (the gate or the native connection), so events follow it.
    fn publish<D: BusinessDocument>(&self, scope: SettingsScope, revision: &str) {
        self.emit(
            &settings_changes(D::NAMESPACE),
            SettingsChanged {
                source: D::NAMESPACE,
                scope,
                revision: revision.to_owned(),
            },
        );
    }
    /// The change is already committed; a refused announcement cannot undo it,
    /// so it is recorded rather than returned to the writer.
    fn emit<E: Event>(&self, key: &EventKey<E>, event: E) {
        if let Err(error) = self
            .events
            .events()
            .emit(&self.events, key, Arc::new(event))
        {
            tracing::warn!(event = E::NAME, %error, "Settings change was not announced");
        }
    }
    fn path<D: BusinessDocument>(&self, scope: SettingsScope) -> Result<FilePath> {
        validate_namespace(D::NAMESPACE)?;
        validate_file(D::USER_FILE)?;
        if let Some(file) = D::PROJECT_FILE {
            validate_file(file)?;
        }
        self.claim::<D>()?;
        let (root, file) = match scope {
            SettingsScope::User => (self.home_root, D::USER_FILE),
            SettingsScope::Project(root) => {
                self.project_path(root)?;
                (
                    root,
                    D::PROJECT_FILE
                        .ok_or(SettingsError::Invalid("document has no project source"))?,
                )
            }
        };
        Ok(self.files.path(root, file)?)
    }
    fn claim<D: BusinessDocument>(&self) -> Result<()> {
        let owner = std::any::TypeId::of::<D>();
        let names = [
            format!("namespace {}", D::NAMESPACE),
            format!("user {}", D::USER_FILE),
        ]
        .into_iter()
        .chain(D::PROJECT_FILE.map(|file| format!("project {file}")));
        let mut claims = self.claims.lock().unwrap();
        let names: Vec<_> = names.collect();
        if names
            .iter()
            .any(|name| claims.get(name).is_some_and(|claimed| *claimed != owner))
        {
            return Err(SettingsError::Invalid(
                "settings source belongs to another document type",
            ));
        }
        for name in names {
            claims.insert(name, owner);
        }
        Ok(())
    }
    fn check_open(&self, cancel: &CancellationToken) -> Result<()> {
        if self.stopping.is_cancelled() {
            Err(SettingsError::Closed)
        } else if cancel.is_cancelled() {
            Err(SettingsError::Cancelled)
        } else {
            Ok(())
        }
    }
    pub async fn read<D: BusinessDocument>(
        &self,
        scope: SettingsScope,
        cancel: &CancellationToken,
    ) -> Result<SettingsSnapshot<D>> {
        self.check_open(cancel)?;
        let path = self.path::<D>(scope)?;
        if scope == SettingsScope::User && self.database.is_some() {
            return self.native_read::<D>(path, cancel).await;
        }
        decode(self.files.read(&path, cancel).await)
    }
    pub async fn update<D: BusinessDocument>(
        self: &Arc<Self>,
        scope: SettingsScope,
        expected: &str,
        document: D,
        cancel: &CancellationToken,
    ) -> Result<SettingsSnapshot<D>> {
        document.validate()?;
        let path = self.path::<D>(scope)?;
        let task = {
            let _guard = self.gate.lock().unwrap();
            self.check_open(cancel)?;
            let owner = self.clone();
            let expected = expected.to_owned();
            let cancel = cancel.clone();
            self.updates.spawn(async move {
                if scope == SettingsScope::User && owner.database.is_some() {
                    return owner
                        .native_replace(path, expected, document, &cancel)
                        .await;
                }
                let mut lease = owner.lease(&path, &cancel).await?;
                let current: SettingsSnapshot<D> = decode(lease.read(&cancel).await)?;
                if current.revision != expected {
                    return Err(SettingsError::Conflict);
                }
                let mut bytes = serde_json::to_vec_pretty(&document)
                    .map_err(|_| SettingsError::Invalid("cannot encode settings"))?;
                bytes.push(b'\n');
                let revision = revision(Some(&bytes));
                match lease.replace(bytes, private_permissions(), &cancel).await {
                    Ok(()) => {}
                    // The new content is what readers now see: consumers follow
                    // it, and the caller learns it may not survive a crash.
                    Err(FileStorageError::Unsynced(error)) => {
                        owner.publish::<D>(scope, &revision);
                        return Err(FileStorageError::Unsynced(error).into());
                    }
                    Err(error) => return Err(error.into()),
                }
                owner.publish::<D>(scope, &revision);
                Ok(SettingsSnapshot { revision, document })
            })
        };
        task.await?
    }
    async fn lease(
        &self,
        path: &FilePath,
        cancel: &CancellationToken,
    ) -> Result<dim_agent_capabilities::file_storage::FileLease> {
        loop {
            self.check_open(cancel)?;
            match self.files.lock(path, cancel).await {
                Ok(lease) => return Ok(lease),
                Err(FileStorageError::Locked) => tokio::select! {
                    _ = self.stopping.cancelled() => return Err(SettingsError::Closed),
                    _ = cancel.cancelled() => return Err(SettingsError::Cancelled),
                    _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {},
                },
                Err(error) => return Err(error.into()),
            }
        }
    }
}
fn validate_namespace(value: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'_')
    {
        return Err(SettingsError::Invalid("invalid document namespace"));
    }
    Ok(())
}
fn validate_file(value: &str) -> Result<()> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains(['/', '\\', '\0'])
        || Path::new(value).is_absolute()
    {
        return Err(SettingsError::Invalid("invalid document filename"));
    }
    Ok(())
}
fn private_permissions() -> Option<std::fs::Permissions> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Some(std::fs::Permissions::from_mode(0o600))
    }
    #[cfg(not(unix))]
    {
        None
    }
}
fn revision(bytes: Option<&[u8]>) -> String {
    bytes.map_or_else(
        || "missing".into(),
        |bytes| format!("{:x}", Sha256::digest(bytes)),
    )
}
fn decode<D: BusinessDocument>(
    read: std::result::Result<Vec<u8>, FileStorageError>,
) -> Result<SettingsSnapshot<D>> {
    let bytes = match read {
        Ok(bytes) => Some(bytes),
        Err(FileStorageError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let document: D = match &bytes {
        Some(bytes) => serde_json::from_slice(bytes)
            .map_err(|_| SettingsError::Invalid("invalid settings document"))?,
        None => D::default(),
    };
    document.validate()?;
    Ok(SettingsSnapshot {
        revision: revision(bytes.as_deref()),
        document,
    })
}

#[derive(Clone)]
pub struct SettingsPlugin {
    home_root: usize,
    project_roots: Vec<usize>,
    projects: Arc<Mutex<BTreeMap<PathBuf, usize>>>,
    native: bool,
}
impl SettingsPlugin {
    pub fn new(home_root: usize, project_roots: Vec<usize>) -> Self {
        Self {
            home_root,
            project_roots,
            projects: Arc::new(Mutex::new(BTreeMap::new())),
            native: false,
        }
    }
    pub fn native(home_root: usize, project_roots: Vec<usize>) -> Self {
        Self {
            native: true,
            ..Self::new(home_root, project_roots)
        }
    }
}
impl TypedPlugin for SettingsPlugin {
    type Deps = (Arc<FileStorage>, When<Arc<SqliteStorage>>);
    fn name(&self) -> &str {
        "Settings"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (files, When(database)): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            for root in &self.project_roots {
                let path = files
                    .root_path(*root)
                    .map_err(|error| CordisError::PluginFailed(error.into()))?;
                self.projects.lock().unwrap().insert(path, *root);
            }
            let service = Arc::new(SettingsService {
                files,
                database,
                home_root: self.home_root,
                projects: self.projects.clone(),
                stopping: CancellationToken::new(),
                gate: Mutex::new(()),
                updates: TaskTracker::new(),
                events: ctx.clone(),
                claims: Mutex::new(BTreeMap::new()),
            });
            let owner = service.clone();
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        {
                            let _guard = owner.gate.lock().unwrap();
                            owner.stopping.cancel();
                            owner.updates.close();
                        }
                        owner.updates.wait().await;
                        Ok(())
                    })
                }))
            })?;
            ctx.provide_as(TypeKey::of::<SettingsService>(), service)?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for SettingsPlugin {
    fn keys(&self) -> KeysOf<Self> {
        ((), self.native.then_some(()))
    }
}
