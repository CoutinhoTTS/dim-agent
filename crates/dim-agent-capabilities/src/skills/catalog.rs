use super::{
    CatalogEntry, CatalogVersion, CatalogView, RefreshTarget, SkillIssue, SkillStatus, SkillsError,
    discovery::{RootKind, RootSnapshot, scan_root},
};
use crate::{
    host_platform::HostPlatform,
    skill_settings::{
        SettingsVersion, SkillSettings, SkillSettingsError, SkillSettingsHost,
        SkillSettingsSnapshot,
    },
};
use dim_agent_runtime::{Capability, KeysOf, scoped};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypeKey, TypedPlugin};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::sync::watch;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

const SKILLS_DIR: &str = ".agents/skills";

struct CatalogState {
    user: Option<Arc<RootSnapshot>>,
    projects: HashMap<PathBuf, Arc<RootSnapshot>>,
    settings: Result<SkillSettingsSnapshot, SkillsError>,
}

/// Process-wide Skill discovery and global enablement.
pub struct SkillsCatalog {
    user_root: PathBuf,
    settings: Arc<dyn SkillSettings>,
    state: tokio::sync::Mutex<CatalogState>,
    /// Serializes settings reloads so a later read is never overwritten by
    /// an earlier one.
    reloading: tokio::sync::Mutex<()>,
    version: watch::Sender<CatalogVersion>,
    closed: CancellationToken,
}

async fn scan(root: PathBuf, kind: RootKind) -> Result<RootSnapshot, SkillIssue> {
    let path = root.clone();
    tokio::task::spawn_blocking(move || scan_root(&path, kind))
        .await
        .unwrap_or_else(|_| {
            Err(SkillIssue::RootUnreadable {
                root,
                reason: "scan task failed".into(),
            })
        })
}

impl SkillsCatalog {
    fn check_open(&self) -> Result<(), SkillsError> {
        if self.closed.is_cancelled() {
            Err(SkillsError::Closed)
        } else {
            Ok(())
        }
    }

    /// The project root of `cwd`, unless it is the user root.
    fn project_root(&self, cwd: &Path) -> Option<PathBuf> {
        let root = cwd.join(SKILLS_DIR);
        let same = matches!(
            (root.canonicalize(), self.user_root.canonicalize()),
            (Ok(a), Ok(b)) if a == b
        );
        (!same).then_some(root)
    }

    /// A root that cannot be read on first use is recorded as an issue.
    async fn first_scan(root: PathBuf, kind: RootKind) -> Arc<RootSnapshot> {
        Arc::new(scan(root, kind).await.unwrap_or_else(|issue| RootSnapshot {
            skills: Vec::new(),
            issues: vec![issue],
        }))
    }

    pub async fn query(&self, cwd: Option<&Path>) -> Result<CatalogView, SkillsError> {
        self.check_open()?;
        // A failed settings read is retried here, so the cache recovers
        // without waiting for another settings change.
        if self.state.lock().await.settings.is_err() {
            self.reload_settings().await?;
        }
        let mut state = self.state.lock().await;
        if state.user.is_none() {
            state.user = Some(Self::first_scan(self.user_root.clone(), RootKind::User).await);
        }
        let project_root = cwd.and_then(|cwd| self.project_root(cwd));
        if let Some(root) = &project_root
            && !state.projects.contains_key(root)
        {
            let snapshot = Self::first_scan(root.clone(), RootKind::Project).await;
            state.projects.insert(root.clone(), snapshot);
        }
        let roots = std::iter::once(state.user.clone().expect("user root scanned"))
            .chain(project_root.and_then(|root| state.projects.get(&root).cloned()));
        let mut winners: BTreeMap<String, String> = BTreeMap::new();
        let mut entries = Vec::new();
        let mut issues = Vec::new();
        let settings = state.settings.as_ref().map_err(Clone::clone)?;
        for snapshot in roots {
            issues.extend(snapshot.issues.iter().cloned());
            for skill in &snapshot.skills {
                let status = match winners.get(&skill.name) {
                    Some(by) => SkillStatus::Shadowed { by: by.clone() },
                    None => {
                        winners.insert(skill.name.clone(), skill.id.clone());
                        SkillStatus::Active
                    }
                };
                entries.push(CatalogEntry {
                    definition: skill.clone(),
                    enabled: !settings.disabled.contains(&skill.id),
                    status,
                });
            }
        }
        Ok(CatalogView {
            version: *self.version.borrow(),
            entries,
            issues,
        })
    }

