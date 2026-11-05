//! Import of a standard `mcpServers` document into one MCP source.
//!
//! The client reads a file and sends its text; this module decodes it with the
//! one entry decoder, previews what each entry would do to the target source,
//! and plans the entries a selection accepts. A preview writes nothing; the
//! plan is checked again against the source the write reads.

use super::config::{
    self, BearerAuth, ENTRY_MEMBERS, McpServerSettings, McpSettings, OAUTH_MEMBERS, RemoteTransport,
};
use super::{McpError, Result};
use crate::protocol::optional_non_null;
use crate::settings::SettingsError;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

/// The largest document a client may send. A reply names only ids and field
/// names taken from this document, plus a bounded field list per entry, so it
/// stays under the transport frame's 16 MiB.
pub(crate) const MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;

/// The largest number of entries one document may carry.
pub(crate) const MAX_ENTRIES: usize = 1_000;

/// The entry fields a project import asks the user to confirm: literal values
/// that the workspace file would record.
const CONFIRM_FIELDS: &[&str] = &["args", "env", "headers", "url"];

/// One entry's resolution in an import request.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ImportResolution {
    pub id: String,
    pub action: ImportAction,
    #[serde(default, deserialize_with = "optional_non_null")]
    pub new_id: Option<String>,
    #[serde(default)]
    pub confirm: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ImportAction {
    Import,
    Skip,
    Replace,
    Rename,
}

/// Decodes one client-supplied document with the existing decoder, refusing
/// one beyond the size caps. Each entry decodes on its own.
pub(crate) fn parse_document(text: &str) -> Result<McpSettings> {
    if text.len() > MAX_DOCUMENT_BYTES {
        return Err(SettingsError::Document(format!(
            "MCP import document exceeds {MAX_DOCUMENT_BYTES} bytes"
        ))
        .into());
    }
    let document = McpSettings::from_file(text.as_bytes())?;
    if document.raw_entries().len() > MAX_ENTRIES {
        return Err(SettingsError::Document(format!(
            "MCP import document exceeds {MAX_ENTRIES} entries"
        ))
        .into());
    }
    Ok(document)
}

/// Checks a request's resolutions against the document and returns them by
/// id. An action's legality against the target source is checked when the
/// source is read, not here.
pub(crate) fn check_resolutions(
    document: &McpSettings,
    resolutions: &[ImportResolution],
) -> std::result::Result<BTreeMap<String, ImportResolution>, String> {
    let mut checked = BTreeMap::new();
    for resolution in resolutions {
        if resolution.id.trim().is_empty() {
            return Err("a resolution id is empty".to_owned());
        }
        if document.get(&resolution.id).is_none()
            && document.invalid_error(&resolution.id).is_none()
        {
            return Err(format!(
                "a resolution names {}, which the document does not contain",
                resolution.id
            ));
        }
        match resolution.action {
            ImportAction::Rename => {
                if resolution
                    .new_id
                    .as_deref()
                    .is_none_or(|id| id.trim().is_empty())
                {
                    return Err("a rename resolution requires a nonempty newId".to_owned());
                }
            }
            _ if resolution.new_id.is_some() => {
                return Err("only a rename resolution takes newId".to_owned());
            }
            _ => {}
        }
        if let Some(field) = resolution
            .confirm
            .iter()
            .find(|field| !CONFIRM_FIELDS.contains(&field.as_str()))
        {
            return Err(format!(
                "confirm names {field}, which is not a field an import confirms"
            ));
        }
        if checked
            .insert(resolution.id.clone(), resolution.clone())
            .is_some()
        {
            return Err(format!("two resolutions name {}", resolution.id));
        }
    }
    Ok(checked)
}

