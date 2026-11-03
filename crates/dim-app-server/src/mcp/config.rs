//! Typed MCP documents and the effective configuration of each entry.
use crate::settings::{BusinessDocument, Result, SettingsError, SettingsScope, SettingsService};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
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

/// Why one entry did not decode. It names the entry and the field and states
/// the expected kind; it never repeats the value, which may hold a credential.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpEntryError {
    pub id: String,
    pub field: Option<String>,
    pub expected: String,
}
impl fmt::Display for McpEntryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.field {
            Some(field) => write!(
                formatter,
                "MCP server {}: field {} must be {}",
                self.id, field, self.expected
            ),
            None => write!(
                formatter,
                "MCP server {}: expected {}",
                self.id, self.expected
            ),
        }
    }
}
impl std::error::Error for McpEntryError {}

/// One entry that did not decode, kept exactly as the document wrote it so a
/// later write puts it back unchanged.
#[derive(Clone, Debug, PartialEq)]
pub struct InvalidMcpServer {
    pub value: Value,
    pub error: McpEntryError,
}

/// Every entry under `mcpServers`. An entry that does not decode is kept out
/// of `mcp_servers` and listed in `invalid`; `raw` holds each entry's value as
/// read so a write can put every untouched entry back as the file wrote it.
#[derive(Clone, Default, PartialEq)]
pub struct McpSettings {
    mcp_servers: BTreeMap<String, McpServerSettings>,
    invalid: BTreeMap<String, InvalidMcpServer>,
    raw: BTreeMap<String, Value>,
    dirty: BTreeSet<String>,
    pub extra: BTreeMap<String, Value>,
}
impl McpSettings {
    /// Replaces an entry. Only this entry is serialised from its typed value
    /// on the next write.
    pub fn insert(&mut self, id: impl Into<String>, configuration: McpServerSettings) {
        let id = id.into();
        self.invalid.remove(&id);
        self.mcp_servers.insert(id.clone(), configuration);
        self.dirty.insert(id);
    }

    /// Removes a decoded or a broken entry. Only this entry's value is
    /// dropped from the document.
    pub fn remove(&mut self, id: &str) -> Option<McpServerSettings> {
        self.invalid.remove(id);
        self.dirty.insert(id.to_owned());
        self.mcp_servers.remove(id)
    }

    /// A mutable decoded entry, marked for serialisation from its typed value.
    pub fn configure_mut(&mut self, id: &str) -> Option<&mut McpServerSettings> {
        if self.mcp_servers.contains_key(id) {
            self.dirty.insert(id.to_owned());
        }
        self.mcp_servers.get_mut(id)
    }

    pub fn invalid_error(&self, id: &str) -> Option<&McpEntryError> {
        self.invalid.get(id).map(|entry| &entry.error)
    }

    /// The decoded entries, by id.
    pub fn entries(&self) -> &BTreeMap<String, McpServerSettings> {
        &self.mcp_servers
    }

    /// One decoded entry, by id.
    pub fn get(&self, id: &str) -> Option<&McpServerSettings> {
        self.mcp_servers.get(id)
    }

    /// Whether `id` names a decoded entry.
    pub fn contains(&self, id: &str) -> bool {
        self.mcp_servers.contains_key(id)
    }

    /// Entries that did not decode, by id.
    pub fn invalid_entries(&self) -> &BTreeMap<String, InvalidMcpServer> {
        &self.invalid
    }

    /// Consumes the document into its decoded entries and its broken entries.
    pub fn into_parts(
        self,
    ) -> (
        BTreeMap<String, McpServerSettings>,
        BTreeMap<String, InvalidMcpServer>,
    ) {
        (self.mcp_servers, self.invalid)
    }

    fn from_entries(entries: BTreeMap<String, Value>, extra: BTreeMap<String, Value>) -> Self {
        let mut document = Self {
            extra,
            ..Self::default()
        };
        for (id, value) in entries {
            let decoded = if id.trim().is_empty() {
                Err(McpEntryError {
                    id: id.clone(),
                    field: None,
                    expected: "a nonempty id".into(),
                })
            } else {
                decode_entry(&id, &value)
            };
            match decoded {
                Ok(configuration) => {
                    document.mcp_servers.insert(id.clone(), configuration);
                }
                Err(error) => {
                    document.invalid.insert(
                        id.clone(),
                        InvalidMcpServer {
                            value: value.clone(),
                            error,
                        },
                    );
                }
            }
            document.raw.insert(id, value);
        }
        document
    }

