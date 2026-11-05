//! The known MCP configuration stores of other local clients.
//!
//! `mcp.import.sources` reports them, and `mcp.import.preview`/`apply` consume
//! one through their `from` input. The converted document never leaves the
//! daemon: the import methods name the store, read it here and use the text
//! in place of the client's `document`, so no value of another client's
//! configuration reaches a client (D26).

use super::import::{MAX_DOCUMENT_BYTES, MAX_ENTRIES};
use serde::{
    Serialize,
    de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{
    cell::Cell,
    ffi::OsString,
    fmt,
    io::{BufReader, Read},
    path::{Path, PathBuf},
    rc::Rc,
};

/// The largest store file read. `~/.claude.json` holds application state
/// beside the MCP servers, so it is larger than a configuration document; the
/// converted document is still held to the import's own limits.
pub(crate) const MAX_SOURCE_BYTES: u64 = 32 * 1024 * 1024;

/// The largest message a row or an error carries.
const MAX_MESSAGE_BYTES: usize = 4 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StoreId {
    ClaudeUser,
    ClaudeLocal,
    CodexUser,
    CodexProject,
    OmpUser,
    OmpProject,
}

impl StoreId {
    pub(crate) fn parse(id: &str) -> Option<Self> {
        match id {
            "claude-code:user" => Some(Self::ClaudeUser),
            "claude-code:local" => Some(Self::ClaudeLocal),
            "codex:user" => Some(Self::CodexUser),
            "codex:project" => Some(Self::CodexProject),
            "omp:user" => Some(Self::OmpUser),
            "omp:project" => Some(Self::OmpProject),
            _ => None,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::ClaudeUser => "claude-code:user",
            Self::ClaudeLocal => "claude-code:local",
            Self::CodexUser => "codex:user",
            Self::CodexProject => "codex:project",
            Self::OmpUser => "omp:user",
            Self::OmpProject => "omp:project",
        }
    }

    fn client(self) -> &'static str {
        match self {
            Self::ClaudeUser | Self::ClaudeLocal => "claude-code",
            Self::CodexUser | Self::CodexProject => "codex",
            Self::OmpUser | Self::OmpProject => "omp",
        }
    }

    fn scope(self) -> &'static str {
        match self {
            Self::ClaudeUser | Self::CodexUser | Self::OmpUser => "user",
            Self::ClaudeLocal => "local",
            Self::CodexProject | Self::OmpProject => "project",
        }
    }

    /// Stores whose file or record is found through the authorized workspace.
    pub(crate) fn requires_workspace(self) -> bool {
        matches!(
            self,
            Self::ClaudeLocal | Self::CodexProject | Self::OmpProject
        )
    }

    /// Stores whose file lives inside the workspace.
    fn workspace_file(self) -> bool {
        matches!(self, Self::CodexProject | Self::OmpProject)
    }

    fn project_file(self, workspace: &Workspace) -> PathBuf {
        match self {
            Self::CodexProject => workspace.canonical.join(".codex/config.toml"),
            Self::OmpProject => workspace.canonical.join(".omp/mcp.json"),
            _ => unreachable!("only a project store has a workspace file"),
        }
    }

    fn fallback(self) -> &'static str {
        match self {
            Self::ClaudeUser | Self::ClaudeLocal => "~/.claude.json",
            Self::CodexUser => "~/.codex/config.toml",
            Self::CodexProject => ".codex/config.toml",
            Self::OmpUser => "~/.omp/agent/mcp.json",
            Self::OmpProject => ".omp/mcp.json",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum State {
    Ready,
    Absent,
    Unreadable,
    TooLarge,
    Malformed,
}

/// Why one store could not be read. It names the problem, never a value.
#[derive(Clone, Debug)]
pub(crate) struct StoreError {
    pub(crate) state: State,
    pub(crate) message: String,
}

impl StoreError {
    fn new(state: State, message: impl Into<String>) -> Self {
        let message = message.into();
        let message = if message.len() <= MAX_MESSAGE_BYTES {
            message
        } else {
            let mut end = MAX_MESSAGE_BYTES;
            while !message.is_char_boundary(end) {
                end -= 1;
            }
            message[..end].to_owned()
        };
        Self { state, message }
    }
    fn absent(message: impl Into<String>) -> Self {
        Self::new(State::Absent, message)
    }
    fn unreadable(message: impl Into<String>) -> Self {
        Self::new(State::Unreadable, message)
    }
    fn too_large(message: impl Into<String>) -> Self {
        Self::new(State::TooLarge, message)
    }
    fn malformed(message: impl Into<String>) -> Self {
        Self::new(State::Malformed, message)
    }

    pub(crate) fn code(&self) -> &'static str {
        match self.state {
            State::Absent | State::Unreadable => "mcp_settings_read",
            State::TooLarge | State::Malformed => "mcp_invalid_config",
            State::Ready => "mcp_settings_read",
        }
    }
}

/// One row of `mcp.import.sources`. It carries the store's identity, the file
/// read, its state and how many entries it holds; no entry id and no value.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Source {
    pub(crate) id: &'static str,
    pub(crate) client: &'static str,
    pub(crate) scope: &'static str,
    pub(crate) path: String,
    pub(crate) state: State,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) entries: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
}

/// One store converted to the standard `mcpServers` document, and the digest
/// that tells one read of it from another.
pub(crate) struct Document {
    pub(crate) text: String,
    pub(crate) revision: String,
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("bytes", &self.text.len())
            .field("revision", &self.revision)
            .finish()
    }
}

/// The authorized workspace a project store belongs to: its canonical path,
/// and the string the request named it by (Claude Code records either).
#[derive(Clone, Debug)]
pub(crate) struct Workspace {
    pub(crate) canonical: PathBuf,
    pub(crate) request: String,
}

