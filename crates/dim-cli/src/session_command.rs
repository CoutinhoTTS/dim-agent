//! `dimcode session rename`: the manual title of a Session, as the terminal
//! UI's `/rename` and the command line both set it.

use crate::{Result, client::Client, model::session::RenameReply, sanitize::safe};
use serde::Deserialize;
use serde_json::json;

const RENAME_UNSUPPORTED: &str = "This daemon does not support renaming Sessions";

#[derive(Deserialize)]
struct Renamed {
    title: Option<String>,
}

#[derive(Deserialize)]
struct Listed {
    methods: Vec<Declared>,
}

#[derive(Deserialize)]
struct Declared {
    name: String,
}

/// `dimcode session rename`: sets the manual title, or clears it when `title`
/// is `None`, and prints what the server reports.
pub async fn rename(client: &mut Client, session_id: &str, title: Option<&str>) -> Result<()> {
    match set_title(client, session_id, title).await {
        RenameReply::Titled(title) => {
            println!("Renamed to {}", one_line(&title));
            Ok(())
        }
        RenameReply::Untitled => {
            println!("The Session is now untitled.");
            Ok(())
        }
        RenameReply::Unsupported => Err(RENAME_UNSUPPORTED.into()),
        RenameReply::Failed(message) => Err(message.into()),
    }
}

/// Sets or clears the manual title, as both `dimcode session rename` and
/// `/rename` report it.
///
/// `session.rename` is a Session method, so `hello.features` does not list
/// it: the Session's own method list is where a client discovers it
/// ([session boundary](../../../docs/design/contracts/session-boundary.md)).
pub(crate) async fn set_title(
    client: &mut Client,
    session_id: &str,
    title: Option<&str>,
) -> RenameReply {
    match serves_rename(client, session_id).await {
        Ok(false) => return RenameReply::Unsupported,
        Ok(true) => {}
        Err(error) => return RenameReply::Failed(error.to_string()),
    }
    match client
        .typed::<Renamed>(
            "session.rename",
            json!({"sessionId": session_id, "title": title}),
        )
        .await
    {
        Ok(renamed) => match renamed.title {
            Some(title) => RenameReply::Titled(title),
            None => RenameReply::Untitled,
        },
        Err(error) => RenameReply::Failed(error.to_string()),
    }
}

/// Whether the Session lists `session.rename` among the methods it serves.
async fn serves_rename(client: &mut Client, session_id: &str) -> Result<bool> {
    if !client.supports(&["session.methods"]) {
        return Ok(false);
    }
    let listed: Listed = client
        .typed("session.methods", json!({"sessionId": session_id}))
        .await?;
    Ok(listed
        .methods
        .iter()
        .any(|declared| declared.name == "session.rename"))
}

/// Text a daemon supplies as one printable line.
fn one_line(text: &str) -> String {
    safe(text).replace(['\n', '\t'], " ")
}
