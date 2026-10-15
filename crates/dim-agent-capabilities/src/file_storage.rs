//! Process-owned byte storage beneath explicitly authorized directory handles.
//!
//! Unix uses `openat(O_NOFOLLOW)`; Windows uses handle-relative `NtCreateFile`
//! and rejects every reparse point. Reads, locks and replacement stay relative to
//! held directories. Roots must already exist. Business owners choose authorization,
//! formats and permissions. Other hosts are unsupported; no pathname fallback.
//!
//! Ordinary physical lock files remain after release: unlinking them would let
//! another process lock a new inode while the old inode is still leased.
//! `auth.json` instead uses the existing TypeScript-compatible exclusive marker
//! protocol at `auth.json.lock`; other files use the reserved hidden prefix below.
//! Lock entries retain the original basename on the same filesystem, including
//! the filesystem's own case and Unicode name equivalence.
//! Temporary files stay inside that physically protected directory namespace.
//! Listing hides those resources; removing a directory removes them with it.
use dim_agent_runtime::{Capability, KeysOf};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypeKey, TypedPlugin};
use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    fs::{File, Permissions, TryLockError},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, Weak},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const RESOURCE_PREFIX: &str = ".dim-file-storage-";
const LOCK_DIRECTORY: &str = ".dim-file-storage-locks";
const TEMPORARY_DIRECTORY: &str = ".dim-file-storage-temps";

pub type Result<T> = std::result::Result<T, FileStorageError>;

#[derive(Debug, thiserror::Error)]
pub enum FileStorageError {
    #[error("invalid storage path: {0}")]
    InvalidPath(&'static str),
    #[error("file storage is closed")]
    Closed,
    #[error("file operation cancelled before commit")]
    Cancelled,
    #[error("file is locked by another lease")]
    Locked,
    #[error("file storage requires a Unix or Windows host")]
    Unsupported,
    #[error("file I/O: {0}")]
    Io(#[from] io::Error),
    #[error("file worker: {0}")]
    Worker(#[from] tokio::task::JoinError),
    #[error("{operation}; temporary file cleanup: {cleanup}")]
    Cleanup {
        operation: Box<FileStorageError>,
        #[source]
        cleanup: io::Error,
    },
    #[error("file lock cleanup: {0:?}")]
    Release(Vec<io::Error>),
    /// The replacement is in place and visible, but syncing it failed, so it
    /// may not survive a crash. It is not an unchanged file.
    #[error("file replaced, but its durability could not be confirmed: {0}")]
    Unsynced(#[source] io::Error),
}

/// A relative path bound to one authorized root and one service generation.
#[derive(Clone, Debug)]
pub struct FilePath {
    generation: Uuid,
    root: usize,
    relative: PathBuf,
}

/// One entry of a directory listing: a single component as the filesystem
/// stores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub name: OsString,
    pub kind: EntryKind,
}

/// What a directory entry is, without following links.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
    /// A symbolic link, reparse point or special file. Storage operations never
    /// follow it; removal deletes the entry itself.
    Other,
}

/// Names reserved for this module's lock, temporary and account-marker resources.
fn reserved_name(name: &OsStr) -> bool {
    name.to_string_lossy().starts_with(RESOURCE_PREFIX) || name == "auth.json.lock"
}

struct State {
    roots: Vec<File>,
    active: usize,
    leases: HashMap<Uuid, Weak<Lease>>,
    cleanup_errors: Vec<io::Error>,
}

pub struct FileStorage {
    generation: Uuid,
    roots: Arc<Mutex<Vec<PathBuf>>>,
    state: Mutex<State>,
    stopping: CancellationToken,
    drained: Notify,
}

struct Operation(Arc<FileStorage>);
impl Drop for Operation {
    fn drop(&mut self) {
        self.0.state.lock().unwrap().active -= 1;
        self.0.drained.notify_waiters();
    }
}

struct LeaseResources {
    _lock: PhysicalLock,
    parent: File,
    name: OsString,
}
struct Lease {
    id: Uuid,
    owner: Weak<FileStorage>,
    path: FilePath,
    resources: Mutex<Option<LeaseResources>>,
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(owner) = self.owner.upgrade() {
            owner.state.lock().unwrap().leases.remove(&self.id);
        }
    }
}

/// Owns one physical file lock across a business read-modify-write operation.
/// Mutable methods prevent concurrent operations through the same lease.
pub struct FileLease(Option<Arc<Lease>>);
impl Drop for FileLease {
    fn drop(&mut self) {
        let Some(lease) = self.0.take() else { return };
        // Keep cleanup admitted before dropping the last public reference.
        // The weak registry must not disappear before its physical lock settles.
        let cleanup = lease.owner.upgrade().map(|owner| {
            owner.state.lock().unwrap().active += 1;
            Operation(owner)
        });
        drop(lease);
        drop(cleanup);
    }
}

impl FileStorage {
    fn open(roots: Arc<Mutex<Vec<PathBuf>>>) -> Result<Arc<Self>> {
        if cfg!(not(any(unix, windows))) {
            return Err(FileStorageError::Unsupported);
        }
        let handles = roots
            .lock()
            .unwrap()
            .iter()
            .map(|root| native::open_root(root))
            .collect::<Result<Vec<_>>>()?;
        Ok(Arc::new(Self {
            generation: Uuid::new_v4(),
            roots,
            state: Mutex::new(State {
                roots: handles,
                active: 0,
                leases: HashMap::new(),
                cleanup_errors: Vec::new(),
            }),
            stopping: CancellationToken::new(),
            drained: Notify::new(),
        }))
    }

