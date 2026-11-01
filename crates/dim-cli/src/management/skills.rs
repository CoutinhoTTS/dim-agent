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
    ActionKind, EntryState, ManagementAction, ManagementEntry, ManagementView,
};
use crate::ui::i18n::t;

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

/// What a Skills action came to.
pub enum Outcome {
    /// The area as its owner reports it after the action.
    Catalog(Catalog),
    /// The action was applied and the read that follows failed.
    ReadFailed(String),
    /// Nothing was applied, and this is why.
    Refused(String),
}

/// The message a kind this area never offers is refused with, or none for a
/// kind the adapter performs.
///
/// `management::supported` refuses these kinds before the adapter runs; the
/// adapter refuses them too, naming the kind, rather than reading the area
/// as if nothing was asked.
fn refusal(action: &ManagementAction) -> Option<&'static str> {
    Some(match action {
        ManagementAction::Add { .. } => "A Skill has no add action.",
        ManagementAction::Remove { .. } => "A Skill has no remove action.",
        ManagementAction::Run { .. } => "A Skill has no run action.",
        ManagementAction::Inspect { .. } => "A Skill has no inspect action.",
        ManagementAction::List
        | ManagementAction::Toggle { .. }
        | ManagementAction::Reload
        | ManagementAction::Save { .. } => return None,
    })
}

/// Performs `action`, then reads the area back as its owner reports it.
///
/// The action is applied once the method behind it returns: the owner
/// declares every Skills method except `skills.catalog` mutating, so a
/// `skills.refresh` that succeeded rescanned the roots even when the read
/// after it fails.
pub async fn apply(client: &mut Client, action: &ManagementAction, cwd: &Path) -> Outcome {
    if let Some(message) = refusal(action) {
        return Outcome::Refused(message.to_owned());
    }
    let params = json!({"cwd": cwd.to_string_lossy()});
    let applied = match action {
        ManagementAction::Reload => {
            if let Err(error) = client.rpc("skills.refresh", params.clone()).await {
                return Outcome::Refused(error.to_string());
            }
            true
        }
        ManagementAction::Toggle { id, enabled } => {
            let mut params = params;
            params["id"] = json!(id);
            params["enabled"] = json!(enabled);
            if let Err(error) = client.rpc("skills.set_enabled", params).await {
                return Outcome::Refused(error.to_string());
            }
            true
        }
        // A read applies nothing, and a Skill has no document, so
        // `management::supported` refuses a save before the adapter runs.
        _ => false,
    };
    match catalog(client, &cwd.to_string_lossy()).await {
        Ok(catalog) => Outcome::Catalog(catalog),
        Err(error) if applied => Outcome::ReadFailed(error),
        Err(error) => Outcome::Refused(error),
    }
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
            entry.source.as_deref().unwrap_or(t("skills")),
            ManagementEntry {
                id: entry.id.clone(),
                label: entry.name.clone(),
                detail: [entry.description.clone(), entry.file_path.clone()].to_vec(),
                enabled: Some(entry.enabled),
                states,
                actions: super::entry_kinds(actions),
                document: None,
            },
        );
    }
    ManagementView {
        revision: None,
        document: None,
        create: None,
        actions: super::area_kinds(actions),
        groups: groups.into_vec(),
        issues: catalog
            .issues
            .iter()
            .map(|issue| format!("{}: {}", issue.path, issue.message))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The kinds this area never offers are refused by name before anything
    /// is read, so a caller that skipped `management::supported` cannot make
    /// the adapter treat one as a read; a kind it does offer is not refused.
    #[test]
    fn a_kind_the_area_never_offers_is_refused_by_name() {
        assert_eq!(
            refusal(&ManagementAction::Add {
                document: json!({})
            }),
            Some("A Skill has no add action.")
        );
        assert_eq!(
            refusal(&ManagementAction::Remove { id: "skill".into() }),
            Some("A Skill has no remove action.")
        );
        assert_eq!(
            refusal(&ManagementAction::Run { id: "skill".into() }),
            Some("A Skill has no run action.")
        );
        assert_eq!(
            refusal(&ManagementAction::Inspect { id: "skill".into() }),
            Some("A Skill has no inspect action.")
        );
        assert_eq!(refusal(&ManagementAction::List), None);
        assert_eq!(refusal(&ManagementAction::Reload), None);
        assert_eq!(
            refusal(&ManagementAction::Toggle {
                id: "skill".into(),
                enabled: true
            }),
            None
        );
        assert_eq!(
            refusal(&ManagementAction::Save {
                id: None,
                expected_revision: "1".into(),
                document: json!({}),
            }),
            None
        );
    }
}