    /// Decodes each entry on its own. Only a file that is not JSON and an
    /// `mcpServers` that is not an object fail the whole document.
    pub fn from_file(bytes: &[u8]) -> Result<Self> {
        let value: Value = serde_json::from_slice(bytes).map_err(|error| {
            SettingsError::Document(format!(
                "MCP document is not valid JSON at line {} column {}",
                error.line(),
                error.column()
            ))
        })?;
        let object = match value.as_object() {
            Some(object) => object,
            None => {
                let position = serde_json::from_slice::<BTreeMap<String, Value>>(bytes)
                    .err()
                    .map(|error| format!(" at line {} column {}", error.line(), error.column()))
                    .unwrap_or_default();
                return Err(SettingsError::Document(format!(
                    "MCP document must be a JSON object{position}"
                )));
            }
        };
        let servers = match object.get("mcpServers") {
            Some(Value::Object(servers)) => servers,
            Some(_) => {
                let position = serde_json::from_slice::<RequiredServers>(bytes)
                    .err()
                    .map(|error| format!(" at line {} column {}", error.line(), error.column()))
                    .unwrap_or_default();
                return Err(SettingsError::Document(format!(
                    "mcpServers must be a JSON object{position}"
                )));
            }
            None => {
                let position = serde_json::from_slice::<RequiredServers>(bytes)
                    .err()
                    .map(|error| format!(" at line {} column {}", error.line(), error.column()))
                    .unwrap_or_default();
                return Err(SettingsError::Document(format!(
                    "MCP document must contain an mcpServers object{position}"
                )));
            }
        };
        let extra = object
            .iter()
            .filter(|(key, _)| key.as_str() != "mcpServers")
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        Ok(Self::from_entries(
            servers
                .iter()
                .map(|(id, value)| (id.clone(), value.clone()))
                .collect(),
            extra,
        ))
    }
}
impl Serialize for McpSettings {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::{Error as _, SerializeMap};
        let mut map = serializer.serialize_map(Some(1 + self.extra.len()))?;
        let mut servers = BTreeMap::<&str, Value>::new();
        for (id, raw) in &self.raw {
            if let Some(configuration) = self.mcp_servers.get(id) {
                let value = if self.dirty.contains(id) {
                    serde_json::to_value(configuration).map_err(S::Error::custom)?
                } else {
                    raw.clone()
                };
                servers.insert(id, value);
            } else if self.invalid.contains_key(id) {
                servers.insert(id, raw.clone());
            }
        }
        for (id, configuration) in &self.mcp_servers {
            if !self.raw.contains_key(id) {
                servers.insert(
                    id,
                    serde_json::to_value(configuration).map_err(S::Error::custom)?,
                );
            }
        }
        map.serialize_entry("mcpServers", &servers)?;
        for (key, value) in &self.extra {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}
impl<'de> Deserialize<'de> for McpSettings {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(rename = "mcpServers")]
            mcp_servers: BTreeMap<String, Value>,
            #[serde(flatten)]
            extra: BTreeMap<String, Value>,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(Self::from_entries(raw.mcp_servers, raw.extra))
    }
}
#[derive(Deserialize)]
struct RequiredServers {
    #[serde(rename = "mcpServers")]
    _mcp_servers: BTreeMap<String, Value>,
}

enum Transport {
    Stdio,
    Http,
    Sse,
}

fn entry_error(id: &str, field: Option<&str>, expected: &str) -> McpEntryError {
    McpEntryError {
        id: id.to_owned(),
        field: field.map(str::to_owned),
        expected: expected.to_owned(),
    }
}

