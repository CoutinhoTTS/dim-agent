//! One line's adopted configuration, read from its own event history.
use crate::{
    Client, Result,
    wire::{Configuration, Event, EventContent, HistoryWindow},
};
use serde_json::json;

const PAGE: u64 = 256;

/// The newest configuration `branch` adopted at or below `through`, with the
/// sequence it was adopted at. A display transcript omits `context/configured`
/// and a bounded window can begin after the adoption, so this walks the
/// branch's own facts back to the newest one.
pub async fn adopted(
    client: &mut Client,
    session_id: &str,
    branch: &str,
    through: u64,
) -> Result<Option<(u64, Configuration)>> {
    let mut before = through.saturating_add(1);
    loop {
        let page: HistoryWindow = client
            .typed(
                "session.history",
                json!({
                    "sessionId": session_id,
                    "branchId": branch,
                    "through": through,
                    "before": before,
                    "limit": PAGE,
                }),
            )
            .await?;
        for event in page.events.iter().rev() {
            let event: Event = serde_json::from_value(event.clone())
                .map_err(|_| "Invalid session.history event")?;
            if let EventContent::Configuration(configuration) = event.content {
                return Ok(Some((event.sequence, configuration)));
            }
        }
        match page.next(Some(before), Some(through))? {
            Some(next) => before = next,
            None => return Ok(None),
        }
    }
}