/// The daemon's user home and the two documented store overrides.
#[derive(Clone, Debug, Default)]
pub(crate) struct Environment {
    home: Option<PathBuf>,
    claude_config_dir: Option<PathBuf>,
    codex_home: Option<PathBuf>,
}

impl Environment {
    pub(crate) fn from_process() -> Self {
        Self::from_vars(std::env::vars_os())
    }

    fn from_vars(vars: impl IntoIterator<Item = (OsString, OsString)>) -> Self {
        let home_key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let mut environment = Self::default();
        for (name, value) in vars {
            let Some(name) = name.to_str() else { continue };
            let value = PathBuf::from(value);
            if !value.is_absolute() {
                continue;
            }
            match name {
                name if name == home_key => environment.home = Some(value),
                "CLAUDE_CONFIG_DIR" => environment.claude_config_dir = Some(value),
                "CODEX_HOME" => environment.codex_home = Some(value),
                _ => {}
            }
        }
        environment
    }

    fn claude_file(&self) -> Option<PathBuf> {
        match (&self.claude_config_dir, &self.home) {
            (Some(directory), _) => Some(directory.join(".claude.json")),
            (None, Some(home)) => Some(home.join(".claude.json")),
            (None, None) => None,
        }
    }

    fn codex_file(&self) -> Option<PathBuf> {
        match (&self.codex_home, &self.home) {
            (Some(directory), _) => Some(directory.join("config.toml")),
            (None, Some(home)) => Some(home.join(".codex").join("config.toml")),
            (None, None) => None,
        }
    }

    fn omp_file(&self) -> Option<PathBuf> {
        self.home
            .as_ref()
            .map(|home| home.join(".omp").join("agent").join("mcp.json"))
    }
}

struct Prepared {
    text: String,
    entries: usize,
}

/// The rows of `mcp.import.sources`: the three user stores, and with an
/// authorized workspace also Claude Code's local record and the two project
/// stores. Each distinct file is read once.
pub(crate) fn sources(environment: &Environment, workspace: Option<&Workspace>) -> Vec<Source> {
    let mut rows = Vec::with_capacity(6);
    let claude_path = display_path(StoreId::ClaudeUser, environment, workspace);
    let claude = environment
        .claude_file()
        .map(|file| read_claude(&file, workspace, MAX_SOURCE_BYTES));
    rows.push(match &claude {
        Some(Ok(file)) => match claude_user(file) {
            Ok(prepared) => ready(StoreId::ClaudeUser, claude_path, &prepared),
            Err(error) => failed(StoreId::ClaudeUser, claude_path, error),
        },
        Some(Err(error)) => failed(StoreId::ClaudeUser, claude_path, error.clone()),
        None => failed(
            StoreId::ClaudeUser,
            claude_path,
            StoreError::unreadable("the home directory is not set"),
        ),
    });
    if let Some(workspace) = workspace {
        let path = display_path(StoreId::ClaudeLocal, environment, Some(workspace));
        let key = workspace.canonical.to_str().unwrap_or_default().to_owned();
        rows.push(match &claude {
            Some(Ok(file)) => match claude_local(file, workspace) {
                Ok((prepared, matched)) => ready(
                    StoreId::ClaudeLocal,
                    format!("{path} · projects[{matched:?}]"),
                    &prepared,
                ),
                Err(error) => failed(
                    StoreId::ClaudeLocal,
                    format!("{path} · projects[{key:?}]"),
                    error,
                ),
            },
            Some(Err(error)) => failed(
                StoreId::ClaudeLocal,
                format!("{path} · projects[{key:?}]"),
                error.clone(),
            ),
            None => failed(
                StoreId::ClaudeLocal,
                format!("{path} · projects[{key:?}]"),
                StoreError::unreadable("the home directory is not set"),
            ),
        });
    }
    rows.push(store_source(StoreId::CodexUser, environment, workspace));
    if workspace.is_some() {
        rows.push(store_source(StoreId::CodexProject, environment, workspace));
    }
    rows.push(store_source(StoreId::OmpUser, environment, workspace));
    if workspace.is_some() {
        rows.push(store_source(StoreId::OmpProject, environment, workspace));
    }
    rows
}

/// Reads one store and converts it to the standard document. `document()` is
/// the import's input; `sources()` reports the same store's state.
pub(crate) fn document(
    store: StoreId,
    environment: &Environment,
    workspace: Option<&Workspace>,
) -> Result<Document, StoreError> {
    let prepared = prepare(store, environment, workspace)?;
    let revision = format!("{:x}", Sha256::digest(prepared.text.as_bytes()));
    Ok(Document {
        text: prepared.text,
        revision,
    })
}

fn store_source(
    store: StoreId,
    environment: &Environment,
    workspace: Option<&Workspace>,
) -> Source {
    let path = display_path(store, environment, workspace);
    match prepare(store, environment, workspace) {
        Ok(prepared) => ready(store, path, &prepared),
        Err(error) => failed(store, path, error),
    }
}

fn ready(store: StoreId, path: String, prepared: &Prepared) -> Source {
    Source {
        id: store.id(),
        client: store.client(),
        scope: store.scope(),
        path,
        state: State::Ready,
        entries: Some(prepared.entries),
        error: None,
    }
}

fn failed(store: StoreId, path: String, error: StoreError) -> Source {
    let state = error.state;
    let error = matches!(state, State::Unreadable | State::Malformed).then_some(error.message);
    Source {
        id: store.id(),
        client: store.client(),
        scope: store.scope(),
        path,
        state,
        entries: None,
        error,
    }
}