/// Decodes one entry. The error names the entry and the field and never
/// repeats the value.
pub fn decode_entry(
    id: &str,
    value: &Value,
) -> std::result::Result<McpServerSettings, McpEntryError> {
    if id.trim().is_empty() {
        return Err(entry_error(id, None, "a nonempty id"));
    }
    let entry = value
        .as_object()
        .ok_or_else(|| entry_error(id, None, "a JSON object"))?;
    let transport = match entry.get("type") {
        Some(Value::String(kind)) => match kind.as_str() {
            "stdio" => Transport::Stdio,
            "http" | "streamable-http" => Transport::Http,
            "sse" => Transport::Sse,
            "ws" => {
                return Err(entry_error(
                    id,
                    Some("type"),
                    "http or sse; ws is not a supported transport",
                ));
            }
            _ => return Err(entry_error(id, Some("type"), "stdio, http or sse")),
        },
        Some(_) => return Err(entry_error(id, Some("type"), "a string")),
        None if entry.contains_key("command") => Transport::Stdio,
        None if entry.contains_key("url") => {
            return Err(entry_error(
                id,
                Some("type"),
                "http or sse; an entry with a url must name its type",
            ));
        }
        None => {
            return Err(entry_error(
                id,
                Some("type"),
                "stdio, http or sse, or a command for stdio",
            ));
        }
    };
    for key in entry.keys() {
        if key == "$generation" {
            return Err(entry_error(id, Some(key), "not a reserved field"));
        }
        if key == "transport" {
            return Err(entry_error(id, Some(key), "not a field; set type instead"));
        }
    }
    if let Some(metadata) = entry.get("metadata")
        && !metadata.is_object()
    {
        return Err(entry_error(id, Some("metadata"), "an object"));
    }
    match transport {
        Transport::Stdio => decode_stdio(id, entry),
        Transport::Http => decode_remote(id, entry, RemoteTransport::Http),
        Transport::Sse => decode_remote(id, entry, RemoteTransport::Sse),
    }
}

fn field_string(
    entry: &Map<String, Value>,
    id: &str,
    field: &str,
    expected: &str,
) -> std::result::Result<Option<String>, McpEntryError> {
    match entry.get(field) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(entry_error(id, Some(field), expected)),
    }
}

fn string_map(
    entry: &Map<String, Value>,
    id: &str,
    field: &str,
) -> std::result::Result<BTreeMap<String, String>, McpEntryError> {
    match entry.get(field) {
        None => Ok(BTreeMap::new()),
        Some(Value::Object(values)) => values
            .iter()
            .map(|(name, value)| {
                value
                    .as_str()
                    .map(|value| (name.clone(), value.to_owned()))
                    .ok_or_else(|| entry_error(id, Some(&format!("{field}.{name}")), "a string"))
            })
            .collect(),
        Some(_) => Err(entry_error(id, Some(field), "an object of strings")),
    }
}

fn field_enabled(
    entry: &Map<String, Value>,
    id: &str,
) -> std::result::Result<Option<bool>, McpEntryError> {
    match entry.get("enabled") {
        None => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(entry_error(id, Some("enabled"), "a boolean")),
    }
}

fn decode_stdio(
    id: &str,
    entry: &Map<String, Value>,
) -> std::result::Result<McpServerSettings, McpEntryError> {
    let command = match field_string(entry, id, "command", "a nonempty string")? {
        Some(command) if !command.trim().is_empty() => command,
        _ => return Err(entry_error(id, Some("command"), "a nonempty string")),
    };
    let args = match entry.get("args") {
        None => Vec::new(),
        Some(Value::Array(values)) => values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| entry_error(id, Some(&format!("args[{index}]")), "a string"))
            })
            .collect::<std::result::Result<Vec<_>, _>>()?,
        Some(_) => return Err(entry_error(id, Some("args"), "an array of strings")),
    };
    let env = string_map(entry, id, "env")?;
    let cwd = match entry.get("cwd") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(value.clone()),
        Some(_) => {
            return Err(entry_error(id, Some("cwd"), "a string or null"));
        }
    };
    let enabled = field_enabled(entry, id)?;
    for member in ["url", "headers", "auth", "oauth"] {
        if entry.contains_key(member) {
            return Err(entry_error(
                id,
                Some(member),
                "not a member of the stdio transport",
            ));
        }
    }
    let known = ["type", "command", "args", "env", "cwd", "enabled"];
    let extra = entry
        .iter()
        .filter(|(key, _)| !known.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    Ok(McpServerSettings::Stdio {
        kind: entry.contains_key("type").then_some(Stdio::Stdio),
        command,
        args,
        env,
        cwd,
        enabled,
        extra,
    })
}

