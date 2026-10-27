//! The data PluginManager reads at its boundary: `plugins.json` (its own
//! record), the default configuration file and plugin manifests
//! (PluginManager §2).
use super::PluginError;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
};

/// The manifest file at a plugin directory's root.
pub const MANIFEST: &str = "dim-plugin.json";
const RECORD_VERSION: u32 = 1;

/// `<home>/plugins.json` (§2.2): configuration only.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub version: u32,
    #[serde(default)]
    pub installed: Vec<Installed>,
    #[serde(default)]
    pub names: BTreeMap<String, Choice>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Installed {
    pub id: String,
    pub name: String,
    pub source: Source,
    pub dir: PathBuf,
    pub kinds: Vec<Kind>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(untagged, deny_unknown_fields)]
pub enum Source {
    Path {
        path: PathBuf,
    },
    Git {
        git: String,
        #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
        reference: Option<String>,
        commit: String,
    },
}

/// The user's decision about one name; absent fields are the shipped state.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    #[serde(rename = "use", default, skip_serializing_if = "Option::is_none")]
    pub use_: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub off: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Code,
    Skills,
    Mcp,
    Hooks,
}

impl Record {
    /// Reads `path`; a missing file is the empty record. A file that does not
    /// parse, or has another version, is `record_unreadable` with its path.
    pub fn read(path: &Path) -> Result<Self, PluginError> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self {
                    version: RECORD_VERSION,
                    ..Self::default()
                });
            }
            Err(error) => return Err(unreadable(path, error)),
        };
        let record: Self =
            serde_json::from_slice(&bytes).map_err(|error| unreadable(path, error))?;
        if record.version != RECORD_VERSION {
            return Err(unreadable(
                path,
                format!("unknown version {}", record.version),
            ));
        }
        Ok(record)
    }

    /// Writes atomically: a temporary file in the same directory, synced
    /// through its own writable handle, renamed over `path` (§2.2).
    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        let directory = path.parent().expect("plugins.json has a directory");
        let temporary = directory.join(format!(".plugins.json.{}", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut file = std::fs::File::create(&temporary)?;
            let mut text = serde_json::to_vec_pretty(self).expect("record serializes");
            text.push(b'\n');
            file.write_all(&text)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }

    pub fn installed(&self, id: &str) -> Option<&Installed> {
        self.installed.iter().find(|entry| entry.id == id)
    }
}

fn unreadable(path: &Path, error: impl std::fmt::Display) -> PluginError {
    PluginError::new("record_unreadable", format!("{}: {error}", path.display()))
}

/// The default configuration file (§2.3), shipped read-only.
#[derive(Clone, Debug, Default)]
pub struct Defaults {
    /// Each default plugin's name and directory, resolved against the file.
    pub plugins: BTreeMap<String, PathBuf>,
    pub locked: Vec<String>,
    /// RuntimeManager's preset (RuntimeManager §2.2).
    pub runtimes: crate::runtime_manager::Choices,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DefaultsFile {
    #[serde(default)]
    plugins: BTreeMap<String, PathBuf>,
    #[serde(default)]
    locked: Vec<String>,
    #[serde(default)]
    runtimes: crate::runtime_manager::Choices,
}

impl Defaults {
    pub fn read(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let file: DefaultsFile = serde_json::from_slice(&bytes)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let base = path.parent().unwrap_or(Path::new("."));
        Ok(Self {
            plugins: file
                .plugins
                .into_iter()
                .map(|(name, dir)| (name, base.join(dir)))
                .collect(),
            locked: file.locked,
            runtimes: file.runtimes,
        })
    }
}

/// A plugin manifest, read and checked at its boundary (§2.1): every path
/// it names is resolved inside the plugin directory.
#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub name: String,
    pub code: Option<Code>,
    pub skills: Option<PathBuf>,
    pub mcp: Option<PathBuf>,
    pub hooks: Option<PathBuf>,
}

/// The rutis row fields of a plugin's code, with `entry` resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct Code {
    pub entry: PathBuf,
    pub inject: Vec<String>,
    pub provides: Map<String, Value>,
    pub instance_names: Map<String, Value>,
    pub config: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestFile {
    name: String,
    #[serde(default)]
    #[allow(dead_code)]
    description: Option<String>,
    code: Option<CodeFile>,
    skills: Option<String>,
    mcp: Option<String>,
    hooks: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CodeFile {
    entry: String,
    #[serde(default)]
    inject: Vec<String>,
    #[serde(default)]
    provides: Map<String, Value>,
    #[serde(default)]
    instance_names: Map<String, Value>,
    #[serde(default)]
    config: Option<Value>,
}

fn invalid(field: &str, reason: impl std::fmt::Display) -> PluginError {
    PluginError::new("invalid_manifest", format!("{field}: {reason}"))
}

impl Manifest {
    /// Reads `<dir>/dim-plugin.json`. `dir` is canonical.
    pub fn read(dir: &Path) -> Result<Self, PluginError> {
        let path = inside(dir, MANIFEST, MANIFEST)?;
        let bytes = std::fs::read(&path).map_err(|error| invalid(MANIFEST, error))?;
        let file: ManifestFile =
            serde_json::from_slice(&bytes).map_err(|error| invalid(MANIFEST, error))?;
        if file.name.is_empty() || file.name.chars().any(char::is_control) {
            return Err(invalid("name", "must be a non-empty name"));
        }
        let code = file
            .code
            .map(|code| {
                for (service, group) in &code.instance_names {
                    if !matches!(group.as_str(), Some("session" | "branch")) {
                        return Err(invalid(
                            &format!("code.instanceNames.{service}"),
                            "must be \"session\" or \"branch\"",
                        ));
                    }
                }
                let entry = inside(dir, &code.entry, "code.entry")?;
                if !entry.is_file() {
                    return Err(invalid("code.entry", "is not a file"));
                }
                Ok(Code {
                    entry,
                    inject: code.inject,
                    provides: code.provides,
                    instance_names: code.instance_names,
                    config: code.config.unwrap_or_else(|| Value::Object(Map::new())),
                })
            })
            .transpose()?;
        let path = |value: Option<String>, field| {
            value.map(|value| inside(dir, &value, field)).transpose()
        };
        Ok(Self {
            name: file.name,
            code,
            skills: path(file.skills, "skills")?,
            mcp: path(file.mcp, "mcp")?,
            hooks: path(file.hooks, "hooks")?,
        })
    }

    pub fn kinds(&self) -> Vec<Kind> {
        [
            (self.code.is_some(), Kind::Code),
            (self.skills.is_some(), Kind::Skills),
            (self.mcp.is_some(), Kind::Mcp),
            (self.hooks.is_some(), Kind::Hooks),
        ]
        .into_iter()
        .filter_map(|(present, kind)| present.then_some(kind))
        .collect()
    }
}

/// `relative` resolved inside the canonical `dir` after following symbolic
/// links; anything that leaves it is `invalid_manifest`.
fn inside(dir: &Path, relative: &str, field: &str) -> Result<PathBuf, PluginError> {
    let path = Path::new(relative);
    if relative.is_empty() || path.is_absolute() || path.has_root() {
        return Err(invalid(field, "must be a relative path"));
    }
    let resolved = std::fs::canonicalize(dir.join(path)).map_err(|error| invalid(field, error))?;
    if !resolved.starts_with(dir) {
        return Err(invalid(field, "leaves the plugin directory"));
    }
    Ok(resolved)
}