    /// The trusted host supplies a canonical absolute directory. Root indices
    /// are append-only and the Plugin retains their configuration across reloads.
    pub async fn register_root(
        self: &Arc<Self>,
        root: PathBuf,
        cancel: &CancellationToken,
    ) -> Result<usize> {
        let operation = {
            let mut state = self.state.lock().unwrap();
            self.check(cancel)?;
            if let Some(index) = self
                .roots
                .lock()
                .unwrap()
                .iter()
                .position(|path| path == &root)
            {
                return Ok(index);
            }
            state.active += 1;
            Operation(self.clone())
        };
        let owner = self.clone();
        let cancel = cancel.clone();
        tokio::task::spawn_blocking(move || {
            let _operation = operation;
            owner.check(&cancel)?;
            let handle = native::open_root(&root)?;
            let mut state = owner.state.lock().unwrap();
            owner.check(&cancel)?;
            let mut roots = owner.roots.lock().unwrap();
            if let Some(index) = roots.iter().position(|path| path == &root) {
                return Ok(index);
            }
            let index = roots.len();
            state.roots.push(handle);
            roots.push(root);
            Ok(index)
        })
        .await?
    }

    pub fn root_path(&self, root: usize) -> Result<PathBuf> {
        if self.stopping.is_cancelled() {
            return Err(FileStorageError::Closed);
        }
        self.roots
            .lock()
            .unwrap()
            .get(root)
            .cloned()
            .ok_or(FileStorageError::InvalidPath("unknown authorized root"))
    }

    pub fn path(&self, root: usize, relative: impl AsRef<Path>) -> Result<FilePath> {
        {
            let state = self.state.lock().unwrap();
            if self.stopping.is_cancelled() {
                return Err(FileStorageError::Closed);
            }
            if root >= state.roots.len() {
                return Err(FileStorageError::InvalidPath("unknown authorized root"));
            }
        }
        let mut path = PathBuf::new();
        for component in relative.as_ref().components() {
            let Component::Normal(name) = component else {
                return Err(FileStorageError::InvalidPath(
                    "expected a relative file path",
                ));
            };
            #[cfg(windows)]
            native::validate_component(name)?;
            if reserved_name(name) {
                return Err(FileStorageError::InvalidPath(
                    "reserved storage resource name",
                ));
            }
            path.push(name);
        }
        if path.as_os_str().is_empty() {
            return Err(FileStorageError::InvalidPath("file name is required"));
        }
        Ok(FilePath {
            generation: self.generation,
            root,
            relative: path,
        })
    }

    fn check(&self, cancel: &CancellationToken) -> Result<()> {
        if self.stopping.is_cancelled() {
            Err(FileStorageError::Closed)
        } else if cancel.is_cancelled() {
            Err(FileStorageError::Cancelled)
        } else {
            Ok(())
        }
    }

    async fn run<T: Send + 'static>(
        self: &Arc<Self>,
        path: &FilePath,
        cancel: &CancellationToken,
        action: impl FnOnce(File, Arc<Self>, CancellationToken) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (root, operation) = {
            let mut state = self.state.lock().unwrap();
            self.check(cancel)?;
            if path.generation != self.generation {
                return Err(FileStorageError::InvalidPath(
                    "path belongs to another generation",
                ));
            }
            let root = state.roots[path.root].try_clone()?;
            state.active += 1;
            (root, Operation(self.clone()))
        };
        let owner = self.clone();
        let cancel = cancel.clone();
        // The worker owns admission, including when its awaiting caller is dropped.
        tokio::task::spawn_blocking(move || {
            let _operation = operation;
            // Captured lease resources must drop before admission, including
            // when cancellation returns before the action is called.
            (move || {
                owner.check(&cancel)?;
                action(root, owner, cancel)
            })()
        })
        .await?
    }

    pub async fn read(
        self: &Arc<Self>,
        path: &FilePath,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>> {
        let relative = path.relative.clone();
        self.run(path, cancel, move |root, owner, cancel| {
            let (parent, name) = native::parent(root, &relative)?;
            read(&parent, &name, &owner, &cancel)
        })
        .await
    }

    /// Attempts admission immediately; retry policy belongs to the consumer.
    pub async fn lock(
        self: &Arc<Self>,
        path: &FilePath,
        cancel: &CancellationToken,
    ) -> Result<FileLease> {
        let path = path.clone();
        self.run(&path.clone(), cancel, move |root, owner, cancel| {
            let (parent, name) = native::parent(root, &path.relative)?;
            let (name, lock) = physical_lock(&parent, &name, &owner)?;
            let lease = Arc::new(Lease {
                id: Uuid::new_v4(),
                owner: Arc::downgrade(&owner),
                path,
                resources: Mutex::new(Some(LeaseResources {
                    _lock: lock,
                    parent,
                    name,
                })),
            });
            {
                let mut state = owner.state.lock().unwrap();
                owner.check(&cancel)?;
                state.leases.insert(lease.id, Arc::downgrade(&lease));
            }
            Ok(FileLease(Some(lease)))
        })
        .await
    }

    /// Replaces bytes under the same lock used by explicit read-modify-write leases.
    pub async fn replace(
        self: &Arc<Self>,
        path: &FilePath,
        bytes: Vec<u8>,
        permissions: Option<Permissions>,
        cancel: &CancellationToken,
    ) -> Result<()> {
        self.lock(path, cancel)
            .await?
            .replace(bytes, permissions, cancel)
            .await
    }

    /// Lists a directory below an authorized root, sorted by name. This
    /// module's own lock, temporary and account-marker entries are omitted,
    /// including filesystem aliases of them. Links are listed, never followed.
    pub async fn list(
        self: &Arc<Self>,
        path: &FilePath,
        cancel: &CancellationToken,
    ) -> Result<Vec<DirectoryEntry>> {
        let relative = path.relative.clone();
        self.run(path, cancel, move |root, _, _| {
            let (parent, name) = native::parent(root, &relative)?;
            let mut entries = business_entries(&native::open_directory(&parent, &name)?)?;
            entries.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(entries)
        })
        .await
    }

    /// Removes the entry at `path` and, for a directory, everything beneath
    /// it, including this module's resources there. Files are removed under
    /// the lock leases use, so a lease held on or beneath the entry stops
    /// removal with `Locked`; a link is removed itself. An absent entry counts
    /// as removed, so retrying is safe. Removal is not atomic: an error or
    /// cancellation can leave part of a tree removed. The directory's owner
    /// orders removal after its writers; entries or leases created beneath it
    /// meanwhile make removal fail.
    pub async fn remove(
        self: &Arc<Self>,
        path: &FilePath,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let relative = path.relative.clone();
        self.run(path, cancel, move |root, owner, cancel| {
            absent_is_removed(
                native::parent(root, &relative)
                    .map_err(Into::into)
                    .and_then(|(parent, name)| match native::entry_kind(&parent, &name)? {
                        Some(kind) => remove_entry(&parent, &name, kind, &owner, &cancel),
                        None => Ok(()),
                    }),
            )
        })
        .await
    }

    async fn close(&self) -> Result<()> {
        self.stopping.cancel();
        loop {
            let changed = self.drained.notified();
            let leases = {
                let mut state = self.state.lock().unwrap();
                if state.active == 0 {
                    state.roots.clear();
                    Some(
                        state
                            .leases
                            .drain()
                            .filter_map(|(_, lease)| lease.upgrade())
                            .collect::<Vec<_>>(),
                    )
                } else {
                    None
                }
            };
            if let Some(leases) = leases {
                for lease in leases {
                    lease.resources.lock().unwrap().take();
                }
                let errors = std::mem::take(&mut self.state.lock().unwrap().cleanup_errors);
                return if errors.is_empty() {
                    Ok(())
                } else {
                    Err(FileStorageError::Release(errors))
                };
            }
            changed.await;
        }
    }
}

impl FileLease {
    async fn run<T: Send + 'static>(
        &mut self,
        cancel: &CancellationToken,
        action: impl FnOnce(File, OsString, Arc<FileStorage>, CancellationToken) -> Result<T>
        + Send
        + 'static,
    ) -> Result<T> {
        let lease = self.0.as_ref().expect("live file lease").clone();
        let owner = lease.owner.upgrade().ok_or(FileStorageError::Closed)?;
        owner
            .run(&lease.path.clone(), cancel, move |_, owner, cancel| {
                let resources = lease.resources.lock().unwrap();
                let resource = resources.as_ref().ok_or(FileStorageError::Closed)?;
                let parent = resource.parent.try_clone()?;
                let name = resource.name.clone();
                owner.check(&cancel)?;
                let result = action(parent, name, owner, cancel);
                drop(resources);
                result
            })
            .await
    }

