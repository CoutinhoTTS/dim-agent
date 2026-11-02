//! Typed MCP documents and the effective configuration of each entry.
use crate::settings::{BusinessDocument, Result, SettingsError, SettingsScope, SettingsService};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

/// Wakes the returned receiver on every committed write to MCP's Settings
/// sources and on every newly authorized project. The listeners belong to
/// `ctx` and end with it; register before the first read of the sources.
pub(super) fn settings_changes(
    ctx: &rutis::Ctx,
) -> std::result::Result<tokio::sync::watch::Receiver<()>, rutis::Error> {
    let wake = std::sync::Arc::new(tokio::sync::watch::Sender::new(()));
    for source in [
        McpSettings::NAMESPACE,
        McpProjectEnablement::NAMESPACE,
        McpPluginServers::NAMESPACE,
        McpSecrets::NAMESPACE,
    ] {
        let wake = wake.clone();
        ctx.listen::<crate::settings::SettingsChanged>()
            .name(crate::settings::settings_changes(source))
            .on(move |_, _| {
                wake.send_replace(());
                async { Ok(None) }
            })?;
    }
    let authorized = wake.clone();
    ctx.listen::<crate::settings::ProjectAuthorized>()
        .name(crate::settings::project_authorizations())
        .on(move |_, _| {
            authorized.send_replace(());
            async { Ok(None) }
        })?;
    Ok(wake.subscribe())
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
        oauth: Option<Box<McpOAuthSettings>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        enabled: Option<bool>,
        #[serde(flatten)]
        extra: BTreeMap<String, serde_json::Value>,
    },
}
impl McpServerSettings {
    /// The enablement an entry's own field states; a project entry ignores it
    /// (`config::effective`).
    pub fn enabled(&self) -> bool {
        match self {
            Self::Stdio { enabled, .. } | Self::Remote { enabled, .. } => enabled.unwrap_or(true),
        }
    }
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

/// A remote entry's OAuth settings, with the field names and types of
/// Claude Code's `.mcp.json`. They carry no secret.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpOAuthSettings {
    /// A pre-registered client (registration priority 1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// Port of the redirect `http://localhost:{port}/callback`; default 1456.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_port: Option<u16>,
    /// Authorization server metadata used only when the server publishes no
    /// valid Protected Resource Metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_server_metadata_url: Option<String>,
    /// Space-separated scopes requested as they are.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scopes: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}
impl McpOAuthSettings {
    fn validate(&self) -> Result<()> {
        let metadata_url = self
            .auth_server_metadata_url
            .as_deref()
            .map(url::Url::parse);
        if self.client_id.as_ref().is_some_and(|id| id.trim().is_empty())
            || self.callback_port == Some(0)
            || metadata_url.is_some_and(|url| {
                !url.is_ok_and(|url| {
                    url.username().is_empty()
                        && url.password().is_none()
                        && url.fragment().is_none()
                        && (url.scheme() == "https"
                            || (url.scheme() == "http"
                                && matches!(url.host(), Some(host) if loopback(&host))))
                })
            })
            // RFC 6749 §3.3 scope tokens, separated by spaces.
            || self.scopes.as_ref().is_some_and(|scopes| {
                scopes.trim().is_empty()
                    || !scopes
                        .bytes()
                        .all(|b| b == b' ' || (b.is_ascii_graphic() && b != b'"' && b != b'\\'))
            })
        {
            return Err(SettingsError::Invalid("invalid MCP OAuth configuration"));
        }
        Ok(())
    }
}

pub(crate) fn loopback(host: &url::Host<&str>) -> bool {
    match host {
        url::Host::Domain(host) => host.eq_ignore_ascii_case("localhost"),
        url::Host::Ipv4(ip) => ip.is_loopback(),
        url::Host::Ipv6(ip) => ip.is_loopback(),
    }
}

/// The hidden part of a project entry moved from user scope, keyed by
/// canonical workspace and server id. It never enters a project file and no
/// client method returns it.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpSecrets {
    #[serde(default)]
    pub entries: BTreeMap<String, BTreeMap<String, McpSecretEntry>>,
}
impl McpSecrets {
    pub(crate) fn get(&self, workspace: &str, id: &str) -> Option<&McpSecretEntry> {
        self.entries
            .get(workspace)
            .and_then(|servers| servers.get(id))
    }
    pub(crate) fn set(&mut self, workspace: &str, id: &str, secret: McpSecretEntry) {
        self.entries
            .entry(workspace.to_owned())
            .or_default()
            .insert(id.to_owned(), secret);
    }
    pub(crate) fn remove(&mut self, workspace: &str, id: &str) -> bool {
        let Some(servers) = self.entries.get_mut(workspace) else {
            return false;
        };
        let removed = servers.remove(id);
        if servers.is_empty() {
            self.entries.remove(workspace);
        }
        removed.is_some()
    }
}
impl BusinessDocument for McpSecrets {
    const NAMESPACE: &'static str = "mcp-secrets";
    const USER_FILE: &'static str = "mcp-secrets.json";

