//! The file tools `read`, `write`, `edit`, `glob` and `grep`.
//!
//! Behavior, texts, error codes and limits follow the TS tools
//! (`packages/agent/src/tool/builtin/` and `packages/plugin/src/builtin/grep-glob/`).
//! Workspace I/O runs on blocking threads with `std::fs`.

#![expect(
    clippy::result_large_err,
    reason = "the `Err` variant is `ToolResult`, the model-facing failure reply; boxing it would change the tools' contract"
)]

mod codec;
mod edit;
mod read;
mod search;
mod write;

use super::file_reads::FileReads;
use crate::host::HostError;
use dim_agent_types::{JsonObject, ToolCall, ToolDefinition, ToolEffect, ToolResult};
use serde_json::{Value, json};
use sha2::Digest;
use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::sync::OwnedMutexGuard;
use tokio_util::sync::CancellationToken;

const TOOL_NAMES: [&str; 5] = ["read", "write", "edit", "glob", "grep"];
const CANCELLED: &str = "Tool execution cancelled.";

/// The process umask as the last [`install_process_umask`] read it.
#[cfg(target_os = "macos")]
static PROCESS_UMASK: std::sync::OnceLock<u32> = std::sync::OnceLock::new();

/// Records the process umask for the new files `write` creates.
///
/// macOS has no `getumask` on the supported baseline, so each product entry
/// calls this once from `main`, before its runtime starts any thread: the
/// read-modify-write below touches one process-wide value. A test or library
/// caller that does not call it keeps the 0600 staging mode for new files. The
/// first call wins; Linux reads `/proc/self/status` instead.
#[cfg(target_os = "macos")]
pub fn install_process_umask() {
    // SAFETY: `umask` has no preconditions; the value is restored immediately.
    let umask = unsafe {
        let current = libc::umask(0o077);
        libc::umask(current);
        current
    };
    let _ = PROCESS_UMASK.set(umask.into());
}

/// The umask a direct creation would apply. Linux reads it from the process
/// status; macOS reports the value captured at startup. `None` means no value
/// is available, and a new file keeps its 0600 staging mode.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn creation_umask() -> Option<u32> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/self/status")
            .ok()?
            .lines()
            .find_map(|line| line.strip_prefix("Umask:"))
            .and_then(|value| u32::from_str_radix(value.trim(), 8).ok())
    }
    #[cfg(target_os = "macos")]
    {
        PROCESS_UMASK.get().copied()
    }
}

/// Session-level executor of the file tools. Bindings belong to the calling
/// branch and are passed per call. Product Sessions share a process-owned
/// coordinator; standalone callers may use an isolated coordinator.
pub struct FileTools {
    cwd: PathBuf,
    home: PathBuf,
    /// Session output directory; large `read` results are written here.
    output_dir: PathBuf,
    /// The root boundary a host configured for this session's file tools;
    /// `None` keeps the unbounded behaviour.
    boundary: Option<FileToolBoundary>,
    locks: Arc<FileWriteCoordinator>,
    #[cfg(test)]
    /// Runs after a mutation's target resolved and before the per-path
    /// coordinator is awaited; no sleeps, production hooks or global test
    /// locks. A test holds the coordinator and swaps the path here.
    before_lock: Option<ResolveHook>,
    #[cfg(test)]
    /// Runs after a complete sibling is synced and before commit verification;
    /// no sleeps, production hooks or global test locks.
    before_commit: Option<CommitHook>,
    #[cfg(test)]
    /// Runs inside a bounded `grep` walk for each walked entry, before the
    /// entry is read; no sleeps, production hooks or global test locks. A
    /// test swaps a directory here.
    before_search: Option<SearchHook>,
}

#[cfg(test)]
type CommitHook = Arc<dyn Fn(&Path) + Send + Sync>;
#[cfg(test)]
type ResolveHook = Arc<dyn Fn(&Path) + Send + Sync>;
#[cfg(test)]
type SearchHook = Arc<dyn Fn(&Path) + Send + Sync>;

impl FileTools {
    pub fn new(cwd: PathBuf, home: PathBuf, output_dir: PathBuf) -> Self {
        Self::with_coordinator(cwd, home, output_dir, Arc::default(), None)
    }

    /// [`new`](Self::new) with the root boundary resolved file paths must
    /// stay inside. The boundary's home is the home `~` resolves against.
    pub fn with_boundary(cwd: PathBuf, output_dir: PathBuf, boundary: FileToolBoundary) -> Self {
        Self::with_coordinator(
            cwd,
            boundary.home.clone(),
            output_dir,
            Arc::default(),
            Some(boundary),
        )
    }

    pub fn with_coordinator(
        cwd: PathBuf,
        home: PathBuf,
        output_dir: PathBuf,
        locks: Arc<FileWriteCoordinator>,
        boundary: Option<FileToolBoundary>,
    ) -> Self {
        Self {
            cwd,
            home,
            output_dir,
            boundary,
            locks,
            #[cfg(test)]
            before_lock: None,
            #[cfg(test)]
            before_commit: None,
            #[cfg(test)]
            before_search: None,
        }
    }

    pub fn definitions() -> Vec<ToolDefinition> {
        [
            read::definition(),
            write::definition(),
            edit::definition(),
            search::glob_definition(),
            search::grep_definition(),
        ]
        .into_iter()
        .map(|value| serde_json::from_value(value).expect("built-in tool definition is valid"))
        .collect()
    }

    pub fn handles(name: &str) -> bool {
        TOOL_NAMES.contains(&name)
    }

    /// The home `~` resolves against: the boundary's when one is configured,
    /// the platform's otherwise.
    pub(crate) fn home(&self) -> &Path {
        &self.home
    }

    pub async fn execute(
        &self,
        call: &ToolCall,
        reads: &FileReads,
        cancel: CancellationToken,
    ) -> Result<ToolResult, HostError> {
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        let input = &call.input;
        match call.name.as_str() {
            "read" => read::run(self, &call.id, input, reads, cancel).await,
            "write" => write::run(self, input, reads, cancel).await,
            "edit" => edit::run(self, input, reads, cancel).await,
            "glob" => search::glob(self, input, cancel).await,
            "grep" => search::grep(self, input, cancel).await,
            name => Err(HostError::failed(format!("unknown file tool: {name}"))),
        }
    }
}

pub(crate) fn cancelled() -> HostError {
    HostError::Cancelled(CANCELLED.into())
}

/// Cancellation observed on a blocking thread before any write began.
struct Cancelled;
impl Cancelled {
    fn check(cancel: &CancellationToken) -> Result<(), Cancelled> {
        if cancel.is_cancelled() {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }
}

/// Whether a resolved path is admitted for reading or writing: reads may
/// enter the workspace, the configured home and the read-only roots; writes
/// only the workspace.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Access {
    Read,
    Write,
}

/// The one tilde rule of the file tools: `~`, `~/…` and `~\…` expand
/// against `home`; `None` when the input does not start with a tilde.
///
/// `resolve_path` always applies it. `glob` and `grep` apply it only when a
/// boundary is configured: without one the product has always taken `~` as
/// a literal component under the cwd (`<cwd>/~`, `resolve_scope` in
/// `files/search.rs` on main), and that behaviour is kept (ADR 014).
fn expand_tilde(input: &str, home: &Path) -> Option<PathBuf> {
    if input == "~" {
        return Some(home.to_owned());
    }
    input
        .strip_prefix("~/")
        .or_else(|| input.strip_prefix("~\\"))
        .map(|rest| home.join(rest))
}

/// Resolves a model-supplied file path (TS `path.ts`, design §7.2): `~`
/// expansion, Windows corrections, join onto cwd, lexical `.`/`..`
/// removal, then realpath of the longest existing prefix. With `boundary`
/// configured, a resolved path outside the roots is refused for `Access::Write`
/// (workspace only) or for `Access::Read` (workspace, home and the read-only
/// roots); the check compares the canonical path, so a symlink out of the
/// roots is refused as well.
pub(crate) fn resolve_path(
    cwd: &Path,
    home: &Path,
    boundary: Option<&FileToolBoundary>,
    access: Access,
    input: &str,
) -> Result<PathBuf, PathError> {
    let expanded = match expand_tilde(input, home) {
        Some(path) => path,
        None => windows_path(input)?,
    };
    match canonical_prefix(lexical(&cwd.join(expanded))) {
        Ok(path) if boundary.is_some_and(|boundary| !inside(boundary, &path, access)) => {
            Err(PathError::Refused(path))
        }
        Ok(path) => Ok(path),
        Err(message) => Err(PathError::Message(message)),
    }
}

/// Why a resolved file path failed.
pub(crate) enum PathError {
    /// The canonical path lies outside the configured roots.
    Refused(PathBuf),
    Message(String),
}
impl std::fmt::Display for PathError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(path) => write!(
                formatter,
                "Path is outside the configured file roots: {}",
                path.display()
            ),
            Self::Message(message) => formatter.write_str(message),
        }
    }
}
impl From<String> for PathError {
    fn from(message: String) -> Self {
        Self::Message(message)
    }
}
impl From<PathError> for String {
    fn from(error: PathError) -> Self {
        error.to_string()
    }
}

/// The roots a host configures for a session's file tools: they bound the
/// model's file-tool calls — `read`, `write`, `edit`, `glob` and `grep` —
/// when a host configures them. Content reads and mutations are
/// handle-verified: `read`, each file `grep` matches, and `write`/`edit`
/// act on the target's parent opened through a symlink-free walk, so a path
/// swapped after admission fails closed. `glob` returns names only; each
/// result's canonical path is re-checked against the roots before it is
/// returned and a failed one is dropped. The roots make no sandbox claim:
/// `exec` is bounded by the sandbox (ADR 010 owns it). Client-initiated
/// file paths — `@` mention expansion and the App Server's
/// `session.files.*` methods — are outside the roots, because the model
/// cannot make those calls: in a run it has only its tools, and the run's
/// App Server socket is outside every root and unreachable from a sandboxed
/// command. The paths are absolute and canonical: the evaluation harness
/// resolves them from directories it created, and a root reached through a
/// symlink would answer `starts_with` with its lexical path. The product
/// passes `None`, so its behaviour does not change (ADR 014 keeps deciding
/// file access for it).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FileToolBoundary {
    /// The session's workspace: reads and writes. A model's cwd is inside
    /// it.
    pub workspace: PathBuf,
    /// The home `~` resolves against and reads are allowed in; writes are
    /// not. An evaluation run points it at the run's disposable home.
    pub home: PathBuf,
    /// The read-only system roots a toolchain needs (the platform's fixed
    /// roots, ADR 014): reads only.
    pub read_only_roots: Vec<PathBuf>,
}