    pub async fn read(&mut self, cancel: &CancellationToken) -> Result<Vec<u8>> {
        self.run(cancel, |parent, name, owner, cancel| {
            read(&parent, &name, &owner, &cancel)
        })
        .await
    }

    /// Creates a durable, exclusive sibling copy while retaining this lease.
    /// An existing or partially written backup is never overwritten or removed.
    pub async fn backup(
        &mut self,
        suffix: &str,
        permissions: Option<Permissions>,
        cancel: &CancellationToken,
    ) -> Result<()> {
        if suffix.is_empty()
            || !suffix
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(FileStorageError::InvalidPath("invalid backup suffix"));
        }
        let suffix = suffix.to_owned();
        self.run(cancel, move |parent, name, owner, cancel| {
            let bytes = read(&parent, &name, &owner, &cancel)?;
            let mut backup = name.clone();
            backup.push(format!(".{suffix}"));
            let mut file =
                native::create_temp(&parent, &backup, &parent, &name, permissions.as_ref())?;
            // Once created, retain a failed backup for explicit operator recovery.
            file.write_all(&bytes)?;
            file.sync_all()?;
            native::sync_replacement(&file, &parent, &parent)?;
            Ok(())
        })
        .await
    }

    /// Permissions are supplied by the business owner; absent permissions preserve
    /// an existing file's mode/readonly attribute. New files use Unix umask or the
    /// Windows business parent DACL; existing Windows DACLs are preserved.
    /// Cancellation is checked through preparation. Once rename starts, the
    /// rename and host-supported flush settle before storage can be unloaded.
    pub async fn replace(
        &mut self,
        bytes: Vec<u8>,
        permissions: Option<Permissions>,
        cancel: &CancellationToken,
    ) -> Result<()> {
        self.run(cancel, move |parent, name, owner, cancel| {
            let permissions = permissions.or(native::permissions(&parent, &name)?);
            let temporary_directory = native::temporary_directory(&parent)?;
            let temporary = OsString::from(format!("{RESOURCE_PREFIX}temp-{}", Uuid::new_v4()));
            let mut temp = native::create_temp(
                &temporary_directory,
                &temporary,
                &parent,
                &name,
                permissions.as_ref(),
            )?;
            let result = (|| {
                for chunk in bytes.chunks(64 * 1024) {
                    owner.check(&cancel)?;
                    temp.write_all(chunk)?;
                }
                // Writes can clear Unix set-id bits; apply the final mode afterwards.
                if let Some(permissions) = permissions {
                    temp.set_permissions(permissions)?;
                }
                temp.sync_all()?;
                // This check is the commit boundary. No cancellation after it
                // may abandon rename or its durability result.
                owner.check(&cancel)?;
                native::replace(&temp, &temporary_directory, &temporary, &parent, &name)?;
                native::sync_replacement(&temp, &temporary_directory, &parent)
                    .map_err(FileStorageError::Unsynced)
            })();
            // On success rename consumed the temporary name. On failure before
            // rename it is still ours; after rename it no longer exists.
            if result
                .as_ref()
                .is_err_and(|error| !matches!(error, FileStorageError::Unsynced(_)))
                && let Err(cleanup) = native::remove(&temporary_directory, &temporary)
                && cleanup.kind() != io::ErrorKind::NotFound
            {
                return Err(FileStorageError::Cleanup {
                    operation: Box::new(result.unwrap_err()),
                    cleanup,
                });
            }
            result
        })
        .await
    }
}

