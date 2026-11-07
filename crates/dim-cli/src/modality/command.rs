//! `dimcode modality`: the capability table and `set-default` (Modality A.5).

use super::{Command, Modality, model_reference, operation_key, refusal, row_of, table};
use crate::client::{Client, app_error};
use crate::{Result, args::Options};
use serde_json::{Value, json};
use std::io::Write;

pub(crate) async fn run<W: std::io::Write>(
    client: &mut Client,
    options: &Options,
    command: &Command,
    out: &mut W,
) -> Result<i32> {
    match command {
        Command::List => list(client, options, out).await,
        Command::SetDefault {
            operation,
            model,
            clear,
        } => set_default(client, options, operation, model.as_deref(), *clear, out).await,
    }
}

/// `dimcode modality`: the whole snapshot, as a table or unchanged.
async fn list(client: &mut Client, options: &Options, out: &mut impl Write) -> Result<i32> {
    let mut modality = Modality::open(client)?;
    let snapshot = modality.capabilities().await?;
    if options.json {
        writeln!(out, "{}", serde_json::to_string(&snapshot)?)?;
    } else {
        write!(out, "{}", table(&snapshot))?;
    }
    Ok(0)
}

/// `dimcode modality set-default <category>[.<operation>] <provider>/<model>`
/// or `--clear`: `modality.configure` with the revision read immediately
/// before, retried once after a `modality_conflict` because only another
/// client's save can cause one (A.3, A.4).
async fn set_default(
    client: &mut Client,
    options: &Options,
    key: &str,
    reference: Option<&str>,
    clear: bool,
    out: &mut impl Write,
) -> Result<i32> {
    let (category, operation) = operation_key(key);
    if category.is_empty() || operation == Some("") {
        return Err(format!("{key} is not <category>[.<operation>]").into());
    }
    let model = match (reference, clear) {
        (Some(_), true) => return Err("Use --clear without PROVIDER/MODEL".into()),
        (Some(reference), false) => Some(model_reference(reference)?),
        (None, true) => None,
        (None, false) => return Err("Name PROVIDER/MODEL, or use --clear".into()),
    };
    let mut modality = Modality::open(client)?;
    let mut revision = revision_of(&mut modality).await?;
    let mut retried = false;
    loop {
        let mut params = json!({
            "expectedRevision": revision,
            "category": category,
            "model": model.clone().unwrap_or(Value::Null),
        });
        if let Some(operation) = operation {
            params["operation"] = json!(operation);
        }
        let reply = match modality.call("modality.configure", params).await {
            Ok(reply) => reply,
            Err(error) if app_error(&error) == Some("modality_conflict") && !retried => {
                retried = true;
                revision = revision_of(&mut modality).await?;
                continue;
            }
            Err(error) => return Err(refusal(error)),
        };
        if options.json {
            writeln!(out, "{}", serde_json::to_string(&reply)?)?;
            return Ok(0);
        }
        match reference {
            Some(reference) => writeln!(out, "Set {key} to {reference}.")?,
            None => writeln!(out, "Cleared {key}.")?,
        }
        if let Some(row) = row_of(&reply, key) {
            writeln!(out, "{row}")?;
        }
        return Ok(0);
    }
}

/// The revision the next `modality.configure` carries.
async fn revision_of(modality: &mut Modality<'_>) -> Result<String> {
    let snapshot = modality.capabilities().await?;
    snapshot["revision"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "modality.capabilities answered without a revision".into())
}
