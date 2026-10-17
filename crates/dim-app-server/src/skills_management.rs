//! Skills management wire boundary; discovery and enablement belong to SkillsCatalog.
use crate::{
    methods::{ManagementMethods, SelectionScope},
    protocol::{canonical_cwd, optional_non_null},
    settings::{BusinessDocument, SettingsError, SettingsScope, SettingsService},
};
use dim_agent_capabilities::methods::{Interruption, MethodDecl, RpcError, params, reply};
use dim_agent_capabilities::skills::{RefreshTarget, SkillStatus, SkillsCatalog, SkillsError};
use dim_agent_capabilities::{
    host_platform::HostPlatform,
    skill_settings::{
        SOURCE, SettingsVersion, SkillSettings, SkillSettingsError, SkillSettingsHost,
        SkillSettingsSnapshot,
    },
};
use dim_agent_runtime::{Capability, KeysOf, scoped};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SkillDocument {
    disabled_skill_ids: Vec<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}
impl BusinessDocument for SkillDocument {
    const NAMESPACE: &'static str = SOURCE;
    const USER_FILE: &'static str = "skills.json";
    fn validate(&self) -> crate::settings::Result<()> {
        if self.extra.contains_key("disabledSkillIds") {
            return Err(SettingsError::Invalid("reserved Skill configuration field"));
        }
        if self
            .disabled_skill_ids
            .iter()
            .any(|id| id.trim().is_empty())
        {
            return Err(SettingsError::Invalid("empty Skill id"));
        }
        Ok(())
    }
}