impl FileToolBoundary {
    /// The absolute canonical form of each root: a boundary built from
    /// directories the caller created, as the evaluation harness does.
    pub fn canonicalize(&self) -> std::io::Result<Self> {
        Ok(Self {
            workspace: std::fs::canonicalize(&self.workspace)?,
            home: std::fs::canonicalize(&self.home)?,
            read_only_roots: self
                .read_only_roots
                .iter()
                .map(std::fs::canonicalize)
                .collect::<std::io::Result<_>>()?,
        })
    }
}

/// Whether a canonicalized path lies inside the roots for the access: reads
/// in the workspace, the home and the read-only roots; writes only in the
/// workspace.
fn inside(boundary: &FileToolBoundary, path: &Path, access: Access) -> bool {
    if path.starts_with(&boundary.workspace) {
        true
    } else if access == Access::Write {
        false
    } else {
        path.starts_with(&boundary.home)
            || boundary
                .read_only_roots
                .iter()
                .any(|root| path.starts_with(root))
    }
}

/// The refusal for a resolved path outside the configured roots.
pub(crate) fn refused(path: &Path) -> ToolResult {
    failed(
        format!(
            "Path is outside the configured file roots: {}",
            path.display()
        ),
        json!({
            "code": "outside_file_roots",
            "path": path.to_string_lossy(),
        }),
    )
}

/// SHA-256 of raw file bytes.
fn revision(bytes: &[u8]) -> [u8; 32] {
    sha2::Sha256::digest(bytes).into()
}

/// Runs blocking file-system work off the async runtime.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, HostError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| HostError::failed(format!("file tool task failed: {error}")))
}

/// TS `failedResult`: the message is the model-visible content, the code is
/// structured.
fn failed(message: impl Into<String>, structured: Value) -> ToolResult {
    ToolResult {
        is_error: Some(true),
        structured_content: Some(object(structured)),
        ..ToolResult::text(message)
    }
}

fn invalid_input(message: impl Into<String>) -> ToolResult {
    failed(message, json!({ "code": "invalid_input" }))
}

fn succeeded(content: impl Into<String>, structured: Value) -> ToolResult {
    ToolResult {
        structured_content: Some(object(structured)),
        ..ToolResult::text(content)
    }
}

/// A successful `write` or `edit` result: the published file is the call's
/// `fileWritten` effect (session boundary A.6).
fn wrote(content: impl Into<String>, structured: Value, path: &str) -> ToolResult {
    ToolResult {
        structured_content: Some(object(structured)),
        effects: Some(vec![ToolEffect::FileWritten {
            path: path.to_owned(),
        }]),
        ..ToolResult::text(content)
    }
}

fn object(value: Value) -> JsonObject {
    match value {
        Value::Object(map) => map,
        _ => unreachable!("structured content is an object"),
    }
}

/// A present, non-null input field.
fn field<'a>(input: &'a JsonObject, key: &str) -> Option<&'a Value> {
    input.get(key).filter(|value| !value.is_null())
}

/// Length in UTF-16 code units, the unit of every TS character limit.
fn js_len(text: &str) -> usize {
    text.encode_utf16().count()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// TS `correctWindowsPath`, plus the ADR agent-020 rejection of
/// drive-relative paths.
#[cfg(windows)]
fn windows_path(input: &str) -> Result<PathBuf, String> {
    let corrected = if let Some(rest) = input.strip_prefix("/cygdrive/")
        && let Some(drive) = rest.chars().next().filter(char::is_ascii_alphabetic)
        && (rest.len() == 1 || rest.as_bytes()[1] == b'/')
    {
        let tail = rest.get(2..).unwrap_or("");
        format!(
            "{}:\\{}",
            drive.to_ascii_uppercase(),
            tail.replace('/', "\\")
        )
    } else if input.len() >= 3
        && input.as_bytes()[0] == b'/'
        && input.as_bytes()[1].is_ascii_alphabetic()
        && input.as_bytes()[2] == b':'
    {
        format!("{}{}", &input[1..3], input[3..].replace('/', "\\"))
    } else {
        input.to_owned()
    };
    let path = PathBuf::from(corrected);
    if matches!(path.components().next(), Some(Component::Prefix(_))) && !path.has_root() {
        return Err(format!(
            "Drive-relative path is not supported: {input}. Use an absolute path such as C:\\dir\\file."
        ));
    }
    Ok(path)
}
#[cfg(not(windows))]
fn windows_path(input: &str) -> Result<PathBuf, String> {
    Ok(PathBuf::from(input))
}

/// Node `path.resolve` normalization: drops `.` and folds `..`.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// Realpath of the longest existing prefix with the missing rest appended.
pub(crate) fn canonical_prefix(path: PathBuf) -> Result<PathBuf, String> {
    let mut existing = path.as_path();
    let mut missing = Vec::new();
    loop {
        match std::fs::canonicalize(existing) {
            Ok(mut canonical) => {
                canonical.extend(missing.iter().rev());
                return Ok(canonical);
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && std::fs::symlink_metadata(existing)
                        .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound) => {}
            Err(error) => return Err(format!("Failed to resolve {}: {error}", existing.display())),
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name.to_owned());
                existing = parent;
            }
            _ => return Err(format!("Cannot resolve {}", path.display())),
        }
    }
}

/// Opens an existing target without following a final symlink.
#[cfg_attr(
    windows,
    expect(unused_mut, reason = "only the unix branch sets the open flags")
)]
fn open_options() -> std::fs::OpenOptions {
    let mut options = std::fs::OpenOptions::new();
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(&mut options, libc::O_NOFOLLOW);
    options
}

/// The target exists but is not a regular file (a FIFO, device or socket).
pub(crate) struct NotRegular;

/// Opens an existing regular file for reading. The open never blocks (a FIFO
/// without a writer would block it, uncancellably), and the type is checked
/// on the opened handle, so a swap after a path check cannot slip through.
fn open_regular(path: &Path) -> std::io::Result<Result<std::fs::File, NotRegular>> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(
        &mut options,
        libc::O_NOFOLLOW | libc::O_NONBLOCK,
    );
    let file = options.open(path)?;
    Ok(if file.metadata()?.is_file() {
        Ok(file)
    } else {
        Err(NotRegular)
    })
}

fn not_regular(display: &str) -> ToolResult {
    failed(
        format!("Not a regular file: {display}"),
        json!({ "code": "not_regular_file" }),
    )
}

/// Whether the file at `path` is the opened file (the staged sibling has not
/// been swapped).
fn owned_temporary(path: &Path, owned: &std::fs::File) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (owned.metadata(), std::fs::symlink_metadata(path)) {
            (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
            _ => false,
        }
    }
    #[cfg(windows)]
    {
        open_regular(path)
            .ok()
            .and_then(Result::ok)
            .is_some_and(|current| write::same_identity(owned, &current).unwrap_or(false))
    }
}

/// The size class of a target entry, read without following a final symlink.
pub(crate) struct Stat {
    /// A directory sits at the path.
    pub(crate) directory: bool,
    /// The entry's size in bytes.
    pub(crate) len: u64,
}

/// Why an operation on an admitted target failed.
pub(crate) enum TargetError {
    /// A component met at I/O time is a symlink or not a directory: the path
    /// changed after admission, so the call fails closed.
    Refused,
    /// Any other I/O failure.
    Io(std::io::Error),
}
impl From<std::io::Error> for TargetError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Where a file tool's I/O acts for one admitted path.
///
/// `Path` is the unbounded behaviour (no boundary configured): the
/// path-based calls the tools always made. `Handle` is the bounded
/// behaviour: the target's parent directory, opened at I/O time through a
/// symlink-free walk — after the per-path coordinator is held for a
/// mutation — with every operation of the call running relative to that
/// handle (the stat, the opens, the staged sibling, the install and the
/// parent fsync). A parent component swapped for a symlink after admission
/// therefore fails closed with the boundary refusal instead of redirecting
/// the I/O outside the roots. The admitted canonical path has no symlinks by
/// construction, so a symlink the walk meets means the path changed after
/// admission.
pub(crate) enum Target {
    Path(PathBuf),
    #[cfg(unix)]
    Handle(Parent),
}

#[cfg(unix)]
pub(crate) struct Parent {
    path: PathBuf,
    /// The parent directory, opened one component at a time.
    dir: std::fs::File,
    /// The target's name inside `dir`.
    name: std::ffi::OsString,
}

#[cfg(unix)]
fn c_name(name: &std::ffi::OsStr) -> std::io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(name.as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "a path component holds a NUL byte",
        )
    })
}

/// Opens `name` below `parent` without following a symlink.
#[cfg(unix)]
fn open_relative(
    parent: &std::fs::File,
    name: &std::ffi::OsStr,
    flags: libc::c_int,
    mode: libc::c_uint,
) -> std::io::Result<std::fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let name = c_name(name)?;
    // SAFETY: `parent` is an open directory handle for the duration of the
    // call and `name` is a live NUL-terminated string.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            mode,
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        // SAFETY: `fd` is non-negative (checked above) and was just returned
        // by `openat`, so this takes ownership of a fresh descriptor no other
        // owner holds.
        Ok(unsafe { std::fs::File::from_raw_fd(fd) })
    }
}

/// A walk that met a symlink (`ELOOP`) or a non-directory (`ENOTDIR`) is the
/// path-changed-after-admission refusal; every other error is I/O.
#[cfg(unix)]
fn classify(error: std::io::Error) -> TargetError {
    match error.raw_os_error() {
        Some(libc::ELOOP | libc::ENOTDIR) => TargetError::Refused,
        _ => TargetError::Io(error),
    }
}

/// Creates the named 0600 staging sibling below `dir`.
#[cfg(unix)]
fn create_staged_at(dir: &std::fs::File, name: &std::ffi::OsStr) -> std::io::Result<std::fs::File> {
    open_relative(
        dir,
        name,
        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        0o600,
    )
}

/// Whether the entry `name` below `dir` still is the open regular file `file`.
#[cfg(unix)]
fn staged_owned_at(dir: &std::fs::File, name: &std::ffi::OsStr, file: &std::fs::File) -> bool {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::MetadataExt;
    let (Ok(metadata), Ok(name)) = (file.metadata(), c_name(name)) else {
        return false;
    };
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: the handle and the NUL-terminated name are live for the call,
    // and `stat` is a valid uninitialized `libc::stat` buffer.
    let result = unsafe {
        libc::fstatat(
            dir.as_raw_fd(),
            name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result < 0 {
        return false;
    }
    // SAFETY: `fstatat` returned 0 (checked above), so it initialized the
    // struct.
    let stat = unsafe { stat.assume_init() };
    #[expect(
        clippy::unnecessary_cast,
        reason = "`st_dev` is 32-bit on macOS and 64-bit on Linux, so its cast is needed on one and lints on the other; `st_ino` is 64-bit on both"
    )]
    let (device, inode) = (stat.st_dev as u64, stat.st_ino as u64);
    stat.st_mode & libc::S_IFMT == libc::S_IFREG
        && (device, inode) == (metadata.dev(), metadata.ino())
}

