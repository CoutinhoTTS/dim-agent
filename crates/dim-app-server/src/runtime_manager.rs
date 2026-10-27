//! RuntimeManager (docs/design/plugins/RuntimeManager.md): acquires the Node
//! and Python rutis runtimes into `<home>/runtimes/`, verified against pins
//! compiled into Dim, and mounts each one once it is installed. Plugin rows
//! gate on the runtime's name under rutis' rules.
use crate::methods::ManagementMethods;
use dim_agent_capabilities::methods::{Interruption, MethodDecl, RpcError, params, reply};
use dim_agent_runtime::Names;
use rutis::Ctx;
use rutis::host::{BoxFuture, CtxHost, DynPlugin, Effect, FiberView, Key, TypeKey};
use rutis_bridge::runtime::{LocalRuntime, RuntimeState};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

const PINS: &str = include_str!("../../../runtime/runtimes.json");
const NODE_PACKAGE: &str = include_str!("../../../runtime/package.json");
const NODE_LOCK: &str = include_str!("../../../runtime/package-lock.json");
const PYTHON_REQUIREMENTS: &str = include_str!("../../../runtime/python-requirements.txt");

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pins {
    node: Pin,
    python: Pin,
    npm_registries: Vec<String>,
    python_indexes: Vec<String>,
}
#[derive(Deserialize)]
struct Pin {
    version: String,
    sources: Vec<String>,
    files: HashMap<String, PinFile>,
}
#[derive(Deserialize, Clone, Debug)]
struct PinFile {
    file: String,
    sha256: String,
}

/// Where a runtime's interpreter comes from (design §2.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Download,
    System,
}

/// The two runtimes RuntimeManager supplies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Node,
    Python,
}
impl Kind {
    pub const ALL: [Kind; 2] = [Kind::Node, Kind::Python];
    pub fn name(self) -> &'static str {
        match self {
            Kind::Node => "node",
            Kind::Python => "python",
        }
    }
    fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }
}

/// One runtime's configuration and the paths it is installed at.
#[derive(Clone, Debug)]
pub struct Plan {
    pub source: Source,
    pub auto: bool,
    pub version: String,
    /// The interpreter the runtime runs with.
    pub interpreter: PathBuf,
    /// The pinned archive (download) and its unpacked directory.
    archive: Option<(PinFile, PathBuf)>,
    sources: Vec<String>,
    /// The packages directory: Node's `npm ci` tree, Python's `pip --target`.
    pub packages: PathBuf,
    indexes: Vec<String>,
}

/// The runtimes of a home (design §2.3): every path follows from the pins,
/// the configuration and the home.
#[derive(Clone, Debug)]
pub struct Runtimes {
    root: PathBuf,
    node: Plan,
    python: Plan,
}

#[derive(Deserialize, Default, Clone, Copy)]
#[serde(deny_unknown_fields)]
struct ChoiceFile {
    source: Option<Source>,
    auto: Option<bool>,
}