fn prepare(
    store: StoreId,
    environment: &Environment,
    workspace: Option<&Workspace>,
) -> Result<Prepared, StoreError> {
    match store {
        StoreId::ClaudeUser => {
            let file = environment
                .claude_file()
                .ok_or_else(|| StoreError::unreadable("the home directory is not set"))?;
            claude_user(&read_claude(&file, None, MAX_SOURCE_BYTES)?)
        }
        StoreId::ClaudeLocal => {
            let workspace =
                workspace.ok_or_else(|| StoreError::unreadable("a workspace is required"))?;
            let file = environment
                .claude_file()
                .ok_or_else(|| StoreError::unreadable("the home directory is not set"))?;
            let parsed = read_claude(&file, Some(workspace), MAX_SOURCE_BYTES)?;
            let (prepared, _) = claude_local(&parsed, workspace)?;
            Ok(prepared)
        }
        StoreId::CodexUser | StoreId::CodexProject => {
            let file = store_path(store, environment, workspace)?;
            let text = read_text(&file, MAX_SOURCE_BYTES)?;
            prepared(codex_entries(&text)?, Map::new())
        }
        StoreId::OmpUser => {
            let file = environment
                .omp_file()
                .ok_or_else(|| StoreError::unreadable("the home directory is not set"))?;
            omp(&read_json(&file, MAX_SOURCE_BYTES)?)
        }
        StoreId::OmpProject => {
            let file = store_path(store, environment, workspace)?;
            omp(&read_json(&file, MAX_SOURCE_BYTES)?)
        }
    }
}

/// The file a project store names, resolved and confined to the workspace.
/// A user store's file is read through a symlink; a project store's target
/// must stay inside the workspace the request authorized.
fn store_path(
    store: StoreId,
    environment: &Environment,
    workspace: Option<&Workspace>,
) -> Result<PathBuf, StoreError> {
    if store.workspace_file() {
        let workspace =
            workspace.ok_or_else(|| StoreError::unreadable("a workspace is required"))?;
        let path = store.project_file(workspace);
        let canonical = std::fs::canonicalize(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                StoreError::absent("the configuration file does not exist")
            } else {
                StoreError::unreadable(error.to_string())
            }
        })?;
        if !canonical.starts_with(&workspace.canonical) {
            return Err(StoreError::unreadable(
                "the file resolves outside the workspace",
            ));
        }
        Ok(canonical)
    } else {
        match store {
            StoreId::CodexUser => environment
                .codex_file()
                .ok_or_else(|| StoreError::unreadable("the home directory is not set")),
            StoreId::OmpUser => environment
                .omp_file()
                .ok_or_else(|| StoreError::unreadable("the home directory is not set")),
            _ => Err(StoreError::unreadable("the store is read from a workspace")),
        }
    }
}

fn display_path(
    store: StoreId,
    environment: &Environment,
    workspace: Option<&Workspace>,
) -> String {
    let path = if store.workspace_file() {
        workspace.map(|workspace| store.project_file(workspace))
    } else {
        match store {
            StoreId::ClaudeUser | StoreId::ClaudeLocal => environment.claude_file(),
            StoreId::CodexUser => environment.codex_file(),
            StoreId::OmpUser => environment.omp_file(),
            StoreId::CodexProject | StoreId::OmpProject => None,
        }
    };
    let Some(path) = path else {
        return store.fallback().to_owned();
    };
    if store.workspace_file()
        && let Some(workspace) = workspace
        && let Ok(relative) = path.strip_prefix(&workspace.canonical)
    {
        return relative.display().to_string();
    }
    if let Some(home) = &environment.home
        && let Ok(relative) = path.strip_prefix(home)
    {
        return format!("~/{}", relative.display());
    }
    path.display().to_string()
}

fn read_json(path: &Path, limit: u64) -> Result<Value, StoreError> {
    let text = read_text(path, limit)?;
    serde_json::from_str(&text).map_err(|error| {
        StoreError::malformed(format!(
            "the file is not valid JSON at line {} column {}",
            error.line(),
            error.column()
        ))
    })
}

fn read_text(path: &Path, limit: u64) -> Result<String, StoreError> {
    let bytes = read_file(path, limit)?;
    String::from_utf8(bytes).map_err(|_| StoreError::malformed("the file is not UTF-8"))
}

fn read_file(path: &Path, limit: u64) -> Result<Vec<u8>, StoreError> {
    let file = open_source(path, limit)?;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| StoreError::unreadable(error.to_string()))?;
    if bytes.len() as u64 > limit {
        return Err(StoreError::too_large(format!(
            "the file exceeds {limit} bytes"
        )));
    }
    Ok(bytes)
}

/// Opens a store file after the same checks every source gets: it must exist,
/// be a regular file, be openable, and its metadata must not exceed `limit`.
/// The open file is measured again, and the read paths measure what they
/// actually read, because the file can grow after this check.
fn open_source(path: &Path, limit: u64) -> Result<std::fs::File, StoreError> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Err(StoreError::unreadable("the path is not a regular file")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(StoreError::absent("the configuration file does not exist"));
        }
        Err(error) => return Err(StoreError::unreadable(error.to_string())),
    }
    let file =
        std::fs::File::open(path).map_err(|error| StoreError::unreadable(error.to_string()))?;
    let metadata = file
        .metadata()
        .map_err(|error| StoreError::unreadable(error.to_string()))?;
    if !metadata.is_file() {
        return Err(StoreError::unreadable("the path is not a regular file"));
    }
    if metadata.len() > limit {
        return Err(StoreError::too_large(format!(
            "the file exceeds {limit} bytes"
        )));
    }
    Ok(file)
}

