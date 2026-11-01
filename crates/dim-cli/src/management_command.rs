//! `dimcode <area> ...`: a management area through its adapter, with no
//! Session (cli-commands.md "Management surfaces").
//!
//! The panel and these subcommands are two surfaces of the same adapter, so
//! an area's methods, view and refusals are written once. `hooks edit` and
//! `hooks set` are the edit kind at this surface: both write the document
//! with a revision read just before, and keep the user's text when the owner
//! refuses it.

use std::io::{IsTerminal, Read};
use std::path::Path;

use serde_json::Value;

use crate::Result;
use crate::args::Options;
use crate::client::Client;
use crate::management::{self, editor, hooks, skills};
use crate::mcp_command::print_report;
use crate::model::session::{ActionKind, Area, ManagementAction, ManagementRequest};
use crate::sanitize::safe;

/// Runs `request` and prints its area's report.
pub async fn run(
    client: &mut Client,
    options: &Options,
    request: &ManagementRequest,
) -> Result<i32> {
    match request.area {
        Area::Skills => run_skills(client, options, request).await,
        Area::Hooks => run_hooks(client, options, request).await,
    }
}

/// `<area> edit`: the area's document in the external editor.
pub async fn edit(client: &mut Client, options: &Options, area: Area) -> Result<i32> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(format!(
            "{} edit needs a terminal; {} set PATH writes a file, or - its input",
            area.name(),
            area.name()
        )
        .into());
    }
    // Refuse a write the daemon cannot serve before the user edits.
    management::supports_kind(client, area, ActionKind::Edit).map_err(failed)?;
    let (revision, document) = read(client, options, area).await?;
    let text = serde_json::to_string_pretty(&document)
        .map(|text| format!("{text}\n"))
        .map_err(|error| error.to_string())?;
    let draft = editor::Draft::open(&options.home, area, &text).map_err(failed)?;
    let kept = draft.path().display().to_string();
    let mut command = editor::command(draft.path());
    let program = command.get_program().to_string_lossy().into_owned();
    let status = match command.status() {
        Ok(status) => status,
        Err(error) => {
            // The editor never took the text: the draft is the stored document again.
            draft.remove();
            return Err(format!("{program}: {error}").into());
        }
    };
    if !status.success() {
        // The text the editor leaves is not the user's decision to write.
        return Err(format!(
            "{} Nothing was written. Your text is kept in {kept}.",
            editor::exited(status)
        )
        .into());
    }
    let text = draft
        .text()
        .map_err(|error| format!("{error} Your text is kept in {kept}."))?;
    let outcome = write(client, options, area, &text, &revision, &document, &kept).await;
    if outcome.is_ok() {
        draft.remove();
    }
    outcome
}

/// `<area> set PATH`: the same write without an editor, from a file or from
/// stdin.
pub async fn set(client: &mut Client, options: &Options, area: Area, path: &Path) -> Result<i32> {
    // Refuse a write the daemon cannot serve before the input can hold the
    // call: the same check `edit` makes before it opens the editor.
    management::supports_kind(client, area, ActionKind::Edit).map_err(failed)?;
    // The input first and the revision second, so the revision is read
    // immediately before the write however long the input takes.
    let text = if path == Path::new("-") {
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text)?;
        text
    } else {
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?
    };
    let (revision, document) = read(client, options, area).await?;
    if path == Path::new("-") {
        let draft = editor::Draft::open(&options.home, area, &text).map_err(failed)?;
        let kept = draft.path().display().to_string();
        let outcome = write(client, options, area, &text, &revision, &document, &kept).await;
        if outcome.is_ok() {
            draft.remove();
        }
        return outcome;
    }
    let kept = path.display().to_string();
    write(client, options, area, &text, &revision, &document, &kept).await
}

/// The document and the revision to write it at, read immediately before.
async fn read(client: &mut Client, options: &Options, area: Area) -> Result<(String, Value)> {
    management::supported(
        client,
        &ManagementRequest {
            area,
            action: ManagementAction::List,
        },
    )
    .map_err(failed)?;
    let cwd = options.cwd.to_string_lossy();
    match area {
        Area::Skills => Err("A Skill has no document to edit.".to_owned().into()),
        Area::Hooks => {
            let catalog = hooks::catalog(client, &cwd).await.map_err(failed)?;
            let document = catalog
                .user_document
                .ok_or_else(|| failed("The daemon reports no hook document to edit.".to_owned()))?;
            Ok((catalog.user_revision, document))
        }
    }
}