impl Runtimes {
    /// The runtimes of `home`: the `runtimes` section of the default
    /// configuration file `defaults`, overridden by `<home>/runtimes/config.json`.
    pub fn load(home: &Path, defaults: &Path) -> Result<Self, String> {
        #[derive(Deserialize)]
        struct Defaults {
            #[serde(default)]
            runtimes: BTreeMap<String, ChoiceFile>,
        }
        let read = |path: &Path| -> Result<BTreeMap<String, ChoiceFile>, String> {
            let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            serde_json::from_slice::<Defaults>(&bytes)
                .map(|file| file.runtimes)
                .map_err(|e| format!("{}: {e}", path.display()))
        };
        let preset = read(defaults)?;
        let root = home.join("runtimes");
        let user_file = root.join("config.json");
        let user = match std::fs::read(&user_file) {
            Ok(bytes) => serde_json::from_slice::<BTreeMap<String, ChoiceFile>>(&bytes)
                .map_err(|e| format!("{}: {e}", user_file.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(format!("{}: {error}", user_file.display())),
        };
        for name in preset.keys().chain(user.keys()) {
            if Kind::parse(name).is_none() {
                return Err(format!("unknown runtime {name}"));
            }
        }
        let choice = |kind: Kind| {
            let preset = preset.get(kind.name()).copied().unwrap_or_default();
            let user = user.get(kind.name()).copied().unwrap_or_default();
            (
                user.source.or(preset.source).unwrap_or(Source::Download),
                user.auto.or(preset.auto).unwrap_or(false),
            )
        };
        Self::new(root, choice(Kind::Node), choice(Kind::Python))
    }

    fn new(root: PathBuf, node: (Source, bool), python: (Source, bool)) -> Result<Self, String> {
        let pins: Pins = serde_json::from_str(PINS).expect("runtime/runtimes.json is valid");
        let platform = platform();
        let mirror = |variable: &str| std::env::var(variable).ok().filter(|v| !v.is_empty());
        let sources = |pin: &Pin, variable: &str| {
            mirror(variable)
                .map(|base| format!("{}/{{file}}", base.trim_end_matches('/')))
                .into_iter()
                .chain(pin.sources.iter().cloned())
                .map(|source| source.replace("{version}", &pin.version))
                .collect::<Vec<_>>()
        };
        let registries: Vec<String> = mirror("DIMCODE_NPM_REGISTRY")
            .into_iter()
            .chain(pins.npm_registries.iter().cloned())
            .collect();
        let indexes: Vec<String> = mirror("DIMCODE_PYTHON_INDEX")
            .into_iter()
            .chain(pins.python_indexes.iter().cloned())
            .collect();
        let digest = |parts: &[&str]| {
            let mut hash = Sha256::new();
            parts.iter().for_each(|part| hash.update(part.as_bytes()));
            hex(&hash.finalize())[..16].to_owned()
        };
        let archive = |pin: &Pin, directory: String| {
            pin.files
                .get(&platform)
                .cloned()
                .map(|file| (file, root.join(directory)))
        };
        let node_archive = archive(
            &pins.node,
            format!("node-v{}-{platform}", pins.node.version),
        );
        let node_interpreter = match node.0 {
            Source::Download => {
                let (_, directory) = node_archive
                    .as_ref()
                    .ok_or_else(|| format!("no Node download for {platform}"))?;
                if cfg!(windows) {
                    directory.join("node.exe")
                } else {
                    directory.join("bin/node")
                }
            }
            // Not found: the acquisition fails and says so (§3.3).
            Source::System => which("node").unwrap_or_else(|| root.join("no-system-node")),
        };
        let python_archive = archive(
            &pins.python,
            format!("python-{}-{platform}", pins.python.version),
        );
        let python_interpreter = match python.0 {
            Source::Download => {
                let (_, directory) = python_archive
                    .as_ref()
                    .ok_or_else(|| format!("no Python download for {platform}"))?;
                if cfg!(windows) {
                    directory.join("python.exe")
                } else {
                    directory.join("bin/python3")
                }
            }
            Source::System => {
                let name = if cfg!(windows) { "python" } else { "python3" };
                which(name).unwrap_or_else(|| root.join("no-system-python"))
            }
        };
        let node_plan = Plan {
            source: node.0,
            auto: node.1,
            version: pins.node.version.clone(),
            interpreter: node_interpreter,
            archive: (node.0 == Source::Download)
                .then_some(node_archive)
                .flatten(),
            sources: sources(&pins.node, "DIMCODE_NODE_MIRROR"),
            packages: root.join(format!(
                "node-packages-{}",
                digest(&[NODE_PACKAGE, NODE_LOCK])
            )),
            indexes: registries,
        };
        let python_plan = Plan {
            source: python.0,
            auto: python.1,
            version: pins.python.version.clone(),
            interpreter: python_interpreter,
            archive: (python.0 == Source::Download)
                .then_some(python_archive)
                .flatten(),
            sources: sources(&pins.python, "DIMCODE_PYTHON_MIRROR"),
            packages: root.join(format!(
                "python-packages-{}",
                digest(&[PYTHON_REQUIREMENTS])
            )),
            indexes,
        };
        Ok(Self {
            root,
            node: node_plan,
            python: python_plan,
        })
    }

    pub fn plan(&self, kind: Kind) -> &Plan {
        match kind {
            Kind::Node => &self.node,
            Kind::Python => &self.python,
        }
    }
    /// `@arcships/rutis-runtime` in the Node packages directory.
    pub fn js_runtime(&self) -> PathBuf {
        self.node
            .packages
            .join("node_modules/@arcships/rutis-runtime")
    }
    /// The anchor Node resolves packages from.
    pub fn anchor(&self) -> PathBuf {
        self.node.packages.join("package.json")
    }
    fn record_file(&self) -> PathBuf {
        self.root.join("runtimes.json")
    }
}

/// The platform name of the release matrix.
fn platform() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        other => other,
    };
    format!("{os}-{arch}")
}

