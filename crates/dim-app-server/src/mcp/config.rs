//! Typed MCP documents and the effective configuration of each entry.
use crate::settings::{BusinessDocument, Result, SettingsError, SettingsScope, SettingsService};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};

/// Wakes the returned receiver on every committed write to MCP's Settings
/// sources and on every newly authorized project. The listeners belong to
/// `ctx` and end with it; register before the first read of the sources.
pub(super) fn settings_changes(
    ctx: &rutis::Ctx,
) -> std::result::Result<tokio::sync::watch::Receiver<()>, rutis::CordisError> {
    let wake = Wake(Arc::new(tokio::sync::watch::Sender::new(())));
    for source in [McpSettings::NAMESPACE, McpProjectEnablement::NAMESPACE] {
        ctx.events().on(
            ctx,
            &crate::settings::settings_changes(source),
            wake.clone(),
        )?;
    }
    ctx.events().on(
        ctx,
        &crate::settings::project_authorizations(),
        wake.clone(),
    )?;
    Ok(wake.0.subscribe())
}
#[derive(Clone)]
struct Wake(Arc<tokio::sync::watch::Sender<()>>);
impl<E: rutis::Event<Value = ()>> rutis::Listener<E> for Wake {
    fn call<'a>(
        &'a self,
        _: &'a rutis::Ctx,
        _: &'a E,
    ) -> rutis::BoxFuture<'a, std::result::Result<Option<()>, rutis::CordisError>> {
        self.0.send_replace(());
        Box::pin(async { Ok(None) })
    }
}

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

/// The user's enablement of project entries (ADR 003 D3), keyed by canonical
/// workspace and server id. A project source starts disabled: its own files
/// cannot enable an entry, and only this user-local record does.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpProjectEnablement {
    #[serde(default)]
    pub enabled: BTreeMap<String, BTreeSet<String>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}
impl McpProjectEnablement {
    pub fn contains(&self, workspace: &str, id: &str) -> bool {
        self.enabled
            .get(workspace)
            .is_some_and(|ids| ids.contains(id))
    }
    /// Returns whether the record changed.
    pub fn set(&mut self, workspace: &str, id: &str, enabled: bool) -> bool {
        if enabled {
            return self
                .enabled
                .entry(workspace.to_owned())
                .or_default()
                .insert(id.to_owned());
        }
        let Some(ids) = self.enabled.get_mut(workspace) else {
            return false;
        };
        let changed = ids.remove(id);
        if ids.is_empty() {
            self.enabled.remove(workspace);
        }
        changed
    }
}
impl BusinessDocument for McpProjectEnablement {
    const NAMESPACE: &'static str = "mcp-projects";
    const USER_FILE: &'static str = "mcp-projects.json";

    fn validate(&self) -> Result<()> {
        if self.extra.contains_key("enabled")
            || self.enabled.iter().any(|(workspace, ids)| {
                !Path::new(workspace).is_absolute()
                    || ids.is_empty()
                    || ids.iter().any(|id| id.trim().is_empty())
            })
        {
            return Err(SettingsError::Invalid("invalid MCP project enablement"));
        }
        Ok(())
    }
}

/// The canonical workspace of an authorized project source, as the
/// enablement key.
pub(crate) fn workspace(settings: &SettingsService, scope: SettingsScope) -> Result<String> {
    let SettingsScope::Project(root) = scope else {
        return Err(SettingsError::Invalid("not a project MCP source"));
    };
    settings
        .project_path(root)?
        .to_str()
        .map(str::to_owned)
        .ok_or(SettingsError::Invalid("project cwd must be UTF-8"))
}

/// The configuration an entry runs with. A project entry resolves its stdio
/// cwd against the workspace and takes its enablement only from the user's
/// record; its own `enabled` field is ignored.
pub(crate) fn effective(
    settings: &SettingsService,
    scope: SettingsScope,
    id: &str,
    mut config: McpServerSettings,
    enablement: &McpProjectEnablement,
) -> Result<McpServerSettings> {
    if matches!(scope, SettingsScope::Project(_)) {
        let workspace = workspace(settings, scope)?;
        let (McpServerSettings::Stdio { enabled, .. } | McpServerSettings::Remote { enabled, .. }) =
            &mut config;
        *enabled = Some(enablement.contains(&workspace, id));
        if let McpServerSettings::Stdio { cwd, .. } = &mut config {
            // Resolve relative directories against the workspace, not the daemon.
            let workspace = PathBuf::from(&workspace);
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