/// What importing the document would do to one source. It is a function of
/// the document, the source snapshot and the target scope: it writes,
/// executes and contacts nothing.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportPreview {
    /// The target source revision the apply's `expectedVersion` takes.
    pub revision: String,
    pub entries: Vec<ImportEntryPreview>,
    /// Top-level members other than `mcpServers`; an import does not carry
    /// them.
    pub unknown_top_level: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportEntryPreview {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<&'static str>,
    pub status: ImportStatus,
    /// Field names and the kind each holds; never a value.
    pub changes: Vec<ImportField>,
    pub unsupported: Vec<ImportUnsupported>,
    /// Project scope only: fields whose literal value needs confirmation.
    pub confirm: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ImportStatus {
    /// The target source already holds this entry and equal content.
    Identical,
    /// The target source would gain this entry.
    Changed,
    /// The target source holds this id with different content.
    Conflict,
    /// The entry does not decode, or a project source refuses it.
    Invalid,
}

#[derive(Debug, Serialize)]
pub(crate) struct ImportField {
    pub field: String,
    pub kind: &'static str,
}

#[derive(Debug, Serialize)]
pub(crate) struct ImportUnsupported {
    pub field: String,
    pub effect: UnsupportedEffect,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum UnsupportedEffect {
    /// Kept in the entry's extension members and not acted on.
    KeptUnused,
    /// Dropped by the decoder.
    Dropped,
}

pub(crate) fn preview(
    document: &McpSettings,
    target: &McpSettings,
    project: bool,
    revision: &str,
) -> ImportPreview {
    ImportPreview {
        revision: revision.to_owned(),
        entries: document
            .raw_entries()
            .keys()
            .map(|id| entry_preview(id, document, target, project))
            .collect(),
        unknown_top_level: document.extra.keys().cloned().collect(),
    }
}

fn entry_preview(
    id: &str,
    document: &McpSettings,
    target: &McpSettings,
    project: bool,
) -> ImportEntryPreview {
    let mut preview = ImportEntryPreview {
        id: id.to_owned(),
        transport: document.get(id).map(transport),
        status: ImportStatus::Invalid,
        changes: Vec::new(),
        unsupported: unsupported(document, id),
        confirm: Vec::new(),
        error: document.invalid_error(id).map(ToString::to_string),
    };
    let Some(entry) = document.get(id) else {
        return preview;
    };
    let import = normalize(entry, project);
    if project && let Some(reason) = refusal(&import) {
        preview.error = Some(reason);
        return preview;
    }
    let existing = target.get(id);
    preview.status = if target.invalid_error(id).is_some() {
        ImportStatus::Conflict
    } else {
        match existing {
            Some(existing) if same_configuration(existing, &import) => ImportStatus::Identical,
            Some(_) => ImportStatus::Conflict,
            None => ImportStatus::Changed,
        }
    };
    if preview.status != ImportStatus::Identical {
        preview.changes = changed_fields(existing, &import);
        if project {
            preview.confirm = confirm_fields(&import);
        }
    }
    preview
}

/// The entries a request's resolutions accept, by destination id. Every
/// check runs against the target snapshot read with the revision, before the
/// caller writes anything.
pub(crate) fn plan(
    document: &McpSettings,
    target: &McpSettings,
    project: bool,
    overlays: &BTreeSet<String>,
    resolutions: &BTreeMap<String, ImportResolution>,
) -> Result<BTreeMap<String, McpServerSettings>> {
    let mut planned = BTreeMap::new();
    for (id, resolution) in resolutions {
        if resolution.action == ImportAction::Skip {
            continue;
        }
        let Some(entry) = document.get(id) else {
            return Err(import_conflict(id, "the entry does not decode"));
        };
        let import = normalize(entry, project);
        if project && let Some(reason) = refusal(&import) {
            return Err(import_conflict(id, &reason));
        }
        let destination = match resolution.action {
            ImportAction::Skip => unreachable!("a skip resolution continues above"),
            ImportAction::Import => {
                if let Some(existing) = target.get(id) {
                    if same_configuration(existing, &import) {
                        continue;
                    }
                    return Err(import_conflict(
                        id,
                        "the target source holds this id with a different configuration; replace or rename it",
                    ));
                }
                if target.invalid_error(id).is_some() {
                    return Err(import_conflict(
                        id,
                        "the target source holds this id as an entry that does not decode; replace or rename it",
                    ));
                }
                id.clone()
            }
            ImportAction::Replace => {
                if target.get(id).is_none() && target.invalid_error(id).is_none() {
                    return Err(import_conflict(
                        id,
                        "the target source does not hold this id",
                    ));
                }
                if project && overlays.contains(id) {
                    return Err(McpError::CredentialsAreUserScoped);
                }
                id.clone()
            }
            ImportAction::Rename => {
                let Some(new_id) = resolution.new_id.as_deref() else {
                    return Err(import_conflict(id, "the rename has no new id"));
                };
                if new_id.trim().is_empty() {
                    return Err(import_conflict(id, "the rename has an empty new id"));
                }
                if target.contains(new_id)
                    || target.invalid_error(new_id).is_some()
                    || planned.contains_key(new_id)
                {
                    return Err(import_conflict(id, "the new id is already taken"));
                }
                new_id.to_owned()
            }
        };
        if project {
            let fields = confirm_fields(&import);
            if fields
                .iter()
                .any(|field| !resolution.confirm.contains(field))
            {
                return Err(McpError::ImportUnconfirmed {
                    id: id.clone(),
                    fields,
                });
            }
        }
        if planned.insert(destination, import).is_some() {
            return Err(import_conflict(id, "two entries would write one id"));
        }
    }
    Ok(planned)
}

fn import_conflict(id: &str, reason: &str) -> McpError {
    McpError::ImportConflict {
        id: id.to_owned(),
        reason: reason.to_owned(),
    }
}

fn transport(entry: &McpServerSettings) -> &'static str {
    match entry {
        McpServerSettings::Stdio { .. } => "stdio",
        McpServerSettings::Remote {
            kind: RemoteTransport::Http,
            ..
        } => "http",
        McpServerSettings::Remote {
            kind: RemoteTransport::Sse,
            ..
        } => "sse",
    }
}

/// An import never enables anything. A user entry is written disabled and a
/// project entry carries no `enabled` field, so its enablement stays the
/// user's record (ADR 003 D3).
fn normalize(entry: &McpServerSettings, project: bool) -> McpServerSettings {
    let mut entry = entry.clone();
    set_enabled(&mut entry, if project { None } else { Some(false) });
    entry
}

fn set_enabled(entry: &mut McpServerSettings, enabled: Option<bool>) {
    match entry {
        McpServerSettings::Stdio { enabled: field, .. }
        | McpServerSettings::Remote { enabled: field, .. } => *field = enabled,
    }
}

/// Whether two entries hold the same configuration. Enablement is the user's
/// state, not the entry's configuration: a source that already holds this
/// configuration is `identical` however it is enabled, and an `import` of it
/// writes nothing.
fn same_configuration(left: &McpServerSettings, right: &McpServerSettings) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    set_enabled(&mut left, None);
    set_enabled(&mut right, None);
    left == right
}