fn object_of(value: &Value) -> Result<&Map<String, Value>, StoreError> {
    value
        .as_object()
        .ok_or_else(|| StoreError::malformed("the configuration file must be a JSON object"))
}

fn servers_of(object: &Map<String, Value>) -> Result<Map<String, Value>, StoreError> {
    match object.get("mcpServers") {
        None => Ok(Map::new()),
        Some(Value::Object(servers)) => Ok(servers.clone()),
        Some(_) => Err(StoreError::malformed("mcpServers must be an object")),
    }
}

/// The part of `~/.claude.json` the two Claude rows use. Every other member
/// and every other project's record is skipped with `IgnoredAny`, so the parse
/// never builds it; a file with thousands of project records costs one scan,
/// not one tree.
///
/// `Debug` is redacted: this type and `Projects` hold the raw `mcpServers`
/// values, which may be an environment, a header or a bearer token. It names
/// the type and counts, never a value, as `Document` does.
struct ClaudeFile {
    mcp_servers: Option<Value>,
    projects: Option<Projects>,
}

impl fmt::Debug for ClaudeFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClaudeFile")
            .field(
                "mcp_servers",
                &self
                    .mcp_servers
                    .as_ref()
                    .map(|value| value.as_object().map_or(0, Map::len)),
            )
            .field("projects", &self.projects)
            .finish()
    }
}

/// The `projects` member of `ClaudeFile`; see there for the redacted `Debug`.
enum Projects {
    NotObject,
    /// The record under the canonical cwd and the one under the request's
    /// string. At most those two are built; the selection prefers the first.
    Records {
        canonical: Option<Value>,
        request: Option<Value>,
    },
}

impl fmt::Debug for Projects {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotObject => f.write_str("Projects::NotObject"),
            Self::Records { .. } => f
                .debug_struct("Projects::Records")
                .field("retained", &self.retained())
                .finish(),
        }
    }
}

impl Projects {
    /// How many project records the parse kept.
    fn retained(&self) -> usize {
        match self {
            Self::NotObject => 0,
            Self::Records { canonical, request } => {
                canonical.is_some() as usize + request.is_some() as usize
            }
        }
    }
}

/// Reads and parses one Claude file. `limit` is the size cap; production
/// passes `MAX_SOURCE_BYTES`.
fn read_claude(
    path: &Path,
    workspace: Option<&Workspace>,
    limit: u64,
) -> Result<ClaudeFile, StoreError> {
    parse_claude(open_source(path, limit)?, workspace, limit)
}

/// Parses the file as it streams through the capped reader, so a discovery
/// call holds no copy of the file and its memory does not follow the file's
/// size. The size was already checked from the metadata by `open_source`;
/// `limit` bounds this read too, in case the file grew since.
fn parse_claude<R: Read>(
    reader: R,
    workspace: Option<&Workspace>,
    limit: u64,
) -> Result<ClaudeFile, StoreError> {
    let read = Rc::new(Cell::new(0));
    let reader = BufReader::new(CountingReader {
        inner: reader.take(limit + 1),
        read: Rc::clone(&read),
    });
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    let top = ClaudeTopSeed { workspace }.deserialize(&mut deserializer);
    let parsed = match top {
        Ok(top) => match deserializer.end() {
            Ok(()) => match top {
                Some(file) => Ok(file),
                None => Err(StoreError::malformed(
                    "the configuration file must be a JSON object",
                )),
            },
            Err(error) => Err(invalid_json(error)),
        },
        Err(error) => Err(invalid_json(error)),
    };
    // The cap follows what the read saw, not what the parse needed: a file
    // that reaches the capped end is too large whatever the parse made of it.
    if read.get() > limit {
        return Err(StoreError::too_large(format!(
            "the file exceeds {limit} bytes"
        )));
    }
    parsed
}

/// Counts the bytes a parser pulls, so the size cap follows the read.
struct CountingReader<R> {
    inner: R,
    read: Rc<Cell<u64>>,
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let count = self.inner.read(buffer)?;
        self.read.set(self.read.get() + count as u64);
        Ok(count)
    }
}

fn invalid_json(error: serde_json::Error) -> StoreError {
    StoreError::malformed(format!(
        "the file is not valid JSON at line {} column {}",
        error.line(),
        error.column()
    ))
}

/// The top-level reader: `Some` for the configuration object and `None` for a
/// value of another shape, which keeps its old message. Every shape reaches
/// the parser the same way, so long leading whitespace still counts as one.
struct ClaudeTopSeed<'a> {
    workspace: Option<&'a Workspace>,
}

impl<'de> DeserializeSeed<'de> for ClaudeTopSeed<'_> {
    type Value = Option<ClaudeFile>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(ClaudeTopVisitor {
            workspace: self.workspace,
        })
    }
}

struct ClaudeTopVisitor<'a> {
    workspace: Option<&'a Workspace>,
}

impl<'de> Visitor<'de> for ClaudeTopVisitor<'_> {
    type Value = Option<ClaudeFile>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a Claude Code configuration object")
    }

    fn visit_map<M>(self, map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        ClaudeVisitor {
            workspace: self.workspace,
        }
        .visit_map(map)
        .map(Some)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while sequence.next_element::<IgnoredAny>()?.is_some() {}
        Ok(None)
    }

    fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_str<E>(self, _value: &str) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(None)
    }
}

struct ClaudeVisitor<'a> {
    workspace: Option<&'a Workspace>,
}

impl<'de> Visitor<'de> for ClaudeVisitor<'_> {
    type Value = ClaudeFile;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a Claude Code configuration object")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut mcp_servers = None;
        let mut projects = None;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "mcpServers" => mcp_servers = Some(map.next_value::<Value>()?),
                "projects" => {
                    projects = Some(map.next_value_seed(ProjectsSeed {
                        workspace: self.workspace,
                    })?)
                }
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(ClaudeFile {
            mcp_servers,
            projects,
        })
    }
}