fn read(
    parent: &File,
    name: &OsString,
    owner: &FileStorage,
    cancel: &CancellationToken,
) -> Result<Vec<u8>> {
    let mut file = native::read_user_file(parent, name)?;
    let mut bytes = Vec::new();
    let mut chunk = [0; 64 * 1024];
    loop {
        owner.check(cancel)?;
        let count = file.read(&mut chunk)?;
        if count == 0 {
            return Ok(bytes);
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
}

/// Locks a regular or missing file under the name its filesystem resolves.
fn physical_lock(
    parent: &File,
    name: &OsString,
    owner: &Arc<FileStorage>,
) -> Result<(OsString, PhysicalLock)> {
    let name = native::target_name(parent, name)?;
    native::permissions(parent, &name)?;
    let lock = native::lock(parent, &name, owner)?;
    Ok((name, lock))
}

/// Entries a business owner may address: this module's resources are
/// omitted by name and by the identity the filesystem gives them.
fn business_entries(directory: &File) -> Result<Vec<DirectoryEntry>> {
    let reserved = native::reserved(directory)?;
    Ok(native::entries(directory)?
        .into_iter()
        .filter(|(entry, identity)| !reserved_name(&entry.name) && !reserved.contains(identity))
        .map(|(entry, _)| entry)
        .collect())
}

/// Removes one entry below `parent`. A link is removed, never followed; a
/// directory is emptied through held handles and never on another filesystem.
fn remove_entry(
    parent: &File,
    name: &OsString,
    kind: EntryKind,
    owner: &Arc<FileStorage>,
    cancel: &CancellationToken,
) -> Result<()> {
    match kind {
        EntryKind::File => {
            let (name, _lock) = physical_lock(parent, name, owner)?;
            owner.check(cancel)?;
            Ok(native::remove(parent, &name)?)
        }
        EntryKind::Other => Ok(native::remove(parent, name)?),
        EntryKind::Directory => {
            let directory = native::subdirectory(parent, name)?;
            for entry in business_entries(&directory)? {
                owner.check(cancel)?;
                absent_is_removed(remove_entry(
                    &directory,
                    &entry.name,
                    entry.kind,
                    owner,
                    cancel,
                ))?;
            }
            owner.check(cancel)?;
            let _keys = remove_resources(&directory)?;
            Ok(native::remove_directory(parent, name)?)
        }
    }
}

/// Removes this module's resources from a directory whose business entries
/// are gone. Every lock key is locked before any is unlinked and stays locked
/// until the caller has removed the directory, so no lease can hold a key
/// that was unlinked while its file could still exist.
fn remove_resources(directory: &File) -> Result<Vec<HeldKey>> {
    let mut keys = Vec::new();
    let mut contents = Vec::new();
    let locks = native::reserved_directory(directory)?;
    if let Some(locks) = &locks {
        for (entry, _) in native::entries(locks)? {
            if entry.kind == EntryKind::File {
                match native::open_key(locks, &entry.name) {
                    Ok(key) => keys.push(HeldKey::lock(key)?),
                    // Unlinked meanwhile by a concurrent removal of this directory.
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error.into()),
                }
            }
            contents.push(entry);
        }
    }
    // A live account marker belongs to another holder of `auth.json`.
    if !reclaim_account(directory)? {
        return Err(FileStorageError::Locked);
    }
    if let Some(locks) = &locks {
        for entry in contents {
            absent_is_removed(if entry.kind == EntryKind::Directory {
                remove_temporary(locks, &entry.name)
            } else {
                native::remove(locks, &entry.name).map_err(Into::into)
            })?;
        }
        absent_is_removed(
            native::remove_directory(directory, OsStr::new(LOCK_DIRECTORY)).map_err(Into::into),
        )?;
    }
    Ok(keys)
}

/// Removes a directory of temporary files. Each replacement that writes one
/// holds a key locked by `remove_resources`.
fn remove_temporary(locks: &File, name: &OsStr) -> Result<()> {
    let temporary = native::subdirectory(locks, name)?;
    for (file, _) in native::entries(&temporary)? {
        absent_is_removed(native::remove(&temporary, &file.name).map_err(Into::into))?;
    }
    Ok(native::remove_directory(locks, name)?)
}

/// Removal ensures absence: an entry or parent that no longer exists is removed.
fn absent_is_removed(result: Result<()>) -> Result<()> {
    match result {
        Err(FileStorageError::Io(error)) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

/// Clones share authorized root identities; each apply opens fresh I/O resources.
#[derive(Clone)]
pub struct FileStoragePlugin {
    roots: Arc<Mutex<Vec<PathBuf>>>,
}
impl FileStoragePlugin {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self {
            roots: Arc::new(Mutex::new(roots)),
        }
    }
}
impl TypedPlugin for FileStoragePlugin {
    type Deps = ();
    fn name(&self) -> &str {
        "FileStorage"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            let roots = self.roots.clone();
            let storage = tokio::task::spawn_blocking(move || FileStorage::open(roots))
                .await
                .map_err(|error| CordisError::PluginFailed(error.into()))?
                .map_err(|error| CordisError::PluginFailed(error.into()))?;
            let owner = storage.clone();
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        owner
                            .close()
                            .await
                            .map_err(|error| CordisError::PluginFailed(error.into()))
                    })
                }))
            })?;
            ctx.provide_as(TypeKey::of::<FileStorage>(), storage)?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for FileStoragePlugin {
    fn keys(&self) -> KeysOf<Self> {}
}

struct PhysicalLock {
    file: File,
    _alias_lock: Option<File>,
    account_parent: Option<File>,
    owner: Option<Vec<u8>>,
    storage: Weak<FileStorage>,
}
impl Drop for PhysicalLock {
    fn drop(&mut self) {
        if let Some(parent) = &self.account_parent
            && let Err(error) = release_account(parent, &self.file, self.owner.as_deref())
            && let Some(storage) = self.storage.upgrade()
        {
            storage.state.lock().unwrap().cleanup_errors.push(error);
        }
        // Windows may defer an implicit unlock after CloseHandle. Settle the
        // real byte-range lock before a new generation is admitted.
        if let Err(error) = self._alias_lock.as_ref().unwrap_or(&self.file).unlock()
            && let Some(storage) = self.storage.upgrade()
        {
            storage.state.lock().unwrap().cleanup_errors.push(error);
        }
    }
}