    /// Reads the settings service and stores its snapshot; a changed
    /// disabled set is a new version. Reloads run one at a time, in order.
    async fn reload_settings(&self) -> Result<(), SkillsError> {
        let _order = self.reloading.lock().await;
        let snapshot = self
            .settings
            .skills(self.closed.child_token())
            .await
            .map_err(|error| SkillsError::SettingsRead(error.to_string()));
        let mut state = self.state.lock().await;
        let changed = state.settings.as_ref().map(|settings| &settings.disabled)
            != snapshot.as_ref().map(|settings| &settings.disabled);
        let result = snapshot.as_ref().map(|_| ()).map_err(Clone::clone);
        state.settings = snapshot;
        if changed {
            self.version.send_modify(|version| *version += 1);
        }
        result
    }

    pub async fn set_enabled(
        &self,
        id: &str,
        enabled: bool,
        cwd: Option<&Path>,
    ) -> Result<(CatalogEntry, CatalogVersion), SkillsError> {
        let view = self.query(cwd).await?;
        if !view.entries.iter().any(|entry| entry.definition.id == id) {
            return Err(SkillsError::UnknownSkill(id.into()));
        }
        let settings_error = |error: SkillSettingsError| match error {
            SkillSettingsError::Conflict => SkillsError::SettingsConflict,
            SkillSettingsError::Closed | SkillSettingsError::Cancelled => SkillsError::Closed,
            other => SkillsError::SettingsWrite(other.to_string()),
        };
        let cancel = self.closed.child_token();
        self.reload_settings().await?;
        let current = self
            .state
            .lock()
            .await
            .settings
            .as_ref()
            .map_err(Clone::clone)?
            .version
            .clone();
        self.settings
            .clone()
            .update_skills(current, id.into(), enabled, cancel)
            .await
            .map_err(settings_error)?;
        // The cache follows the settings service, never a pre-write snapshot.
        self.reload_settings()
            .await
            .map_err(|error| SkillsError::SettingsSync(error.to_string()))?;
        let view = self.query(cwd).await?;
        let entry = view
            .entries
            .into_iter()
            .find(|entry| entry.definition.id == id)
            .ok_or_else(|| SkillsError::UnknownSkill(id.into()))?;
        Ok((entry, view.version))
    }

    /// Rescans whole roots; any unreadable root keeps every old snapshot.
    pub async fn refresh(
        &self,
        target: RefreshTarget,
    ) -> Result<(CatalogVersion, Vec<SkillIssue>), SkillsError> {
        self.check_open()?;
        self.reload_settings().await?;
        let mut state = self.state.lock().await;
        let root_error = |issue| match issue {
            SkillIssue::RootUnreadable { root, reason } => {
                SkillsError::RootUnreadable { root, reason }
            }
            _ => unreachable!("scan_root fails only for an unreadable root"),
        };
        let user = match target.user {
            true => Some(
                scan(self.user_root.clone(), RootKind::User)
                    .await
                    .map_err(root_error)?,
            ),
            false => None,
        };
        let project = match target.project.and_then(|cwd| self.project_root(&cwd)) {
            Some(root) => Some((
                root.clone(),
                scan(root, RootKind::Project).await.map_err(root_error)?,
            )),
            None => None,
        };
        let mut issues = Vec::new();
        if let Some(user) = user {
            issues.extend(user.issues.iter().cloned());
            state.user = Some(Arc::new(user));
        }
        if let Some((root, project)) = project {
            issues.extend(project.issues.iter().cloned());
            state.projects.insert(root, Arc::new(project));
        }
        self.version.send_modify(|version| *version += 1);
        Ok((*self.version.borrow(), issues))
    }

    pub fn subscribe(&self) -> Result<watch::Receiver<CatalogVersion>, SkillsError> {
        self.check_open()?;
        Ok(self.version.subscribe())
    }
}

