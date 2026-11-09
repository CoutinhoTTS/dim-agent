//! The file tools `read`, `write`, `edit`, `glob` and `grep`.
//!
//! Behavior, texts, error codes and limits follow the TS tools
//! (`packages/agent/src/tool/builtin/` and `packages/plugin/src/builtin/grep-glob/`).
//! Workspace I/O runs on blocking threads with `std::fs`.

#![allow(clippy::result_large_err)]

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
    /// Runs after a complete sibling is synced and before commit verification;
    /// no sleeps, production hooks or global test locks.
    before_commit: Option<CommitHook>,
}

#[cfg(test)]
type CommitHook = Arc<dyn Fn(&Path) + Send + Sync>;

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
            before_commit: None,
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

/// Resolves a model-supplied file path (TS `path.ts`, design §7.2): `~`
/// expansion, Windows corrections, join onto cwd, lexical `.`/`..`
/// removal, then realpath of the longest existing prefix. With `boundary`
/// configured, a resolved path outside the roots is refused for `write`
/// (workspace only) or for reads (workspace, home and the read-only
/// roots); the check compares the canonical path, so a symlink out of the
/// roots is refused as well.
pub(crate) fn resolve_path(
    cwd: &Path,
    home: &Path,
    boundary: Option<&FileToolBoundary>,
    write: bool,
    input: &str,
) -> Result<PathBuf, PathError> {
    let expanded = if input == "~" {
        home.to_owned()
    } else if let Some(rest) = input.strip_prefix("~/").or(input.strip_prefix("~\\")) {
        home.join(rest)
    } else {
        windows_path(input)?
    };
    match canonical_prefix(lexical(&cwd.join(expanded))) {
        Ok(path) if boundary.is_some_and(|boundary| !inside(boundary, &path, write)) => {
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

/// The roots a host configures for a session's file tools. The roots bound
/// the model's file-tool calls when a host configures them; they make no
/// sandbox claim (ADR 010 owns the sandbox). The paths are absolute and
/// canonical: the evaluation harness resolves them from directories it
/// created, and a root reached through a symlink would answer `starts_with`
/// with its lexical path. The product passes `None`, so its behaviour does
/// not change (ADR 014 keeps deciding file access for it).
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// Whether a canonicalized path lies inside the roots for the mode: reads in
/// the workspace, the home and the read-only roots; writes only in the
/// workspace.
fn inside(boundary: &FileToolBoundary, path: &Path, write: bool) -> bool {
    if path.starts_with(&boundary.workspace) {
        true
    } else if write {
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
fn open_options() -> std::fs::OpenOptions {
    #[allow(unused_mut)]
    let mut options = std::fs::OpenOptions::new();
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(&mut options, libc::O_NOFOLLOW);
    options
}

/// The target exists but is not a regular file (a FIFO, device or socket).
struct NotRegular;

/// Opens an existing regular file for reading. The open never blocks (a FIFO
/// without a writer would block it, uncancellably), and the type is checked
/// on the opened handle, so a swap after a path check cannot slip through.
fn open_regular(path: &Path) -> std::io::Result<Result<std::fs::File, NotRegular>> {
    #[allow(unused_mut)]
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

/// The root boundary tests: each refusal carries an ablation — the same call
/// with no boundary at all must not refuse, so reverting the boundary code
/// fails the test.
#[cfg(all(test, unix))]
/// The root boundary tests: each refusal carries an ablation — the same call
/// with no boundary at all must not refuse, so reverting the boundary code
/// fails the test.
#[cfg(all(test, unix))]
mod boundary_tests {
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
                session_id: "boundary".into(),
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
    ) -> Result<ToolResult, HostError> {
        tools
            .execute(
                &ToolCall {
                    id: "boundary".into(),
                    name: name.into(),
                    input: input.as_object().unwrap().clone(),
                },
                reads,
                CancellationToken::new(),
            )
            .await
    }

    /// A canonical temp tree with a workspace, the configured home (with a
    /// `sub/` directory under it), a read-only root, an outside directory
    /// and a separate platform home the no-boundary `~` resolves against.
    struct Boundary {
        workspace: PathBuf,
        platform_home: PathBuf,
        home: PathBuf,
        /// A sibling directory no root covers.
        outside: PathBuf,
        read_only: PathBuf,
        boundary: FileToolBoundary,
    }

    fn fixture() -> Boundary {
        let root = std::env::temp_dir().join(format!("dim-boundary-{}", uuid::Uuid::new_v4()));
        for dir in [
            "workspace",
            "home/sub",
            "platform-home",
            "read-only",
            "outside",
        ] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::write(root.join("workspace/notes.txt"), "workspace-notes").unwrap();
        std::fs::write(root.join("home/marker.txt"), "home-marker").unwrap();
        std::fs::write(root.join("home/sub/marker.txt"), "home-sub-marker").unwrap();
        std::fs::write(root.join("platform-home/marker.txt"), "platform-marker").unwrap();
        std::fs::write(root.join("read-only/facts.txt"), "read-only-facts").unwrap();
        std::fs::write(root.join("outside/secret.txt"), "secret").unwrap();
        let root = root.canonicalize().unwrap();
        let workspace = root.join("workspace");
        let boundary = FileToolBoundary {
            workspace: workspace.clone(),
            home: root.join("home"),
            read_only_roots: vec![root.join("read-only")],
        };
        Boundary {
            workspace,
            platform_home: root.join("platform-home"),
            home: root.join("home"),
            outside: root.join("outside"),
            read_only: root.join("read-only"),
            boundary,
        }
    }

    /// The trees' tools with the boundary, and the same tree's tools without
    /// one (today's unbounded behaviour).
    fn tools(fixture: &Boundary) -> (FileTools, FileTools) {
        let output = fixture.workspace.join("out");
        (
            FileTools::with_boundary(
                fixture.workspace.clone(),
                output.clone(),
                fixture.boundary.clone(),
            ),
            FileTools::new(
                fixture.workspace.clone(),
                fixture.platform_home.clone(),
                output,
            ),
        )
    }

    /// The refusal a call must produce: an error whose structured code is
    /// the outside-roots code.
    fn assert_refused(result: &ToolResult) {
        assert_eq!(result.is_error, Some(true), "{result:?}");
        assert_eq!(
            result.structured_content.as_ref().unwrap()["code"],
            "outside_file_roots",
            "{result:?}"
        );
    }

    fn text(result: &ToolResult) -> String {
        match &result.content {
            dim_agent_types::ToolContent::Text(text) => text.clone(),
            other => panic!("expected a text result, got {other:?}"),
        }
    }

    fn remove(fixture: &Boundary) {
        std::fs::remove_dir_all(fixture.boundary.workspace.parent().unwrap()).unwrap();
    }

    #[tokio::test]
    async fn read_is_allowed_inside_the_roots() {
        let fixture = fixture();
        let reads = reads();
        let (tools, _) = tools(&fixture);
        let result = call(&tools, &reads, "read", json!({"path": "notes.txt"}))
            .await
            .unwrap();
        assert!(text(&result).contains("workspace-notes"), "{result:?}");
        let home_read = call(&tools, &reads, "read", json!({"path": "~/marker.txt"}))
            .await
            .unwrap();
        assert!(text(&home_read).contains("home-marker"), "{home_read:?}");
        let root_read = call(
            &tools,
            &reads,
            "read",
            json!({"path": fixture.read_only.join("facts.txt")}),
        )
        .await
        .unwrap();
        assert!(
            text(&root_read).contains("read-only-facts"),
            "{root_read:?}"
        );
        remove(&fixture);
    }

    #[tokio::test]
    async fn read_outside_is_refused_and_allowed_without_a_boundary() {
        let fixture = fixture();
        let reads = reads();
        let (tools, plain) = tools(&fixture);
        let result = call(
            &tools,
            &reads,
            "read",
            json!({"path": "../outside/secret.txt"}),
        )
        .await
        .unwrap();
        assert_refused(&result);
        // Ablation: without the boundary the same read is today's behaviour.
        let result = call(
            &plain,
            &reads,
            "read",
            json!({"path": "../outside/secret.txt"}),
        )
        .await
        .unwrap();
        assert!(text(&result).contains("secret"), "{result:?}");
        remove(&fixture);
    }

    #[tokio::test]
    async fn read_through_a_symlink_out_is_refused_and_allowed_without_a_boundary() {
        let fixture = fixture();
        std::os::unix::fs::symlink(&fixture.outside, fixture.workspace.join("link")).unwrap();
        let reads = reads();
        let (tools, plain) = tools(&fixture);
        let result = call(&tools, &reads, "read", json!({"path": "link/secret.txt"}))
            .await
            .unwrap();
        assert_refused(&result);
        // Ablation: canonicalization is what refuses the symlink; without
        // the boundary the same read follows it.
        let result = call(&plain, &reads, "read", json!({"path": "link/secret.txt"}))
            .await
            .unwrap();
        assert!(text(&result).contains("secret"), "{result:?}");
        remove(&fixture);
    }

    #[tokio::test]
    async fn read_tilde_resolves_to_the_configured_home() {
        let fixture = fixture();
        let reads = reads();
        let (tools, plain) = tools(&fixture);
        let result = call(&tools, &reads, "read", json!({"path": "~/marker.txt"}))
            .await
            .unwrap();
        assert!(text(&result).contains("home-marker"), "{result:?}");
        // Ablation: without the boundary, `~` is the platform home, which
        // holds a different marker, so the assertion above would fail.
        let result = call(&plain, &reads, "read", json!({"path": "~/marker.txt"}))
            .await
            .unwrap();
        assert!(text(&result).contains("platform-marker"), "{result:?}");
        remove(&fixture);
    }

    #[tokio::test]
    async fn write_is_allowed_inside_the_workspace() {
        let fixture = fixture();
        let reads = reads();
        let (tools, _) = tools(&fixture);
        let result = call(
            &tools,
            &reads,
            "write",
            json!({"path": "made.txt", "content": "made\n"}),
        )
        .await
        .unwrap();
        assert_ne!(result.is_error, Some(true), "{result:?}");
        assert_eq!(
            std::fs::read_to_string(fixture.workspace.join("made.txt")).unwrap(),
            "made\n"
        );
        remove(&fixture);
    }

    #[tokio::test]
    async fn write_outside_is_refused_and_allowed_without_a_boundary() {
        let fixture = fixture();
        let reads = reads();
        let (tools, plain) = tools(&fixture);
        for input in [
            json!({"path": "../outside/one.txt", "content": "made\n"}),
            json!({"path": fixture.outside.join("two.txt"), "content": "made\n"}),
        ] {
            let result = call(&tools, &reads, "write", input.clone()).await.unwrap();
            assert_refused(&result);
            // Ablation.
            let result = call(&plain, &reads, "write", input).await.unwrap();
            assert_ne!(result.is_error, Some(true), "{result:?}");
        }
        for name in ["one.txt", "two.txt"] {
            assert_eq!(
                std::fs::read_to_string(fixture.outside.join(name)).unwrap(),
                "made\n"
            );
        }
        remove(&fixture);
    }

    #[tokio::test]
    async fn write_through_a_symlink_out_is_refused_and_allowed_without_a_boundary() {
        let fixture = fixture();
        std::os::unix::fs::symlink(&fixture.outside, fixture.workspace.join("link")).unwrap();
        let reads = reads();
        let (tools, plain) = tools(&fixture);
        let result = call(
            &tools,
            &reads,
            "write",
            json!({"path": "link/made.txt", "content": "made\n"}),
        )
        .await
        .unwrap();
        assert_refused(&result);
        // Ablation.
        let result = call(
            &plain,
            &reads,
            "write",
            json!({"path": "link/made.txt", "content": "made\n"}),
        )
        .await
        .unwrap();
        assert_ne!(result.is_error, Some(true), "{result:?}");
        remove(&fixture);
    }

    #[tokio::test]
    async fn write_tilde_resolves_to_the_configured_home_and_is_refused_there() {
        let fixture = fixture();
        let reads = reads();
        let (tools, plain) = tools(&fixture);
        let result = call(
            &tools,
            &reads,
            "write",
            json!({"path": "~/made.txt", "content": "x\n"}),
        )
        .await
        .unwrap();
        assert_refused(&result);
        // The refusal names the resolved path: the configured home, not the
        // platform home.
        assert_eq!(
            result.structured_content.as_ref().unwrap()["path"],
            fixture.home.join("made.txt").to_string_lossy().as_ref(),
            "{result:?}"
        );
        // Ablation: without the boundary the same write succeeds at the
        // platform home, so the refusal assertion above would fail.
        let result = call(
            &plain,
            &reads,
            "write",
            json!({"path": "~/made.txt", "content": "x\n"}),
        )
        .await
        .unwrap();
        assert_ne!(result.is_error, Some(true), "{result:?}");
        assert_eq!(
            std::fs::read_to_string(fixture.platform_home.join("made.txt")).unwrap(),
            "x\n"
        );
        remove(&fixture);
    }

    #[tokio::test]
    async fn write_to_a_read_only_root_is_refused_and_allowed_without_a_boundary() {
        let fixture = fixture();
        let reads = reads();
        let (tools, plain) = tools(&fixture);
        let target = fixture.read_only.join("made.txt");
        let result = call(
            &tools,
            &reads,
            "write",
            json!({"path": target, "content": "x\n"}),
        )
        .await
        .unwrap();
        assert_refused(&result);
        // Ablation.
        let result = call(
            &plain,
            &reads,
            "write",
            json!({"path": target, "content": "x\n"}),
        )
        .await
        .unwrap();
        assert_ne!(result.is_error, Some(true), "{result:?}");
        remove(&fixture);
    }

    #[tokio::test]
    async fn edit_is_allowed_inside_the_workspace() {
        let fixture = fixture();
        let reads = reads();
        let (tools, _) = tools(&fixture);
        let read = call(&tools, &reads, "read", json!({"path": "notes.txt"}))
            .await
            .unwrap();
        assert!(text(&read).contains("workspace-notes"), "{read:?}");
        let result = call(
            &tools,
            &reads,
            "edit",
            json!({"path": "notes.txt", "oldText": "workspace-notes", "newText": "edited"}),
        )
        .await
        .unwrap();
        assert_ne!(result.is_error, Some(true), "{result:?}");
        remove(&fixture);
    }

    #[tokio::test]
    async fn edit_outside_is_refused_and_allowed_without_a_boundary() {
        let fixture = fixture();
        let reads = reads();
        let (tools, plain) = tools(&fixture);
        let outside = fixture.outside.join("secret.txt");
        let result = call(
            &tools,
            &reads,
            "edit",
            json!({"path": outside, "oldText": "secret", "newText": "changed"}),
        )
        .await
        .unwrap();
        assert_refused(&result);
        // Ablation: read the target first, as the tool requires, then edit.
        call(
            &plain,
            &reads,
            "read",
            json!({"path": "../outside/secret.txt"}),
        )
        .await
        .unwrap();
        let result = call(
            &plain,
            &reads,
            "edit",
            json!({"path": outside, "oldText": "secret", "newText": "changed"}),
        )
        .await
        .unwrap();
        assert_ne!(result.is_error, Some(true), "{result:?}");
        remove(&fixture);
    }

    #[tokio::test]
    async fn edit_through_a_symlink_out_is_refused_and_allowed_without_a_boundary() {
        let fixture = fixture();
        std::os::unix::fs::symlink(&fixture.outside, fixture.workspace.join("link")).unwrap();
        let reads = reads();
        let (tools, plain) = tools(&fixture);
        let result = call(
            &tools,
            &reads,
            "edit",
            json!({"path": "link/secret.txt", "oldText": "secret", "newText": "changed"}),
        )
        .await
        .unwrap();
        assert_refused(&result);
        // Ablation: read then edit, as the tool requires.
        call(&plain, &reads, "read", json!({"path": "link/secret.txt"}))
            .await
            .unwrap();
        let result = call(
            &plain,
            &reads,
            "edit",
            json!({"path": "link/secret.txt", "oldText": "secret", "newText": "changed"}),
        )
        .await
        .unwrap();
        assert_ne!(result.is_error, Some(true), "{result:?}");
        remove(&fixture);
    }

    #[tokio::test]
    async fn edit_tilde_resolves_to_the_configured_home_and_is_refused_there() {
        let fixture = fixture();
        let reads = reads();
        let (tools, plain) = tools(&fixture);
        let result = call(
            &tools,
            &reads,
            "edit",
            json!({"path": "~/marker.txt", "oldText": "home-marker", "newText": "edited"}),
        )
        .await
        .unwrap();
        assert_refused(&result);
        assert_eq!(
            result.structured_content.as_ref().unwrap()["path"],
            fixture.home.join("marker.txt").to_string_lossy().as_ref(),
            "{result:?}"
        );
        // Ablation: without the boundary the same edit resolves to the
        // platform home and succeeds there, not an outside-roots refusal.
        call(&plain, &reads, "read", json!({"path": "~/marker.txt"}))
            .await
            .unwrap();
        let result = call(
            &plain,
            &reads,
            "edit",
            json!({"path": "~/marker.txt", "oldText": "platform-marker", "newText": "edited"}),
        )
        .await
        .unwrap();
        assert_ne!(result.is_error, Some(true), "{result:?}");
        remove(&fixture);
    }

    #[tokio::test]
    async fn edit_a_read_only_root_is_refused_and_allowed_without_a_boundary() {
        let fixture = fixture();
        let reads = reads();
        let (tools, plain) = tools(&fixture);
        let target = fixture.read_only.join("facts.txt");
        let result = call(
            &tools,
            &reads,
            "edit",
            json!({"path": target, "oldText": "read-only-facts", "newText": "edited"}),
        )
        .await
        .unwrap();
        assert_refused(&result);
        // Ablation: read then edit, as the tool requires.
        call(
            &plain,
            &reads,
            "read",
            json!({"path": "../read-only/facts.txt"}),
        )
        .await
        .unwrap();
        let result = call(
            &plain,
            &reads,
            "edit",
            json!({"path": target, "oldText": "read-only-facts", "newText": "edited"}),
        )
        .await
        .unwrap();
        assert_ne!(result.is_error, Some(true), "{result:?}");
        remove(&fixture);
    }

    /// Glob and grep share the search scope resolution.
    #[tokio::test]
    async fn glob_is_allowed_inside_the_roots() {
        let fixture = fixture();
        let reads = reads();
        let (tools, _) = tools(&fixture);
        let result = call(
            &tools,
            &reads,
            "glob",
            json!({"pattern": "*.txt", "path": "."}),
        )
        .await
        .unwrap();
        assert!(text(&result).contains("notes.txt"), "{result:?}");
        let result = call(
            &tools,
            &reads,
            "glob",
            json!({"pattern": "*.txt", "path": "~/sub"}),
        )
        .await
        .unwrap();
        assert!(text(&result).contains("marker.txt"), "{result:?}");
        let result = call(
            &tools,
            &reads,
            "glob",
            json!({"pattern": "*.txt", "path": fixture.read_only}),
        )
        .await
        .unwrap();
        assert!(text(&result).contains("facts.txt"), "{result:?}");
        remove(&fixture);
    }

    #[tokio::test]
    async fn glob_outside_is_refused_and_allowed_without_a_boundary() {
        let fixture = fixture();
        let reads = reads();
        let (tools, plain) = tools(&fixture);
        let result = call(
            &tools,
            &reads,
            "glob",
            json!({"pattern": "*.txt", "path": "../outside"}),
        )
        .await
        .unwrap();
        assert_refused(&result);
        // Ablation.
        let result = call(
            &plain,
            &reads,
            "glob",
            json!({"pattern": "*.txt", "path": "../outside"}),
        )
        .await
        .unwrap();
        assert!(text(&result).contains("secret.txt"), "{result:?}");
        remove(&fixture);
    }

    #[tokio::test]
    async fn glob_through_a_symlink_out_is_refused_and_allowed_without_a_boundary() {
        let fixture = fixture();
        std::os::unix::fs::symlink(&fixture.outside, fixture.workspace.join("link")).unwrap();
        let reads = reads();
        let (tools, plain) = tools(&fixture);
        let result = call(
            &tools,
            &reads,
            "glob",
            json!({"pattern": "*.txt", "path": "link"}),
        )
        .await
        .unwrap();
        assert_refused(&result);
        // Ablation.
        let result = call(
            &plain,
            &reads,
            "glob",
            json!({"pattern": "*.txt", "path": "link"}),
        )
        .await
        .unwrap();
        assert!(text(&result).contains("secret.txt"), "{result:?}");
        remove(&fixture);
    }

    #[tokio::test]
    async fn glob_tilde_resolves_to_the_configured_home() {
        let fixture = fixture();
        let reads = reads();
        let (tools, plain) = tools(&fixture);
        let result = call(
            &tools,
            &reads,
            "glob",
            json!({"pattern": "*.txt", "path": "~/sub"}),
        )
        .await
        .unwrap();
        assert!(text(&result).contains("marker.txt"), "{result:?}");
        // Ablation: without the boundary `~/sub` joins the workspace as a
        // literal directory and the search fails as an input error, never
        // as an outside-roots refusal.
        let result = call(
            &plain,
            &reads,
            "glob",
            json!({"pattern": "*.txt", "path": "~/sub"}),
        )
        .await
        .unwrap();
        assert_eq!(
            result
                .structured_content
                .as_ref()
                .and_then(|it| it["code"].as_str()),
            Some("invalid_input"),
            "{result:?}"
        );
        remove(&fixture);
    }

    #[tokio::test]
    async fn grep_is_allowed_inside_the_roots() {
        let fixture = fixture();
        let reads = reads();
        let (tools, _) = tools(&fixture);
        let result = call(
            &tools,
            &reads,
            "grep",
            json!({"pattern": "workspace-notes", "path": "."}),
        )
        .await
        .unwrap();
        assert!(text(&result).contains("notes.txt"), "{result:?}");
        let result = call(
            &tools,
            &reads,
            "grep",
            json!({"pattern": "home-marker", "path": "~/marker.txt"}),
        )
        .await
        .unwrap();
        assert!(text(&result).contains("marker.txt"), "{result:?}");
        let result = call(
            &tools,
            &reads,
            "grep",
            json!({"pattern": "read-only-facts", "path": fixture.read_only}),
        )
        .await
        .unwrap();
        assert!(text(&result).contains("facts.txt"), "{result:?}");
        remove(&fixture);
    }

    #[tokio::test]
    async fn grep_outside_is_refused_and_allowed_without_a_boundary() {
        let fixture = fixture();
        let reads = reads();
        let (tools, plain) = tools(&fixture);
        for input in [
            json!({"pattern": "secret", "path": "../outside"}),
            json!({"pattern": "secret", "path": fixture.outside}),
        ] {
            let result = call(&tools, &reads, "grep", input.clone()).await.unwrap();
            assert_refused(&result);
            // Ablation.
            let result = call(&plain, &reads, "grep", input).await.unwrap();
            assert!(text(&result).contains("secret"), "{result:?}");
        }
        remove(&fixture);
    }

    #[tokio::test]
    async fn grep_through_a_symlink_out_is_refused_and_allowed_without_a_boundary() {
        let fixture = fixture();
        std::os::unix::fs::symlink(&fixture.outside, fixture.workspace.join("link")).unwrap();
        let reads = reads();
        let (tools, plain) = tools(&fixture);
        let result = call(
            &tools,
            &reads,
            "grep",
            json!({"pattern": "secret", "path": "link"}),
        )
        .await
        .unwrap();
        assert_refused(&result);
        // Ablation.
        let result = call(
            &plain,
            &reads,
            "grep",
            json!({"pattern": "secret", "path": "link"}),
        )
        .await
        .unwrap();
        assert!(text(&result).contains("secret"), "{result:?}");
        remove(&fixture);
    }
}