    fn validate(&self) -> Result<()> {
        for (workspace, servers) in &self.entries {
            if !Path::new(workspace).is_absolute() || workspace.is_empty() {
                return Err(SettingsError::Invalid("invalid MCP secret workspace"));
            }
            for (id, secret) in servers {
                if id.trim().is_empty() {
                    return Err(SettingsError::Invalid("empty MCP secret id"));
                }
                secret.validate()?;
            }
        }
        Ok(())
    }
}

/// The sparse overlay record: only the members the project file withholds.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpSecretEntry {
    /// The record's generation, renewed on every write. A removal deletes the
    /// record only while it still carries the generation it captured, so a
    /// replacement with equal values keeps its own record. The reserved name
    /// keeps it out of the entry members flattened into `extra`.
    #[serde(
        rename = "$generation",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub generation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<BearerAuth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth: Option<McpSecretOAuth>,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}
impl McpSecretEntry {
    pub(crate) fn is_empty(&self) -> bool {
        self.args.is_none()
            && self.env.is_none()
            && self.headers.is_none()
            && self.auth.is_none()
            && self.url.is_none()
            && self.oauth.is_none()
            && self.extra.is_empty()
    }
    fn validate(&self) -> Result<()> {
        if self.extra.contains_key("$generation") {
            return Err(SettingsError::Invalid("reserved MCP secret overlay field"));
        }
        if self
            .env
            .as_ref()
            .is_some_and(|env| env.keys().any(|name| name.trim().is_empty()))
            || self
                .headers
                .as_ref()
                .is_some_and(|headers| headers.keys().any(|name| name.trim().is_empty()))
            || self
                .auth
                .as_ref()
                .is_some_and(|BearerAuth::Bearer { token }| token.trim().is_empty())
        {
            return Err(SettingsError::Invalid("invalid MCP secret values"));
        }
        if let Some(url) = &self.url {
            let parsed = url::Url::parse(url)
                .map_err(|_| SettingsError::Invalid("invalid MCP secret URL"))?;
            if parsed.scheme() != "http" && parsed.scheme() != "https" {
                return Err(SettingsError::Invalid("invalid MCP secret URL"));
            }
        }
        if let Some(oauth) = &self.oauth {
            oauth.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpSecretOAuth {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_server_metadata_url: Option<String>,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}
impl McpSecretOAuth {
    fn validate(&self) -> Result<()> {
        if let Some(value) = &self.auth_server_metadata_url {
            let url = url::Url::parse(value)
                .map_err(|_| SettingsError::Invalid("invalid MCP OAuth configuration"))?;
            if !url.username().is_empty()
                || url.password().is_some()
                || url.fragment().is_some()
                || !(url.scheme() == "https"
                    || (url.scheme() == "http"
                        && matches!(url.host(), Some(host) if loopback(&host))))
            {
                return Err(SettingsError::Invalid("invalid MCP OAuth configuration"));
            }
        }
        Ok(())
    }
}

/// The safe part of a project entry, without the overlay-held members.
pub(crate) fn public_url(value: &str) -> String {
    let Ok(url) = url::Url::parse(value) else {
        return value.to_owned();
    };
    if !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        url.origin().ascii_serialization()
    } else {
        value.to_owned()
    }
}

fn public_oauth(oauth: &McpOAuthSettings) -> Option<McpOAuthSettings> {
    let public = McpOAuthSettings {
        client_id: oauth.client_id.clone(),
        callback_port: oauth.callback_port,
        auth_server_metadata_url: None,
        scopes: oauth.scopes.clone(),
        extra: BTreeMap::new(),
    };
    (public.client_id.is_some() || public.callback_port.is_some() || public.scopes.is_some())
        .then_some(public)
}

/// Split one entry into the public skeleton written to `.mcp.json` and the
/// withheld part kept in `mcp-secrets`.
pub(crate) fn split_entry(
    entry: &McpServerSettings,
) -> (McpServerSettings, Option<McpSecretEntry>) {
    match entry {
        McpServerSettings::Stdio {
            kind,
            command,
            args,
            env,
            cwd,
            extra,
            ..
        } => {
            let secret = McpSecretEntry {
                args: (!args.is_empty()).then(|| args.clone()),
                env: (!env.is_empty()).then(|| env.clone()),
                extra: extra.clone(),
                ..Default::default()
            };
            let skeleton = McpServerSettings::Stdio {
                kind: kind.clone(),
                command: command.clone(),
                args: Vec::new(),
                env: BTreeMap::new(),
                cwd: cwd.clone(),
                enabled: None,
                extra: BTreeMap::new(),
            };
            (skeleton, (!secret.is_empty()).then_some(secret))
        }
        McpServerSettings::Remote {
            kind,
            url,
            headers,
            auth,
            oauth,
            extra,
            ..
        } => {
            let public = public_url(url);
            let (metadata, oauth_extra) = oauth.as_ref().map_or((None, BTreeMap::new()), |oauth| {
                (oauth.auth_server_metadata_url.clone(), oauth.extra.clone())
            });
            let secret = McpSecretEntry {
                headers: (!headers.is_empty()).then(|| headers.clone()),
                auth: auth.clone(),
                url: (public != *url).then(|| url.clone()),
                oauth: (metadata.is_some() || !oauth_extra.is_empty()).then(|| McpSecretOAuth {
                    auth_server_metadata_url: metadata,
                    extra: oauth_extra,
                }),
                extra: extra.clone(),
                ..Default::default()
            };
            let skeleton = McpServerSettings::Remote {
                kind: kind.clone(),
                url: public,
                headers: BTreeMap::new(),
                auth: None,
                oauth: oauth.as_deref().and_then(public_oauth).map(Box::new),
                enabled: None,
                extra: BTreeMap::new(),
            };
            (skeleton, (!secret.is_empty()).then_some(secret))
        }
    }
}

/// Merge the overlay into a declared project entry. A member of the wrong
/// transport is an invalid overlay, not a silently ignored value.
pub(crate) fn merge_secret(config: &mut McpServerSettings, secret: &McpSecretEntry) -> Result<()> {
    match config {
        McpServerSettings::Stdio {
            args, env, extra, ..
        } => {
            if secret.url.is_some()
                || secret.headers.as_ref().is_some_and(|map| !map.is_empty())
                || secret.auth.is_some()
                || secret.oauth.as_ref().is_some_and(|oauth| {
                    oauth.auth_server_metadata_url.is_some() || !oauth.extra.is_empty()
                })
            {
                return Err(SettingsError::Invalid(
                    "MCP secret overlay does not match the stdio transport",
                ));
            }
            if let Some(value) = &secret.args {
                *args = value.clone();
            }
            if let Some(values) = &secret.env {
                env.extend(values.clone());
            }
            extra.extend(secret.extra.clone());
        }
        McpServerSettings::Remote {
            url,
            headers,
            auth,
            oauth,
            extra,
            ..
        } => {
            if secret.args.as_ref().is_some_and(|args| !args.is_empty())
                || secret.env.as_ref().is_some_and(|env| !env.is_empty())
            {
                return Err(SettingsError::Invalid(
                    "MCP secret overlay does not match the remote transport",
                ));
            }
            if let Some(value) = &secret.url {
                *url = value.clone();
            }
            if let Some(values) = &secret.headers {
                headers.extend(values.clone());
            }
            if let Some(value) = &secret.auth {
                *auth = Some(value.clone());
            }
            if let Some(secret_oauth) = &secret.oauth {
                let oauth = oauth.get_or_insert_with(Default::default);
                if let Some(value) = &secret_oauth.auth_server_metadata_url {
                    oauth.auth_server_metadata_url = Some(value.clone());
                }
                oauth.extra.extend(secret_oauth.extra.clone());
            }
            extra.extend(secret.extra.clone());
        }
    }
    Ok(())
}

/// Remove the overlay-held members before writing a project entry back.
pub(crate) fn strip_secret(config: &mut McpServerSettings, secret: &McpSecretEntry) {
    match config {
        McpServerSettings::Stdio {
            args, env, extra, ..
        } => {
            if secret.args.is_some() {
                args.clear();
            }
            if let Some(values) = &secret.env {
                for name in values.keys() {
                    env.remove(name);
                }
            }
            for name in secret.extra.keys() {
                extra.remove(name);
            }
        }
        McpServerSettings::Remote {
            url,
            headers,
            auth,
            oauth,
            extra,
            ..
        } => {
            if secret.url.is_some() {
                *url = public_url(url);
            }
            if let Some(values) = &secret.headers {
                for name in values.keys() {
                    headers.remove(name);
                }
            }
            if secret.auth.is_some() {
                *auth = None;
            }
            if let Some(secret_oauth) = &secret.oauth
                && let Some(oauth) = oauth
            {
                if secret_oauth.auth_server_metadata_url.is_some() {
                    oauth.auth_server_metadata_url = None;
                }
                for name in secret_oauth.extra.keys() {
                    oauth.extra.remove(name);
                }
            }
            for name in secret.extra.keys() {
                extra.remove(name);
            }
        }
    }
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
                    url,
                    headers,
                    auth,
                    oauth,
                    extra,
                    ..
                } => {
                    if let Some(BearerAuth::Bearer { token }) = auth
                        && token.trim().is_empty()
                    {
                        return Err(SettingsError::Invalid("empty MCP bearer token"));
                    }
                    if let Some(oauth) = oauth {
                        // Static credentials exclude OAuth (design D15).
                        if auth.is_some()
                            || headers
                                .keys()
                                .any(|name| name.eq_ignore_ascii_case("authorization"))
                        {
                            return Err(SettingsError::Invalid(
                                "MCP OAuth excludes static credentials",
                            ));
                        }
                        oauth.validate()?;
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
                            | "$generation"
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

/// The MCP documents plugins registered, by plugin id (PluginManager §3.7).
/// Mcp keeps them so they survive its reload; their entries run under the
/// ids `<plugin>:<server>`.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpPluginServers {
    #[serde(default)]
    pub plugins: BTreeMap<String, McpPluginDocument>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpPluginDocument {
    /// The plugin directory: a stdio entry's `cwd` is resolved against it.
    pub dir: PathBuf,
    pub mcp_servers: BTreeMap<String, McpServerSettings>,
}
impl McpPluginServers {
    /// Every entry under its run id, with its effective configuration.
    pub(crate) fn entries(self) -> impl Iterator<Item = (String, McpServerSettings)> {
        self.plugins.into_iter().flat_map(|(plugin, document)| {
            let dir = document.dir;
            document
                .mcp_servers
                .into_iter()
                .map(move |(id, mut config)| {
                    if let McpServerSettings::Stdio { cwd, .. } = &mut config {
                        let path = cwd.as_ref().map_or(dir.clone(), |cwd| dir.join(cwd));
                        *cwd = Some(path.to_string_lossy().into_owned());
                    }
                    (format!("{plugin}:{id}"), config)
                })
        })
    }

    /// One entry as its plugin registered it, before the plugin directory
    /// resolves a stdio `cwd`.
    pub(crate) fn declared(self, run_id: &str) -> Option<McpServerSettings> {
        self.plugins
            .into_iter()
            .flat_map(|(plugin, document)| {
                document
                    .mcp_servers
                    .into_iter()
                    .map(move |(id, config)| (format!("{plugin}:{id}"), config))
            })
            .find_map(|(id, config)| (id == run_id).then_some(config))
    }
}
impl BusinessDocument for McpPluginServers {
    const NAMESPACE: &'static str = "mcp-plugins";
    const USER_FILE: &'static str = "mcp-plugins.json";

    fn validate(&self) -> Result<()> {
        if self.extra.contains_key("plugins") {
            return Err(SettingsError::Invalid("reserved MCP plugin field"));
        }
        for (plugin, document) in &self.plugins {
            if plugin.trim().is_empty() || !document.dir.is_absolute() {
                return Err(SettingsError::Invalid("invalid MCP plugin registration"));
            }
            McpSettings {
                mcp_servers: document.mcp_servers.clone(),
                extra: BTreeMap::new(),
            }
            .validate()?;
            for config in document.mcp_servers.values() {
                if let McpServerSettings::Stdio { cwd: Some(cwd), .. } = config
                    && !Path::new(cwd).components().all(|part| {
                        matches!(
                            part,
                            std::path::Component::Normal(_) | std::path::Component::CurDir
                        )
                    })
                {
                    return Err(OUTSIDE_PLUGIN);
                }
            }
        }
        Ok(())
    }
}

/// A plugin stdio entry's `cwd` that is absolute, climbs with `..`, or
/// resolves outside the plugin directory.
pub(crate) const OUTSIDE_PLUGIN: SettingsError =
    SettingsError::Invalid("MCP plugin cwd must be a directory inside the plugin");

impl McpPluginDocument {
    /// Refuses a stdio `cwd` that, after following symbolic links, is not a
    /// directory inside the canonical plugin directory.
    pub(crate) fn confine(&self) -> Result<()> {
        let dir = std::fs::canonicalize(&self.dir).map_err(|_| OUTSIDE_PLUGIN)?;
        for config in self.mcp_servers.values() {
            if let McpServerSettings::Stdio { cwd: Some(cwd), .. } = config {
                match std::fs::canonicalize(dir.join(cwd)) {
                    Ok(resolved) if resolved.starts_with(&dir) && resolved.is_dir() => {}
                    _ => return Err(OUTSIDE_PLUGIN),
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_entry_member_named_generation_keeps_its_own_name() {
        let record = McpSecretEntry {
            generation: "g".into(),
            extra: BTreeMap::from([("generation".into(), json!("custom"))]),
            ..Default::default()
        };
        let value = serde_json::to_value(&record).unwrap();
        assert_eq!(value["$generation"], "g");
        assert_eq!(value["generation"], "custom");
        assert_eq!(
            serde_json::to_value(serde_json::from_value::<McpSecretEntry>(value).unwrap()).unwrap(),
            serde_json::to_value(&record).unwrap()
        );
    }

    #[test]
    fn a_reserved_generation_member_is_refused() {
        let document: McpSettings = serde_json::from_value(json!({
            "mcpServers": {
                "collision": {"type": "stdio", "command": "server", "$generation": "g"}
            }
        }))
        .unwrap();
        assert!(document.validate().is_err());

        let record = McpSecretEntry {
            extra: BTreeMap::from([("$generation".into(), json!("g"))]),
            ..Default::default()
        };
        assert!(record.validate().is_err());
    }
}
