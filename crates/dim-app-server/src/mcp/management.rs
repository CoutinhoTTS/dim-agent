//! MCP management wire boundary over the MCP and Settings owners.
use super::{McpChange, McpError, McpService};
use crate::{
    methods::{Interruption, MethodDecl, Methods, params, reply},
    protocol::{
        ApproveMcp, McpCatalogEntry, McpCatalogResult, McpScope, McpTarget, RemoveMcp, RevokeMcp,
        RpcError, SaveMcp, SelectionScope, SetMcpEnabled, canonical_cwd,
    },
    settings::{
        McpServerSettings, RemoteTransport, SettingsError, SettingsService, SettingsSource,
    },
};
use dim_agent_runtime::{Capability, KeysOf, scoped};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

fn error(error: McpError) -> RpcError {
    let kind = match &error {
        McpError::Settings(SettingsError::Conflict) => "mcp_settings_conflict",
        McpError::Settings(SettingsError::AppliedUnsaved) => "mcp_settings_unsaved",
        McpError::Settings(SettingsError::Invalid(_))
        | McpError::InvalidSource
        | McpError::InvalidId => "mcp_invalid_config",
        McpError::UnknownServer => "mcp_unknown_server",
        McpError::ApprovalRequired => "mcp_approval_required",
        McpError::Closed | McpError::Settings(SettingsError::Closed) => {
            return RpcError::reloading();
        }
        McpError::Cancelled | McpError::Settings(SettingsError::Cancelled) => {
            "operation_interrupted"
        }
        McpError::Settings(_) => "mcp_settings_read",
        _ => "mcp_unavailable",
    };
    RpcError::app(
        -32000,
        kind,
        match error {
            McpError::Worker(_) | McpError::Settings(SettingsError::Worker(_)) => {
                "MCP worker failed".into()
            }
            other => other.to_string(),
        },
    )
}

async fn project(
    settings: &Arc<SettingsService>,
    cwd: Option<String>,
    stopping: &CancellationToken,
) -> Result<Option<usize>, RpcError> {
    let Some(cwd) = cwd else { return Ok(None) };
    let cwd = canonical_cwd(&cwd).await?;
    settings
        .authorize_project(cwd, stopping)
        .await
        .map(Some)
        .map_err(|e| error(e.into()))
}

fn source(scope: McpScope, project: Option<usize>) -> Result<SettingsSource, RpcError> {
    match scope {
        McpScope::User => Ok(SettingsSource::UserMcp),
        McpScope::Project => project
            .map(SettingsSource::ProjectMcp)
            .ok_or_else(|| RpcError::invalid("Project MCP operations require cwd")),
    }
}