fn which(program: &str) -> Option<PathBuf> {
    let file = if cfg!(windows) {
        format!("{program}.exe")
    } else {
        program.to_owned()
    };
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|directory| directory.join(&file))
        .find(|path| path.is_file())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// An installed runtime, as `runtimes.json` records it (design §2.4).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct Record {
    source: Source,
    version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
    packages: String,
}
impl Record {
    fn of(plan: &Plan) -> Self {
        Self {
            source: plan.source,
            version: plan.version.clone(),
            sha256: plan.archive.as_ref().map(|(file, _)| file.sha256.clone()),
            packages: plan
                .packages
                .file_name()
                .expect("a packages directory")
                .to_string_lossy()
                .into_owned(),
        }
    }
}

fn read_records(path: &Path) -> BTreeMap<String, Record> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// A runtime's status in `runtime.list` (design §5).
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct RuntimeStatus {
    pub name: &'static str,
    pub source: Source,
    pub version: String,
    /// `off`, `acquiring`, `failed`, or rutis' runtime state once mounted:
    /// `idle`, `starting`, `ready`, `down`.
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

enum State {
    Off,
    Acquiring,
    Failed(String),
    Mounted(FiberView),
}

/// The process service of RuntimeManager: what other process parts (and the
/// `runtime` methods) call.
pub struct RuntimeService {
    runtimes: Arc<Runtimes>,
    local: [Arc<LocalRuntime>; 2],
    states: Mutex<[State; 2]>,
    /// Serializes writes of `runtimes.json` by the two acquisitions.
    record: tokio::sync::Mutex<()>,
    ctx: Ctx,
    stop: CancellationToken,
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}
impl RuntimeService {
    fn index(kind: Kind) -> usize {
        kind as usize
    }
    /// Every runtime's status.
    pub fn list(&self) -> Vec<RuntimeStatus> {
        let states = self.states.lock().unwrap();
        Kind::ALL
            .into_iter()
            .map(|kind| {
                let plan = self.runtimes.plan(kind);
                let (status, reason) = match &states[Self::index(kind)] {
                    State::Off => ("off", None),
                    State::Acquiring => ("acquiring", None),
                    State::Failed(reason) => ("failed", Some(reason.clone())),
                    State::Mounted(_) => match self.local[Self::index(kind)].handle().state() {
                        RuntimeState::Idle => ("idle", None),
                        RuntimeState::Starting => ("starting", None),
                        RuntimeState::Ready(_) => ("ready", None),
                        RuntimeState::Down(reason) => ("down", Some(reason)),
                    },
                };
                RuntimeStatus {
                    name: kind.name(),
                    source: plan.source,
                    version: plan.version.clone(),
                    status,
                    reason,
                }
            })
            .collect()
    }

    /// Turns `kind` on: acquires and mounts it. While it is being acquired or
    /// runs, nothing happens; after a failure, or once its process ended,
    /// it is tried again.
    pub fn install(self: &Arc<Self>, kind: Kind) {
        let mut states = self.states.lock().unwrap();
        match &states[Self::index(kind)] {
            State::Acquiring => {}
            State::Mounted(view) => {
                if matches!(
                    self.local[Self::index(kind)].handle().state(),
                    RuntimeState::Down(_)
                ) {
                    let restart = view.restart();
                    tokio::spawn(async move {
                        if let Err(error) = restart.await {
                            tracing::warn!(%error, "runtime restart failed");
                        }
                    });
                }
            }
            State::Off | State::Failed(_) => {
                states[Self::index(kind)] = State::Acquiring;
                let service = self.clone();
                let task = tokio::spawn(async move {
                    let result = tokio::select! {
                        result = service.acquire(kind) => result,
                        _ = service.stop.cancelled() => return,
                    };
                    let state = match result {
                        Ok(()) => State::Mounted(
                            service
                                .ctx
                                .plugin(Mounted(service.local[Self::index(kind)].clone(), kind)),
                        ),
                        Err(reason) => {
                            tracing::warn!(runtime = kind.name(), %reason, "runtime acquisition failed");
                            State::Failed(reason)
                        }
                    };
                    service.states.lock().unwrap()[Self::index(kind)] = state;
                });
                self.tasks.lock().unwrap().push(task);
            }
        }
    }

    async fn acquire(&self, kind: Kind) -> Result<(), String> {
        let plan = self.runtimes.plan(kind);
        let record = Record::of(plan);
        let installed = |plan: &Plan| {
            plan.archive
                .as_ref()
                .is_none_or(|(_, directory)| directory.is_dir())
                && plan.packages.is_dir()
        };
        if read_records(&self.runtimes.record_file()).get(kind.name()) == Some(&record)
            && installed(plan)
        {
            return Ok(());
        }
        std::fs::create_dir_all(&self.runtimes.root).map_err(|e| e.to_string())?;
        if let Some((file, directory)) = &plan.archive {
            if !directory.is_dir() {
                download(&plan.sources, file, directory).await?;
            }
        } else {
            check_system(kind, &plan.interpreter).await?;
        }
        if !plan.packages.is_dir() {
            match kind {
                Kind::Node => npm_ci(plan).await?,
                Kind::Python => pip_install(plan).await?,
            }
        }
        let _guard = self.record.lock().await;
        let path = self.runtimes.record_file();
        let mut records = read_records(&path);
        records.insert(kind.name().to_owned(), record);
        let bytes = serde_json::to_vec_pretty(&records).expect("records serialize");
        let partial = path.with_extension("json.partial");
        std::fs::write(&partial, bytes)
            .and_then(|()| std::fs::rename(&partial, &path))
            .map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// A mounted runtime: the composed rutis `LocalRuntime`, named after Dim's
/// runtime.
struct Mounted(Arc<LocalRuntime>, Kind);
impl DynPlugin for Mounted {
    fn name(&self) -> &str {
        match self.1 {
            Kind::Node => "SessionNodeRuntime",
            Kind::Python => "SessionPythonRuntime",
        }
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        DynPlugin::apply(self.0.as_ref(), ctx)
    }
}

const TIMEOUT: Duration = Duration::from_secs(60);

/// Downloads `file` from the first source whose bytes match its pin and
/// unpacks it into `directory` (design §3.2, §7).
async fn download(sources: &[String], file: &PinFile, directory: &Path) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .connect_timeout(TIMEOUT)
        .read_timeout(TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let partial = directory.with_extension("partial");
    let archive = directory.with_extension("download");
    let mut errors = Vec::new();
    for source in sources {
        let url = source.replace("{file}", &file.file.replace('+', "%2B"));
        match fetch(&client, &url, &archive, &file.sha256).await {
            Ok(()) => {
                errors.clear();
                break;
            }
            Err(error) => errors.push(format!("{url}: {error}")),
        }
    }
    if !errors.is_empty() || sources.is_empty() {
        let _ = std::fs::remove_file(&archive);
        return Err(format!("{} not acquired: {}", file.file, errors.join("; ")));
    }
    let _ = std::fs::remove_dir_all(&partial);
    let name = file.file.clone();
    let (from, to) = (archive.clone(), partial.clone());
    let unpacked = tokio::task::spawn_blocking(move || unpack(&name, &from, &to))
        .await
        .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&archive);
    if let Err(error) = unpacked {
        let _ = std::fs::remove_dir_all(&partial);
        return Err(format!("{}: {error}", file.file));
    }
    std::fs::rename(&partial, directory).map_err(|e| format!("{}: {e}", directory.display()))
}

/// Writes `url` to `path`, hashing as it goes; the file stays only if its
/// SHA-256 is `expected`.
async fn fetch(
    client: &reqwest::Client,
    url: &str,
    path: &Path,
    expected: &str,
) -> Result<(), String> {
    let mut response = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| e.to_string())?;
    let mut out = tokio::fs::File::create(path)
        .await
        .map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        hash.update(&chunk);
        out.write_all(&chunk).await.map_err(|e| e.to_string())?;
    }
    out.flush().await.map_err(|e| e.to_string())?;
    drop(out);
    let actual = hex(&hash.finalize());
    if actual != expected {
        let _ = std::fs::remove_file(path);
        return Err(format!("SHA-256 {actual} does not match the pin"));
    }
    Ok(())
}