struct ProjectsSeed<'a> {
    workspace: Option<&'a Workspace>,
}

impl<'de> DeserializeSeed<'de> for ProjectsSeed<'_> {
    type Value = Projects;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(ProjectsVisitor {
            workspace: self.workspace,
        })
    }
}

#[derive(Clone, Copy)]
struct ProjectsVisitor<'a> {
    workspace: Option<&'a Workspace>,
}

impl<'de> Visitor<'de> for ProjectsVisitor<'_> {
    type Value = Projects;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("an object of Claude Code project records")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let keys = self.workspace.and_then(|workspace| {
            workspace
                .canonical
                .to_str()
                .map(|canonical| (canonical, workspace.request.as_str()))
        });
        let mut canonical = None;
        let mut request = None;
        while let Some(key) = map.next_key::<String>()? {
            match keys {
                Some((canonical_key, _)) if key == canonical_key => {
                    canonical = Some(map.next_value::<Value>()?);
                }
                Some((_, request_key)) if key == request_key => {
                    request = Some(map.next_value::<Value>()?);
                }
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(Projects::Records { canonical, request })
    }

    fn visit_seq<S>(self, mut seq: S) -> Result<Self::Value, S::Error>
    where
        S: SeqAccess<'de>,
    {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(Projects::NotObject)
    }

    fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
        Ok(Projects::NotObject)
    }

    fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
        Ok(Projects::NotObject)
    }

    fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
        Ok(Projects::NotObject)
    }

    fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
        Ok(Projects::NotObject)
    }

    fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
        Ok(Projects::NotObject)
    }

    fn visit_string<E>(self, _: String) -> Result<Self::Value, E> {
        Ok(Projects::NotObject)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Projects::NotObject)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Projects::NotObject)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }
}

fn claude_user(file: &ClaudeFile) -> Result<Prepared, StoreError> {
    let servers = match &file.mcp_servers {
        None => Map::new(),
        Some(Value::Object(servers)) => servers.clone(),
        Some(_) => return Err(StoreError::malformed("mcpServers must be an object")),
    };
    prepared(servers, Map::new())
}

/// The workspace's record inside `projects`, under the canonical path and then
/// the request's string. No other record was built, so none is enumerated.
fn claude_local(
    file: &ClaudeFile,
    workspace: &Workspace,
) -> Result<(Prepared, String), StoreError> {
    let (canonical, request) = match &file.projects {
        None => {
            return Err(StoreError::absent(
                "Claude Code records no local servers for this workspace",
            ));
        }
        Some(Projects::NotObject) => {
            return Err(StoreError::malformed("projects must be an object"));
        }
        Some(Projects::Records { canonical, request }) => (canonical, request),
    };
    let canonical_key = workspace
        .canonical
        .to_str()
        .ok_or_else(|| StoreError::unreadable("the workspace path is not UTF-8"))?;
    let (key, record) = match (canonical, request) {
        (Some(record), _) => (canonical_key.to_owned(), record),
        (None, Some(record)) => (workspace.request.clone(), record),
        (None, None) => {
            return Err(StoreError::absent(
                "Claude Code records no local servers for this workspace",
            ));
        }
    };
    let record = record
        .as_object()
        .ok_or_else(|| StoreError::malformed("the project record must be an object"))?;
    Ok((prepared(servers_of(record)?, Map::new())?, key))
}

/// OMP's file is itself an MCP document: the servers member plus the
/// top-level members the import does not carry, so the preview names them.
fn omp(value: &Value) -> Result<Prepared, StoreError> {
    let object = object_of(value)?;
    let servers = servers_of(object)?;
    let mut extras = Map::new();
    for (name, value) in object {
        if name != "mcpServers" {
            extras.insert(name.clone(), value.clone());
        }
    }
    prepared(servers, extras)
}

fn prepared(
    servers: Map<String, Value>,
    extras: Map<String, Value>,
) -> Result<Prepared, StoreError> {
    let entries = servers.len();
    if entries > MAX_ENTRIES {
        return Err(StoreError::too_large(format!(
            "the store holds more than {MAX_ENTRIES} entries"
        )));
    }
    let mut document = Map::new();
    document.insert("mcpServers".to_owned(), Value::Object(servers));
    for (name, value) in extras {
        document.insert(name, value);
    }
    let text = serde_json::to_string(&Value::Object(document)).expect("a JSON object serialises");
    if text.len() > MAX_DOCUMENT_BYTES {
        return Err(StoreError::too_large(format!(
            "the converted document exceeds {MAX_DOCUMENT_BYTES} bytes"
        )));
    }
    Ok(Prepared { text, entries })
}

fn codex_entries(text: &str) -> Result<Map<String, Value>, StoreError> {
    let document = parse_toml(text)?;
    let toml::Value::Table(document) = document else {
        return Err(StoreError::malformed("the file must be a TOML table"));
    };
    let servers = match document.get("mcp_servers") {
        None => Map::new(),
        Some(toml::Value::Table(servers)) => servers
            .iter()
            .map(|(id, value)| (id.clone(), codex_entry(value.clone())))
            .collect(),
        Some(_) => return Err(StoreError::malformed("mcp_servers must be a table")),
    };
    Ok(servers)
}