pub struct McpManagement;
impl TypedPlugin for McpManagement {
    type Deps = (Arc<Methods>, Arc<McpService>, Arc<SettingsService>);
    fn name(&self) -> &str {
        "McpManagement"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (management, directory, settings): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let registration = management
                .register(
                    "mcp",
                    [
                        "mcp.catalog",
                        "mcp.approve",
                        "mcp.revoke",
                        "mcp.save",
                        "mcp.remove",
                        "mcp.set_enabled",
                        "mcp.disconnect",
                    ]
                    .into_iter()
                    .map(|name| {
                        MethodDecl::management(name, name != "mcp.catalog", Interruption::Report)
                    })
                    .collect(),
                    vec![],
                    Arc::new(move |method, value, stopping| {
                        let directory = directory.clone();
                        let settings = settings.clone();
                        Box::pin(async move {
                            match method.as_str() {
                                "mcp.catalog" => {
                                    let p: SelectionScope = params(value)?;
                                    let project = project(&settings, p.cwd, &stopping).await?;
                                    let catalog = directory
                                        .catalog(project, &stopping)
                                        .await
                                        .map_err(error)?;
                                    return reply(McpCatalogResult {
                                        user_revision: catalog.user_revision,
                                        project_revision: catalog.project_revision,
                                        user_application: directory
                                            .application(SettingsSource::UserMcp),
                                        project_application: project.map(|root| {
                                            directory.application(SettingsSource::ProjectMcp(root))
                                        }),
                                        servers: catalog
                                            .servers
                                            .into_iter()
                                            .map(|(id, entry)| McpCatalogEntry {
                                                id,
                                                source: match entry.source {
                                                    SettingsSource::UserMcp => McpScope::User,
                                                    SettingsSource::ProjectMcp(_) => {
                                                        McpScope::Project
                                                    }
                                                    SettingsSource::Skills => unreachable!(
                                                        "MCP directory excludes Skills"
                                                    ),
                                                },
                                                enabled: entry.enabled(),
                                                approval: entry.approval,
                                                approval_token: entry.approval_token,
                                                transport: match entry.configuration {
                                                    McpServerSettings::Stdio { .. } => "stdio",
                                                    McpServerSettings::Remote {
                                                        kind: RemoteTransport::Http,
                                                        ..
                                                    } => "http",
                                                    McpServerSettings::Remote {
                                                        kind: RemoteTransport::Sse,
                                                        ..
                                                    } => "sse",
                                                },
                                            })
                                            .collect(),
                                    });
                                }
                                "mcp.approve" => {
                                    let p: ApproveMcp = params(value)?;
                                    let project = project(&settings, Some(p.cwd), &stopping)
                                        .await?
                                        .expect("required cwd");
                                    directory
                                        .approve(project, p.id, p.approval_token, &stopping)
                                        .await
                                        .map_err(error)?;
                                }
                                "mcp.revoke" => {
                                    let p: RevokeMcp = params(value)?;
                                    let project = project(&settings, Some(p.cwd), &stopping)
                                        .await?
                                        .expect("required cwd");
                                    directory
                                        .revoke(project, p.id, &stopping)
                                        .await
                                        .map_err(error)?;
                                }
                                "mcp.save" => {
                                    let p: SaveMcp = params(value)?;
                                    let source = source(
                                        p.source,
                                        project(&settings, p.cwd, &stopping).await?,
                                    )?;
                                    directory
                                        .update(
                                            source,
                                            p.expected_version,
                                            p.id,
                                            McpChange::Put(p.configuration),
                                            &stopping,
                                        )
                                        .await
                                        .map_err(error)?;
                                }
                                "mcp.remove" => {
                                    let p: RemoveMcp = params(value)?;
                                    let source = source(
                                        p.source,
                                        project(&settings, p.cwd, &stopping).await?,
                                    )?;
                                    directory
                                        .update(
                                            source,
                                            p.expected_version,
                                            p.id,
                                            McpChange::Remove,
                                            &stopping,
                                        )
                                        .await
                                        .map_err(error)?;
                                }
                                "mcp.set_enabled" => {
                                    let p: SetMcpEnabled = params(value)?;
                                    let source = source(
                                        p.source,
                                        project(&settings, p.cwd, &stopping).await?,
                                    )?;
                                    directory
                                        .update(
                                            source,
                                            p.expected_version,
                                            p.id,
                                            McpChange::SetEnabled(p.enabled),
                                            &stopping,
                                        )
                                        .await
                                        .map_err(error)?;
                                }
                                "mcp.disconnect" => {
                                    let p: McpTarget = params(value)?;
                                    if p.id.trim().is_empty() {
                                        return Err(RpcError::invalid("id must not be empty"));
                                    }
                                    let source = source(
                                        p.source,
                                        project(&settings, p.cwd, &stopping).await?,
                                    )?;
                                    directory
                                        .disconnect(source, p.id, &stopping)
                                        .await
                                        .map_err(error)?;
                                }
                                _ => unreachable!("registered MCP management method"),
                            }
                            reply(serde_json::json!({}))
                        })
                    }),
                )
                .map_err(|error| CordisError::PluginFailed(error.message.into()))?;
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        registration.unregister_and_drain().await;
                        Ok(())
                    })
                }))
            })?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for McpManagement {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&())
    }
}
