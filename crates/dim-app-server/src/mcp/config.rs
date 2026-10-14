//! Typed MCP documents and host-scoped execution identity.
use crate::settings::{BusinessDocument, Result, SettingsError, SettingsScope, SettingsService};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpSettings {
    pub mcp_servers: BTreeMap<String, McpServerSettings>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum McpServerSettings {
    Stdio {
        #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
        kind: Option<Stdio>,
        command: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env: BTreeMap<String, String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        enabled: Option<bool>,
        #[serde(flatten)]
        extra: BTreeMap<String, serde_json::Value>,
    },
    Remote {
        #[serde(rename = "type")]
        kind: RemoteTransport,
        url: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        headers: BTreeMap<String, String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        auth: Option<BearerAuth>,
        #[serde(skip_serializing_if = "Option::is_none")]
        enabled: Option<bool>,
        #[serde(flatten)]
        extra: BTreeMap<String, serde_json::Value>,
    },
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Stdio {
    Stdio,
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RemoteTransport {
    Http,
    Sse,
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum BearerAuth {
    Bearer { token: String },
}

impl BusinessDocument for McpSettings {
    const NAMESPACE: &'static str = "mcp";
    const USER_FILE: &'static str = "mcp.json";
    const PROJECT_FILE: Option<&'static str> = Some(".mcp.json");

    fn validate(&self) -> Result<()> {
        if self.extra.contains_key("mcpServers") {
            return Err(SettingsError::Invalid("reserved MCP configuration field"));
        }
        for (id, server) in &self.mcp_servers {
            if id.trim().is_empty() {
                return Err(SettingsError::Invalid("empty MCP server id"));
            }
            let (target, extra) = match server {
                McpServerSettings::Stdio { command, extra, .. } => (command, extra),
                McpServerSettings::Remote {
                    url, auth, extra, ..
                } => {
                    if let Some(BearerAuth::Bearer { token }) = auth
                        && token.trim().is_empty()
                    {
                        return Err(SettingsError::Invalid("empty MCP bearer token"));
                    }
                    (url, extra)
                }
            };
            if target.trim().is_empty()
                || extra.keys().any(|key| {
                    matches!(
                        key.as_str(),
                        "type"
                            | "command"
                            | "args"
                            | "env"
                            | "cwd"
                            | "enabled"
                            | "url"
                            | "headers"
                            | "auth"
                            | "transport"
                            | "oauth"
                    )
                })
            {
                return Err(SettingsError::Invalid(
                    "invalid MCP transport configuration",
                ));
            }
            if extra
                .get("metadata")
                .is_some_and(|value| !value.is_object())
            {
                return Err(SettingsError::Invalid("MCP metadata must be an object"));
            }
        }
        Ok(())
    }
}

pub(crate) fn workspace(settings: &SettingsService, scope: SettingsScope) -> Result<PathBuf> {
    let SettingsScope::Project(root) = scope else {
        return Err(SettingsError::Invalid(
            "MCP approval requires a project source",
        ));
    };
    settings.project_path(root)
}

pub(crate) fn effective(
    settings: &SettingsService,
    scope: SettingsScope,
    mut config: McpServerSettings,
) -> Result<McpServerSettings> {
    if matches!(scope, SettingsScope::Project(_)) {
        let workspace = workspace(settings, scope)?;
        if let McpServerSettings::Stdio { cwd, .. } = &mut config {
            // Resolve relative directories against the workspace, not the daemon.
            let path = cwd
                .as_ref()
                .map_or(workspace.clone(), |cwd| workspace.join(cwd));
            *cwd = Some(
                path.to_str()
                    .ok_or(SettingsError::Invalid("project cwd must be UTF-8"))?
                    .to_owned(),
            );
        }
    }
    Ok(config)
}

pub(crate) fn grant_key(
    settings: &SettingsService,
    scope: SettingsScope,
    id: &str,
) -> Result<String> {
    let workspace = workspace(settings, scope)?;
    let bytes = serde_json::to_vec(&(workspace, id))
        .map_err(|_| SettingsError::Invalid("invalid MCP identity"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub(crate) fn fingerprint(
    settings: &SettingsService,
    scope: SettingsScope,
    id: &str,
    config: &McpServerSettings,
) -> Result<String> {
    let key = grant_key(settings, scope, id)?;
    let mut config = config.clone();
    // Enablement is intent, not a new executable configuration.
    let (McpServerSettings::Stdio { enabled, .. } | McpServerSettings::Remote { enabled, .. }) =
        &mut config;
    *enabled = None;
    let bytes = serde_json::to_vec(&("dim/mcp-execution/v1", key, config))
        .map_err(|_| SettingsError::Invalid("invalid MCP configuration"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