fn decode_remote(
    id: &str,
    entry: &Map<String, Value>,
    kind: RemoteTransport,
) -> std::result::Result<McpServerSettings, McpEntryError> {
    let url = match field_string(entry, id, "url", "a nonempty string")? {
        Some(url) if !url.trim().is_empty() => url,
        _ => return Err(entry_error(id, Some("url"), "a nonempty string")),
    };
    let headers = string_map(entry, id, "headers")?;
    let auth = decode_auth(id, entry)?;
    let oauth = decode_oauth(id, entry)?;
    if oauth.is_some() {
        if auth.is_some() {
            return Err(entry_error(
                id,
                Some("oauth"),
                "absent when a bearer token is set",
            ));
        }
        if headers
            .keys()
            .any(|name| name.eq_ignore_ascii_case("authorization"))
        {
            return Err(entry_error(
                id,
                Some("oauth"),
                "absent when an Authorization header is set",
            ));
        }
    }
    let enabled = field_enabled(entry, id)?;
    for member in ["command", "args", "env", "cwd"] {
        if entry.contains_key(member) {
            return Err(entry_error(
                id,
                Some(member),
                "not a member of the remote transport",
            ));
        }
    }
    let known = ["type", "url", "headers", "auth", "oauth", "enabled"];
    let extra = entry
        .iter()
        .filter(|(key, _)| !known.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    Ok(McpServerSettings::Remote {
        kind,
        url,
        headers,
        auth,
        oauth,
        enabled,
        extra,
    })
}

fn decode_auth(
    id: &str,
    entry: &Map<String, Value>,
) -> std::result::Result<Option<BearerAuth>, McpEntryError> {
    let Some(value) = entry.get("auth") else {
        return Ok(None);
    };
    let auth = value
        .as_object()
        .ok_or_else(|| entry_error(id, Some("auth"), "an object"))?;
    match auth.get("type") {
        Some(Value::String(kind)) if kind == "bearer" => {}
        Some(Value::String(_)) => return Err(entry_error(id, Some("auth.type"), "bearer")),
        Some(_) => return Err(entry_error(id, Some("auth.type"), "a string")),
        None => return Err(entry_error(id, Some("auth.type"), "bearer")),
    }
    let token = auth
        .get("token")
        .and_then(Value::as_str)
        .filter(|token| !token.trim().is_empty())
        .ok_or_else(|| entry_error(id, Some("auth.token"), "a nonempty string"))?;
    Ok(Some(BearerAuth::Bearer {
        token: token.to_owned(),
    }))
}

fn decode_oauth(
    id: &str,
    entry: &Map<String, Value>,
) -> std::result::Result<Option<Box<McpOAuthSettings>>, McpEntryError> {
    let Some(value) = entry.get("oauth") else {
        return Ok(None);
    };
    let oauth = value
        .as_object()
        .ok_or_else(|| entry_error(id, Some("oauth"), "an object"))?;
    let client_id = match oauth.get("clientId") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) if !value.trim().is_empty() => Some(value.clone()),
        Some(_) => {
            return Err(entry_error(
                id,
                Some("oauth.clientId"),
                "a nonempty string or null",
            ));
        }
    };
    let callback_port = match oauth.get("callbackPort") {
        None | Some(Value::Null) => None,
        Some(Value::Number(value)) => match value.as_u64() {
            Some(port) if (1..=65535).contains(&port) => Some(port as u16),
            _ => {
                return Err(entry_error(
                    id,
                    Some("oauth.callbackPort"),
                    "an integer from 1 to 65535 or null",
                ));
            }
        },
        Some(_) => {
            return Err(entry_error(
                id,
                Some("oauth.callbackPort"),
                "an integer from 1 to 65535 or null",
            ));
        }
    };
    let auth_server_metadata_url = match oauth.get("authServerMetadataUrl") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) if valid_metadata_url(value) => Some(value.clone()),
        Some(_) => {
            return Err(entry_error(
                id,
                Some("oauth.authServerMetadataUrl"),
                "an https URL or a loopback http URL",
            ));
        }
    };
    let scopes = match oauth.get("scopes") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) if valid_scopes(value) => Some(value.clone()),
        Some(_) => {
            return Err(entry_error(
                id,
                Some("oauth.scopes"),
                "space-separated RFC 6749 scope tokens",
            ));
        }
    };
    let known = [
        "clientId",
        "callbackPort",
        "authServerMetadataUrl",
        "scopes",
    ];
    let extra = oauth
        .iter()
        .filter(|(key, _)| !known.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    Ok(Some(Box::new(McpOAuthSettings {
        client_id,
        callback_port,
        auth_server_metadata_url,
        scopes,
        extra,
    })))
}