/// Why a project source refuses an entry outright. A reference is not a
/// literal value; a URL with userinfo is a credential wherever it lives.
fn refusal(entry: &McpServerSettings) -> Option<String> {
    let McpServerSettings::Remote { url, auth, .. } = entry else {
        return None;
    };
    if let Some(BearerAuth::Bearer { token }) = auth
        && !config::reference_only(token)
    {
        return Some(
            "a project entry cannot carry a literal auth.token; use a reference".to_owned(),
        );
    }
    if url::Url::parse(url)
        .is_ok_and(|parsed| !parsed.username().is_empty() || parsed.password().is_some())
    {
        return Some("a project entry cannot carry a URL with userinfo".to_owned());
    }
    None
}

fn confirm_fields(entry: &McpServerSettings) -> Vec<String> {
    let mut fields = BTreeSet::new();
    match entry {
        McpServerSettings::Stdio { args, env, .. } => {
            if env.values().any(|value| literal(value)) {
                fields.insert("env");
            }
            if args.iter().any(|value| literal(value)) {
                fields.insert("args");
            }
        }
        McpServerSettings::Remote { url, headers, .. } => {
            if headers.values().any(|value| literal(value)) {
                fields.insert("headers");
            }
            if url_query(url) {
                fields.insert("url");
            }
        }
    }
    fields.into_iter().map(str::to_owned).collect()
}

fn literal(value: &str) -> bool {
    !value.is_empty() && !config::reference_only(value)
}

fn url_query(url: &str) -> bool {
    match url::Url::parse(url) {
        Ok(parsed) => parsed.query().is_some_and(|query| !query.is_empty()),
        // A reference may hide the scheme, so an unparseable value with a
        // query separator asks for confirmation too.
        Err(_) => url.contains('?'),
    }
}

/// Compares the full normalized entry, so an enablement the import would
/// change is reported even though `status` ignores enablement.
fn changed_fields(
    before: Option<&McpServerSettings>,
    after: &McpServerSettings,
) -> Vec<ImportField> {
    let before = before.map(members).unwrap_or_default();
    let after = members(after);
    let mut fields = BTreeMap::new();
    for (name, value) in &after {
        if before.get(name) != Some(value) {
            fields.insert(name.clone(), kind(value));
        }
    }
    for (name, value) in &before {
        if !after.contains_key(name) {
            fields.insert(name.clone(), kind(value));
        }
    }
    fields
        .into_iter()
        .map(|(field, kind)| ImportField { field, kind })
        .collect()
}

fn members(entry: &McpServerSettings) -> Map<String, Value> {
    match serde_json::to_value(entry) {
        Ok(Value::Object(members)) => members,
        _ => Map::new(),
    }
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Members the decoder keeps unused or drops. The names come from the
/// document; no value is carried.
fn unsupported(document: &McpSettings, id: &str) -> Vec<ImportUnsupported> {
    let Some(Value::Object(entry)) = document.raw_entries().get(id) else {
        return Vec::new();
    };
    let mut fields = BTreeMap::new();
    for name in entry
        .keys()
        .filter(|name| !ENTRY_MEMBERS.contains(&name.as_str()))
    {
        fields.insert(name.clone(), UnsupportedEffect::KeptUnused);
    }
    if let Some(Value::Object(oauth)) = entry.get("oauth") {
        for name in oauth
            .keys()
            .filter(|name| !OAUTH_MEMBERS.contains(&name.as_str()))
        {
            fields.insert(format!("oauth.{name}"), UnsupportedEffect::KeptUnused);
        }
    }
    if let Some(Value::Object(auth)) = entry.get("auth") {
        for name in auth
            .keys()
            .filter(|name| !matches!(name.as_str(), "type" | "token"))
        {
            fields.insert(format!("auth.{name}"), UnsupportedEffect::Dropped);
        }
    }
    fields
        .into_iter()
        .map(|(field, effect)| ImportUnsupported { field, effect })
        .collect()
}
