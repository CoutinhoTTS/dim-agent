//! MCP management wire boundary over the MCP and Settings owners.
use super::config::{McpServerSettings, RemoteTransport};
use super::{McpChange, McpError, McpService};
use crate::{
    methods::{Interruption, ManagementMethods, MethodDecl, SelectionScope, params, reply},
    protocol::{RpcError, canonical_cwd, optional_non_null},
    settings::{SettingsError, SettingsScope, SettingsService},
};
use rutis::{CordisError, Ctx};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum McpScope {
    User,
    Project,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpTarget {
    #[serde(default, deserialize_with = "optional_non_null")]
    pub cwd: Option<String>,
    pub id: String,
    pub source: McpScope,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetMcpEnabled {
    #[serde(default, deserialize_with = "optional_non_null")]
    pub cwd: Option<String>,
    pub id: String,
    pub source: McpScope,
    pub expected_version: String,
    pub enabled: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveMcp {
    #[serde(default, deserialize_with = "optional_non_null")]
    pub cwd: Option<String>,
    pub id: String,
    pub source: McpScope,
    pub expected_version: String,
    pub configuration: McpServerSettings,
}
impl std::fmt::Debug for SaveMcp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveMcp").finish_non_exhaustive()
    }
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoveMcp {
    #[serde(default, deserialize_with = "optional_non_null")]
    pub cwd: Option<String>,
    pub id: String,
    pub source: McpScope,
    pub expected_version: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCatalogResult {
    pub user_revision: String,
    pub project_revision: Option<String>,
    pub user_application: crate::mcp::McpApplication,
    pub project_application: Option<crate::mcp::McpApplication>,
    pub servers: Vec<McpCatalogEntry>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCatalogEntry {
    pub id: String,
    pub source: McpScope,
    pub enabled: bool,
    pub transport: &'static str,
}

pub(crate) fn error(error: McpError) -> RpcError {
    let kind = match &error {
        McpError::Settings(SettingsError::Conflict) => "mcp_settings_conflict",
        McpError::Settings(SettingsError::Invalid(_)) | McpError::InvalidId => "mcp_invalid_config",
        McpError::UnknownServer => "mcp_unknown_server",
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

fn source(scope: McpScope, project: Option<usize>) -> Result<SettingsScope, RpcError> {
    match scope {
        McpScope::User => Ok(SettingsScope::User),
        McpScope::Project => project
            .map(SettingsScope::Project)
            .ok_or_else(|| RpcError::invalid("Project MCP operations require cwd")),
    }
}

pub(super) fn provide(
    ctx: &Ctx,
    directory: Arc<McpService>,
    settings: Arc<SettingsService>,
) -> Result<(), CordisError> {
    ManagementMethods::provide(
        ctx,
        "mcp",
        [
            "mcp.catalog",
            "mcp.save",
            "mcp.remove",
            "mcp.set_enabled",
            "mcp.disconnect",
        ]
        .into_iter()
        .map(|name| MethodDecl::management(name, name != "mcp.catalog", Interruption::Report))
        .collect(),
        Arc::new(move |method, value, stopping| {
            let directory = directory.clone();
            let settings = settings.clone();
            Box::pin(async move {
                match method.as_str() {
                    "mcp.catalog" => {
                        let p: SelectionScope = params(value)?;
                        let project = project(&settings, p.cwd, &stopping).await?;
                        let catalog = directory.catalog(project, &stopping).await.map_err(error)?;
                        return reply(McpCatalogResult {
                            user_revision: catalog.user_revision,
                            project_revision: catalog.project_revision,
                            user_application: directory.application(SettingsScope::User),
                            project_application: project
                                .map(|root| directory.application(SettingsScope::Project(root))),
                            servers: catalog
                                .servers
                                .into_iter()
                                .map(|(id, entry)| McpCatalogEntry {
                                    id,
                                    source: match entry.source {
                                        SettingsScope::User => McpScope::User,
                                        SettingsScope::Project(_) => McpScope::Project,
                                    },
                                    enabled: entry.enabled(),
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
                    "mcp.save" => {
                        let p: SaveMcp = params(value)?;
                        let source = source(p.source, project(&settings, p.cwd, &stopping).await?)?;
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
                        let source = source(p.source, project(&settings, p.cwd, &stopping).await?)?;
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
                        let source = source(p.source, project(&settings, p.cwd, &stopping).await?)?;
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
                        let source = source(p.source, project(&settings, p.cwd, &stopping).await?)?;
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
    )?;
    Ok(())
}
