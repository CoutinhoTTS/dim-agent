//! `dimcode <area> ...`: a management area through its adapter, with no
//! Session (cli-commands.md "Management surfaces").
//!
//! The panel and these subcommands are two surfaces of the same adapter, so
//! an area's methods, view and refusals are written once.

use crate::Result;
use crate::args::Options;
use crate::client::Client;
use crate::management::{self, skills};
use crate::mcp_command::print_report;
use crate::model::session::{Area, ManagementAction, ManagementRequest};
use crate::sanitize::safe;

/// Runs `request` and prints its area's report.
pub async fn run(
    client: &mut Client,
    options: &Options,
    request: &ManagementRequest,
) -> Result<i32> {
    match request.area {
        Area::Skills => run_skills(client, options, request).await,
    }
}

fn failed(message: String) -> crate::Error {
    message.into()
}

async fn run_skills(
    client: &mut Client,
    options: &Options,
    request: &ManagementRequest,
) -> Result<i32> {
    management::supported(client, request).map_err(failed)?;
    let catalog = skills::apply(client, &request.action, &options.cwd)
        .await
        .map_err(failed)?;
    match &request.action {
        ManagementAction::Toggle { id, enabled } => {
            let name = catalog
                .entries
                .iter()
                .find(|entry| &entry.id == id)
                .map(|entry| entry.name.clone())
                .unwrap_or_else(|| id.clone());
            print_report(
                vec![format!(
                    "{} {}.",
                    if *enabled { "Enabled" } else { "Disabled" },
                    safe(&name)
                )],
                Vec::new(),
            );
        }
        _ => print_report(rows(&catalog), notes(&catalog)),
    }
    Ok(0)
}

/// One Skill: its name, source, state, file and description.
fn rows(catalog: &skills::Catalog) -> Vec<String> {
    catalog
        .entries
        .iter()
        .map(|entry| {
            let mut state = if entry.enabled { "enabled" } else { "disabled" }.to_owned();
            if let Some(by) = &entry.shadowed_by {
                state.push_str(&format!(", shadowed by {}", safe(by)));
            }
            format!(
                "{}\t{}\t{}\t{}\t{}",
                safe(&entry.name),
                safe(&entry.source),
                state,
                safe(&entry.file_path),
                safe(&entry.description),
            )
        })
        .collect()
}

fn notes(catalog: &skills::Catalog) -> Vec<String> {
    let mut notes = if catalog.entries.is_empty() {
        vec!["No Skill is configured.".to_owned()]
    } else {
        vec!["Columns: name, source, state, file, description.".to_owned()]
    };
    notes.extend(
        catalog
            .issues
            .iter()
            .map(|issue| format!("{}: {}", safe(&issue.path), safe(&issue.message))),
    );
    notes
}