/// Writes `text` as the area's document with `revision`: nothing when it
/// says what is stored, and a refusal names `kept`, where the user's text
/// stays.
async fn write(
    client: &mut Client,
    options: &Options,
    area: Area,
    text: &str,
    revision: &str,
    stored: &Value,
    kept: &str,
) -> Result<i32> {
    let document: Value = serde_json::from_str(text)
        .map_err(|error| format!("{error} Your text is kept in {kept}."))?;
    if &document == stored {
        print_report(vec!["No change.".to_owned()], Vec::new());
        return Ok(0);
    }
    let request = ManagementRequest {
        area,
        action: ManagementAction::Save {
            expected_revision: revision.to_owned(),
            document,
        },
    };
    if let Err(error) = management::apply(client, &request, &options.cwd).await {
        return Err(format!("{error} Your text is kept in {kept}.").into());
    }
    print_report(
        vec![format!("Saved the {} document.", area.name())],
        Vec::new(),
    );
    Ok(0)
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

async fn run_hooks(
    client: &mut Client,
    options: &Options,
    request: &ManagementRequest,
) -> Result<i32> {
    management::supported(client, request).map_err(failed)?;
    let catalog = hooks::apply(client, &request.action, &options.cwd)
        .await
        .map_err(|error| error.to_string())?;
    match &request.action {
        ManagementAction::Toggle { enabled, .. } => {
            let cwd = options.cwd.to_string_lossy();
            let note = if *enabled {
                "It runs this workspace's .dimcode/hooks.json, including later edits to it, in active and new sessions."
            } else {
                "The file is kept; it runs again after hooks enable."
            };
            print_report(
                vec![format!(
                    "{} project hooks for {}.",
                    if *enabled { "Enabled" } else { "Disabled" },
                    safe(&cwd)
                )],
                vec![note.to_owned()],
            );
        }
        _ => print_report(hook_rows(&catalog), hook_notes(&catalog)),
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
                safe(entry.source.as_deref().unwrap_or("-")),
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

/// One handler: its event, matcher, source, enablement, state and settings.
fn hook_rows(catalog: &hooks::Catalog) -> Vec<String> {
    catalog
        .hooks
        .iter()
        .map(|hook| {
            let enabled = catalog
                .sources
                .iter()
                .find(|source| source.source == hook.source)
                .is_some_and(|source| source.enabled);
            format!(
                "{}\t{}\t{}\t{}\t{}\t{}s\t{}\t{}\t{}",
                safe(&hook.event),
                safe(hook.matcher.as_deref().unwrap_or("*")),
                safe(&hook.source),
                if enabled { "enabled" } else { "disabled" },
                safe(&hook.state),
                hook.timeout,
                safe(&hook.on_failure),
                if hook.r#async { "async" } else { "sync" },
                safe(hook.command.as_deref().unwrap_or("—")),
            )
        })
        .collect()
}

fn hook_notes(catalog: &hooks::Catalog) -> Vec<String> {
    // The columns describe rows: with no handler, the report says so instead.
    let mut notes = if catalog.hooks.is_empty() {
        vec!["No hook is configured.".to_owned()]
    } else {
        vec![
            "Columns: event, matcher, source, enablement, state, timeout, on failure, async, command.\nA handler whose state is ok runs at its event once its source is enabled; unsupported and invalid handlers never run.\nProject hooks stay disabled until you run hooks enable.".to_owned(),
        ]
    };
    for source in &catalog.sources {
        let name = match source.path.as_deref() {
            Some(path) => format!("{} hooks ({})", safe(&source.source), safe(path)),
            None => format!("{} hooks", safe(&source.source)),
        };
        if source.state != "ok" {
            notes.push(format!(
                "{name}: {}{}",
                safe(&source.state),
                source
                    .message
                    .as_deref()
                    .map(|message| format!(": {}", safe(message)))
                    .unwrap_or_default()
            ));
        } else if !source.enabled {
            notes.push(format!("{name}: disabled"));
        }
    }
    if catalog.disable_all_hooks {
        notes.push("disableAllHooks in the user document turns every hook off.".to_owned());
    }
    notes
}