/// One Codex `[mcp_servers.<id>]` table in Dim's vocabulary. A member that
/// does not map stays as written, so the preview reports it unused.
fn codex_entry(value: toml::Value) -> Value {
    let toml::Value::Table(table) = value else {
        return json_of(value);
    };
    let mut members = Map::new();
    let mut headers = Map::new();
    let mut remote = false;
    for (name, value) in table {
        match name.as_str() {
            "url" => {
                remote = true;
                members.insert("url".to_owned(), json_of(value));
            }
            "bearer_token_env_var" => match value {
                toml::Value::String(variable) => {
                    members.insert(
                        "auth".to_owned(),
                        json!({"type": "bearer", "token": format!("${{{variable}}}")}),
                    );
                }
                other => {
                    members.insert(name.clone(), json_of(other));
                }
            },
            "http_headers" => match value {
                toml::Value::Table(received) => {
                    for (header, value) in received {
                        headers.insert(header, json_of(value));
                    }
                }
                other => {
                    members.insert(name.clone(), json_of(other));
                }
            },
            "env_http_headers" => match value {
                toml::Value::Table(env_headers) => {
                    for (header, value) in env_headers {
                        let value = match value {
                            toml::Value::String(variable) => {
                                Value::String(format!("${{{variable}}}"))
                            }
                            other => json_of(other),
                        };
                        headers.insert(header, value);
                    }
                }
                other => {
                    members.insert(name.clone(), json_of(other));
                }
            },
            "tool_timeout_sec" => match value {
                toml::Value::Integer(seconds) => match seconds.checked_mul(1000) {
                    Some(millis) => {
                        members.insert("timeout".to_owned(), Value::Number(millis.into()));
                    }
                    None => {
                        members.insert(name.clone(), json_of(toml::Value::Integer(seconds)));
                    }
                },
                other => {
                    members.insert(name.clone(), json_of(other));
                }
            },
            _ => {
                members.insert(name, json_of(value));
            }
        }
    }
    if !headers.is_empty() {
        members.insert("headers".to_owned(), Value::Object(headers));
    }
    if remote {
        members.insert("type".to_owned(), Value::String("http".to_owned()));
    }
    Value::Object(members)
}

fn parse_toml(text: &str) -> Result<toml::Value, StoreError> {
    toml::from_str(text).map_err(|error| {
        let position = error
            .span()
            .and_then(|span| text.get(..span.start))
            .map(|prefix| {
                let line = prefix.matches('\n').count() + 1;
                let column = prefix
                    .rsplit('\n')
                    .next()
                    .map_or(1, |line| line.chars().count() + 1);
                format!(" at line {line} column {column}")
            })
            .unwrap_or_default();
        // The parser's own message can quote the source line, which may hold
        // a credential; only its position is reported.
        StoreError::malformed(format!("the file is not valid TOML{position}"))
    })
}