/// The archive's single top directory, unpacked as `into`. Entries leaving
/// it, and links pointing outside it, are refused.
fn unpack(name: &str, archive: &Path, into: &Path) -> Result<(), String> {
    std::fs::create_dir_all(into).map_err(|e| e.to_string())?;
    let inside = |path: &Path| -> Result<PathBuf, String> {
        let relative: PathBuf = path.components().skip(1).collect();
        if !relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        {
            return Err(format!("refusing archive path {}", path.display()));
        }
        Ok(relative)
    };
    if name.ends_with(".zip") {
        return unzip(archive, into, &inside);
    }
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    for entry in tar.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        let relative = inside(&path)?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        let destination = into.join(&relative);
        let kind = entry.header().entry_type();
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        match kind {
            tar::EntryType::Directory => {
                std::fs::create_dir_all(&destination).map_err(|e| e.to_string())?
            }
            tar::EntryType::Regular | tar::EntryType::Continuous => {
                let mut out = std::fs::File::create(&destination).map_err(|e| e.to_string())?;
                std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = entry.header().mode().map_err(|e| e.to_string())? & 0o755;
                    std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(mode))
                        .map_err(|e| e.to_string())?;
                }
            }
            tar::EntryType::Symlink | tar::EntryType::Link => {
                let target = entry
                    .link_name()
                    .map_err(|e| e.to_string())?
                    .ok_or("a link without a target")?
                    .into_owned();
                // A symbolic link resolves from its own directory, a hard
                // link from the archive's root.
                let resolved = if kind == tar::EntryType::Symlink {
                    relative.parent().unwrap_or(Path::new("")).join(&target)
                } else {
                    inside(&target)?
                };
                let mut depth = 0i32;
                for component in resolved.components() {
                    depth += match component {
                        Component::Normal(_) => 1,
                        Component::ParentDir => -1,
                        Component::CurDir => 0,
                        _ => return Err(format!("refusing link {}", path.display())),
                    };
                    if depth < 0 {
                        return Err(format!("refusing link {}", path.display()));
                    }
                }
                if kind == tar::EntryType::Link {
                    std::fs::copy(into.join(&resolved), &destination).map_err(|e| e.to_string())?;
                } else {
                    #[cfg(unix)]
                    std::os::unix::fs::symlink(&target, &destination).map_err(|e| e.to_string())?;
                    #[cfg(windows)]
                    std::fs::copy(into.join(&resolved), &destination).map_err(|e| e.to_string())?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(windows)]
fn unzip(
    archive: &Path,
    into: &Path,
    inside: &dyn Fn(&Path) -> Result<PathBuf, String>,
) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
        let path = entry
            .enclosed_name()
            .ok_or_else(|| format!("refusing archive path {}", entry.name()))?;
        let relative = inside(&path)?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        let destination = into.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&destination).map_err(|e| e.to_string())?;
        } else if entry.is_file() {
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut out = std::fs::File::create(&destination).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
#[cfg(not(windows))]
fn unzip(_: &Path, _: &Path, _: &dyn Fn(&Path) -> Result<PathBuf, String>) -> Result<(), String> {
    Err("zip archives are Windows downloads".into())
}

/// Runs `command`; its failure carries the end of its output.
async fn run(mut command: tokio::process::Command, what: &str) -> Result<String, String> {
    let output = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| format!("{what}: {e}"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let text = String::from_utf8_lossy(&output.stderr);
    let tail: Vec<&str> = text.lines().rev().take(5).collect();
    Err(format!(
        "{what}: {}: {}",
        output.status,
        tail.into_iter().rev().collect::<Vec<_>>().join(" | ")
    ))
}

/// A `system` interpreter meets the runtime's requirement (design §2.2).
async fn check_system(kind: Kind, interpreter: &Path) -> Result<(), String> {
    let mut command = tokio::process::Command::new(interpreter);
    let (minimum, accepts): (&str, fn(u32, u32) -> bool) = match kind {
        Kind::Node => {
            command.arg("--version");
            // `^22.19.0 || >=23.11.0`, the engines of @arcships/rutis-runtime.
            ("22.19", |major, minor| {
                major > 23 || (major == 23 && minor >= 11) || (major == 22 && minor >= 19)
            })
        }
        Kind::Python => {
            command.args(["-c", "import sys; print('%d.%d' % sys.version_info[:2])"]);
            ("3.10", |major, minor| {
                major > 3 || (major == 3 && minor >= 10)
            })
        }
    };
    if !interpreter.is_file() {
        return Err(format!("no {} interpreter on PATH", kind.name()));
    }
    let what = format!("{} {}", kind.name(), interpreter.display());
    let version = run(command, &what).await?;
    let mut parts = version
        .trim()
        .trim_start_matches('v')
        .split('.')
        .map(|part| part.parse::<u32>().ok());
    match (parts.next().flatten(), parts.next().flatten()) {
        (Some(major), Some(minor)) if accepts(major, minor) => Ok(()),
        _ => Err(format!(
            "{what} is {}, {minimum} or later is required",
            version.trim()
        )),
    }
}

/// `npm-cli.js` of the Node installation `node` belongs to.
fn npm_cli(node: &Path) -> PathBuf {
    let node = node.canonicalize().unwrap_or_else(|_| node.to_owned());
    let directory = node.parent().unwrap_or(Path::new(""));
    if cfg!(windows) {
        directory.join("node_modules/npm/bin/npm-cli.js")
    } else {
        directory.join("../lib/node_modules/npm/bin/npm-cli.js")
    }
}

/// The pinned Node packages, installed by `npm ci` against the pinned lock.
async fn npm_ci(plan: &Plan) -> Result<(), String> {
    let partial = plan.packages.with_extension("partial");
    let _ = std::fs::remove_dir_all(&partial);
    std::fs::create_dir_all(&partial).map_err(|e| e.to_string())?;
    std::fs::write(partial.join("package.json"), NODE_PACKAGE)
        .and_then(|()| std::fs::write(partial.join("package-lock.json"), NODE_LOCK))
        .map_err(|e| e.to_string())?;
    let cache = plan.packages.with_extension("npm-cache");
    let mut errors = Vec::new();
    for registry in &plan.indexes {
        let mut command = tokio::process::Command::new(&plan.interpreter);
        command
            .arg(npm_cli(&plan.interpreter))
            .args([
                "ci",
                "--ignore-scripts",
                "--omit=dev",
                "--no-audit",
                "--no-fund",
                "--registry",
                registry,
            ])
            .env("npm_config_cache", &cache)
            .current_dir(&partial);
        match run(command, "npm ci").await {
            Ok(_) => {
                errors.clear();
                break;
            }
            Err(error) => errors.push(format!("{registry}: {error}")),
        }
    }
    let _ = std::fs::remove_dir_all(&cache);
    if !errors.is_empty() {
        let _ = std::fs::remove_dir_all(&partial);
        return Err(format!(
            "Node packages not installed: {}",
            errors.join("; ")
        ));
    }
    std::fs::rename(&partial, &plan.packages).map_err(|e| e.to_string())
}

/// The pinned `rutis` package, installed by pip with required hashes.
async fn pip_install(plan: &Plan) -> Result<(), String> {
    let partial = plan.packages.with_extension("partial");
    let requirements = plan.packages.with_extension("requirements.txt");
    let _ = std::fs::remove_dir_all(&partial);
    std::fs::write(&requirements, PYTHON_REQUIREMENTS).map_err(|e| e.to_string())?;
    let mut errors = Vec::new();
    for index in &plan.indexes {
        let mut command = tokio::process::Command::new(&plan.interpreter);
        command
            .args([
                "-I",
                "-m",
                "pip",
                "install",
                "--require-hashes",
                "--no-deps",
                "--only-binary",
                ":all:",
                "--no-input",
                "--no-cache-dir",
                "--disable-pip-version-check",
                "--index-url",
                index,
                "--target",
            ])
            .arg(&partial)
            .arg("-r")
            .arg(&requirements)
            .env(
                "PIP_CONFIG_FILE",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            );
        match run(command, "pip install").await {
            Ok(_) => {
                errors.clear();
                break;
            }
            Err(error) => {
                let _ = std::fs::remove_dir_all(&partial);
                errors.push(format!("{index}: {error}"));
            }
        }
    }
    let _ = std::fs::remove_file(&requirements);
    if !errors.is_empty() {
        return Err(format!(
            "Python packages not installed: {}",
            errors.join("; ")
        ));
    }
    std::fs::rename(&partial, &plan.packages).map_err(|e| e.to_string())
}

/// The process row `RuntimeManager`.
pub struct RuntimeManagerPlugin {
    methods: Key<ManagementMethods>,
    service: Key<RuntimeService>,
    runtimes: Arc<Runtimes>,
    local: [Arc<LocalRuntime>; 2],
}
impl RuntimeManagerPlugin {
    pub fn new(
        names: &Names<'_>,
        runtimes: Arc<Runtimes>,
        node: Arc<LocalRuntime>,
        python: Arc<LocalRuntime>,
    ) -> rutis::Result<Self> {
        Ok(Self {
            methods: names.key()?,
            service: names.key()?,
            runtimes,
            local: [node, python],
        })
    }
}
impl DynPlugin for RuntimeManagerPlugin {
    fn name(&self) -> &str {
        "RuntimeManager"
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![self.methods.erase()]
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let methods = ctx.require_at(&self.methods)?;
            let records = read_records(&self.runtimes.record_file());
            let service = Arc::new(RuntimeService {
                runtimes: self.runtimes.clone(),
                local: self.local.clone(),
                states: Mutex::new([State::Off, State::Off]),
                record: tokio::sync::Mutex::new(()),
                ctx: ctx.clone(),
                stop: CancellationToken::new(),
                tasks: Mutex::new(Vec::new()),
            });
            // Registered first, so the tasks are settled before the service
            // and methods go.
            let owned = service.clone();
            ctx.effect(move || async move {
                owned.stop.cancel();
                let tasks = std::mem::take(&mut *owned.tasks.lock().unwrap());
                for task in tasks {
                    let _ = task.await;
                }
                Ok(())
            })?;
            ctx.provide_at(self.service.clone(), service.clone())?;
            let handler = service.clone();
            methods.register(
                ctx,
                "runtime",
                vec![
                    MethodDecl::new("runtime.list", false, Interruption::Report),
                    MethodDecl::new("runtime.install", true, Interruption::Report),
                ],
                Arc::new(move |method, value, _stopping, _| {
                    let service = handler.clone();
                    Box::pin(async move {
                        match method.as_str() {
                            "runtime.list" => reply(service.list()),
                            "runtime.install" => {
                                #[derive(Deserialize)]
                                struct Install {
                                    name: String,
                                }
                                let Install { name } = params(value)?;
                                let kind = Kind::parse(&name).ok_or_else(|| {
                                    RpcError::invalid(format!("unknown runtime {name}"))
                                })?;
                                service.install(kind);
                                reply(service.list().into_iter().find(|s| s.name == kind.name()))
                            }
                            _ => unreachable!("registered runtime management method"),
                        }
                    })
                }),
            )?;
            for kind in Kind::ALL {
                if self.runtimes.plan(kind).auto || records.contains_key(kind.name()) {
                    service.install(kind);
                }
            }
            Ok(Effect::Done)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{PINS, PinFile, Pins, download, unpack};
    use std::path::Path;

    /// Every platform of the release matrix has a pinned Node and Python
    /// file with a lowercase hex SHA-256.
    #[test]
    fn every_release_platform_is_pinned() {
        let pins: Pins = serde_json::from_str(PINS).unwrap();
        for platform in [
            "darwin-arm64",
            "darwin-x64",
            "linux-arm64",
            "linux-x64",
            "windows-arm64",
            "windows-x64",
        ] {
            for pin in [&pins.node, &pins.python] {
                let sha = &pin.files[platform].sha256;
                assert_eq!(sha.len(), 64, "{platform}");
                assert!(sha.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
            }
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dim-rm-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A source whose bytes do not match the pin is refused: nothing is
    /// unpacked and no download stays behind, and the reason names it.
    #[tokio::test]
    async fn a_download_that_does_not_match_its_pin_is_refused() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0u8; 1024];
                let _ = stream.read(&mut request).await;
                let _ = stream
                    .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 8\r\nconnection: close\r\n\r\ntampered")
                    .await;
            }
        });
        let root = scratch("pin");
        let directory = root.join("node-v0-test");
        let file = PinFile {
            file: "node.tar.gz".into(),
            sha256: "0".repeat(64),
        };
        let error = download(&[format!("http://{address}/{{file}}")], &file, &directory)
            .await
            .unwrap_err();
        assert!(error.contains("does not match the pin"), "{error}");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    fn archive(path: &Path, build: impl FnOnce(&mut tar::Builder<Vec<u8>>)) {
        let mut builder = tar::Builder::new(Vec::new());
        build(&mut builder);
        let tar = builder.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &tar).unwrap();
        std::fs::write(path, gz.finish().unwrap()).unwrap();
    }

    /// Entries leaving the archive's directory, and links pointing outside
    /// it, are refused; links inside it are kept.
    #[test]
    fn unpacking_stays_inside_its_directory() {
        let root = scratch("unpack");
        let entry = |builder: &mut tar::Builder<Vec<u8>>, kind, path: &str, link: Option<&str>| {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(kind);
            header.set_mode(0o755);
            header.set_size(if link.is_some() { 0 } else { 2 });
            if let Some(link) = link {
                header.set_link_name(link).unwrap();
            }
            // `set_path` refuses `..`; the raw name is what a hostile archive carries.
            let name = &mut header.as_gnu_mut().unwrap().name;
            name[..path.len()].copy_from_slice(path.as_bytes());
            header.set_cksum();
            let data: &[u8] = if link.is_some() { b"" } else { b"ok" };
            builder.append(&header, data).unwrap();
        };
        let good = root.join("good.tar.gz");
        archive(&good, |b| {
            entry(b, tar::EntryType::Regular, "top/bin/node", None);
            entry(
                b,
                tar::EntryType::Symlink,
                "top/bin/npm",
                Some("../lib/npm.js"),
            );
        });
        unpack("good.tar.gz", &good, &root.join("good")).unwrap();
        assert_eq!(std::fs::read(root.join("good/bin/node")).unwrap(), b"ok");
        assert!(root.join("good/bin/npm").is_symlink());
        for (name, kind, path, link) in [
            ("parent", tar::EntryType::Regular, "top/../../escape", None),
            (
                "link",
                tar::EntryType::Symlink,
                "top/bin/x",
                Some("../../../etc/passwd"),
            ),
            (
                "absolute",
                tar::EntryType::Symlink,
                "top/bin/y",
                Some("/etc/passwd"),
            ),
        ] {
            let file = root.join(format!("{name}.tar.gz"));
            archive(&file, |b| entry(b, kind, path, link));
            let error = unpack("x.tar.gz", &file, &root.join(name)).unwrap_err();
            assert!(error.contains("refusing"), "{name}: {error}");
        }
        assert!(!root.parent().unwrap().join("escape").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
