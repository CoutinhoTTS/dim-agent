//! `dimcode prompt ...`: the operator's overrides of the built-in policy texts
//! through the App Server's `settings` methods ([prompt
//! overrides](../docs/design/appserver/prompt-overrides.md)).
//!
//! The daemon owns the document and each capability reads its own field. This
//! client presents one field as plain text: an empty draft writes nothing, and
//! `reset` removes the override. Every write uses the revision it read.
use crate::{Result, args::Options, client::Client, management::editor};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::io::{IsTerminal, Read};

const SOURCE: &str = "prompts";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Base,
    Summary,
    Skills,
}

impl Field {
    fn parse(name: &str) -> Result<Self> {
        match name {
            "base" => Ok(Self::Base),
            "summary" => Ok(Self::Summary),
            "skills" => Ok(Self::Skills),
            _ => Err(format!("Use base, summary or skills, not {name}").into()),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Summary => "summary",
            Self::Skills => "skills",
        }
    }
}

#[derive(Clone, Debug)]
pub enum Command {
    List,
    Show { field: Field },
    Edit { field: Field },
    Set { field: Field, path: String },
    Reset { field: Field },
}

#[derive(Deserialize)]
struct Snapshot {
    revision: String,
    document: Value,
}

pub async fn run(client: &mut Client, options: &Options, command: Command) -> Result<i32> {
    if !client.supports(&["settings.read", "settings.update"]) {
        return Err("This daemon does not provide prompt overrides".into());
    }
    match command {
        Command::List => list(client).await,
        Command::Show { field } => show(client, field).await,
        Command::Edit { field } => edit(client, options, field).await,
        Command::Set { field, path } => set(client, field, &path).await,
        Command::Reset { field } => reset(client, field).await,
    }
}

async fn read(client: &mut Client) -> Result<Snapshot> {
    client
        .typed("settings.read", json!({"source": SOURCE, "scope": "user"}))
        .await
}

fn text(snapshot: &Snapshot, field: Field) -> Option<&str> {
    snapshot.document.get(field.name()).and_then(Value::as_str)
}

fn document(snapshot: &Snapshot) -> Map<String, Value> {
    snapshot.document.as_object().cloned().unwrap_or_default()
}

async fn write(
    client: &mut Client,
    snapshot: &Snapshot,
    field: Field,
    value: Option<&str>,
) -> Result<Snapshot> {
    let mut document = document(snapshot);
    match value {
        Some(value) => {
            document.insert(field.name().into(), Value::String(value.to_owned()));
        }
        None => {
            document.remove(field.name());
        }
    }
    client
        .typed(
            "settings.update",
            json!({
                "source": SOURCE,
                "scope": "user",
                "expectedRevision": snapshot.revision,
                "document": document,
            }),
        )
        .await
}

async fn list(client: &mut Client) -> Result<i32> {
    let snapshot = read(client).await?;
    println!("prompts (user scope, revision {})", snapshot.revision);
    for field in [Field::Base, Field::Summary, Field::Skills] {
        match text(&snapshot, field) {
            Some(text) => println!("  {:<8} custom ({} B)", field.name(), text.len()),
            None => println!("  {:<8} built-in", field.name()),
        }
    }
    Ok(0)
}

async fn show(client: &mut Client, field: Field) -> Result<i32> {
    let snapshot = read(client).await?;
    match text(&snapshot, field) {
        Some(text) => {
            println!("{text}");
            Ok(0)
        }
        None => Err(format!("{}: built-in (no override)", field.name()).into()),
    }
}

async fn edit(client: &mut Client, options: &Options, field: Field) -> Result<i32> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(format!(
            "prompt edit needs a terminal; prompt set {} PATH writes a file, or - its input",
            field.name()
        )
        .into());
    }
    let snapshot = read(client).await?;
    let current = text(&snapshot, field).unwrap_or_default().to_owned();
    let draft = editor::Draft::open_text(&options.home, field.name(), "md", &current)
        .map_err(|error| error.to_string())?;
    let kept = draft.path().display().to_string();
    let mut command = editor::command(draft.path());
    let program = command.get_program().to_string_lossy().into_owned();
    let status = match command.status() {
        Ok(status) => status,
        Err(error) => {
            draft.remove();
            return Err(format!("{program}: {error}").into());
        }
    };
    if !status.success() {
        return Err(format!(
            "{} Nothing was written. Your text is kept in {kept}.",
            editor::exited(status)
        )
        .into());
    }
    let text = draft
        .text()
        .map_err(|error| format!("{error} Your text is kept in {kept}."))?;
    if text.trim().is_empty() {
        draft.remove();
        return Err(format!(
            "An empty draft writes nothing; use `dimcode prompt reset {}` to remove the override.",
            field.name()
        )
        .into());
    }
    if text == current {
        draft.remove();
        println!("{}: unchanged", field.name());
        return Ok(0);
    }
    match write(client, &snapshot, field, Some(&text)).await {
        Ok(_) => {
            draft.remove();
            println!(
                "{}: saved; applies to Sessions started after this",
                field.name()
            );
            Ok(0)
        }
        Err(error) => Err(format!("{error} Your text is kept in {kept}.").into()),
    }
}

async fn set(client: &mut Client, field: Field, path: &str) -> Result<i32> {
    let mut body = String::new();
    if path == "-" {
        std::io::stdin().read_to_string(&mut body)?;
    } else {
        body = std::fs::read_to_string(path)?;
    }
    if body.trim().is_empty() {
        return Err(format!(
            "An empty text writes nothing; use `dimcode prompt reset {}` to remove the override.",
            field.name()
        )
        .into());
    }
    let snapshot = read(client).await?;
    if text(&snapshot, field) == Some(body.as_str()) {
        println!("{}: unchanged", field.name());
        return Ok(0);
    }
    write(client, &snapshot, field, Some(&body)).await?;
    println!(
        "{}: saved; applies to Sessions started after this",
        field.name()
    );
    Ok(0)
}

async fn reset(client: &mut Client, field: Field) -> Result<i32> {
    let snapshot = read(client).await?;
    if text(&snapshot, field).is_none() {
        println!("{}: built-in", field.name());
        return Ok(0);
    }
    write(client, &snapshot, field, None).await?;
    println!(
        "{}: reset to the built-in text; applies to Sessions started after this",
        field.name()
    );
    Ok(0)
}

pub fn field(name: &str) -> Result<Field> {
    Field::parse(name)
}