/// A business lock key held while its PhysicalLock is built. Every early
/// return unlocks it explicitly, as PhysicalLock's drop does: Windows may
/// defer the implicit unlock after CloseHandle.
struct HeldKey(Option<File>);
impl HeldKey {
    fn lock(file: File) -> Result<Self> {
        file.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => FileStorageError::Locked,
            TryLockError::Error(error) => FileStorageError::Io(error),
        })?;
        Ok(Self(Some(file)))
    }
    fn file(&self) -> &File {
        self.0.as_ref().expect("held lock key")
    }
    /// Hands the locked file to its PhysicalLock, which unlocks it.
    fn into_file(mut self) -> File {
        self.0.take().expect("held lock key")
    }
}
impl Drop for HeldKey {
    fn drop(&mut self) {
        if let Some(file) = &self.0 {
            let _ = file.unlock();
        }
    }
}

fn account_lock(
    parent: &File,
    alias_lock: HeldKey,
    storage: &Arc<FileStorage>,
) -> Result<PhysicalLock> {
    let account_parent = parent.try_clone()?;
    let name = OsStr::new("auth.json.lock");
    for _ in 0..2 {
        match native::create_account_file(parent, name) {
            Ok(file) => {
                let mut lock = PhysicalLock {
                    file,
                    _alias_lock: Some(alias_lock.into_file()),
                    account_parent: Some(account_parent),
                    owner: None,
                    storage: Arc::downgrade(storage),
                };
                let millis = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(io::Error::other)?
                    .as_millis();
                let owner =
                    format!("{}\n{millis}\n{}\n", std::process::id(), Uuid::new_v4()).into_bytes();
                lock.file.write_all(&owner)?;
                lock.file.sync_all()?;
                lock.owner = Some(owner);
                return Ok(lock);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                if !reclaim_account(parent)? {
                    return Err(FileStorageError::Locked);
                }
            }
            Err(error) => return Err(error.into()),
        }
    }
    Err(FileStorageError::Locked)
}

fn reclaim_account(parent: &File) -> io::Result<bool> {
    let name = OsString::from("auth.json.lock");
    let mut file = match native::read_account_file(parent, &name) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(error),
    };
    let observed = file.metadata()?;
    let age = SystemTime::now()
        .duration_since(observed.modified()?)
        .unwrap_or_default();
    if age <= Duration::from_secs(30) {
        return Ok(false);
    }
    let mut owner = Vec::new();
    (&mut file).take(256).read_to_end(&mut owner)?;
    let pid = owner
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default();
    let pid = std::str::from_utf8(pid)
        .ok()
        .filter(|pid| !pid.is_empty() && pid.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|pid| pid.parse::<u32>().ok())
        .filter(|pid| *pid > 0);
    if let Some(pid) = pid {
        let alive = native::process_alive(pid)?;
        if alive {
            return Ok(false);
        }
    }
    let mut current = match native::read_account_file(parent, &name) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(error),
    };
    if !native::same_identity(&file, &current)?
        || observed.modified()? != current.metadata()?.modified()?
    {
        return Ok(false);
    }
    let mut current_owner = Vec::new();
    (&mut current).take(256).read_to_end(&mut current_owner)?;
    if current_owner != owner {
        return Ok(false);
    }
    match native::remove_account(parent, &name, &current) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error),
    }
}

