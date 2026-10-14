//! Private user documents referenced by the root loader's editable layer.
//! The late-bound port contains wiring and immutable desired values, not resources.
pub mod management;
use crate::{settings::*, storage::SqliteStorage};
use dim_agent_capabilities::file_storage::FileStorage;
use dim_agent_runtime::{Capability, KeysOf, StateCell};
use rusqlite::{OptionalExtension, params};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};
use rutis_loader::{Editable, Layer, Loader, LoaderError, Patch, Persist, PersistError, Version};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

pub const USER_LAYER: &str = "user";
const MCP: &str = "UserMcpDocument";
const SKILLS: &str = "UserSkillsDocument";
fn id(source: SettingsSource) -> Result<&'static str> {
    match source {
        SettingsSource::UserMcp => Ok(MCP),
        SettingsSource::Skills => Ok(SKILLS),
        _ => Err(SettingsError::Invalid("not a user source")),
    }
}
fn source(id: &str) -> Result<SettingsSource> {
    match id {
        MCP => Ok(SettingsSource::UserMcp),
        SKILLS => Ok(SettingsSource::Skills),
        _ => Err(SettingsError::Invalid("protected configuration row")),
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrivateDocument {
    source: String,
    document: SettingsDocument,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    format: u32,
    patches: Vec<Patch>,
    documents: BTreeMap<String, PrivateDocument>,
    imports: BTreeMap<String, String>,
}
impl Envelope {
    fn empty() -> Self {
        Self {
            format: 1,
            patches: vec![],
            documents: BTreeMap::new(),
            imports: BTreeMap::new(),
        }
    }
    fn validate(&self) -> Result<()> {
        if self.format != 1 {
            return Err(SettingsError::Invalid("unsupported private envelope"));
        }
        for (token, doc) in &self.documents {
            uuid::Uuid::parse_str(token)
                .map_err(|_| SettingsError::Invalid("invalid private token"))?;
            validate(source(&doc.source)?, &doc.document)?;
        }
        if self.imports.len() != 2 {
            return Err(SettingsError::Invalid("missing import provenance"));
        }
        for (key, provenance) in &self.imports {
            source(key)?;
            if provenance != "missing"
                && (provenance.len() != 64
                    || !provenance
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
            {
                return Err(SettingsError::Invalid("invalid import provenance"));
            }
        }
        // Only root inserts of the two typed document rows are legal. In
        // particular no generic override, group, inject or infrastructure target.
        let mut found = BTreeMap::new();
        for patch in &self.patches {
            if patch.id.is_some() || patch.name.is_some() || !patch.overrides.is_empty() {
                return Err(SettingsError::Invalid("protected user patch"));
            }
            for row in patch
                .insert
                .as_ref()
                .ok_or(SettingsError::Invalid("invalid user patch"))?
            {
                let object = row
                    .as_object()
                    .ok_or(SettingsError::Invalid("invalid user row"))?;
                if object.len() != 3 {
                    return Err(SettingsError::Invalid("invalid user row fields"));
                }
                let name = row["id"]
                    .as_str()
                    .ok_or(SettingsError::Invalid("invalid user row id"))?;
                source(name)?;
                if row["name"] != name || found.insert(name, ()).is_some() {
                    return Err(SettingsError::Invalid("invalid user row identity"));
                }
                let reference = reference(&row["config"])?;
                if self
                    .documents
                    .get(reference)
                    .is_none_or(|doc| doc.source != name)
                {
                    return Err(SettingsError::Invalid("missing private document"));
                }
            }
        }
        if found.len() != 2 {
            return Err(SettingsError::Invalid("missing user document rows"));
        }
        Ok(())
    }
}
fn reference(value: &serde_json::Value) -> Result<&str> {
    if value.as_object().is_none_or(|v| v.len() != 1) {
        return Err(SettingsError::Invalid("invalid private reference"));
    }
    let token = value["reference"]
        .as_str()
        .ok_or(SettingsError::Invalid("invalid private reference"))?;
    uuid::Uuid::parse_str(token)
        .map_err(|_| SettingsError::Invalid("invalid private reference"))?;
    Ok(token)
}
fn loader_error(_: impl std::fmt::Display) -> LoaderError {
    LoaderError::PersistFailed("private configuration storage unavailable or invalid".into())
}

#[derive(Default)]
pub struct UserPersist {
    backend: Mutex<Option<Arc<SqliteStorage>>>,
    desired: Mutex<Option<Envelope>>,
}
impl UserPersist {
    fn backend(&self) -> std::result::Result<Arc<SqliteStorage>, LoaderError> {
        self.backend
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| loader_error("unbound"))
    }
    /// Outside plugin apply and before full assembly: import both documents or
    /// fail without switching authority. Original files are never modified.
    pub async fn initialize(
        &self,
        files: &Arc<FileStorage>,
        home: usize,
    ) -> std::result::Result<(Layer, Editable), LoaderError> {
        let db = self.backend()?;
        let stored = db
            .run(|c| {
                c.query_row(
                    "SELECT version, envelope FROM user_configuration WHERE layer = 'user'",
                    [],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(|_| {
                    dim_agent_runtime::AgentError::Storage("configuration read failed".into())
                })
            })
            .await
            .map_err(loader_error)?;
        let (envelope, version) = if let Some((version, body)) = stored {
            if version <= 0 {
                return Err(loader_error("version"));
            }
            (
                serde_json::from_str::<Envelope>(&body).map_err(loader_error)?,
                version,
            )
        } else {
            let mut envelope = Envelope::empty();
            for (source, file) in [
                (SettingsSource::UserMcp, "mcp.json"),
                (SettingsSource::Skills, "skills.json"),
            ] {
                let snapshot = decode(
                    source,
                    files
                        .read(
                            &files.path(home, file).map_err(loader_error)?,
                            &CancellationToken::new(),
                        )
                        .await,
                )
                .map_err(|error| match error {
                    SettingsError::Invalid(_) => LoaderError::InvalidEntry(format!(
                        "invalid legacy {file}; original preserved"
                    )),
                    _ => LoaderError::InvalidEntry(format!(
                        "unreadable legacy {file}; original preserved"
                    )),
                })?;
                let name = id(source).map_err(loader_error)?;
                envelope.imports.insert(name.into(), snapshot.revision);
                let token = uuid::Uuid::new_v4().to_string();
                envelope.documents.insert(
                    token.clone(),
                    PrivateDocument {
                        source: name.into(),
                        document: snapshot.document,
                    },
                );
                envelope.patches.push(Patch {
                    insert: Some(vec![
                        serde_json::json!({"id":name,"name":name,"config":{"reference":token}}),
                    ]),
                    ..Default::default()
                });
            }
            envelope.validate().map_err(loader_error)?;
            let body = serde_json::to_string(&envelope).map_err(loader_error)?;
            db.run(move |c| { c.execute("INSERT INTO user_configuration(layer, version, envelope) VALUES ('user', 1, ?1)", [body]).map_err(|_| dim_agent_runtime::AgentError::Storage("configuration import failed".into()))?; Ok(()) }).await.map_err(loader_error)?;
            (envelope, 1)
        };
        envelope.validate().map_err(loader_error)?;
        let layer = Layer::new(USER_LAYER, envelope.patches.clone());
        *self.desired.lock().unwrap() = Some(envelope);
        Ok((
            layer,
            Editable::new(USER_LAYER, Version(version.to_string())),
        ))
    }
}
impl Persist for UserPersist {
    fn load<'a>(
        &'a self,
        layer: &'a str,
    ) -> BoxFuture<'a, std::result::Result<(Vec<Patch>, Version), LoaderError>> {
        Box::pin(async move {
            if layer != USER_LAYER {
                return Err(loader_error("layer"));
            }
            let (version, body) = self
                .backend()?
                .run(|c| {
                    c.query_row(
                        "SELECT version, envelope FROM user_configuration WHERE layer = 'user'",
                        [],
                        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
                    )
                    .map_err(|_| {
                        dim_agent_runtime::AgentError::Storage("configuration load failed".into())
                    })
                })
                .await
                .map_err(loader_error)?;
            let envelope: Envelope = serde_json::from_str(&body).map_err(loader_error)?;
            envelope.validate().map_err(loader_error)?;
            if version <= 0 {
                return Err(loader_error("version"));
            }
            let patches = envelope.patches.clone();
            // Keep snapshots belonging to queued edits for framework CAS replay.
            let mut desired = self.desired.lock().unwrap();
            if let Some(current) = desired.as_mut() {
                current.documents.extend(envelope.documents);
                current.imports = envelope.imports;
            } else {
                *desired = Some(envelope);
            }
            Ok((patches, Version(version.to_string())))
        })
    }
    fn save<'a>(
        &'a self,
        layer: &'a str,
        expected: &'a Version,
        _: &'a [rutis_loader::Edit],
        patches: &'a [Patch],
    ) -> BoxFuture<'a, std::result::Result<Version, PersistError>> {
        Box::pin(async move {
            let failed = || {
                PersistError::Failed(
                    "private configuration save failed; applied intent remains live".into(),
                )
            };
            if layer != USER_LAYER {
                return Err(failed());
            }
            let mut envelope = self.desired.lock().unwrap().clone().ok_or_else(failed)?;
            envelope.patches = patches.to_vec();
            envelope.validate().map_err(|_| failed())?;
            let references: std::collections::BTreeSet<_> = patches
                .iter()
                .flat_map(|patch| patch.insert.iter().flatten())
                .filter_map(|row| reference(&row["config"]).ok())
                .collect();
            envelope
                .documents
                .retain(|token, _| references.contains(token.as_str()));
            let body = serde_json::to_string(&envelope).map_err(|_| failed())?;
            let version: i64 = expected.0.parse().map_err(|_| failed())?;
            let next = version.checked_add(1).ok_or_else(failed)?;
            let count = self.backend().map_err(|_| failed())?.run(move |c| c.execute("UPDATE user_configuration SET version = ?1, envelope = ?2 WHERE layer = 'user' AND version = ?3", params![next, body, version]).map_err(|_| dim_agent_runtime::AgentError::Storage("configuration save failed".into()))).await.map_err(|_| failed())?;
            if count != 1 {
                return Err(PersistError::Conflict);
            }
            Ok(Version(next.to_string()))
        })
    }
}

