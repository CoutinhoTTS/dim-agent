//! The Skills area: the catalog and each entry's enablement
//! (`skills.catalog`, `skills.set_enabled`, `skills.refresh`).
//!
//! Skills reports the root an entry was found in as its `source`; the panel
//! groups by it and never reads the id, which names an absolute project path.

use std::path::Path;

use serde::Deserialize;
use serde_json::json;

use super::Groups;
use crate::client::Client;
use crate::model::session::{
    ActionKind, Area, EntryState, ManagementAction, ManagementEntry, ManagementView,
};

/// One entry of `skills.catalog`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    /// `user`, `project` or `plugin:<plugin id>`. A daemon older than this
    /// fact reports none, and the client never derives one.
    pub source: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub file_path: String,
    pub enabled: bool,
    pub shadowed_by: Option<String>,
}

/// A Skill the catalog could not use.
#[derive(Debug, Deserialize)]
pub struct Issue {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub entries: Vec<Entry>,
    pub issues: Vec<Issue>,
}

/// Performs `action`, then reads the area back as its owner reports it.
pub async fn apply(
    client: &mut Client,
    action: &ManagementAction,
    cwd: &Path,
) -> Result<Catalog, String> {
    let params = json!({"cwd": cwd.to_string_lossy()});
    match action {
        ManagementAction::List => {}
        ManagementAction::Reload => {
            client
                .rpc("skills.refresh", params.clone())
                .await
                .map_err(|error| error.to_string())?;
        }
        ManagementAction::Toggle { id, enabled } => {
            let mut params = params;
            params["id"] = json!(id);
            params["enabled"] = json!(enabled);
            client
                .rpc("skills.set_enabled", params)
                .await
                .map_err(|error| error.to_string())?;
        }
        ManagementAction::Save { .. } => {
            return Err("A Skill has no document to edit.".into());
        }
    }
    catalog(client, &cwd.to_string_lossy()).await
}

/// The catalog as its owner reports it.
pub async fn catalog(client: &mut Client, cwd: &str) -> Result<Catalog, String> {
    client
        .typed("skills.catalog", json!({"cwd": cwd}))
        .await
        .map_err(|error| error.to_string())
}

/// One source's entries under its heading, in the order the owner reports
/// them; `actions` are the kinds the daemon advertises for this area. An
/// entry whose owner reports no source goes under the area's own heading.
pub(super) fn view(catalog: &Catalog, actions: &[ActionKind]) -> ManagementView {
    let mut groups = Groups::default();
    for entry in &catalog.entries {
        let mut states = Vec::new();
        if let Some(by) = &entry.shadowed_by {
            states.push(EntryState::Shadowed { by: by.clone() });
        }
        groups.push(
            entry.source.as_deref().unwrap_or(Area::Skills.name()),
            ManagementEntry {
                id: entry.id.clone(),
                label: entry.name.clone(),
                detail: [entry.description.clone(), entry.file_path.clone()].to_vec(),
                enabled: entry.enabled,
                states,
                actions: super::entry_kinds(actions),
            },
        );
    }
    ManagementView {
        revision: None,
        document: None,
        actions: super::area_kinds(actions),
        groups: groups.into_vec(),
        issues: catalog
            .issues
            .iter()
            .map(|issue| format!("{}: {}", issue.path, issue.message))
            .collect(),
    }
}
