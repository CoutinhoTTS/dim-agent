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
use crate::management::{self, editor, hooks, scheduler, skills};
use crate::mcp_command::print_report;
use crate::model::session::{
    ActionKind, Area, ManagementAction, ManagementReport, ManagementRequest,
};
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
        Area::Scheduled => run_scheduled(client, request).await,
    }
}

/// `<area> edit [ID]`: the area's document in the external editor, or one
/// entry's for an area whose documents are per entry.
pub async fn edit(
    client: &mut Client,
    options: &Options,
    area: Area,
    id: Option<String>,
) -> Result<i32> {
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
    let (revision, document) = read(client, options, area, id.as_deref()).await?;
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
    let outcome = write(
        client,
        options,
        &Draft {
            area,
            id,
            text,
            revision,
            stored: document,
            kept,
        },
    )
    .await;
    if outcome.is_ok() {
        draft.remove();
    }
    outcome
}

/// `<area> set [ID] PATH`: the same write without an editor, from a file or
/// from stdin.
pub async fn set(
    client: &mut Client,
    options: &Options,
    area: Area,
    id: Option<String>,
    path: &Path,
) -> Result<i32> {
    // Refuse a write the daemon cannot serve before the input can hold the
    // call: the same check `edit` makes before it opens the editor.
    management::supports_kind(client, area, ActionKind::Edit).map_err(failed)?;
    // The input first and the revision second, so the revision is read
    // immediately before the write however long the input takes.
    let text = read_text(path)?;
    let (revision, stored) = read(client, options, area, id.as_deref()).await?;
    if path == Path::new("-") {
        let draft = editor::Draft::open(&options.home, area, &text).map_err(failed)?;
        let kept = draft.path().display().to_string();
        let outcome = write(
            client,
            options,
            &Draft {
                area,
                id,
                text,
                revision,
                stored,
                kept,
            },
        )
        .await;
        if outcome.is_ok() {
            draft.remove();
        }
        return outcome;
    }
    write(
        client,
        options,
        &Draft {
            area,
            id,
            text,
            revision,
            stored,
            kept: path.display().to_string(),
        },
    )
    .await
}

/// `<area> add PATH`: the non-interactive twin of the add kind, from a file
/// or from stdin.
pub async fn add(client: &mut Client, options: &Options, area: Area, path: &Path) -> Result<i32> {
    // Refuse a create the daemon cannot serve before the input can hold the
    // call, as `set` does.
    management::supports_kind(client, area, ActionKind::Add).map_err(failed)?;
    let text = read_text(path)?;
    if path == Path::new("-") {
        let draft = editor::Draft::open(&options.home, area, &text).map_err(failed)?;
        let kept = draft.path().display().to_string();
        let outcome = create(client, options, area, &text, &kept).await;
        if outcome.is_ok() {
            draft.remove();
        }
        return outcome;
    }
    let kept = path.display().to_string();
    create(client, options, area, &text, &kept).await
}

/// The document a command reads, from a file or from stdin for `-`.
fn read_text(path: &Path) -> Result<String> {
    if path == Path::new("-") {
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text)?;
        return Ok(text);
    }
    std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()).into())
}

/// The document and the revision to write it at, read immediately before.
async fn read(
    client: &mut Client,
    options: &Options,
    area: Area,
    id: Option<&str>,
) -> Result<(String, Value)> {
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
        Area::Scheduled => {
            let Some(id) = id else {
                return Err("scheduled edit and set take a job id.".to_owned().into());
            };
            let catalog = scheduler::catalog(client).await.map_err(failed)?;
            let job = catalog
                .jobs
                .iter()
                .find(|job| job.id == id)
                .ok_or_else(|| failed(format!("No scheduled job has that id: {id}")))?;
            Ok((job.revision.to_string(), job.document()))
        }
    }
}

/// One document a command writes: where it goes, what it says, and where
/// the user's text stays when the owner refuses it.
struct Draft {
    area: Area,
    id: Option<String>,
    text: String,
    /// The revision the document was read at.
    revision: String,
    /// The document as the owner reported it, which `text` must differ from.
    stored: Value,
    /// The file that keeps the user's text when nothing is written.
    kept: String,
}