pub struct UserConfiguration {
    loader: Arc<Loader>,
    port: Arc<UserPersist>,
    pub(crate) changed: StateCell<u64>,
    pub(crate) skills_changed: StateCell<String>,
    gate: tokio::sync::Mutex<()>,
    closed: Mutex<bool>,
    updates: TaskTracker,
}
/// Sanitized management projection: no document fields or framework handles.
#[derive(Debug, Clone, Serialize)]
pub struct UserConfigurationInfo {
    pub id: &'static str,
    pub revision: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserPersistenceStatus {
    pub base_version: Option<String>,
    pub pending_saves: usize,
    pub backend_available: bool,
}
impl UserConfiguration {
    pub fn get(&self, source: SettingsSource) -> Result<UserConfigurationInfo> {
        if *self.closed.lock().unwrap() {
            return Err(SettingsError::Closed);
        }
        let name = id(source)?;
        let row = self.loader.get(name).ok_or(SettingsError::Closed)?;
        Ok(UserConfigurationInfo {
            id: name,
            revision: reference(&row.options["config"])?.to_owned(),
        })
    }
    pub fn catalog(&self) -> Result<Vec<UserConfigurationInfo>> {
        [SettingsSource::UserMcp, SettingsSource::Skills]
            .into_iter()
            .map(|source| self.get(source))
            .collect()
    }
    pub fn status(&self) -> UserPersistenceStatus {
        UserPersistenceStatus {
            base_version: self.loader.editable().map(|editable| editable.version.0),
            pending_saves: self.pending_saves(),
            backend_available: !*self.closed.lock().unwrap()
                && self.port.backend.lock().unwrap().is_some(),
        }
    }
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changed.subscribe()
    }
    pub fn pending_saves(&self) -> usize {
        self.loader.pending().len()
    }
    // Keep only live or replayable values. Old credentials are not an audit log.
    fn collect_documents(&self) {
        let mut desired = self.port.desired.lock().unwrap();
        let Some(envelope) = desired.as_mut() else {
            return;
        };
        let mut references: std::collections::BTreeSet<String> = self
            .loader
            .entries()
            .iter()
            .filter_map(|row| reference(&row.options["config"]).ok().map(str::to_owned))
            .collect();
        for edit in self.loader.pending() {
            if let rutis_loader::Edit::Update { config, .. } = edit
                && let Ok(token) = reference(&config)
            {
                references.insert(token.to_owned());
            }
        }
        envelope
            .documents
            .retain(|token, _| references.contains(token));
    }
    pub async fn flush(self: &Arc<Self>) -> std::result::Result<(), LoaderError> {
        let task = {
            let closed = self.closed.lock().unwrap();
            if *closed {
                return Err(LoaderError::Closed);
            }
            let owner = self.clone();
            self.updates.spawn(async move {
                let _guard = owner.gate.lock().await;
                let result = owner.loader.flush().await;
                owner.collect_documents();
                owner.changed.update(|version| *version += 1);
                result
            })
        };
        task.await.map_err(loader_error)?
    }
    pub fn read(&self, source: SettingsSource) -> Result<SettingsSnapshot> {
        let desired = self.port.desired.lock().unwrap();
        let info = self.get(source)?;
        let doc = desired
            .as_ref()
            .and_then(|e| e.documents.get(&info.revision))
            .ok_or(SettingsError::Closed)?;
        Ok(SettingsSnapshot {
            revision: info.revision,
            document: doc.document.clone(),
        })
    }
    pub(crate) async fn change(
        &self,
        source: SettingsSource,
        expected: &str,
        change: SettingsChange,
        cancel: &CancellationToken,
    ) -> Result<SettingsSnapshot> {
        let _guard = self.gate.lock().await;
        if cancel.is_cancelled() {
            return Err(SettingsError::Cancelled);
        }
        let current = self.read(source)?;
        if current.revision != expected {
            return Err(SettingsError::Conflict);
        }
        let Some(document) = change.apply(&current.document) else {
            // Legacy success acknowledges durability even for a no-op against
            // intent left live by a previous failed save.
            let result = self.loader.flush().await;
            self.collect_documents();
            self.changed.update(|version| *version += 1);
            result.map_err(|_| SettingsError::AppliedUnsaved)?;
            return self.read(source);
        };
        validate(source, &document)?;
        let name = id(source)?;
        let token = uuid::Uuid::new_v4().to_string();
        self.port
            .desired
            .lock()
            .unwrap()
            .as_mut()
            .ok_or(SettingsError::Closed)?
            .documents
            .insert(
                token.clone(),
                PrivateDocument {
                    source: name.into(),
                    document,
                },
            );
        let result = self
            .loader
            .update(name, serde_json::json!({"reference":token}))
            .await;
        self.collect_documents();
        // A loader edit notification may precede persistence. Also invalidate
        // after its final save attempt so management observes the result.
        self.changed.update(|version| *version += 1);
        result.map_err(|error| match error {
            LoaderError::PersistFailed(_) | LoaderError::Conflict => SettingsError::AppliedUnsaved,
            _ => SettingsError::Persistence,
        })?;
        self.read(source)
    }
}