fn json_of(value: toml::Value) -> Value {
    match value {
        toml::Value::String(text) => Value::String(text),
        toml::Value::Integer(number) => Value::Number(number.into()),
        toml::Value::Float(number) => serde_json::Number::from_f64(number)
            .map_or_else(|| Value::String(number.to_string()), Value::Number),
        toml::Value::Boolean(flag) => Value::Bool(flag),
        toml::Value::Datetime(time) => Value::String(time.to_string()),
        toml::Value::Array(items) => Value::Array(items.into_iter().map(json_of).collect()),
        toml::Value::Table(table) => Value::Object(
            table
                .into_iter()
                .map(|(name, value)| (name, json_of(value)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(path: &str) -> Workspace {
        Workspace {
            canonical: PathBuf::from(path),
            request: path.to_owned(),
        }
    }

    fn servers(prepared: &Prepared) -> Value {
        serde_json::from_str::<Value>(&prepared.text).unwrap()["mcpServers"].clone()
    }

    #[test]
    fn codex_entries_convert_the_documented_members() {
        let entries = codex_entries(
            r#"
[mcp_servers.github]
command = "gh-mcp"
args = ["--flag"]
cwd = "/work"
enabled = false
tool_timeout_sec = 90
startup_timeout_sec = 5
env = { TOKEN = "secret" }

[mcp_servers.remote]
url = "https://example.invalid/mcp"
bearer_token_env_var = "GH_TOKEN"
http_headers = { "X-Static" = "value" }
env_http_headers = { "X-Env" = "OTHER" }
"#,
        )
        .unwrap();
        assert_eq!(entries["github"]["command"], json!("gh-mcp"));
        assert_eq!(entries["github"]["args"], json!(["--flag"]));
        assert_eq!(entries["github"]["cwd"], json!("/work"));
        assert_eq!(entries["github"]["enabled"], json!(false));
        assert_eq!(entries["github"]["timeout"], json!(90_000));
        assert_eq!(entries["github"]["startup_timeout_sec"], json!(5));
        assert_eq!(entries["github"]["env"], json!({"TOKEN": "secret"}));
        assert!(entries["github"].get("tool_timeout_sec").is_none());
        assert_eq!(entries["remote"]["type"], json!("http"));
        assert_eq!(
            entries["remote"]["url"],
            json!("https://example.invalid/mcp")
        );
        assert_eq!(
            entries["remote"]["auth"],
            json!({"type": "bearer", "token": "${GH_TOKEN}"})
        );
        assert_eq!(
            entries["remote"]["headers"],
            json!({"X-Static": "value", "X-Env": "${OTHER}"})
        );
        assert!(entries["remote"].get("bearer_token_env_var").is_none());
    }

    #[test]
    fn a_non_integral_tool_timeout_stays_unmapped() {
        let entries =
            codex_entries("[mcp_servers.x]\r\ncommand = \"x\"\ntool_timeout_sec = 1.5\n").unwrap();
        assert_eq!(entries["x"]["tool_timeout_sec"], json!(1.5));
        assert!(entries["x"].get("timeout").is_none());
    }

    #[test]
    fn claude_reads_only_the_requested_project_record() {
        let text = r#"{
            "mcpServers": {"user": {"command": "u"}},
            "projects": {
                "/workspace": {"mcpServers": {"local": {"command": "l"}}},
                "/other": {"mcpServers": {"other": {"command": "o"}}}
            }
        }"#;
        let workspace = workspace("/workspace");
        let file = parse_claude(text.as_bytes(), Some(&workspace), MAX_SOURCE_BYTES).unwrap();
        let user = claude_user(&file).unwrap();
        assert!(servers(&user).get("user").is_some());
        assert!(servers(&user).get("local").is_none());
        let (local, key) = claude_local(&file, &workspace).unwrap();
        assert_eq!(key, "/workspace");
        assert!(servers(&local).get("local").is_some());
        assert!(servers(&local).get("other").is_none());
        assert!(servers(&local).get("user").is_none());
    }

    /// Another project's record is skipped whole: a value that is not a server
    /// map there cannot change either row, and only the records a request
    /// could name are ever built.
    #[test]
    fn another_projects_record_is_never_built() {
        let text = r#"{
            "mcpServers": {"user": {"command": "u"}},
            "projects": {
                "/workspace": {"mcpServers": {"local": {"command": "l"}}},
                "/other": {"mcpServers": 5}
            }
        }"#;
        let selected = workspace("/workspace");
        let file = parse_claude(text.as_bytes(), Some(&selected), MAX_SOURCE_BYTES).unwrap();
        assert_eq!(file.projects.as_ref().unwrap().retained(), 1);
        let user = claude_user(&file).unwrap();
        assert_eq!(servers(&user), json!({"user": {"command": "u"}}));
        let (local, key) = claude_local(&file, &selected).unwrap();
        assert_eq!(key, "/workspace");
        assert_eq!(servers(&local), json!({"local": {"command": "l"}}));
        // The skipped record is malformed only when a later request names it.
        let other = workspace("/other");
        let file = parse_claude(text.as_bytes(), Some(&other), MAX_SOURCE_BYTES).unwrap();
        let error = match claude_local(&file, &other) {
            Ok(_) => panic!("the other record is malformed when it is read"),
            Err(error) => error,
        };
        assert_eq!(error.state, State::Malformed);
        assert_eq!(error.message, "mcpServers must be an object");
    }

    #[test]
    fn a_claude_file_of_another_shape_keeps_its_old_messages() {
        let error = parse_claude("[1, 2]".as_bytes(), None, MAX_SOURCE_BYTES).unwrap_err();
        assert_eq!(error.state, State::Malformed);
        assert_eq!(
            error.message,
            "the configuration file must be a JSON object"
        );
        let error =
            parse_claude("{\"mcpServers\": }".as_bytes(), None, MAX_SOURCE_BYTES).unwrap_err();
        assert_eq!(error.state, State::Malformed);
        assert!(
            error
                .message
                .starts_with("the file is not valid JSON at line 1 column "),
            "{}",
            error.message
        );
        // Leading whitespace may run past any read buffer; the shape still
        // counts, so a file of either shape reaches the same parser.
        let text = format!("{}  {{\"mcpServers\": {{}}}}", " ".repeat(9 * 1024));
        let file = parse_claude(text.as_bytes(), None, MAX_SOURCE_BYTES).unwrap();
        assert_eq!(file.mcp_servers, Some(json!({})));
    }

    /// The parsed-store types format counts only: a fixture secret must not
    /// reach a log through `{:?}`.
    #[test]
    fn parsed_store_values_stay_out_of_debug() {
        const SECRET: &str = "fixture-store-secret";
        let text = json!({
            "mcpServers": {
                "remote": {"headers": {"Authorization": format!("Bearer {SECRET}")}}
            },
            "projects": {
                "/workspace": {"mcpServers": {"local": {"env": {"TOKEN": SECRET}}}}
            }
        })
        .to_string();
        let workspace = workspace("/workspace");
        let file = parse_claude(text.as_bytes(), Some(&workspace), MAX_SOURCE_BYTES).unwrap();
        let formatted = format!("{file:?}");
        assert!(!formatted.contains(SECRET), "{formatted}");
        assert!(formatted.starts_with("ClaudeFile"), "{formatted}");
        assert!(formatted.contains("mcp_servers: Some(1)"), "{formatted}");
        let projects = format!("{:?}", file.projects.as_ref().expect("the record"));
        assert_eq!(projects, "Projects::Records { retained: 1 }");
    }

    #[test]
    fn omp_carries_its_other_top_level_members() {
        let value: Value = serde_json::from_str(
            r#"{"$schema": "schema", "mcpServers": {"x": {"command": "x"}}, "disabledServers": ["y"]}"#,
        )
        .unwrap();
        let prepared = omp(&value).unwrap();
        let document: Value = serde_json::from_str(&prepared.text).unwrap();
        assert_eq!(document["mcpServers"], json!({"x": {"command": "x"}}));
        assert_eq!(document["$schema"], json!("schema"));
        assert_eq!(document["disabledServers"], json!(["y"]));
        assert_eq!(prepared.entries, 1);
    }

    #[test]
    fn only_absolute_store_overrides_are_used() {
        #[cfg(unix)]
        let vars = vec![
            (OsString::from("HOME"), OsString::from("/home/user")),
            (
                OsString::from("CLAUDE_CONFIG_DIR"),
                OsString::from("relative"),
            ),
            (OsString::from("CODEX_HOME"), OsString::from("/codex")),
        ];
        #[cfg(windows)]
        let vars = vec![
            (
                OsString::from("USERPROFILE"),
                OsString::from(r"C:\Users\user"),
            ),
            (
                OsString::from("CLAUDE_CONFIG_DIR"),
                OsString::from("relative"),
            ),
            (OsString::from("CODEX_HOME"), OsString::from(r"C:\codex")),
        ];
        let environment = Environment::from_vars(vars);
        assert!(environment.home.is_some());
        assert_eq!(environment.claude_config_dir, None);
        assert!(environment.codex_home.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn a_project_store_outside_the_workspace_is_unreadable() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(format!("dim-discovery-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("workspace/.codex")).unwrap();
        std::fs::write(
            root.join("outside.toml"),
            "[mcp_servers.x]\ncommand = \"secret\"\n",
        )
        .unwrap();
        symlink(
            root.join("outside.toml"),
            root.join("workspace/.codex/config.toml"),
        )
        .unwrap();
        let environment = Environment {
            home: Some(root.join("home")),
            ..Default::default()
        };
        let workspace = Workspace {
            canonical: std::fs::canonicalize(root.join("workspace")).unwrap(),
            request: root.join("workspace").to_string_lossy().into_owned(),
        };
        let error = document(StoreId::CodexProject, &environment, Some(&workspace)).unwrap_err();
        assert_eq!(error.state, State::Unreadable);
        assert!(
            error.message.contains("outside the workspace"),
            "{}",
            error.message
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_source_file_larger_than_the_cap_is_too_large() {
        let root = std::env::temp_dir().join(format!("dim-discovery-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join(".omp/agent")).unwrap();
        let file = std::fs::File::create(root.join(".omp/agent/mcp.json")).unwrap();
        file.set_len(MAX_SOURCE_BYTES + 1).unwrap();
        drop(file);
        let environment = Environment {
            home: Some(root.clone()),
            ..Default::default()
        };
        let error = document(StoreId::OmpUser, &environment, None).unwrap_err();
        assert_eq!(error.state, State::TooLarge);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The reader's cap is what the read saw: one byte over a lowered cap is
    /// `too-large`, even though the JSON itself parses.
    #[test]
    fn a_claude_file_just_over_a_lowered_cap_is_too_large() {
        let root = std::env::temp_dir().join(format!("dim-discovery-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join(".claude.json");
        let text = r#"{"mcpServers": {"x": {"command": "x"}}}"#;
        std::fs::write(&path, text).unwrap();
        let limit = text.len() as u64 - 1;
        let error = parse_claude(std::fs::File::open(&path).unwrap(), None, limit).unwrap_err();
        assert_eq!(error.state, State::TooLarge);
        assert_eq!(error.message, format!("the file exceeds {limit} bytes"));
        let file =
            parse_claude(std::fs::File::open(&path).unwrap(), None, text.len() as u64).unwrap();
        assert_eq!(file.mcp_servers, Some(json!({"x": {"command": "x"}})));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The size is classified from the metadata before the parse, so a syntax
    /// error near the start cannot answer `malformed` for a file over the
    /// cap. The lowered cap is above the parser's read buffer: a larger read
    /// would trip the counting reader's own cap and hide the metadata check.
    #[test]
    fn a_malformed_claude_file_just_over_a_lowered_cap_is_too_large() {
        const LIMIT: u64 = 64 * 1024;
        let root = std::env::temp_dir().join(format!("dim-discovery-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join(".claude.json");
        let mut text = String::from("}");
        text.push_str(&" ".repeat(LIMIT as usize));
        std::fs::write(&path, &text).unwrap();
        assert_eq!(text.len() as u64, LIMIT + 1);
        let error = match read_claude(&path, None, LIMIT) {
            Ok(_) => panic!("the over-cap file parses"),
            Err(error) => error,
        };
        assert_eq!(error.state, State::TooLarge);
        assert_eq!(error.message, format!("the file exceeds {LIMIT} bytes"));
        // At the cap the parse runs and reports the syntax error.
        let error = match read_claude(&path, None, text.len() as u64) {
            Ok(_) => panic!("the malformed file parses"),
            Err(error) => error,
        };
        assert_eq!(error.state, State::Malformed);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Every store reads through `open_source`, so the OMP row is classified
    /// from its metadata too.
    #[test]
    fn a_malformed_omp_file_just_over_a_lowered_cap_is_too_large() {
        const LIMIT: u64 = 64 * 1024;
        let root = std::env::temp_dir().join(format!("dim-discovery-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("mcp.json");
        let mut text = String::from("}");
        text.push_str(&" ".repeat(LIMIT as usize));
        std::fs::write(&path, &text).unwrap();
        assert_eq!(text.len() as u64, LIMIT + 1);
        let error = read_json(&path, LIMIT).unwrap_err();
        assert_eq!(error.state, State::TooLarge);
        assert_eq!(error.message, format!("the file exceeds {LIMIT} bytes"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Both reads of one store answer `too-large`: the `sources` row and the
    /// `from` document the import reads. The fixture file is sparse, so it
    /// costs no disk space.
    #[test]
    fn an_oversized_malformed_store_is_too_large_from_sources_and_from() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("dim-discovery-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join(".claude.json");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"}{").unwrap();
        file.set_len(MAX_SOURCE_BYTES + 1).unwrap();
        drop(file);
        let environment = Environment {
            home: Some(root.clone()),
            ..Default::default()
        };
        let error = document(StoreId::ClaudeUser, &environment, None).unwrap_err();
        assert_eq!(error.state, State::TooLarge);
        let rows = sources(&environment, None);
        let user = rows
            .iter()
            .find(|row| row.id == "claude-code:user")
            .expect("the user row is reported");
        assert_eq!(user.state, State::TooLarge);
        assert!(user.error.is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