/// Removes the entry `name` below `dir`, ignoring a failure.
#[cfg(unix)]
fn remove_staged_at(dir: &std::fs::File, name: &std::ffi::OsStr) {
    use std::os::fd::AsRawFd;
    if let Ok(name) = c_name(name) {
        // SAFETY: the handle and the NUL-terminated name are live for the
        // call.
        unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), 0) };
    }
}

/// Links `staged` as `name` below `dir`, without replacing an entry.
#[cfg(unix)]
fn link_new_at(
    dir: &std::fs::File,
    staged: &std::ffi::OsStr,
    name: &std::ffi::OsStr,
) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    let staged = c_name(staged)?;
    let name = c_name(name)?;
    // SAFETY: both names are live NUL-terminated strings and `dir` is an open
    // directory handle for the call.
    let result = unsafe {
        libc::linkat(
            dir.as_raw_fd(),
            staged.as_ptr(),
            dir.as_raw_fd(),
            name.as_ptr(),
            0,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(unix)]
impl Parent {
    /// Walks the parent components from `/`, each opened `O_DIRECTORY |
    /// O_NOFOLLOW` relative to the previous handle. With `create`, a missing
    /// component is made relative to the held handle (the admitted path's
    /// tail may not exist yet for a write).
    fn open(path: &Path, create: bool) -> Result<Self, TargetError> {
        use std::os::fd::AsRawFd;
        let invalid = |message| {
            TargetError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                message,
            ))
        };
        let name = path
            .file_name()
            .ok_or_else(|| invalid("a file target names a file"))?
            .to_owned();
        let parent = path
            .parent()
            .ok_or_else(|| invalid("a file target has a parent directory"))?;
        let mut dir = std::fs::File::open("/").map_err(TargetError::Io)?;
        for component in parent.components() {
            let part = match component {
                Component::RootDir => continue,
                Component::Normal(part) => part,
                _ => return Err(TargetError::Refused),
            };
            dir = match open_relative(&dir, part, libc::O_RDONLY | libc::O_DIRECTORY, 0) {
                Ok(next) => next,
                Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {
                    let part_name = c_name(part)?;
                    // SAFETY: `dir` is an open directory handle and
                    // `part_name` a live NUL-terminated string for the call.
                    if unsafe { libc::mkdirat(dir.as_raw_fd(), part_name.as_ptr(), 0o777) } != 0 {
                        let error = std::io::Error::last_os_error();
                        if error.kind() != std::io::ErrorKind::AlreadyExists {
                            return Err(classify(error));
                        }
                    }
                    open_relative(&dir, part, libc::O_RDONLY | libc::O_DIRECTORY, 0)
                        .map_err(classify)?
                }
                Err(error) => return Err(classify(error)),
            };
        }
        Ok(Self {
            path: path.to_owned(),
            dir,
            name,
        })
    }

    fn stat(&self) -> Result<Option<Stat>, TargetError> {
        use std::os::fd::AsRawFd;
        let name = c_name(&self.name)?;
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: the handle and the NUL-terminated name are live for the
        // call, and `stat` is a valid uninitialized `libc::stat` buffer.
        let result = unsafe {
            libc::fstatat(
                self.dir.as_raw_fd(),
                name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            return match error.kind() {
                std::io::ErrorKind::NotFound => Ok(None),
                _ => Err(classify(error)),
            };
        }
        // SAFETY: `fstatat` returned 0 above, so it initialized the struct.
        let stat = unsafe { stat.assume_init() };
        if stat.st_mode & libc::S_IFMT == libc::S_IFLNK {
            return Err(TargetError::Refused);
        }
        Ok(Some(Stat {
            directory: stat.st_mode & libc::S_IFMT == libc::S_IFDIR,
            len: stat.st_size as u64,
        }))
    }

    fn open_read(&self) -> Result<Result<std::fs::File, NotRegular>, TargetError> {
        let file = open_relative(&self.dir, &self.name, libc::O_RDONLY, 0).map_err(classify)?;
        Ok(if file.metadata()?.is_file() {
            Ok(file)
        } else {
            Err(NotRegular)
        })
    }

    fn open_write(&self) -> Result<std::fs::File, TargetError> {
        open_relative(&self.dir, &self.name, libc::O_WRONLY, 0).map_err(classify)
    }

    fn create_staged(&self, name: &std::ffi::OsStr) -> Result<std::fs::File, TargetError> {
        create_staged_at(&self.dir, name).map_err(classify)
    }

    fn staged_owned(&self, name: &std::ffi::OsStr, file: &std::fs::File) -> bool {
        staged_owned_at(&self.dir, name, file)
    }

    fn remove_staged(&self, name: &std::ffi::OsStr) {
        remove_staged_at(&self.dir, name)
    }

    fn install(&self, staged: &std::ffi::OsStr, existing: bool) -> Result<(), TargetError> {
        use std::os::fd::AsRawFd;
        if existing {
            let staged = c_name(staged)?;
            let name = c_name(&self.name)?;
            // SAFETY: both names are live NUL-terminated strings and
            // `self.dir` is an open directory handle for the call.
            let result = unsafe {
                libc::renameat(
                    self.dir.as_raw_fd(),
                    staged.as_ptr(),
                    self.dir.as_raw_fd(),
                    name.as_ptr(),
                )
            };
            if result == 0 {
                return Ok(());
            }
            return Err(TargetError::Io(std::io::Error::last_os_error()));
        }
        link_new_at(&self.dir, staged, &self.name).map_err(TargetError::Io)
    }

    fn final_regular(&self) -> bool {
        use std::os::fd::AsRawFd;
        let Ok(name) = c_name(&self.name) else {
            return false;
        };
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: the handle and the NUL-terminated name are live for the
        // call, and `stat` is a valid uninitialized `libc::stat` buffer.
        let result = unsafe {
            libc::fstatat(
                self.dir.as_raw_fd(),
                name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        // SAFETY: the `fstatat` above returned 0 (the left conjunct), so it
        // initialized the struct.
        result == 0 && unsafe { stat.assume_init() }.st_mode & libc::S_IFMT == libc::S_IFREG
    }
}

impl Target {
    pub(crate) fn path(&self) -> &Path {
        match self {
            Self::Path(path) => path,
            #[cfg(unix)]
            Self::Handle(parent) => &parent.path,
        }
    }

    /// The target's final name inside its parent.
    #[cfg(target_os = "linux")]
    fn name(&self) -> Result<&std::ffi::OsStr, TargetError> {
        match self {
            Self::Path(path) => path.file_name().ok_or_else(|| {
                TargetError::Io(std::io::Error::from(std::io::ErrorKind::InvalidInput))
            }),
            Self::Handle(parent) => Ok(&parent.name),
        }
    }

    /// The target of a read: with a boundary, the parent walk must already
    /// exist; a missing component reads as "not found".
    pub(crate) fn for_read(path: &Path, bounded: bool) -> Result<Self, TargetError> {
        if !bounded {
            return Ok(Self::Path(path.to_owned()));
        }
        #[cfg(unix)]
        {
            Ok(Self::Handle(Parent::open(path, false)?))
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Err(TargetError::Refused)
        }
    }

    /// The target of a mutation: with a boundary, missing parent components
    /// are created relative to the walked handle when `create`.
    pub(crate) fn for_mutation(
        path: &Path,
        bounded: bool,
        create: bool,
    ) -> Result<Self, TargetError> {
        if !bounded {
            return Ok(Self::Path(path.to_owned()));
        }
        #[cfg(unix)]
        {
            Ok(Self::Handle(Parent::open(path, create)?))
        }
        #[cfg(not(unix))]
        {
            let (_, _) = (path, create);
            Err(TargetError::Refused)
        }
    }

    pub(crate) fn stat(&self) -> Result<Option<Stat>, TargetError> {
        match self {
            Self::Path(path) => match std::fs::metadata(path) {
                Ok(metadata) => Ok(Some(Stat {
                    directory: metadata.is_dir(),
                    len: metadata.len(),
                })),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(TargetError::Io(error)),
            },
            #[cfg(unix)]
            Self::Handle(parent) => parent.stat(),
        }
    }

    pub(crate) fn open_regular(&self) -> Result<Result<std::fs::File, NotRegular>, TargetError> {
        match self {
            Self::Path(path) => open_regular(path).map_err(TargetError::Io),
            #[cfg(unix)]
            Self::Handle(parent) => parent.open_read(),
        }
    }

    /// Opens the existing target for writing, without following a final
    /// symlink.
    pub(crate) fn open_writable(&self) -> Result<std::fs::File, TargetError> {
        match self {
            Self::Path(path) => {
                let mut options = open_options();
                #[cfg(unix)]
                std::os::unix::fs::OpenOptionsExt::custom_flags(
                    &mut options,
                    libc::O_NOFOLLOW | libc::O_NONBLOCK,
                );
                options.write(true).open(path).map_err(TargetError::Io)
            }
            #[cfg(unix)]
            Self::Handle(parent) => parent.open_write(),
        }
    }

    pub(crate) fn create_staged(
        &self,
        name: &std::ffi::OsStr,
    ) -> Result<std::fs::File, TargetError> {
        match self {
            Self::Path(path) => {
                #[cfg(windows)]
                {
                    let parent =
                        crate::file_storage::windows::open_root(path.parent().ok_or_else(
                            || TargetError::Io(std::io::Error::other("a target has a parent")),
                        )?)
                        .map_err(|error| TargetError::Io(std::io::Error::other(error)))?;
                    let file = crate::file_storage::windows::create_temp(
                        &parent,
                        &name.to_owned(),
                        &parent,
                        &path
                            .file_name()
                            .ok_or_else(|| {
                                TargetError::Io(std::io::Error::other("a target names a file"))
                            })?
                            .to_owned(),
                        None,
                    );
                    return file.map_err(TargetError::Io);
                }
                #[cfg(not(windows))]
                {
                    let mut options = open_options();
                    options.write(true).create_new(true);
                    #[cfg(unix)]
                    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
                    options
                        .open(path.with_file_name(name))
                        .map_err(TargetError::Io)
                }
            }
            #[cfg(unix)]
            Self::Handle(parent) => parent.create_staged(name),
        }
    }

    pub(crate) fn staged_owned(&self, name: &std::ffi::OsStr, file: &std::fs::File) -> bool {
        match self {
            Self::Path(path) => owned_temporary(&path.with_file_name(name), file),
            #[cfg(unix)]
            Self::Handle(parent) => parent.staged_owned(name, file),
        }
    }

    pub(crate) fn remove_staged(&self, name: &std::ffi::OsStr) {
        match self {
            Self::Path(path) => {
                let _ = std::fs::remove_file(path.with_file_name(name));
            }
            #[cfg(unix)]
            Self::Handle(parent) => parent.remove_staged(name),
        }
    }

    /// Installs the staged sibling as the target: a rename over an existing
    /// file, a link for a new one.
    pub(crate) fn install(
        &self,
        staged: &std::ffi::OsStr,
        file: &std::fs::File,
        existing: bool,
    ) -> Result<(), TargetError> {
        let _ = file;
        match self {
            Self::Path(path) => {
                #[cfg(windows)]
                if existing {
                    let parent =
                        crate::file_storage::windows::open_root(path.parent().ok_or_else(
                            || TargetError::Io(std::io::Error::other("a target has a parent")),
                        )?)
                        .map_err(|error| TargetError::Io(std::io::Error::other(error)))?;
                    return crate::file_storage::windows::replace(
                        file,
                        &parent,
                        &staged.to_owned(),
                        &parent,
                        &path
                            .file_name()
                            .ok_or_else(|| {
                                TargetError::Io(std::io::Error::other("a target names a file"))
                            })?
                            .to_owned(),
                    )
                    .map_err(TargetError::Io);
                }
                let result = if existing {
                    std::fs::rename(path.with_file_name(staged), path)
                } else {
                    std::fs::hard_link(path.with_file_name(staged), path)
                };
                result.map_err(TargetError::Io)
            }
            #[cfg(unix)]
            Self::Handle(parent) => parent.install(staged, existing),
        }
    }

    /// Flushes the installed directory entry to disk.
    pub(crate) fn sync_parent(&self) -> Result<(), TargetError> {
        match self {
            Self::Path(path) => {
                std::fs::File::open(path.parent().expect("resolved target has parent"))
                    .and_then(|parent| parent.sync_all())
                    .map_err(TargetError::Io)
            }
            #[cfg(unix)]
            Self::Handle(parent) => parent.dir.sync_all().map_err(TargetError::Io),
        }
    }

    /// Re-checks under the preparation that the target is still a regular
    /// file, not a symlink.
    pub(crate) fn final_regular(&self) -> bool {
        match self {
            Self::Path(path) => std::fs::symlink_metadata(path)
                .is_ok_and(|metadata| metadata.is_file() && !metadata.is_symlink()),
            #[cfg(unix)]
            Self::Handle(parent) => parent.final_regular(),
        }
    }
}

/// A new file's preparation and publication, bound to one parent directory.
///
/// On Linux the parent is opened once, before anything is created, and every
/// step runs relative to that handle: the anonymous `O_TMPFILE` inode, the
/// default-ACL probe, the named sibling, the publication and the directory
/// sync. A rename of the target's parent after that cannot publish the file
/// into a directory other than the one whose mode, ACL and group it inherited.
/// The other Unix platforms have no anonymous inode and no ACL probe to keep
/// together, and keep acting on the target's path.
#[cfg(unix)]
pub(crate) struct NewFile<'a> {
    target: &'a Target,
    #[cfg(target_os = "linux")]
    parent: std::fs::File,
}

#[cfg(unix)]
impl<'a> NewFile<'a> {
    /// Opens the parent once: `O_PATH` for a path target, or a duplicate of
    /// the bounded target's held handle. Creating and publishing an entry
    /// needs write and search permission, not read, so a directory that
    /// cannot be read still opens; if it cannot be opened at all the write
    /// fails before anything is staged rather than publishing by path.
    pub(crate) fn new(target: &'a Target) -> Result<Self, TargetError> {
        #[cfg(target_os = "linux")]
        let parent = match target {
            Target::Path(path) => {
                let parent = path.parent().ok_or_else(|| {
                    TargetError::Io(std::io::Error::from(std::io::ErrorKind::InvalidInput))
                })?;
                let mut options = std::fs::OpenOptions::new();
                options.read(true);
                std::os::unix::fs::OpenOptionsExt::custom_flags(
                    &mut options,
                    libc::O_PATH | libc::O_DIRECTORY,
                );
                options.open(parent).map_err(TargetError::Io)?
            }
            Target::Handle(parent) => parent.dir.try_clone().map_err(TargetError::Io)?,
        };
        Ok(Self {
            target,
            #[cfg(target_os = "linux")]
            parent,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        self.target.path()
    }

    /// Creates an anonymous `O_TMPFILE` inode in the held parent for a new
    /// file, or `None` when the filesystem or the process cannot publish one
    /// through `/proc/self/fd`: the caller then stages a named sibling. `0o666`
    /// lets the kernel settle the mode, the parent's POSIX default ACL and the
    /// `setgid` group exactly as a direct creation, with no directory entry
    /// until publication.
    #[cfg(target_os = "linux")]
    pub(crate) fn create_anonymous(&self) -> Result<Option<std::fs::File>, TargetError> {
        use std::os::fd::{AsRawFd, FromRawFd};
        #[cfg(any(test, feature = "testing"))]
        if testing::named_staging_forced(self.path()) {
            return Ok(None);
        }
        if !Path::new("/proc/self/fd").is_dir() {
            return Ok(None);
        }
        // SAFETY: `self.parent` is an open directory handle and the path is a
        // live NUL-terminated string for the call.
        let fd = unsafe {
            libc::openat(
                self.parent.as_raw_fd(),
                c".".as_ptr(),
                libc::O_TMPFILE | libc::O_WRONLY | libc::O_CLOEXEC,
                0o666,
            )
        };
        if fd < 0 {
            let error = std::io::Error::last_os_error();
            return match error.raw_os_error() {
                Some(libc::EOPNOTSUPP | libc::EISDIR | libc::EINVAL | libc::ENOENT) => Ok(None),
                _ => Err(TargetError::Io(error)),
            };
        }
        // SAFETY: `fd` is non-negative (checked above) and was just returned
        // by `openat`, so this takes ownership of a fresh descriptor no other
        // owner holds.
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        #[cfg(any(test, feature = "testing"))]
        if testing::tmpfile_umask_leak(self.path()) {
            // The inode a kernel before 4.19 left at `0o666` in `vfs_tmpfile`
            // because `ac6800e279a2` was not backported.
            // SAFETY: `file` owns an open descriptor for the call.
            if unsafe { libc::fchmod(fd, 0o666) } != 0 {
                return Err(TargetError::Io(std::io::Error::last_os_error()));
            }
        }
        #[cfg(any(test, feature = "testing"))]
        if testing::narrow_tmpfile_mode(self.path()) {
            // An inode the kernel already left narrower than 0600 while the
            // policy probes report nothing: the clamp must not add owner bits.
            // SAFETY: `file` owns an open descriptor for the call.
            if unsafe { libc::fchmod(fd, 0o400) } != 0 {
                return Err(TargetError::Io(std::io::Error::last_os_error()));
            }
        }
        Ok(Some(file))
    }

    /// Gives an anonymous `O_TMPFILE` inode the mode a direct creation would
    /// have. The kernel settles that mode unless it is older than 4.19, where
    /// `vfs_tmpfile` omitted the umask strip on a filesystem without POSIX ACL
    /// support (`ac6800e279a2`). A parent with a default ACL already merged
    /// it. A policy the probes cannot establish clamps the inode's observed
    /// mode to 0600 instead of setting one: an unknown umask or ACL may not
    /// add owner bits the inode does not already have.
    #[cfg(target_os = "linux")]
    pub(crate) fn strip_tmpfile_umask(&self, file: &std::fs::File) -> Result<(), TargetError> {
        let mode = match self.parent_default_acl()? {
            Some(true) => return Ok(()),
            Some(false) => match self.new_file_umask() {
                Some(umask) => 0o666 & !umask,
                None => private_mode(file)?,
            },
            None => private_mode(file)?,
        };
        use std::os::fd::AsRawFd;
        // SAFETY: `file` is an open descriptor owned for the call.
        if unsafe { libc::fchmod(file.as_raw_fd(), mode as libc::mode_t) } != 0 {
            return Err(TargetError::Io(std::io::Error::last_os_error()));
        }
        Ok(())
    }

    /// Publishes an anonymous staged inode under the target's name without
    /// replacing an existing entry, through the standard unprivileged
    /// `linkat("/proc/self/fd/<fd>", AT_SYMLINK_FOLLOW)` form relative to the
    /// held parent. `EEXIST` leaves the concurrent entry in place for the
    /// caller's usual refusal.
    #[cfg(target_os = "linux")]
    pub(crate) fn publish_anonymous(&self, file: &std::fs::File) -> Result<(), TargetError> {
        use std::os::fd::AsRawFd;
        #[cfg(any(test, feature = "testing"))]
        if testing::anonymous_publication_unsupported(self.path()) {
            return Err(TargetError::Io(std::io::Error::from_raw_os_error(
                libc::EOPNOTSUPP,
            )));
        }
        let name = c_name(self.target.name()?)?;
        let source = std::ffi::CString::new(format!("/proc/self/fd/{}", file.as_raw_fd()))
            .map_err(|_| TargetError::Io(std::io::Error::other("an anonymous file has an fd")))?;
        // SAFETY: both names are live NUL-terminated strings, `self.parent` is
        // an open directory handle and `file` owns its descriptor for the
        // call.
        let result = unsafe {
            libc::linkat(
                libc::AT_FDCWD,
                source.as_ptr(),
                self.parent.as_raw_fd(),
                name.as_ptr(),
                libc::AT_SYMLINK_FOLLOW,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(TargetError::Io(std::io::Error::last_os_error()))
        }
    }

    /// Creates the named 0600 staging sibling below the held parent on
    /// Linux; the other Unix platforms keep staging by the target's path.
    pub(crate) fn create_staged(
        &self,
        name: &std::ffi::OsStr,
    ) -> Result<std::fs::File, TargetError> {
        #[cfg(target_os = "linux")]
        {
            create_staged_at(&self.parent, name).map_err(classify)
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.target.create_staged(name)
        }
    }

    pub(crate) fn staged_owned(&self, name: &std::ffi::OsStr, file: &std::fs::File) -> bool {
        #[cfg(target_os = "linux")]
        {
            staged_owned_at(&self.parent, name, file)
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.target.staged_owned(name, file)
        }
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn remove_staged(&self, name: &std::ffi::OsStr) {
        remove_staged_at(&self.parent, name)
    }

    /// Gives a named staged new file the mode a direct creation would have,
    /// immediately before publication. The 0600 staging mode stays when the
    /// policy is unknown: on Linux when the parent's ACL cannot be ruled out
    /// or the umask cannot be read, and on macOS when no process umask was
    /// recorded.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn prepare(&self, file: &std::fs::File) -> Result<(), TargetError> {
        #[cfg(target_os = "linux")]
        match self.parent_default_acl()? {
            Some(false) => {}
            Some(true) | None => return Ok(()),
        }
        let Some(mode) = self.new_file_umask().map(|umask| 0o666 & !umask) else {
            return Ok(());
        };
        use std::os::fd::AsRawFd;
        // SAFETY: `file` is an open descriptor owned for the call.
        if unsafe { libc::fchmod(file.as_raw_fd(), mode as libc::mode_t) } != 0 {
            return Err(TargetError::Io(std::io::Error::last_os_error()));
        }
        Ok(())
    }

    /// Restores the private staging mode after a failed publication, so an
    /// entry the parent refuses to remove keeps no wider access than
    /// preparation gave it.
    pub(crate) fn restore_private(&self, file: &std::fs::File) {
        use std::os::fd::AsRawFd;
        // SAFETY: `file` is an open descriptor owned for the call.
        let _ = unsafe { libc::fchmod(file.as_raw_fd(), 0o600) };
    }

    /// Links the staged sibling under the target's name without replacing an
    /// entry, below the held parent on Linux.
    pub(crate) fn install(
        &self,
        staged: &std::ffi::OsStr,
        file: &std::fs::File,
    ) -> Result<(), TargetError> {
        #[cfg(target_os = "linux")]
        let _ = file;
        #[cfg(target_os = "linux")]
        {
            link_new_at(&self.parent, staged, self.target.name()?).map_err(TargetError::Io)
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.target.install(staged, file, false)
        }
    }

    /// Flushes the published entry through the held parent on Linux; the
    /// other Unix platforms sync the parent by the target's path.
    pub(crate) fn sync(&self) -> Result<(), TargetError> {
        #[cfg(target_os = "linux")]
        {
            open_relative(
                &self.parent,
                std::ffi::OsStr::new("."),
                libc::O_RDONLY | libc::O_DIRECTORY,
                0,
            )
            .map_err(TargetError::Io)?
            .sync_all()
            .map_err(TargetError::Io)
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.target.sync_parent()
        }
    }

    /// The umask for a new file: Linux reads the process status, macOS
    /// reports the value captured at startup. `None` means no value is
    /// available, and a new file keeps its 0600 staging mode.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn new_file_umask(&self) -> Option<u32> {
        #[cfg(all(target_os = "linux", any(test, feature = "testing")))]
        if testing::tmpfile_umask_leak(self.path()) || testing::narrow_tmpfile_mode(self.path()) {
            return None;
        }
        creation_umask()
    }

    /// Whether the held parent carries a POSIX default ACL: `Some(true)` when
    /// it does, `Some(false)` when there is none or the filesystem has no ACL
    /// support, and `None` when the probe has no read permission for it.
    /// Callers treat `None` as a default ACL and keep the staged file at 0600.
    #[cfg(target_os = "linux")]
    fn parent_default_acl(&self) -> Result<Option<bool>, TargetError> {
        #[cfg(any(test, feature = "testing"))]
        if testing::tmpfile_umask_leak(self.path()) {
            return Ok(Some(false));
        }
        #[cfg(any(test, feature = "testing"))]
        if testing::narrow_tmpfile_mode(self.path()) {
            return Ok(None);
        }
        let parent = match open_relative(
            &self.parent,
            std::ffi::OsStr::new("."),
            libc::O_RDONLY | libc::O_DIRECTORY,
            0,
        ) {
            Ok(parent) => parent,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return Ok(None),
            Err(error) => return Err(TargetError::Io(error)),
        };
        use std::os::fd::AsRawFd;
        // SAFETY: `parent` is an open directory handle and the name is a live
        // NUL-terminated string for the call.
        let result = unsafe {
            libc::fgetxattr(
                parent.as_raw_fd(),
                c"system.posix_acl_default".as_ptr(),
                std::ptr::null_mut(),
                0,
            )
        };
        if result >= 0 {
            return Ok(Some(true));
        }
        Ok(match std::io::Error::last_os_error().raw_os_error() {
            Some(libc::ENODATA | libc::EOPNOTSUPP) => Some(false),
            _ => None,
        })
    }
}

/// The mode an inode with an unknown creation policy may publish: the owner
/// bits it already has, never a `0o666` an older kernel may have left on an
/// `O_TMPFILE` inode and never an owner bit it lacks.
#[cfg(any(target_os = "linux", all(test, unix)))]
fn owner_bits(observed: u32) -> u32 {
    observed & 0o600
}

/// The private staging mode for an inode whose creation policy is unknown.
#[cfg(target_os = "linux")]
fn private_mode(file: &std::fs::File) -> Result<u32, TargetError> {
    use std::os::unix::fs::PermissionsExt;
    Ok(owner_bits(
        file.metadata()
            .map_err(TargetError::Io)?
            .permissions()
            .mode(),
    ))
}

/// Test-only controls over how a new file is staged.
#[cfg(all(target_os = "linux", any(test, feature = "testing")))]
pub mod testing {
    use std::{
        path::{Path, PathBuf},
        sync::{LazyLock, Mutex},
    };

    static NAMED_STAGING: LazyLock<Mutex<Vec<PathBuf>>> = LazyLock::new(Mutex::default);

    /// The parents whose new files simulate the pre-4.19 `O_TMPFILE` leak.
    static TMPFILE_UMASK_LEAK: LazyLock<Mutex<Vec<PathBuf>>> = LazyLock::new(Mutex::default);

    /// The parents whose new files simulate an anonymous inode left narrower
    /// than 0600 under an unknown creation policy.
    static NARROW_TMPFILE_MODE: LazyLock<Mutex<Vec<PathBuf>>> = LazyLock::new(Mutex::default);

    /// The parents whose anonymous publication is refused as unsupported.
    static ANONYMOUS_PUBLICATION_UNSUPPORTED: LazyLock<Mutex<Vec<PathBuf>>> =
        LazyLock::new(Mutex::default);

    /// Stages new files under `parent` as named siblings, the fallback a
    /// filesystem without `O_TMPFILE` takes. [`allow_anonymous_staging`]
    /// releases the override.
    pub fn force_named_staging(parent: &Path) {
        NAMED_STAGING.lock().unwrap().push(parent.to_owned());
    }

    /// Releases [`force_named_staging`] for `parent`.
    pub fn allow_anonymous_staging(parent: &Path) {
        NAMED_STAGING
            .lock()
            .unwrap()
            .retain(|forced| forced != parent);
    }

    pub(crate) fn named_staging_forced(path: &Path) -> bool {
        let forced = NAMED_STAGING.lock().unwrap();
        path.parent()
            .is_some_and(|parent| forced.iter().any(|entry| entry == parent))
    }

    /// Simulates the `O_TMPFILE` mode leak a kernel before 4.19 left on a
    /// filesystem without POSIX ACL support: the new inode starts at `0o666`,
    /// the directory reports no ACL, and the umask cannot be read.
    /// [`release_tmpfile_umask_leak`] releases the override.
    pub fn force_tmpfile_umask_leak(parent: &Path) {
        TMPFILE_UMASK_LEAK.lock().unwrap().push(parent.to_owned());
    }

    /// Releases [`force_tmpfile_umask_leak`] for `parent`.
    pub fn release_tmpfile_umask_leak(parent: &Path) {
        TMPFILE_UMASK_LEAK
            .lock()
            .unwrap()
            .retain(|forced| forced != parent);
    }

    pub(crate) fn tmpfile_umask_leak(path: &Path) -> bool {
        forced(&TMPFILE_UMASK_LEAK, path)
    }

    /// Simulates an inode the kernel already left narrower than 0600 while the
    /// default-ACL probe and the umask cannot answer. [`release_narrow_tmpfile_mode`]
    /// releases the override.
    pub fn force_narrow_tmpfile_mode(parent: &Path) {
        NARROW_TMPFILE_MODE.lock().unwrap().push(parent.to_owned());
    }

    /// Releases [`force_narrow_tmpfile_mode`] for `parent`.
    pub fn release_narrow_tmpfile_mode(parent: &Path) {
        NARROW_TMPFILE_MODE
            .lock()
            .unwrap()
            .retain(|forced| forced != parent);
    }

    pub(crate) fn narrow_tmpfile_mode(path: &Path) -> bool {
        forced(&NARROW_TMPFILE_MODE, path)
    }

    /// Simulates a filesystem or process that cannot link an anonymous inode
    /// through `/proc/self/fd`: publication fails with `EOPNOTSUPP`, the way a
    /// filesystem without support for the link does. [`allow_anonymous_publication`]
    /// releases the override.
    pub fn force_anonymous_publication_unsupported(parent: &Path) {
        ANONYMOUS_PUBLICATION_UNSUPPORTED
            .lock()
            .unwrap()
            .push(parent.to_owned());
    }

    /// Releases [`force_anonymous_publication_unsupported`] for `parent`.
    pub fn allow_anonymous_publication(parent: &Path) {
        ANONYMOUS_PUBLICATION_UNSUPPORTED
            .lock()
            .unwrap()
            .retain(|forced| forced != parent);
    }

    pub(crate) fn anonymous_publication_unsupported(path: &Path) -> bool {
        forced(&ANONYMOUS_PUBLICATION_UNSUPPORTED, path)
    }

    fn forced(entries: &LazyLock<Mutex<Vec<PathBuf>>>, path: &Path) -> bool {
        let forced = entries.lock().unwrap();
        path.parent()
            .is_some_and(|parent| forced.iter().any(|entry| entry == parent))
    }
}

/// Per-path FIFO mutation locks shared by `write` and `edit` (TS `locks.ts`).
#[derive(Default)]
pub struct FileWriteCoordinator {
    entries: Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>,
}
struct PathLock {
    locks: Arc<FileWriteCoordinator>,
    path: PathBuf,
    entry: Option<Arc<tokio::sync::Mutex<()>>>,
    guard: Option<OwnedMutexGuard<()>>,
}
impl FileWriteCoordinator {
    /// Waits in FIFO order; a cancelled waiter returns at once.
    async fn lock(
        self: &Arc<Self>,
        path: &Path,
        cancel: &CancellationToken,
    ) -> Result<PathLock, HostError> {
        let entry = self
            .entries
            .lock()
            .unwrap()
            .entry(path.to_owned())
            .or_default()
            .clone();
        let mut lock = PathLock {
            locks: self.clone(),
            path: path.to_owned(),
            entry: Some(entry),
            guard: None,
        };
        let acquire = lock.entry.as_ref().unwrap().clone().lock_owned();
        lock.guard = tokio::select! {
            biased;
            _ = cancel.cancelled() => None,
            guard = acquire => Some(guard),
        };
        if lock.guard.is_none() {
            return Err(cancelled());
        }
        Ok(lock)
    }
}
impl Drop for PathLock {
    fn drop(&mut self) {
        let mut entries = self.locks.entries.lock().unwrap();
        self.guard.take();
        self.entry.take();
        if entries
            .get(&self.path)
            .is_some_and(|entry| Arc::strong_count(entry) == 1)
        {
            entries.remove(&self.path);
        }
    }
}

#[cfg(all(test, unix))]
mod mutation_tests {
    use super::*;
    use dim_agent_runtime::{MemoryTimelineStore, Timeline, TimelineKey};
    use rutis::host::CtxHost;

    /// One branch with its own empty Timeline.
    fn branch() -> FileReads {
        FileReads::new(Arc::new(Timeline::new(
            rutis::Ctx::root().unwrap(),
            TimelineKey {
                session_id: "mutation".into(),
                branch_id: "main".into(),
            },
            Vec::new(),
            Arc::new(MemoryTimelineStore::default()),
        )))
    }

    async fn call(
        tools: &FileTools,
        reads: &FileReads,
        name: &str,
        input: Value,
        cancel: CancellationToken,
    ) -> Result<ToolResult, HostError> {
        tools
            .execute(
                &ToolCall {
                    id: "mutation".into(),
                    name: name.into(),
                    input: input.as_object().unwrap().clone(),
                },
                reads,
                cancel,
            )
            .await
    }

    #[tokio::test]
    async fn staged_mutations_fail_closed() {
        for name in ["write", "edit"] {
            for scenario in [
                "external",
                "replacement",
                "cancel",
                "absent",
                "commit_failure",
                "permissions",
                "temp_swap",
                #[cfg(target_os = "macos")]
                "metadata",
            ] {
                if name == "edit" && scenario == "absent" {
                    continue;
                }
                let root = std::env::temp_dir().join(format!("dim-stage-{}", uuid::Uuid::new_v4()));
                std::fs::create_dir(&root).unwrap();
                let root = root.canonicalize().unwrap();
                let target = root.join("file");
                let mut tools = FileTools::new(root.clone(), root.clone(), root.join("out"));
                let reads = branch();
                if scenario != "absent" {
                    std::fs::write(&target, b"before\r\n").unwrap();
                    call(
                        &tools,
                        &reads,
                        "read",
                        json!({"path": "file"}),
                        CancellationToken::new(),
                    )
                    .await
                    .unwrap();
                }
                let cancel = CancellationToken::new();
                let cancellation = cancel.clone();
                tools.before_commit = Some(Arc::new(move |path| match scenario {
                    "external" | "absent" => std::fs::write(path, b"external").unwrap(),
                    "replacement" => {
                        let replacement = path.with_file_name("replacement");
                        std::fs::write(&replacement, b"before\r\n").unwrap();
                        std::fs::rename(replacement, path).unwrap();
                    }
                    "cancel" => cancellation.cancel(),
                    "permissions" => {
                        use std::os::unix::fs::PermissionsExt;
                        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o400))
                            .unwrap();
                    }
                    #[cfg(target_os = "macos")]
                    "metadata" => {
                        assert!(
                            std::process::Command::new("xattr")
                                .args(["-w", "com.dim.concurrent", "changed"])
                                .arg(path)
                                .status()
                                .unwrap()
                                .success()
                        );
                    }
                    "temp_swap" => {
                        for entry in std::fs::read_dir(path.parent().unwrap()).unwrap() {
                            let entry = entry.unwrap();
                            if entry
                                .file_name()
                                .to_string_lossy()
                                .starts_with(".dim-write-")
                            {
                                std::fs::remove_file(entry.path()).unwrap();
                                std::fs::write(entry.path(), b"foreign").unwrap();
                            }
                        }
                    }
                    "commit_failure" => {
                        for entry in std::fs::read_dir(path.parent().unwrap()).unwrap() {
                            let entry = entry.unwrap();
                            if entry
                                .file_name()
                                .to_string_lossy()
                                .starts_with(".dim-write-")
                            {
                                std::fs::remove_file(entry.path()).unwrap();
                            }
                        }
                    }
                    _ => unreachable!(),
                }));
                let input = if name == "write" {
                    json!({"path": "file", "content": "after\n"})
                } else {
                    json!({"path": "file", "oldText": "before", "newText": "after"})
                };
                let result = call(&tools, &reads, name, input, cancel).await;
                if scenario == "cancel" {
                    assert!(matches!(result, Err(HostError::Cancelled(_))));
                } else {
                    let result = result.unwrap();
                    assert_eq!(result.is_error, Some(true));
                    assert_eq!(
                        result.structured_content.unwrap()["code"],
                        if scenario == "absent" {
                            "text_binding_required"
                        } else if matches!(scenario, "commit_failure" | "temp_swap") {
                            "write_error"
                        } else {
                            "file_modified_since_read"
                        }
                    );
                }
                assert_eq!(
                    std::fs::read(&target).unwrap(),
                    if matches!(scenario, "external" | "absent") {
                        b"external".as_slice()
                    } else {
                        b"before\r\n".as_slice()
                    }
                );
                assert_eq!(
                    std::fs::read_dir(&root).unwrap().count(),
                    if scenario == "temp_swap" { 2 } else { 1 },
                    "temporary artifact in {scenario}"
                );
                if scenario == "temp_swap" {
                    let foreign = std::fs::read_dir(&root)
                        .unwrap()
                        .map(Result::unwrap)
                        .find(|entry| {
                            entry
                                .file_name()
                                .to_string_lossy()
                                .starts_with(".dim-write-")
                        })
                        .unwrap();
                    assert_eq!(std::fs::read(foreign.path()).unwrap(), b"foreign");
                }
                std::fs::remove_dir_all(root).unwrap();
            }
        }
    }

    /// Restores anonymous staging when it drops, a panic included.
    #[cfg(target_os = "linux")]
    struct ForcedNamedStaging(PathBuf);
    #[cfg(target_os = "linux")]
    impl ForcedNamedStaging {
        fn new(parent: &Path) -> Self {
            testing::force_named_staging(parent);
            Self(parent.to_owned())
        }
    }
    #[cfg(target_os = "linux")]
    impl Drop for ForcedNamedStaging {
        fn drop(&mut self) {
            testing::allow_anonymous_staging(&self.0);
        }
    }

    /// Restores the umask-leak override when it drops, a panic included.
    #[cfg(target_os = "linux")]
    struct ForcedTmpfileUmaskLeak(PathBuf);
    #[cfg(target_os = "linux")]
    impl ForcedTmpfileUmaskLeak {
        fn new(parent: &Path) -> Self {
            testing::force_tmpfile_umask_leak(parent);
            Self(parent.to_owned())
        }
    }
    #[cfg(target_os = "linux")]
    impl Drop for ForcedTmpfileUmaskLeak {
        fn drop(&mut self) {
            testing::release_tmpfile_umask_leak(&self.0);
        }
    }

    /// Restores the narrow-mode override when it drops, a panic included.
    #[cfg(target_os = "linux")]
    struct ForcedNarrowTmpfileMode(PathBuf);
    #[cfg(target_os = "linux")]
    impl ForcedNarrowTmpfileMode {
        fn new(parent: &Path) -> Self {
            testing::force_narrow_tmpfile_mode(parent);
            Self(parent.to_owned())
        }
    }
    #[cfg(target_os = "linux")]
    impl Drop for ForcedNarrowTmpfileMode {
        fn drop(&mut self) {
            testing::release_narrow_tmpfile_mode(&self.0);
        }
    }

    /// Restores the anonymous-publication override when it drops, a panic
    /// included.
    #[cfg(target_os = "linux")]
    struct ForcedAnonymousPublicationUnsupported(PathBuf);
    #[cfg(target_os = "linux")]
    impl ForcedAnonymousPublicationUnsupported {
        fn new(parent: &Path) -> Self {
            testing::force_anonymous_publication_unsupported(parent);
            Self(parent.to_owned())
        }
    }
    #[cfg(target_os = "linux")]
    impl Drop for ForcedAnonymousPublicationUnsupported {
        fn drop(&mut self) {
            testing::allow_anonymous_publication(&self.0);
        }
    }

    /// Fails with the reason when `parent` cannot hold an `O_TMPFILE` inode,
    /// so a test that needs the anonymous path never passes on the named
    /// fallback.
    #[cfg(target_os = "linux")]
    fn require_anonymous_staging(parent: &Path) {
        use std::os::fd::{AsRawFd, FromRawFd};
        assert!(
            Path::new("/proc/self/fd").is_dir(),
            "this test needs /proc/self/fd to publish an O_TMPFILE inode"
        );
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        std::os::unix::fs::OpenOptionsExt::custom_flags(
            &mut options,
            libc::O_PATH | libc::O_DIRECTORY,
        );
        let directory = options.open(parent).unwrap();
        // SAFETY: `directory` is an open directory handle and the path is a
        // live NUL-terminated string for the call.
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                c".".as_ptr(),
                libc::O_TMPFILE | libc::O_WRONLY | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            panic!(
                "this test needs a filesystem with O_TMPFILE: {}",
                std::io::Error::last_os_error()
            );
        }
        // SAFETY: `fd` is non-negative (checked above) and was just returned
        // by `openat`, so this closes a fresh descriptor no other owner holds.
        drop(unsafe { std::fs::File::from_raw_fd(fd) });
    }

    /// A new file staged in a named sibling (the fallback a filesystem without
    /// `O_TMPFILE` takes) stays 0600 while it is prepared and takes the
    /// creation mode when it is published.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_named_staging_new_file_is_private_until_publication() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("dim-new-file-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let mut tools = FileTools::new(root.clone(), root.clone(), root.join("out"));
        let prepared = root.clone();
        tools.before_commit = Some(Arc::new(move |path| {
            let staged: Vec<_> = std::fs::read_dir(&prepared)
                .unwrap()
                .map(Result::unwrap)
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".dim-write-")
                })
                .collect();
            assert_eq!(staged.len(), 1, "the fallback stages one named sibling");
            assert_eq!(
                staged[0].metadata().unwrap().permissions().mode() & 0o777,
                0o600,
                "the staged new file stays private"
            );
            assert!(!path.exists(), "preparation does not create the target");
        }));
        let _forced = ForcedNamedStaging::new(&root);
        let result = call(
            &tools,
            &branch(),
            "write",
            json!({"path": "new", "content": "text\n"}),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(result.is_error, None);
        assert_eq!(std::fs::read(root.join("new")).unwrap(), b"text\n");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o666);
        options.open(root.join("control")).unwrap();
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode(&root.join("new")),
            mode(&root.join("control")),
            "the fallback publishes the creation mode"
        );
        let leftovers: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .map(Result::unwrap)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".dim-write-")
            })
            .collect();
        assert!(
            leftovers.is_empty(),
            "a successful fallback write leaves no staging entry: {leftovers:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A cancelled new file staged in a named sibling leaves no target and no
    /// staging entry.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_cancelled_named_staging_new_file_leaves_no_entry() {
        let root = std::env::temp_dir().join(format!("dim-new-file-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let mut tools = FileTools::new(root.clone(), root.clone(), root.join("out"));
        let cancel = CancellationToken::new();
        let cancellation = cancel.clone();
        tools.before_commit = Some(Arc::new(move |_| cancellation.cancel()));
        let _forced = ForcedNamedStaging::new(&root);
        let result = call(
            &tools,
            &branch(),
            "write",
            json!({"path": "new", "content": "text\n"}),
            cancel,
        )
        .await;
        assert!(matches!(result, Err(HostError::Cancelled(_))));
        assert!(!root.join("new").exists());
        assert_eq!(
            std::fs::read_dir(&root).unwrap().count(),
            0,
            "cancellation removes the staging entry"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A new file whose umask cannot be read keeps the 0600 staging mode
    /// instead of publishing the `0o666` an `O_TMPFILE` inode kept in a
    /// `vfs_tmpfile` that omitted the umask strip (`ac6800e279a2`).
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn an_unread_umask_keeps_an_anonymous_new_file_private() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("dim-new-file-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        require_anonymous_staging(&root);
        let tools = FileTools::new(root.clone(), root.clone(), root.join("out"));
        let _leak = ForcedTmpfileUmaskLeak::new(&root);
        let result = call(
            &tools,
            &branch(),
            "write",
            json!({"path": "new", "content": "text\n"}),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(result.is_error, None);
        assert_eq!(std::fs::read(root.join("new")).unwrap(), b"text\n");
        assert_eq!(
            std::fs::metadata(root.join("new"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600,
            "an unread umask publishes an anonymous new file as 0600"
        );
        assert_eq!(
            std::fs::read_dir(&root).unwrap().count(),
            1,
            "a successful anonymous write leaves no staging entry"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// An `O_TMPFILE` new file has no directory entry while it is prepared, and
    /// its publication does not replace a name created in that window.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn an_anonymous_new_file_has_no_entry_and_does_not_overwrite() {
        let root = std::env::temp_dir().join(format!("dim-new-file-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        require_anonymous_staging(&root);
        let mut tools = FileTools::new(root.clone(), root.clone(), root.join("out"));
        let prepared = root.clone();
        tools.before_commit = Some(Arc::new(move |path| {
            assert_eq!(
                std::fs::read_dir(&prepared).unwrap().count(),
                0,
                "an O_TMPFILE inode has no directory entry during preparation"
            );
            std::fs::write(path, b"external").unwrap();
        }));
        let result = call(
            &tools,
            &branch(),
            "write",
            json!({"path": "new", "content": "text\n"}),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert_eq!(
            result.structured_content.unwrap()["code"],
            "text_binding_required"
        );
        assert_eq!(std::fs::read(root.join("new")).unwrap(), b"external");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A new file stays on the parent it opened for preparation: renaming
    /// that directory before publication leaves the file in the renamed
    /// directory, and the directory put at the old path receives nothing.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_new_file_stays_on_its_opened_parent_through_publication() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("dim-new-file-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("parent")).unwrap();
        let root = root.canonicalize().unwrap();
        let (parent, moved) = (root.join("parent"), root.join("moved"));
        let mut tools = FileTools::new(root.clone(), root.clone(), root.join("out"));
        let swap = (parent.clone(), moved.clone());
        tools.before_commit = Some(Arc::new(move |path| {
            assert_eq!(path, swap.0.join("new"));
            std::fs::rename(&swap.0, &swap.1).unwrap();
            std::fs::create_dir(&swap.0).unwrap();
            std::fs::set_permissions(&swap.0, std::fs::Permissions::from_mode(0o300)).unwrap();
        }));
        let result = call(
            &tools,
            &branch(),
            "write",
            json!({"path": "parent/new", "content": "text\n"}),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(result.is_error, None, "{result:?}");
        assert_eq!(std::fs::read(moved.join("new")).unwrap(), b"text\n");
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            std::fs::read_dir(&parent).unwrap().count(),
            0,
            "the directory now at the old path receives nothing"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A filesystem or process that cannot publish an anonymous inode falls
    /// back to the named sibling: the new file still takes the creation mode
    /// and leaves no staging entry.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn an_unsupported_anonymous_publication_falls_back_to_named_staging() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("dim-new-file-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        require_anonymous_staging(&root);
        let tools = FileTools::new(root.clone(), root.clone(), root.join("out"));
        let _forced = ForcedAnonymousPublicationUnsupported::new(&root);
        let result = call(
            &tools,
            &branch(),
            "write",
            json!({"path": "new", "content": "text\n"}),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(result.is_error, None, "{result:?}");
        assert_eq!(std::fs::read(root.join("new")).unwrap(), b"text\n");
        assert_eq!(
            std::fs::read_dir(&root).unwrap().count(),
            1,
            "a successful fallback leaves no staging entry"
        );
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o666);
        options.open(root.join("control")).unwrap();
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode(&root.join("new")),
            mode(&root.join("control")),
            "the fallback publishes the creation mode"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The named fallback keeps the no-replace guarantee at publication.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_fallback_publication_does_not_replace_an_existing_entry() {
        let root = std::env::temp_dir().join(format!("dim-new-file-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        require_anonymous_staging(&root);
        let mut tools = FileTools::new(root.clone(), root.clone(), root.join("out"));
        let existing = root.join("new");
        tools.before_commit = Some(Arc::new(move |path| {
            std::fs::write(path, b"external").unwrap();
        }));
        let _forced = ForcedAnonymousPublicationUnsupported::new(&root);
        let result = call(
            &tools,
            &branch(),
            "write",
            json!({"path": "new", "content": "text\n"}),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert_eq!(
            result.structured_content.unwrap()["code"],
            "text_binding_required"
        );
        assert_eq!(std::fs::read(existing).unwrap(), b"external");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The unknown-policy clamp keeps an inode's observed owner bits and drops
    /// every group and other bit, so a narrower inode never gains access.
    #[test]
    fn an_unknown_policy_keeps_only_the_observed_owner_bits() {
        assert_eq!(owner_bits(0o644), 0o600);
        assert_eq!(owner_bits(0o400), 0o400);
        assert_eq!(owner_bits(0o040), 0);
        assert_eq!(owner_bits(0o600), 0o600);
    }

    /// An unknown creation policy clamps an anonymous inode's observed mode: an
    /// inode the kernel already left at owner read only gets no owner write
    /// bit, so the published file stays as narrow as a direct creation.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn an_unknown_policy_clamps_an_anonymous_new_file_to_its_owner_bits() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("dim-new-file-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        require_anonymous_staging(&root);
        let tools = FileTools::new(root.clone(), root.clone(), root.join("out"));
        let _forced = ForcedNarrowTmpfileMode::new(&root);
        let result = call(
            &tools,
            &branch(),
            "write",
            json!({"path": "new", "content": "text\n"}),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(result.is_error, None, "{result:?}");
        assert_eq!(std::fs::read(root.join("new")).unwrap(), b"text\n");
        assert_eq!(
            std::fs::metadata(root.join("new"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o400,
            "an unknown policy never adds owner bits to a narrower inode"
        );
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A failed publication restores the staged sibling's private mode. A
    /// parent whose inherited ACL denies delete keeps that unpublished entry,
    /// so the entry must not keep the widened creation mode. The child process
    /// fixes the umask and records it through the product entry point, because
    /// the value is read once per process.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn a_failed_publication_does_not_leave_a_widened_staging_entry() {
        use std::os::unix::fs::PermissionsExt;
        const UNSET: &str = "DIM_TEST_FAILED_PUBLICATION_UMASK";
        if std::env::var_os(UNSET).is_none() {
            let binary = std::env::current_exe().unwrap();
            let script = r#"umask "$1" && exec "$0" a_failed_publication_does_not_leave_a_widened_staging_entry --nocapture"#;
            let child = std::process::Command::new("sh")
                .args(["-c", script])
                .arg(&binary)
                .arg("022")
                .env(UNSET, "022")
                .output()
                .unwrap();
            assert!(
                child.status.success(),
                "{}{}",
                String::from_utf8_lossy(&child.stdout),
                String::from_utf8_lossy(&child.stderr)
            );
            return;
        }
        install_process_umask();
        let root = std::env::temp_dir().join(format!("dim-new-file-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let group = std::process::Command::new("id")
            .arg("-gn")
            .output()
            .unwrap();
        let group = String::from_utf8_lossy(&group.stdout).trim().to_owned();
        assert!(
            std::process::Command::new("chmod")
                .arg("+a")
                .arg(format!(
                    "group:{group} deny delete,file_inherit,only_inherit"
                ))
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        let mut tools = FileTools::new(root.clone(), root.clone(), root.join("out"));
        let existing = root.join("new");
        tools.before_commit = Some(Arc::new(move |path| {
            std::fs::write(path, b"external").unwrap();
        }));
        let result = call(
            &tools,
            &branch(),
            "write",
            json!({"path": "new", "content": "text\n"}),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert_eq!(
            result.structured_content.unwrap()["code"],
            "text_binding_required"
        );
        assert_eq!(std::fs::read(&existing).unwrap(), b"external");
        let staged = std::fs::read_dir(&root)
            .unwrap()
            .map(Result::unwrap)
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".dim-write-")
            })
            .expect("the denied delete keeps the failed staging entry")
            .path();
        assert_eq!(
            std::fs::metadata(&staged).unwrap().permissions().mode() & 0o777,
            0o600,
            "a failed publication restores the private staging mode"
        );
        for path in [&staged, &existing, &root] {
            let _ = std::process::Command::new("chmod")
                .arg("-N")
                .arg(path)
                .status();
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn coordinated_sessions_wait_through_preparation() {
        let root = std::env::temp_dir().join(format!("dim-coordinate-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        std::fs::write(root.join("file"), b"before").unwrap();
        let coordinator: Arc<FileWriteCoordinator> = Arc::default();
        let mut first = FileTools::with_coordinator(
            root.clone(),
            root.clone(),
            root.join("out"),
            coordinator.clone(),
            None,
        );
        let second = FileTools::with_coordinator(
            root.clone(),
            root.clone(),
            root.join("out"),
            coordinator,
            None,
        );
        let (a, b) = (branch(), branch());
        for (tools, reads) in [(&first, &a), (&second, &b)] {
            call(
                tools,
                reads,
                "read",
                json!({"path": "file"}),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        }
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let release = barrier.clone();
        let (ready, prepared) = tokio::sync::oneshot::channel();
        let ready = Mutex::new(Some(ready));
        first.before_commit = Some(Arc::new(move |_| {
            ready.lock().unwrap().take().unwrap().send(()).unwrap();
            release.wait();
        }));
        let first = tokio::spawn(async move {
            call(
                &first,
                &a,
                "write",
                json!({"path": "file", "content": "one"}),
                CancellationToken::new(),
            )
            .await
        });
        prepared.await.unwrap();
        let second_result = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            call(
                &second,
                &b,
                "write",
                json!({"path": "file", "content": "two"}),
                CancellationToken::new(),
            ),
        )
        .await;
        barrier.wait();
        let first_result = first.await.unwrap().unwrap();
        std::fs::remove_dir_all(root).unwrap();
        assert!(
            second_result.is_err(),
            "another Session entered during preparation"
        );
        assert_eq!(first_result.is_error, None);
    }
}

/// The queued-mutation regressions: a bounded write and edit admit one
/// canonical path, then a parent component is swapped for a symlink to an
/// outside sibling while the call waits on the per-path coordinator. The I/O
/// must act on what was admitted — the walk fails closed — and the outside
/// file must stay untouched. They need the internal coordinator and its
/// before-lock hook, so they stay in-crate.
#[cfg(all(test, unix))]
mod queued_boundary_tests {
    use super::*;
    use crate::workspace_tools::files::{FileToolBoundary, FileTools};
    use dim_agent_runtime::{MemoryTimelineStore, Timeline, TimelineKey};
    use rutis::host::CtxHost;

    /// One branch with its own empty Timeline; one per test, so read
    /// bindings survive between the calls of a test.
    fn reads() -> FileReads {
        FileReads::new(Arc::new(Timeline::new(
            rutis::Ctx::root().unwrap(),
            TimelineKey {
                session_id: "queued".into(),
                branch_id: "main".into(),
            },
            Vec::new(),
            Arc::new(MemoryTimelineStore::default()),
        )))
    }

    struct Fixture {
        root: PathBuf,
        workspace: PathBuf,
        outside: PathBuf,
        boundary: FileToolBoundary,
    }

    fn fixture() -> Fixture {
        let root = std::env::temp_dir().join(format!("dim-queued-{}", uuid::Uuid::new_v4()));
        for dir in ["workspace/sub", "home", "outside"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::write(root.join("workspace/sub/note.txt"), "before").unwrap();
        std::fs::write(root.join("outside/note.txt"), "before").unwrap();
        let root = root.canonicalize().unwrap();
        let workspace = root.join("workspace");
        Fixture {
            boundary: FileToolBoundary {
                workspace: workspace.clone(),
                home: root.join("home"),
                read_only_roots: Vec::new(),
            },
            workspace,
            outside: root.join("outside"),
            root,
        }
    }

    fn tools(fixture: &Fixture, coordinator: Arc<FileWriteCoordinator>) -> FileTools {
        FileTools::with_coordinator(
            fixture.workspace.clone(),
            fixture.boundary.home.clone(),
            fixture.workspace.join("out"),
            coordinator,
            Some(fixture.boundary.clone()),
        )
    }

    async fn call(tools: &FileTools, reads: &FileReads, name: &str, input: Value) -> ToolResult {
        tools
            .execute(
                &ToolCall {
                    id: "queued".into(),
                    name: name.into(),
                    input: input.as_object().unwrap().clone(),
                },
                reads,
                CancellationToken::new(),
            )
            .await
            .unwrap()
    }

    fn assert_refused(result: &ToolResult) {
        assert_eq!(result.is_error, Some(true), "{result:?}");
        assert_eq!(
            result.structured_content.as_ref().unwrap()["code"],
            "outside_file_roots",
            "{result:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_queued_write_refuses_a_parent_swap() {
        let fixture = fixture();
        let coordinator: Arc<FileWriteCoordinator> = Arc::default();
        let mut tools = tools(&fixture, coordinator.clone());
        let reads = reads();
        let target = fixture.workspace.join("sub/escaped.txt");
        let cancel = CancellationToken::new();
        let held = coordinator.lock(&target, &cancel).await.unwrap();
        let (queued, release) = tokio::sync::oneshot::channel();
        let queued = Mutex::new(Some(queued));
        tools.before_lock = Some(Arc::new(move |_| {
            if let Some(queued) = queued.lock().unwrap().take() {
                let _ = queued.send(());
            }
        }));
        let (workspace, outside) = (fixture.workspace.clone(), fixture.outside.clone());
        let task = tokio::spawn(async move {
            call(
                &tools,
                &reads,
                "write",
                json!({"path": "sub/escaped.txt", "content": "escaped\n"}),
            )
            .await
        });
        release.await.unwrap();
        // The caller resolved its target and now waits on the held
        // coordinator: swap the parent for a symlink to the outside sibling.
        std::fs::rename(workspace.join("sub"), workspace.join("sub-moved")).unwrap();
        std::os::unix::fs::symlink(&outside, workspace.join("sub")).unwrap();
        drop(held);
        let result = task.await.unwrap();
        assert_refused(&result);
        assert!(
            !outside.join("escaped.txt").exists(),
            "the outside file must not be created"
        );
        assert!(
            !workspace.join("sub-moved/escaped.txt").exists(),
            "the moved directory must not receive the write"
        );
        std::fs::remove_dir_all(&fixture.root).unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_queued_edit_refuses_a_parent_swap() {
        let fixture = fixture();
        let coordinator: Arc<FileWriteCoordinator> = Arc::default();
        let mut tools = tools(&fixture, coordinator.clone());
        let reads = reads();
        let binding = call(&tools, &reads, "read", json!({"path": "sub/note.txt"})).await;
        assert_ne!(binding.is_error, Some(true), "{binding:?}");
        let target = fixture.workspace.join("sub/note.txt");
        let cancel = CancellationToken::new();
        let held = coordinator.lock(&target, &cancel).await.unwrap();
        let (queued, release) = tokio::sync::oneshot::channel();
        let queued = Mutex::new(Some(queued));
        tools.before_lock = Some(Arc::new(move |_| {
            if let Some(queued) = queued.lock().unwrap().take() {
                let _ = queued.send(());
            }
        }));
        let (workspace, outside) = (fixture.workspace.clone(), fixture.outside.clone());
        let task = tokio::spawn(async move {
            call(
                &tools,
                &reads,
                "edit",
                json!({"path": "sub/note.txt", "oldText": "before", "newText": "changed"}),
            )
            .await
        });
        release.await.unwrap();
        std::fs::rename(workspace.join("sub"), workspace.join("sub-moved")).unwrap();
        std::os::unix::fs::symlink(&outside, workspace.join("sub")).unwrap();
        drop(held);
        let result = task.await.unwrap();
        assert_refused(&result);
        assert_eq!(
            std::fs::read_to_string(outside.join("note.txt")).unwrap(),
            "before",
            "the outside bytes must be unchanged"
        );
        std::fs::remove_dir_all(&fixture.root).unwrap();
    }
}

/// The bounded-search regression: `grep` reads each file it matches through
/// the admitted walk, so a subdirectory swapped for a symlink to an outside
/// sibling after the walk listed it yields no content from outside. The
/// hook runs inside the walk on each entry, before the entry is read; no
/// sleeps. In-crate because the hook is a private test field.
#[cfg(all(test, unix))]
mod bounded_search_tests {
    use super::*;
    use crate::workspace_tools::files::{FileToolBoundary, FileTools};
    use dim_agent_runtime::{MemoryTimelineStore, Timeline, TimelineKey};
    use rutis::host::CtxHost;

    fn reads() -> FileReads {
        FileReads::new(Arc::new(Timeline::new(
            rutis::Ctx::root().unwrap(),
            TimelineKey {
                session_id: "search".into(),
                branch_id: "main".into(),
            },
            Vec::new(),
            Arc::new(MemoryTimelineStore::default()),
        )))
    }

    async fn call(tools: &FileTools, input: Value) -> ToolResult {
        tools
            .execute(
                &ToolCall {
                    id: "search".into(),
                    name: "grep".into(),
                    input: input.as_object().unwrap().clone(),
                },
                &reads(),
                CancellationToken::new(),
            )
            .await
            .unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_bounded_grep_reads_no_file_outside_the_roots() {
        let root = std::env::temp_dir().join(format!("dim-search-{}", uuid::Uuid::new_v4()));
        for dir in ["workspace/sub", "home", "outside"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::write(root.join("workspace/sub/note.txt"), "NEEDLE inside").unwrap();
        std::fs::write(root.join("outside/note.txt"), "NEEDLE outside").unwrap();
        let root = root.canonicalize().unwrap();
        let workspace = root.join("workspace");
        let outside = root.join("outside");
        let boundary = FileToolBoundary {
            workspace: workspace.clone(),
            home: root.join("home"),
            read_only_roots: Vec::new(),
        };
        let mut tools =
            FileTools::with_boundary(workspace.clone(), workspace.join("out"), boundary);
        // Control: the file inside the workspace is read.
        let found = call(&tools, json!({"pattern": "NEEDLE", "path": "sub"})).await;
        let matches = found.structured_content.as_ref().unwrap()["matches"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(matches.len(), 1, "{found:?}");
        assert_eq!(matches[0]["content"], "NEEDLE inside");
        // The walk lists `sub`, then the hook swaps it for a symlink to the
        // outside sibling before its contents are read.
        let link = workspace.join("sub");
        tools.before_search = Some(Arc::new(move |path| {
            if path == link.as_path() && !link.is_symlink() {
                let _ = std::fs::remove_file(link.join("note.txt"));
                let _ = std::fs::remove_dir(&link);
                let _ = std::os::unix::fs::symlink(&outside, &link);
            }
        }));
        let escaped = call(&tools, json!({"pattern": "NEEDLE", "path": "."})).await;
        let structured = escaped.structured_content.as_ref().unwrap().clone();
        assert!(
            structured["matches"].as_array().unwrap().is_empty(),
            "no match may come from outside the roots: {structured:?}"
        );
        assert!(
            !serde_json::to_string(&structured)
                .unwrap()
                .contains("NEEDLE outside"),
            "{structured:?}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