#[derive(Clone)]
pub struct UserConfigurationPlugin(pub Arc<UserPersist>);
impl TypedPlugin for UserConfigurationPlugin {
    type Deps = (Arc<SqliteStorage>, Arc<Loader>);
    fn name(&self) -> &str {
        "UserConfiguration"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (db, loader): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            *self.0.backend.lock().unwrap() = Some(db);
            let service = Arc::new(UserConfiguration {
                loader,
                port: self.0.clone(),
                changed: StateCell::new(0),
                skills_changed: StateCell::new("unread".into()),
                gate: tokio::sync::Mutex::new(()),
                closed: Mutex::new(false),
                updates: TaskTracker::new(),
            });
            let owner = service.clone();
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        {
                            let mut closed = owner.closed.lock().unwrap();
                            *closed = true;
                            owner.updates.close();
                        }
                        owner.updates.wait().await;
                        *owner.port.backend.lock().unwrap() = None;
                        Ok(())
                    })
                }))
            })?;
            ctx.provide_as(rutis::TypeKey::of::<UserConfiguration>(), service)?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for UserConfigurationPlugin {
    fn keys(&self) -> KeysOf<Self> {
        ((), ())
    }
}

struct DocumentPlugin {
    name: String,
    token: String,
}
impl TypedPlugin for DocumentPlugin {
    type Deps = Arc<UserConfiguration>;
    fn name(&self) -> &str {
        &self.name
    }
    fn apply<'a>(
        &'a self,
        _: &'a Ctx,
        owner: Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            if owner
                .port
                .desired
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|e| e.documents.get(&self.token))
                .is_none_or(|doc| doc.source != self.name)
            {
                return Err(CordisError::PluginFailed("missing private document".into()));
            }
            if self.name == SKILLS {
                owner.skills_changed.set(self.token.clone());
            }
            owner.changed.update(|v| *v += 1);
            Ok(Effect::Done)
        })
    }
}
struct DocumentFactory(&'static str);
impl rutis::TypedPluginFactory<serde_json::Value> for DocumentFactory {
    type Plugin = DocumentPlugin;
    fn name(&self) -> &str {
        self.0
    }
    fn build(
        &self,
        config: &serde_json::Value,
    ) -> std::result::Result<DocumentPlugin, CordisError> {
        let token = reference(config)
            .map_err(|_| CordisError::PluginFailed("invalid private reference".into()))?;
        Ok(DocumentPlugin {
            name: self.0.into(),
            token: token.into(),
        })
    }
}
pub fn register_documents(builtins: &mut rutis_loader::Builtins) {
    for name in [MCP, SKILLS] {
        builtins.register_raw(name, rutis::TypedFactory::with_keys(DocumentFactory(name), ()), Some(serde_json::json!({"type":"object","additionalProperties":false,"required":["reference"],"properties":{"reference":{"type":"string"}}})));
    }
}

/// Called only by the explicit offline maintenance transaction. Native v13
/// settings win over stale files. Missing v13 rows use preserved legacy files.
pub(crate) fn migrate_envelope(
    tx: &rusqlite::Transaction<'_>,
    home: &std::path::Path,
) -> std::result::Result<String, dim_agent_runtime::AgentError> {
    let failed = || {
        dim_agent_runtime::AgentError::Storage(
            "invalid v13 user configuration; database preserved".into(),
        )
    };
    let rows = tx
        .prepare("SELECT source, version, document, imported_revision FROM settings_documents")
        .map_err(|_| failed())?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .map_err(|_| failed())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| failed())?;
    if rows
        .iter()
        .any(|r| !matches!(r.0.as_str(), "user/mcp" | "user/skills") || r.1 <= 0)
    {
        return Err(failed());
    }
    let mut envelope = Envelope::empty();
    for (source, key, file) in [
        (SettingsSource::UserMcp, "user/mcp", "mcp.json"),
        (SettingsSource::Skills, "user/skills", "skills.json"),
    ] {
        let (snapshot, provenance) = if let Some(row) = rows.iter().find(|r| r.0 == key) {
            (
                decode(source, Ok(row.2.as_bytes().to_vec())).map_err(|_| failed())?,
                row.3.clone(),
            )
        } else {
            let read = std::fs::read(home.join(file))
                .map_err(dim_agent_capabilities::file_storage::FileStorageError::Io);
            let snapshot = decode(source, read).map_err(|_| failed())?;
            let provenance = snapshot.revision.clone();
            (snapshot, provenance)
        };
        let name = id(source).map_err(|_| failed())?;
        let token = uuid::Uuid::new_v4().to_string();
        envelope.imports.insert(name.into(), provenance);
        envelope.documents.insert(
            token.clone(),
            PrivateDocument {
                source: name.into(),
                document: snapshot.document,
            },
        );
        envelope.patches.push(Patch {
            insert: Some(vec![
                serde_json::json!({"id":name,"name":name,"config":{"reference":token}}),
            ]),
            ..Default::default()
        });
    }
    envelope.validate().map_err(|_| failed())?;
    serde_json::to_string(&envelope).map_err(|_| failed())
}