/// Writes a draft: nothing when it says what is stored, and a refusal names
/// `kept`, where the user's text stays.
async fn write(client: &mut Client, options: &Options, draft: &Draft) -> Result<i32> {
    let document: Value = serde_json::from_str(&draft.text)
        .map_err(|error| format!("{error} Your text is kept in {}.", draft.kept))?;
    if document == draft.stored {
        print_report(vec!["No change.".to_owned()], Vec::new());
        return Ok(0);
    }
    let request = ManagementRequest {
        area: draft.area,
        action: ManagementAction::Save {
            id: draft.id.clone(),
            expected_revision: draft.revision.clone(),
            document,
        },
    };
    if let Err(error) = management::apply(client, &request, &options.cwd).await {
        return Err(format!("{error} Your text is kept in {}.", draft.kept).into());
    }
    print_report(vec![saved(draft.area)], Vec::new());
    Ok(0)
}

/// What a create writes, with the message the owner's answer gives.
async fn create(
    client: &mut Client,
    options: &Options,
    area: Area,
    text: &str,
    kept: &str,
) -> Result<i32> {
    let document: Value = serde_json::from_str(text)
        .map_err(|error| format!("{error} Your text is kept in {kept}."))?;
    let request = ManagementRequest {
        area,
        action: ManagementAction::Add { document },
    };
    match management::apply(client, &request, &options.cwd).await {
        Ok(ManagementReport::View { note, .. }) => {
            // The note names the entry the owner saved: made safe, as every
            // line the Scheduler prints is.
            let note = note.iter().map(|note| safe(note)).collect();
            print_report(note, Vec::new());
            Ok(0)
        }
        Ok(ManagementReport::Detail(detail)) => {
            let lines = detail.lines.iter().map(|line| safe(line)).collect();
            print_report(lines, Vec::new());
            Ok(0)
        }
        Err(error) => Err(format!("{error} Your text is kept in {kept}.").into()),
    }
}

/// The notice a successful write leaves.
fn saved(area: Area) -> String {
    match area {
        Area::Scheduled => "Saved the job.".to_owned(),
        Area::Skills | Area::Hooks => format!("Saved the {} document.", area.name()),
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

/// `scheduled ...`: the jobs the Scheduler holds, printed as its owner
/// reports them.
async fn run_scheduled(client: &mut Client, request: &ManagementRequest) -> Result<i32> {
    management::supported(client, request).map_err(failed)?;
    match scheduler::apply(client, &request.action).await {
        Ok(scheduler::Answer::Detail(detail)) => {
            // The owner names jobs and states reasons: what it wrote is
            // made safe before it reaches the terminal.
            let lines = detail.lines.iter().map(|line| safe(line)).collect();
            print_report(lines, Vec::new())
        }
        Ok(scheduler::Answer::Jobs { catalog, note }) => match (&request.action, note) {
            (_, Some(note)) => print_report(vec![safe(&note)], Vec::new()),
            (ManagementAction::Toggle { id, enabled }, None) => {
                let name = catalog
                    .jobs
                    .iter()
                    .find(|job| &job.id == id)
                    .map(|job| job.name.clone())
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
            (ManagementAction::Remove { id }, None) => {
                print_report(vec![format!("Removed {}.", safe(id))], Vec::new());
            }
            _ => print_report(job_rows(&catalog), job_notes(&catalog)),
        },
        Err(error) => return Err(failed(error.to_string())),
    }
    Ok(0)
}

/// One job: its id, name, schedule, state, next window and last outcome.
fn job_rows(catalog: &scheduler::Catalog) -> Vec<String> {
    catalog
        .jobs
        .iter()
        .map(|job| {
            let next = job.next_window.as_deref().unwrap_or("—");
            let last = job.last_run.as_ref().map_or_else(
                || "—".to_owned(),
                |run| format!("{} at {}", run.outcome(), run.instant()),
            );
            format!(
                "{}\t{}\t{}\t{}\t{}\t{}",
                safe(&job.id),
                safe(&job.name),
                safe(&scheduler::schedule_text(&job.schedule)),
                if job.enabled { "enabled" } else { "disabled" },
                safe(next),
                safe(&last),
            )
        })
        .collect()
}

fn job_notes(catalog: &scheduler::Catalog) -> Vec<String> {
    let mut notes = if catalog.jobs.is_empty() {
        vec!["No job is scheduled.".to_owned()]
    } else {
        vec!["Columns: id, name, schedule, state, next run, last run.".to_owned()]
    };
    for job in &catalog.jobs {
        if let Some(error) = &job.schedule_error {
            notes.push(format!("{}: {}", safe(&job.name), safe(error)));
        }
    }
    notes
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
