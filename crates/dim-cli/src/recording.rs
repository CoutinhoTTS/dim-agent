//! `recording` and `/recording`: show or switch the daemon's model request
//! record (the `model-log` Settings source, `docs/design/appserver/model-log.md`).
use crate::{Result, args::Options, client::Client};
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;

const SOURCE: &str = "model-log";

#[derive(Deserialize)]
struct Snapshot {
    revision: String,
    document: Document,
}

#[derive(Deserialize)]
struct Document {
    enabled: bool,
}

pub struct State {
    pub enabled: bool,
    revision: String,
}

/// Where the daemon writes. A product daemon runs with the product home as its
/// execution root; one started from `--model-config` runs under `<home>/rust`.
pub fn path(options: &Options) -> PathBuf {
    let root = match options.model_config {
        Some(_) => options.home.join("rust"),
        None => options.home.clone(),
    };
    root.join("model-log").join("recordings.jsonl")
}

pub fn report(state: &State, options: &Options) -> String {
    if state.enabled {
        format!("Model request recording is on: {}", path(options).display())
    } else {
        "Model request recording is off.".to_owned()
    }
}

pub async fn read(client: &mut Client) -> Result<State> {
    let snapshot: Snapshot = client
        .typed("settings.read", json!({"source": SOURCE, "scope": "user"}))
        .await?;
    Ok(State {
        enabled: snapshot.document.enabled,
        revision: snapshot.revision,
    })
}

pub async fn set(client: &mut Client, enabled: bool) -> Result<State> {
    let current = read(client).await?;
    if current.enabled == enabled {
        return Ok(current);
    }
    let snapshot: Snapshot = client
        .typed(
            "settings.update",
            json!({
                "source": SOURCE,
                "scope": "user",
                "expectedRevision": current.revision,
                "document": {"enabled": enabled},
            }),
        )
        .await?;
    Ok(State {
        enabled: snapshot.document.enabled,
        revision: snapshot.revision,
    })
}

/// `dimcode recording [on|off|status]`: prints the state after the switch.
pub async fn run(client: &mut Client, options: &Options, enabled: Option<bool>) -> Result<i32> {
    let state = match enabled {
        Some(enabled) => set(client, enabled).await?,
        None => read(client).await?,
    };
    println!("{}", report(&state, options));
    Ok(0)
}
