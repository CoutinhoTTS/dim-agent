//! Skills management wire boundary; discovery and enablement belong to SkillsCatalog.
use crate::{
    methods::{Interruption, ManagementMethods, MethodDecl, params, reply},
    protocol::{
        RpcError, SelectionScope, SetSkillEnabled, SkillEntry, SkillIssue, SkillsCatalogResult,
        SkillsRefreshResult, SkillsVersion, canonical_cwd,
    },
};
use dim_agent_capabilities::skills::{RefreshTarget, SkillStatus, SkillsCatalog, SkillsError};
use dim_agent_runtime::{Capability, KeysOf, scoped};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};
use std::sync::Arc;

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
    type Deps = <dim_agent_capabilities::skills::SkillsCatalogPlugin as TypedPlugin>::Deps;
    fn name(&self) -> &str {
        "SkillsCatalog"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        deps: Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let catalog =
                dim_agent_capabilities::skills::SkillsCatalogPlugin::mount(ctx, deps).await?;
            ctx.provide_as(rutis::TypeKey::of::<SkillsCatalog>(), catalog.clone())?;
            ManagementMethods::provide(
                ctx,
                "skills",
                ["skills.catalog", "skills.set_enabled", "skills.refresh"]
                    .into_iter()
                    .map(|name| {
                        MethodDecl::management(name, name != "skills.catalog", Interruption::Report)
                    })
                    .collect(),
                vec![],
                Arc::new(move |method, value, _stopping| {
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