fn release_account(parent: &File, owned: &File, owner: Option<&[u8]>) -> io::Result<()> {
    let name = OsString::from("auth.json.lock");
    let mut current = match native::read_account_file(parent, &name) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let observed = owned.metadata()?;
    if !native::same_identity(owned, &current)?
        || observed.modified()? != current.metadata()?.modified()?
    {
        return Ok(());
    }
    if let Some(owner) = owner {
        let mut bytes = Vec::new();
        (&mut current)
            .take(owner.len() as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes != owner {
            return Ok(());
        }
    }
    let latest = match native::read_account_file(parent, &name) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if native::same_identity(owned, &latest)?
        && observed.modified()? == latest.metadata()?.modified()?
    {
        native::remove_account(parent, &name, &latest)?;
    }
    Ok(())
}
#[cfg(unix)]
mod native {
    use super::*;
    // readdir reports errors only through errno.
    #[cfg(any(target_os = "illumos", target_os = "solaris"))]
    use libc::___errno as errno_location;
    #[cfg(any(target_os = "android", target_os = "netbsd", target_os = "openbsd"))]
    use libc::__errno as errno_location;
    #[cfg(not(any(
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "android",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "illumos",
        target_os = "solaris",
        target_os = "haiku",
        target_os = "aix",
        target_os = "nto"
    )))]
    use libc::__errno_location as errno_location;
    #[cfg(any(target_vendor = "apple", target_os = "freebsd"))]
    use libc::__error as errno_location;
    #[cfg(target_os = "nto")]
    use libc::__get_errno_ptr as errno_location;
    #[cfg(target_os = "aix")]
    use libc::_Errno as errno_location;
    #[cfg(target_os = "haiku")]
    use libc::_errnop as errno_location;
    use std::{
        ffi::{CStr, CString, OsStr},
        os::{
            fd::{AsRawFd, FromRawFd, IntoRawFd},
            unix::{
                ffi::OsStrExt,
                fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
            },
        },
    };

    /// Device and inode.
    pub type Identity = (u64, u64);

    fn c_name(name: &OsStr) -> io::Result<CString> {
        CString::new(name.as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))
    }

    fn directory(parent: &File, name: &OsStr) -> io::Result<File> {
        let directory_name = c_name(name)?;
        let created = unsafe { libc::mkdirat(parent.as_raw_fd(), directory_name.as_ptr(), 0o700) };
        if created < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::AlreadyExists {
                return Err(error);
            }
        } else {
            parent.sync_all()?;
        }
        open(parent, name, libc::O_RDONLY | libc::O_DIRECTORY, 0)
    }
    fn directories(parent: &File) -> io::Result<(File, File)> {
        let locks = directory(parent, OsStr::new(LOCK_DIRECTORY))?;
        let temporary = directory(&locks, OsStr::new(TEMPORARY_DIRECTORY))?;
        Ok((locks, temporary))
    }
    pub fn temporary_directory(parent: &File) -> io::Result<File> {
        Ok(directories(parent)?.1)
    }
    fn lock_key(parent: &File, name: &OsStr) -> io::Result<File> {
        // Concurrent nonexclusive creation can fail with ENOENT on macOS even
        // when the winner created the key. Create once, then open that inode.
        let file = match open(
            parent,
            name,
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
            0o600,
        ) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                open(parent, name, libc::O_RDWR, 0)?
            }
            Err(error) => return Err(error),
        };
        regular(file)
    }
    pub fn lock(parent: &File, name: &OsStr, storage: &Arc<FileStorage>) -> Result<PhysicalLock> {
        // Reserve the fixed temp directory before opening any business lock key,
        // so filesystem-equivalent spellings cannot create a competing inode.
        let (directory, _temporary) = directories(parent)?;
        if let Some(target) = stat(parent, name)? {
            let reserved = directory.metadata()?;
            // dev_t is i32 on macOS and u64 on Linux.
            #[allow(clippy::unnecessary_cast)]
            if target.st_dev == reserved.dev() as libc::dev_t && target.st_ino == reserved.ino() {
                return Err(FileStorageError::InvalidPath("reserved storage directory"));
            }
        }
        // Raw names on the same volume share its case/Unicode equivalence. Hashes
        // or string folding cannot reproduce the filesystem's name identity.
        let alias_lock = HeldKey::lock(lock_key(&directory, name)?)?;
        let marker_key = lock_key(&directory, OsStr::new("auth.json.lock"))?;
        if same_inode(&alias_lock.file().metadata()?, &marker_key.metadata()?) {
            return Err(FileStorageError::InvalidPath("reserved storage lock"));
        }
        let auth_key = lock_key(&directory, OsStr::new("auth.json"))?;
        if same_inode(&alias_lock.file().metadata()?, &auth_key.metadata()?) {
            return account_lock(parent, alias_lock, storage);
        }
        Ok(PhysicalLock {
            file: alias_lock.into_file(),
            _alias_lock: None,
            account_parent: None,
            owner: None,
            storage: Arc::downgrade(storage),
        })
    }

    fn same_inode(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    pub(super) fn same_identity(a: &File, b: &File) -> io::Result<bool> {
        Ok(same_inode(&a.metadata()?, &b.metadata()?))
    }
    pub(super) fn target_name(_: &File, name: &OsString) -> io::Result<OsString> {
        Ok(name.clone())
    }
    pub(super) fn create_account_file(parent: &File, name: &OsStr) -> io::Result<File> {
        open(
            parent,
            name,
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
            0o600,
        )
    }
    pub(super) fn read_account_file(parent: &File, name: &OsString) -> io::Result<File> {
        read_file(parent, name)
    }
    pub(super) fn remove_account(parent: &File, name: &OsString, _: &File) -> io::Result<()> {
        remove(parent, name)
    }
    pub(super) fn process_alive(pid: u32) -> io::Result<bool> {
        let Ok(pid) = libc::pid_t::try_from(pid) else {
            return Ok(false);
        };
        Ok(unsafe { libc::kill(pid, 0) } == 0
            || io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH))
    }

    fn open(
        parent: &File,
        name: &OsStr,
        flags: libc::c_int,
        mode: libc::c_uint,
    ) -> io::Result<File> {
        let name = c_name(name)?;
        // Each name is one already validated component, never a multi-part path.
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
                mode,
            )
        };
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }
    fn regular(file: File) -> io::Result<File> {
        if file.metadata()?.is_file() {
            Ok(file)
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected a regular file",
            ))
        }
    }
    pub fn open_root(path: &Path) -> Result<File> {
        if !path.is_absolute() {
            return Err(FileStorageError::InvalidPath(
                "authorized roots must be absolute",
            ));
        }
        let mut root = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open("/")?;
        for component in path.components() {
            match component {
                Component::RootDir => {}
                Component::Normal(name) => {
                    root = open(&root, name, libc::O_RDONLY | libc::O_DIRECTORY, 0)?
                }
                _ => return Err(FileStorageError::InvalidPath("invalid authorized root")),
            }
        }
        Ok(root)
    }
    pub fn parent(mut root: File, path: &Path) -> io::Result<(File, OsString)> {
        let mut components = path.iter().peekable();
        while let Some(name) = components.next() {
            if components.peek().is_none() {
                return Ok((root, name.to_os_string()));
            }
            root = open_directory(&root, name)?;
        }
        unreachable!("validated file path")
    }
    /// Opens a business directory below `parent`; the reserved directory is
    /// refused under every spelling the filesystem equates with it.
    pub fn open_directory(parent: &File, name: &OsStr) -> io::Result<File> {
        let next = open(parent, name, libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
        match reserved_directory(parent)? {
            Some(reserved) if same_inode(&next.metadata()?, &reserved.metadata()?) => Err(
                io::Error::new(io::ErrorKind::InvalidInput, "reserved storage directory"),
            ),
            _ => Ok(next),
        }
    }
    /// Opens a directory to remove; a filesystem mounted there is never entered.
    pub fn subdirectory(parent: &File, name: &OsStr) -> io::Result<File> {
        let directory = open_directory(parent, name)?;
        if directory.metadata()?.dev() != parent.metadata()?.dev() {
            return Err(io::Error::new(
                io::ErrorKind::CrossesDevices,
                "directory on another filesystem",
            ));
        }
        Ok(directory)
    }
    pub fn reserved_directory(directory: &File) -> io::Result<Option<File>> {
        match open(
            directory,
            OsStr::new(LOCK_DIRECTORY),
            libc::O_RDONLY | libc::O_DIRECTORY,
            0,
        ) {
            Ok(locks) => Ok(Some(locks)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
    pub fn open_key(locks: &File, name: &OsStr) -> io::Result<File> {
        regular(open(locks, name, libc::O_RDWR, 0)?)
    }
    // dev_t is i32 on macOS and u64 on Linux.
    #[allow(clippy::unnecessary_cast)]
    fn identity(stat: &libc::stat) -> Identity {
        (stat.st_dev as u64, stat.st_ino as u64)
    }
    fn kind(stat: &libc::stat) -> EntryKind {
        match stat.st_mode & libc::S_IFMT {
            libc::S_IFREG => EntryKind::File,
            libc::S_IFDIR => EntryKind::Directory,
            _ => EntryKind::Other,
        }
    }
    pub fn entry_kind(parent: &File, name: &OsStr) -> io::Result<Option<EntryKind>> {
        Ok(stat(parent, name)?.as_ref().map(kind))
    }
    /// Identities of the reserved directory and account marker in `directory`.
    pub fn reserved(directory: &File) -> io::Result<Vec<Identity>> {
        let mut identities = Vec::new();
        for name in [LOCK_DIRECTORY, "auth.json.lock"] {
            if let Some(stat) = stat(directory, OsStr::new(name))? {
                identities.push(identity(&stat));
            }
        }
        Ok(identities)
    }
    pub fn entries(directory: &File) -> io::Result<Vec<(DirectoryEntry, Identity)>> {
        let mut entries = Vec::new();
        for name in names(directory)? {
            // An entry removed after the directory was read is gone.
            if let Some(stat) = stat(directory, &name)? {
                let identity = identity(&stat);
                entries.push((
                    DirectoryEntry {
                        name,
                        kind: kind(&stat),
                    },
                    identity,
                ));
            }
        }
        Ok(entries)
    }
    fn names(directory: &File) -> io::Result<Vec<OsString>> {
        // A fresh open file description leaves the caller's offset untouched.
        let fd = open(
            directory,
            OsStr::new("."),
            libc::O_RDONLY | libc::O_DIRECTORY,
            0,
        )?
        .into_raw_fd();
        let stream = unsafe { libc::fdopendir(fd) };
        if stream.is_null() {
            let error = io::Error::last_os_error();
            drop(unsafe { File::from_raw_fd(fd) });
            return Err(error);
        }
        struct Stream(*mut libc::DIR);
        impl Drop for Stream {
            fn drop(&mut self) {
                unsafe { libc::closedir(self.0) };
            }
        }
        let stream = Stream(stream);
        let mut names = Vec::new();
        loop {
            // readdir reports errors only through errno; end of stream keeps it.
            unsafe { *errno_location() = 0 };
            let entry = unsafe { libc::readdir(stream.0) };
            if entry.is_null() {
                let error = io::Error::last_os_error();
                return match error.raw_os_error() {
                    Some(0) => Ok(names),
                    _ => Err(error),
                };
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if name != b"." && name != b".." {
                names.push(OsStr::from_bytes(name).to_os_string());
            }
        }
    }
    pub fn read_file(parent: &File, name: &OsString) -> io::Result<File> {
        regular(open(parent, name, libc::O_RDONLY, 0)?)
    }
    pub fn read_user_file(parent: &File, name: &OsString) -> io::Result<File> {
        let file = read_file(parent, name)?;
        let observed = file.metadata()?;
        // dev_t is i32 on macOS and u64 on Linux.
        #[allow(clippy::unnecessary_cast)]
        if let Some(account_lock) = stat(parent, OsStr::new("auth.json.lock"))?
            && observed.dev() as libc::dev_t == account_lock.st_dev
            && observed.ino() == account_lock.st_ino
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "reserved storage lock",
            ));
        }
        Ok(file)
    }
    pub fn create_temp(
        parent: &File,
        name: &OsString,
        _: &File,
        _: &OsString,
        permissions: Option<&Permissions>,
    ) -> io::Result<File> {
        // Restrict creation itself: chmod cannot revoke an already opened reader.
        let mode = permissions.map_or(0o666, |permissions| permissions.mode() & 0o777);
        regular(open(
            parent,
            name,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
            mode,
        )?)
    }
    fn stat(parent: &File, name: &OsStr) -> io::Result<Option<libc::stat>> {
        let name = c_name(name)?;
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        let result = unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result < 0 {
            let error = io::Error::last_os_error();
            return if error.kind() == io::ErrorKind::NotFound {
                Ok(None)
            } else {
                Err(error)
            };
        }
        Ok(Some(unsafe { stat.assume_init() }))
    }
    pub fn permissions(parent: &File, name: &OsString) -> io::Result<Option<Permissions>> {
        let Some(stat) = stat(parent, name)? else {
            return Ok(None);
        };
        if stat.st_mode & libc::S_IFMT != libc::S_IFREG {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected a regular file",
            ));
        }
        if let Some(account_lock) = self::stat(parent, OsStr::new("auth.json.lock"))?
            && stat.st_dev == account_lock.st_dev
            && stat.st_ino == account_lock.st_ino
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "reserved storage lock",
            ));
        }
        // mode_t is u16 on macOS and u32 on Linux.
        #[allow(clippy::unnecessary_cast)]
        let mode = stat.st_mode as u32 & 0o7777;
        Ok(Some(Permissions::from_mode(mode)))
    }
    pub fn sync_replacement(_: &File, source: &File, target: &File) -> io::Result<()> {
        let target_sync = target.sync_all();
        let source_sync = source.sync_all();
        target_sync?;
        source_sync
    }
    pub fn replace(
        _: &File,
        source_parent: &File,
        source: &OsString,
        target_parent: &File,
        target: &OsString,
    ) -> io::Result<()> {
        let source = c_name(source)?;
        let target = c_name(target)?;
        let result = unsafe {
            libc::renameat(
                source_parent.as_raw_fd(),
                source.as_ptr(),
                target_parent.as_raw_fd(),
                target.as_ptr(),
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    fn unlink(parent: &File, name: &OsStr, flags: libc::c_int) -> io::Result<()> {
        let name = c_name(name)?;
        let result = unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    /// Removes a non-directory entry; a link is removed, never followed.
    pub fn remove(parent: &File, name: &OsString) -> io::Result<()> {
        unlink(parent, name, 0)
    }
    pub fn remove_directory(parent: &File, name: &OsStr) -> io::Result<()> {
        unlink(parent, name, libc::AT_REMOVEDIR)
    }

    // The empty-file reader window cannot be sampled reliably through replace.
    // Check this security boundary directly, with umask isolated in a child.
    #[cfg(test)]
    #[test]
    fn private_temporary_file_creation() {
        const PROBE: &str = "DIM_FILE_STORAGE_TEMP_PRIVACY_PROBE";
        if let Some(directory) = std::env::var_os(PROBE) {
            unsafe { libc::umask(0o022) };
            let parent = File::open(directory).unwrap();
            assert_eq!(
                parent.metadata().unwrap().permissions().mode() & 0o777,
                0o755
            );
            for (name, permissions, expected) in [
                ("private", Some(Permissions::from_mode(0o600)), 0o600),
                ("new", None, 0o644),
            ] {
                let name = OsString::from(name);
                let mut file =
                    create_temp(&parent, &name, &parent, &name, permissions.as_ref()).unwrap();
                assert_eq!(
                    file.metadata().unwrap().permissions().mode() & 0o777,
                    expected
                );
                file.write_all(b"private contents").unwrap();
                assert_eq!(
                    file.metadata().unwrap().permissions().mode() & 0o777,
                    expected
                );
            }
            return;
        }
        let directory = std::env::temp_dir().join(format!("file-storage-{}", Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, Permissions::from_mode(0o755)).unwrap();
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "file_storage::native::private_temporary_file_creation",
                "--nocapture",
            ])
            .env(PROBE, &directory)
            .output()
            .unwrap();
        let contents = std::fs::read(directory.join("private"));
        std::fs::remove_dir_all(directory).unwrap();
        assert!(
            child.status.success(),
            "{}{}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr)
        );
        assert_eq!(contents.unwrap(), b"private contents");
    }
}

#[cfg(windows)]
pub(crate) mod windows;
#[cfg(windows)]
use windows as native;

#[cfg(not(any(unix, windows)))]
use unsupported as native;

#[cfg(not(any(unix, windows)))]
mod unsupported {
    use super::*;
    pub fn open_root(_: &Path) -> Result<File> {
        Err(FileStorageError::Unsupported)
    }
    fn unsupported() -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "file storage requires a Unix or Windows host",
        )
    }
    pub fn lock(_: &File, _: &std::ffi::OsStr, _: &Arc<FileStorage>) -> Result<PhysicalLock> {
        Err(FileStorageError::Unsupported)
    }
    pub fn parent(_: File, _: &Path) -> io::Result<(File, OsString)> {
        Err(unsupported())
    }
    pub fn read_user_file(_: &File, _: &OsString) -> io::Result<File> {
        Err(unsupported())
    }
    pub fn create_temp(
        _: &File,
        _: &OsString,
        _: &File,
        _: &OsString,
        _: Option<&Permissions>,
    ) -> io::Result<File> {
        Err(unsupported())
    }
    pub fn temporary_directory(_: &File) -> io::Result<File> {
        Err(unsupported())
    }
    pub fn permissions(_: &File, _: &OsString) -> io::Result<Option<Permissions>> {
        Err(unsupported())
    }
    pub fn replace(_: &File, _: &File, _: &OsString, _: &File, _: &OsString) -> io::Result<()> {
        Err(unsupported())
    }
    pub fn remove(_: &File, _: &OsString) -> io::Result<()> {
        Err(unsupported())
    }
    pub type Identity = ();
    pub fn open_directory(_: &File, _: &OsStr) -> io::Result<File> {
        Err(unsupported())
    }
    pub fn subdirectory(_: &File, _: &OsStr) -> io::Result<File> {
        Err(unsupported())
    }
    pub fn reserved_directory(_: &File) -> io::Result<Option<File>> {
        Err(unsupported())
    }
    pub fn open_key(_: &File, _: &OsStr) -> io::Result<File> {
        Err(unsupported())
    }
    pub fn entry_kind(_: &File, _: &OsStr) -> io::Result<Option<EntryKind>> {
        Err(unsupported())
    }
    pub fn reserved(_: &File) -> io::Result<Vec<Identity>> {
        Err(unsupported())
    }
    pub fn entries(_: &File) -> io::Result<Vec<(DirectoryEntry, Identity)>> {
        Err(unsupported())
    }
    pub fn remove_directory(_: &File, _: &OsStr) -> io::Result<()> {
        Err(unsupported())
    }
    pub fn same_identity(_: &File, _: &File) -> io::Result<bool> {
        Err(unsupported())
    }
    pub fn create_account_file(_: &File, _: &OsStr) -> io::Result<File> {
        Err(unsupported())
    }
    pub fn read_account_file(_: &File, _: &OsString) -> io::Result<File> {
        Err(unsupported())
    }
    pub fn remove_account(_: &File, _: &OsString, _: &File) -> io::Result<()> {
        Err(unsupported())
    }
    pub fn process_alive(_: u32) -> io::Result<bool> {
        Err(unsupported())
    }
    pub fn target_name(_: &File, _: &OsString) -> io::Result<OsString> {
        Err(unsupported())
    }
    pub fn sync_replacement(_: &File, _: &File, _: &File) -> io::Result<()> {
        Err(unsupported())
    }
}
