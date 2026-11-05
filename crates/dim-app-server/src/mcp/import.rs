//! Import of a standard `mcpServers` document into one MCP source.
//!
//! The client reads a file and sends its text; this module decodes it with the
//! one entry decoder, previews what each entry would do to the target source,
//! and plans the entries a selection accepts. A preview writes nothing; the
//! plan is checked again against the source the write reads.
//!
//! A project import writes what a move into project writes (D25): the public
//! skeleton into `.mcp.json` and the withheld members into the user-scope
//! `mcp-secrets` record. The preview therefore compares the configuration the
//! target would serve, which is the declared entry with its record merged.

use super::config::{
    self, ENTRY_MEMBERS, McpSecretEntry, McpServerSettings, McpSettings, OAUTH_MEMBERS,
    RemoteTransport,
};
use super::{McpError, Result};
use crate::protocol::optional_non_null;
use crate::settings::SettingsError;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// The largest document a client may send. The reply names only ids and field
/// names taken from this document: each entry reports at most
/// `MAX_REPORTED_FIELDS` names in `changes` and in `unsupported`, plus how
/// many it left out, so a 1,000-entry document stays under the transport
/// frame's 16 MiB.
pub(crate) const MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;

/// The largest number of entries one document may carry.
pub(crate) const MAX_ENTRIES: usize = 1_000;

/// The most field names one preview list reports; the rest are counted in
/// the entry's `omitted` and the client says the list is partial.
const MAX_REPORTED_FIELDS: usize = 32;

/// One entry's resolution in an import request.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ImportResolution {
    pub id: String,
    pub action: ImportAction,
    #[serde(default, deserialize_with = "optional_non_null")]
    pub new_id: Option<String>,
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
    /// Field names the cap left out of `changes` and `unsupported`.
    pub omitted: usize,
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
    /// The entry does not decode.
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

/// The overlays are the target workspace's `mcp-secrets` records by id, empty
/// for a user source.
pub(crate) fn preview(
    document: &McpSettings,
    target: &McpSettings,
    project: bool,
    overlays: &BTreeMap<String, McpSecretEntry>,
    revision: &str,
) -> ImportPreview {
    ImportPreview {
        revision: revision.to_owned(),
        entries: document
            .raw_entries()
            .keys()
            .map(|id| entry_preview(id, document, target, project, overlays))
            .collect(),
        unknown_top_level: document.extra.keys().cloned().collect(),
    }
}

fn entry_preview(
    id: &str,
    document: &McpSettings,
    target: &McpSettings,
    project: bool,
    overlays: &BTreeMap<String, McpSecretEntry>,
) -> ImportEntryPreview {
    let (unsupported, omitted) = capped(unsupported(document, id));
    let mut preview = ImportEntryPreview {
        id: id.to_owned(),
        transport: document.get(id).map(transport),
        status: ImportStatus::Invalid,
        changes: Vec::new(),
        unsupported,
        omitted,
        error: document.invalid_error(id).map(ToString::to_string),
    };
    let Some(entry) = document.get(id) else {
        return preview;
    };
    let import = normalize(entry, project);
    let existing = target.get(id);
    let existing = match existing {
        Some(existing) => match resolved(existing, project, overlays.get(id)) {
            Ok(existing) => Some(existing),
            // The stored record does not merge into the declared entry, so
            // the target's configuration cannot be read; the entry it holds
            // is not the one the document carries.
            Err(error) => {
                preview.status = ImportStatus::Conflict;
                preview.error = Some(error.to_string());
                return preview;
            }
        },
        None => None,
    };
    preview.status = if target.invalid_error(id).is_some() {
        ImportStatus::Conflict
    } else {
        match &existing {
            Some(existing) if same_configuration(existing, &import) => ImportStatus::Identical,
            Some(_) => ImportStatus::Conflict,
            None => ImportStatus::Changed,
        }
    };
    // Enablement is not part of `status`. A user entry keeps its own field, so
    // an `identical` entry can still name `enabled`: a replace would write
    // `enabled: false` where the entry is enabled, while an import of it
    // writes nothing. A project entry's member is not compared at all, since
    // an import writes none.
    let import = comparable(&import, project);
    let existing = existing.as_ref().map(|entry| comparable(entry, project));
    let (changes, omitted_changes) = capped(changed_fields(existing.as_ref(), &import));
    preview.changes = changes;
    preview.omitted += omitted_changes;
    preview
}

/// The configuration a target entry would serve: a project entry with its
/// withheld record merged into it.
fn resolved(
    entry: &McpServerSettings,
    project: bool,
    overlay: Option<&McpSecretEntry>,
) -> Result<McpServerSettings> {
    let mut entry = entry.clone();
    if project && let Some(overlay) = overlay {
        config::merge_secret(&mut entry, overlay)?;
    }
    Ok(entry)
}

/// The entry as the comparison sees it. A project entry's own `enabled` member
/// is neither what it runs with nor what an import writes, so both sides drop
/// it; a user entry keeps its member.
fn comparable(entry: &McpServerSettings, project: bool) -> McpServerSettings {
    let mut entry = entry.clone();
    if project {
        set_enabled(&mut entry, None);
    }
    entry
}

/// The reported field names and the count the cap left out. They come from a
/// sorted map, so the reported names are the first in order.
fn capped<T>(mut fields: Vec<T>) -> (Vec<T>, usize) {
    let omitted = fields.len().saturating_sub(MAX_REPORTED_FIELDS);
    fields.truncate(MAX_REPORTED_FIELDS);
    (fields, omitted)
}

/// The entries a request's resolutions accept, by destination id. Every
/// check runs against the target snapshot read with the revision, before the
/// caller writes anything.
pub(crate) fn plan(
    document: &McpSettings,
    target: &McpSettings,
    project: bool,
    overlays: &BTreeMap<String, McpSecretEntry>,
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
        let destination = match resolution.action {
            ImportAction::Skip => unreachable!("a skip resolution continues above"),
            ImportAction::Import => {
                if let Some(existing) = target.get(id) {
                    // The destination's configuration is what the entry it
                    // declares serves, with its withheld record merged; a
                    // record that does not merge is not that configuration.
                    let same = resolved(existing, project, overlays.get(id))
                        .is_ok_and(|existing| same_configuration(&existing, &import));
                    if same {
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