/// Adapts the owner's business document to the reusable Skill catalog port.
/// Settings announces its writes; the catalog listens to them itself.
struct SkillStorage {
    settings: Arc<SettingsService>,
}
impl SkillSettings for SkillStorage {
    fn skills(
        &self,
        cancel: CancellationToken,
    ) -> BoxFuture<'_, Result<SkillSettingsSnapshot, SkillSettingsError>> {
        Box::pin(async move {
            let snapshot = self
                .settings
                .read::<SkillDocument>(SettingsScope::User, &cancel)
                .await?;
            Ok(SkillSettingsSnapshot {
                version: snapshot.revision,
                disabled: snapshot.document.disabled_skill_ids.into_iter().collect(),
            })
        })
    }
    fn update_skills(
        self: Arc<Self>,
        expected: SettingsVersion,
        id: String,
        enabled: bool,
        cancel: CancellationToken,
    ) -> BoxFuture<'static, Result<SettingsVersion, SkillSettingsError>> {
        Box::pin(async move {
            if id.trim().is_empty() {
                return Err(SkillSettingsError::Invalid("empty Skill id".into()));
            }
            let current = self
                .settings
                .read::<SkillDocument>(SettingsScope::User, &cancel)
                .await?;
            if current.revision != expected {
                return Err(SkillSettingsError::Conflict);
            }
            let mut document = current.document;
            if document.disabled_skill_ids.contains(&id) != enabled {
                return Ok(current.revision);
            }
            if enabled {
                document.disabled_skill_ids.retain(|value| value != &id);
            } else {
                document.disabled_skill_ids.push(id);
            }
            Ok(self
                .settings
                .update(SettingsScope::User, &expected, document, &cancel)
                .await?
                .revision)
        })
    }
}
impl From<SettingsError> for SkillSettingsError {
    fn from(error: SettingsError) -> Self {
        match error {
            SettingsError::Conflict => Self::Conflict,
            SettingsError::Invalid(reason) => Self::Invalid(reason.into()),
            SettingsError::Closed => Self::Closed,
            SettingsError::Cancelled => Self::Cancelled,
            error => Self::Failed(error.to_string()),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetSkillEnabled {
    #[serde(default, deserialize_with = "optional_non_null")]
    pub cwd: Option<String>,
    pub id: String,
    pub enabled: bool,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub file_path: String,
    pub allowed_tools: Vec<String>,
    pub enabled: bool,
    pub shadowed_by: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct SkillIssue {
    pub path: String,
    pub message: String,
}
#[derive(Debug, Serialize)]
pub struct SkillsCatalogResult {
    pub version: u64,
    pub entries: Vec<SkillEntry>,
    pub issues: Vec<SkillIssue>,
}
#[derive(Debug, Serialize)]
pub struct SkillsRefreshResult {
    pub version: u64,
    pub issues: Vec<SkillIssue>,
}
#[derive(Debug, Serialize)]
pub struct SkillsVersion {
    pub version: u64,
}

fn error(error: SkillsError) -> RpcError {
    let kind = match &error {
        SkillsError::Closed => return RpcError::reloading(),
        SkillsError::UnknownSkill(_) => "skill_unknown",
        SkillsError::SettingsConflict => "skills_conflict",
        SkillsError::SettingsWrite(_) => "skills_write_failed",
        SkillsError::SettingsSync(_) => "skills_sync_failed",
        SkillsError::SettingsRead(_) | SkillsError::RootUnreadable { .. } => "skills_unavailable",
    };
    RpcError::app(-32000, kind, error.to_string())
}

fn issue(issue: dim_agent_capabilities::skills::SkillIssue) -> SkillIssue {
    use dim_agent_capabilities::skills::SkillIssue as Source;
    match issue {
        Source::InvalidManifest { path, reason } | Source::Unreadable { path, reason } => {
            SkillIssue {
                path: path.to_string_lossy().into_owned(),
                message: reason,
            }
        }
        Source::RootUnreadable { root, reason } => SkillIssue {
            path: root.to_string_lossy().into_owned(),
            message: reason,
        },
        Source::ScanLimitReached { root } => SkillIssue {
            path: root.to_string_lossy().into_owned(),
            message: "Skill scan limit reached".into(),
        },
    }
}

pub struct SkillsCatalogPlugin;
impl TypedPlugin for SkillsCatalogPlugin {
    type Deps = (Arc<HostPlatform>, Arc<SettingsService>);
    fn name(&self) -> &str {
        "SkillsCatalog"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (platform, settings): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let settings = Arc::new(SkillSettingsHost(Arc::new(SkillStorage { settings })));
            let catalog = dim_agent_capabilities::skills::SkillsCatalogPlugin::mount(
                ctx,
                (platform, settings),
            )
            .await?;
            ctx.provide_as(rutis::TypeKey::of::<SkillsCatalog>(), catalog.clone())?;
            ManagementMethods::provide(
                ctx,
                "skills",
                ["skills.catalog", "skills.set_enabled", "skills.refresh"]
                    .into_iter()
                    .map(|name| {
                        MethodDecl::new(name, name != "skills.catalog", Interruption::Report)
                    })
                    .collect(),
                Arc::new(move |method, value, _, _| {
                    let catalog = catalog.clone();
                    Box::pin(async move {
                        // SkillsCatalog has no caller cancellation argument. Keep its
                        // accepted future alive and drain it rather than dropping mutations.
                        match method.as_str() {
                            "skills.catalog" => {
                                let p: SelectionScope = params(value)?;
                                let cwd = match p.cwd {
                                    Some(cwd) => Some(canonical_cwd(&cwd).await?),
                                    None => None,
                                };
                                let view = catalog.query(cwd.as_deref()).await.map_err(error)?;
                                reply(SkillsCatalogResult {
                                    version: view.version,
                                    entries: view
                                        .entries
                                        .into_iter()
                                        .map(|entry| SkillEntry {
                                            id: entry.definition.id.clone(),
                                            name: entry.definition.name.clone(),
                                            description: entry.definition.description.clone(),
                                            file_path: entry
                                                .definition
                                                .file_path
                                                .to_string_lossy()
                                                .into_owned(),
                                            allowed_tools: entry.definition.allowed_tools.clone(),
                                            enabled: entry.enabled,
                                            shadowed_by: match entry.status {
                                                SkillStatus::Active => None,
                                                SkillStatus::Shadowed { by } => Some(by),
                                            },
                                        })
                                        .collect(),
                                    issues: view.issues.into_iter().map(issue).collect(),
                                })
                            }
                            "skills.set_enabled" => {
                                let p: SetSkillEnabled = params(value)?;
                                if p.id.trim().is_empty() {
                                    return Err(RpcError::invalid("id must not be empty"));
                                }
                                let cwd = match p.cwd {
                                    Some(cwd) => Some(canonical_cwd(&cwd).await?),
                                    None => None,
                                };
                                let (_, version) = catalog
                                    .set_enabled(&p.id, p.enabled, cwd.as_deref())
                                    .await
                                    .map_err(error)?;
                                reply(SkillsVersion { version })
                            }
                            "skills.refresh" => {
                                let p: SelectionScope = params(value)?;
                                let project = match p.cwd {
                                    Some(cwd) => Some(canonical_cwd(&cwd).await?),
                                    None => None,
                                };
                                let (version, issues) = catalog
                                    .refresh(RefreshTarget {
                                        user: true,
                                        project,
                                    })
                                    .await
                                    .map_err(error)?;
                                reply(SkillsRefreshResult {
                                    version,
                                    issues: issues.into_iter().map(issue).collect(),
                                })
                            }
                            _ => unreachable!("registered Skills management method"),
                        }
                    })
                }),
            )?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for SkillsCatalogPlugin {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&())
    }
}