/// Process plugin providing `SkillsCatalog`.
pub struct SkillsCatalogPlugin;
impl SkillsCatalogPlugin {
    pub fn new() -> Self {
        Self
    }
}
impl Default for SkillsCatalogPlugin {
    fn default() -> Self {
        Self::new()
    }
}
impl TypedPlugin for SkillsCatalogPlugin {
    type Deps = (Arc<HostPlatform>, Arc<SkillSettingsHost>);
    fn name(&self) -> &str {
        "SkillsCatalog"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        deps: Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let catalog = Self::mount(ctx, deps).await?;
            ctx.provide_as(TypeKey::of::<SkillsCatalog>(), catalog)?;
            Ok(Effect::Done)
        })
    }
}
impl SkillsCatalogPlugin {
    /// Initializes the catalog in its owner's context. An embedding host may
    /// publish additional interfaces in the same plugin without an adapter row.
    pub async fn mount(
        ctx: &Ctx,
        (platform, settings): <Self as TypedPlugin>::Deps,
    ) -> Result<Arc<SkillsCatalog>, CordisError> {
        let settings = settings.0.clone();
        let lifecycle = ctx.cancellation_token();
        let mut changes = settings.changes();
        let initial = settings
            .skills(lifecycle.child_token())
            .await
            .map_err(|error| SkillsError::SettingsRead(error.to_string()));
        let catalog = Arc::new(SkillsCatalog {
            user_root: platform.home.join(SKILLS_DIR),
            settings: settings.clone(),
            state: tokio::sync::Mutex::new(CatalogState {
                user: None,
                projects: HashMap::new(),
                settings: initial,
            }),
            reloading: tokio::sync::Mutex::new(()),
            version: watch::channel(0).0,
            closed: lifecycle.clone(),
        });
        let tasks = TaskTracker::new();
        let draining = tasks.clone();
        ctx.effect(move || {
            Effect::AsyncDisposer(Box::new(move || {
                Box::pin(async move {
                    draining.close();
                    draining.wait().await;
                    Ok(())
                })
            }))
        })?;
        let owner = catalog.clone();
        tasks.spawn(async move {
            loop {
                tokio::select! {
                    _ = owner.closed.cancelled() => return,
                    changed = changes.changed() => if changed.is_err() { return },
                }
                if let Err(error) = owner.reload_settings().await {
                    tracing::warn!(%error, "Skill settings read failed");
                }
            }
        });
        Ok(catalog)
    }
}
impl Capability for SkillsCatalogPlugin {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&())
    }
}

/// In-memory `SkillSettings` for tests and embedded hosts without storage.
pub struct MemorySkillSettings {
    state: Mutex<SkillSettingsSnapshot>,
    changed: watch::Sender<SettingsVersion>,
}
impl Default for MemorySkillSettings {
    fn default() -> Self {
        Self {
            state: Mutex::new(SkillSettingsSnapshot {
                version: "0".into(),
                disabled: BTreeSet::new(),
            }),
            changed: watch::channel("0".into()).0,
        }
    }
}
impl SkillSettings for MemorySkillSettings {
    fn skills(
        &self,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<SkillSettingsSnapshot, SkillSettingsError>> {
        let snapshot = self.state.lock().unwrap().clone();
        Box::pin(async move { Ok(snapshot) })
    }
    fn update_skills(
        self: Arc<Self>,
        expected: SettingsVersion,
        id: String,
        enabled: bool,
        _: CancellationToken,
    ) -> BoxFuture<'static, Result<SettingsVersion, SkillSettingsError>> {
        Box::pin(async move {
            let version = {
                let mut state = self.state.lock().unwrap();
                if state.version != expected {
                    return Err(SkillSettingsError::Conflict);
                }
                if enabled {
                    state.disabled.remove(&id);
                } else {
                    state.disabled.insert(id);
                }
                let next = state.version.parse::<u64>().unwrap_or_default() + 1;
                state.version = next.to_string();
                state.version.clone()
            };
            self.changed.send_replace(version.clone());
            Ok(version)
        })
    }
    fn changes(&self) -> watch::Receiver<SettingsVersion> {
        self.changed.subscribe()
    }
}