fn valid_metadata_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
            && (url.scheme() == "https"
                || (url.scheme() == "http" && matches!(url.host(), Some(host) if loopback(&host))))
    })
}

fn valid_scopes(scopes: &str) -> bool {
    !scopes.trim().is_empty()
        && scopes
            .bytes()
            .all(|byte| byte == b' ' || (byte.is_ascii_graphic() && byte != b'"' && byte != b'\\'))
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

    fn decode(bytes: &[u8]) -> Result<Self> {
        Self::from_file(bytes)
    }

    fn validate(&self) -> Result<()> {
        if self.extra.contains_key("mcpServers") {
            return Err(SettingsError::Invalid("reserved MCP configuration field"));
        }
        for (id, server) in &self.mcp_servers {
            let value = serde_json::to_value(server)
                .map_err(|_| SettingsError::Invalid("cannot encode MCP entry"))?;
            decode_entry(id, &value).map_err(|error| SettingsError::Document(error.to_string()))?;
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
                ..McpSettings::default()
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
        assert!(document.entries().is_empty());
        let error = document.invalid_error("collision").unwrap();
        assert_eq!(error.field.as_deref(), Some("$generation"));
        assert_eq!(error.expected, "not a reserved field");
        assert!(document.validate().is_ok());

        let record = McpSecretEntry {
            extra: BTreeMap::from([("$generation".into(), json!("g"))]),
            ..Default::default()
        };
        assert!(record.validate().is_err());
    }

    #[test]
    fn entries_decode_one_by_one_and_keep_unread_entries_as_written() {
        let source = json!({
            "mcpServers": {
                "stdio": {"command": "server", "args": ["--flag"], "future": {"keep": [1, 2]}},
                "stream": {"type": "streamable-http", "url": "https://stream.test/mcp", "headers": {"X-Key": "value"}},
                "ws": {"type": "ws", "url": "wss://ws.test"},
                "untyped": {"url": "https://untyped.test/mcp"},
                "scalar": "not an object"
            },
            "extension": {"preserve": true}
        });
        let document: McpSettings = serde_json::from_value(source.clone()).unwrap();
        assert_eq!(document.entries().len(), 2);
        assert_eq!(document.invalid_entries().len(), 3);
        assert_eq!(
            document.invalid_entries()["ws"].error.field.as_deref(),
            Some("type")
        );
        assert_eq!(
            document.invalid_entries()["ws"].error.expected,
            "http or sse; ws is not a supported transport"
        );
        assert_eq!(
            document.invalid_entries()["untyped"].error.expected,
            "http or sse; an entry with a url must name its type"
        );
        assert_eq!(document.invalid_entries()["scalar"].error.field, None);
        assert!(document.validate().is_ok());

        let written = serde_json::to_value(&document).unwrap();
        for id in ["stdio", "stream", "ws", "untyped"] {
            assert_eq!(written["mcpServers"][id], source["mcpServers"][id], "{id}");
            assert_eq!(
                serde_json::to_string(&written["mcpServers"][id]).unwrap(),
                serde_json::to_string(&source["mcpServers"][id]).unwrap(),
                "{id}"
            );
        }
        assert_eq!(
            written["mcpServers"]["scalar"],
            source["mcpServers"]["scalar"]
        );
        assert_eq!(written["extension"], source["extension"]);
    }

    #[test]
    fn an_entry_error_names_the_field_and_never_repeats_the_value() {
        let marker = "token-like-value-8f2c";
        let document: McpSettings = serde_json::from_value(json!({
            "mcpServers": {
                "header": {"type": "http", "url": "https://example.test/mcp", "headers": {"Authorization": {"leak": marker}}},
                "arg": {"command": "server", "args": [marker, true]}
            }
        }))
        .unwrap();
        assert_eq!(document.invalid_entries().len(), 2);
        let header = &document.invalid_entries()["header"].error;
        assert_eq!(header.field.as_deref(), Some("headers.Authorization"));
        assert!(header.to_string().contains("must be a string"));
        let arg = &document.invalid_entries()["arg"].error;
        assert_eq!(arg.field.as_deref(), Some("args[1]"));
        assert!(arg.to_string().contains("must be a string"));
        assert!(!header.to_string().contains(marker));
        assert!(!arg.to_string().contains(marker));
    }

    #[test]
    fn a_streamable_http_entry_reads_as_http_and_is_written_back_as_written() {
        let mut document: McpSettings = serde_json::from_value(json!({
            "mcpServers": {
                "stream": {"type": "streamable-http", "url": "https://example.test/mcp", "future": 1}
            }
        }))
        .unwrap();
        let written = serde_json::to_value(&document).unwrap();
        assert_eq!(written["mcpServers"]["stream"]["type"], "streamable-http");
        assert_eq!(written["mcpServers"]["stream"]["future"], 1);
        assert!(document.configure_mut("stream").is_some());
        let written = serde_json::to_value(&document).unwrap();
        assert_eq!(written["mcpServers"]["stream"]["type"], "http");
        assert_eq!(written["mcpServers"]["stream"]["future"], 1);
    }

    #[test]
    fn a_broken_entry_is_removed_or_replaced_without_touching_others() {
        let mut document: McpSettings = serde_json::from_value(json!({
            "mcpServers": {
                "broken": {"type": "ws", "url": "wss://example.test"},
                "kept": {"command": "server", "future": {"keep": true}}
            }
        }))
        .unwrap();
        assert!(document.remove("broken").is_none());
        assert!(document.invalid_entries().is_empty());
        let written = serde_json::to_value(&document).unwrap();
        assert!(written["mcpServers"].get("broken").is_none());
        assert_eq!(
            serde_json::to_string(&written["mcpServers"]["kept"]).unwrap(),
            serde_json::to_string(&json!({"command": "server", "future": {"keep": true}})).unwrap()
        );

        let mut document: McpSettings = serde_json::from_value(json!({
            "mcpServers": {"broken": {"type": "ws", "url": "wss://example.test"}}
        }))
        .unwrap();
        let configuration = decode_entry("broken", &json!({"command": "server"})).unwrap();
        document.insert("broken", configuration);
        assert!(document.invalid_entries().is_empty());
        let written = serde_json::to_value(&document).unwrap();
        assert_eq!(written["mcpServers"]["broken"]["command"], "server");
        assert!(written["mcpServers"]["broken"].get("url").is_none());
    }

    #[test]
    fn only_a_document_level_failure_refuses_the_whole_file() {
        let marker = "secret-marker-4a1f";
        let invalid =
            McpSettings::from_file(format!("{{\"mcpServers\": \"{marker}\"}}").as_bytes())
                .err()
                .unwrap()
                .to_string();
        assert!(invalid.contains("mcpServers must be a JSON object at line 1 column"));
        assert!(!invalid.contains(marker));

        let not_json = McpSettings::from_file(format!("{{{marker}").as_bytes())
            .err()
            .unwrap()
            .to_string();
        assert!(not_json.contains("not valid JSON at line"));
        assert!(!not_json.contains(marker));

        let missing = McpSettings::from_file(b"{}").err().unwrap().to_string();
        assert!(missing.contains("must contain an mcpServers object"));
    }
}
