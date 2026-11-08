use crate::wire::{Event, EventContent, HistoryWindow};
use serde_json::{Value, json};
use std::collections::{HashSet, VecDeque};

/// One bounded transcript load. The consumer takes and releases its raw pages
/// after projecting them; this object is not a history cache.
#[derive(Default)]
pub struct Window {
    pub before: Option<u64>,
    pub through: Option<u64>,
    pages: VecDeque<Vec<Event>>,
    count: usize,
}

impl Window {
    pub fn earlier(before: u64, through: u64) -> Self {
        Self {
            before: Some(before),
            through: Some(through),
            ..Self::default()
        }
    }

    pub fn params(&self, session: &str, branch: &str) -> Value {
        let mut params = json!({"sessionId":session,"branchId":branch,"limit":256});
        if let Some(before) = self.before {
            params["before"] = json!(before);
            params["through"] = json!(self.through);
        } else {
            params["latestTurn"] = json!(true);
        }
        params
    }

    /// Consumes a wire page; true means this load has reached its budget or
    /// the start of the branch, regardless of the latest-turn `done` flag.
    pub fn push(&mut self, page: HistoryWindow) -> crate::Result<bool> {
        let before = page.next(self.before, self.through)?;
        let events = page
            .events
            .into_iter()
            .map(serde_json::from_value)
            .collect::<serde_json::Result<_>>()?;
        self.before = before;
        self.through = Some(page.through);
        self.pages.push_front(events);
        self.count += 1;
        let mut rows = HashSet::new();
        for event in self.pages.iter().flatten() {
            rows_of(event, |row| {
                rows.insert(row);
            });
        }
        Ok(self.before.is_none() || self.count >= 4 || rows.len() >= 100)
    }

    /// Releases the loaded events in forward order, retaining at most 300
    /// transcript identities. The cursor remains available for an explicit
    /// request for earlier content.
    pub fn take(&mut self) -> Vec<Event> {
        let mut rows = HashSet::new();
        let total = self.pages.iter().map(Vec::len).sum::<usize>();
        let mut skip = 0;
        for (index, event) in self
            .pages
            .iter()
            .rev()
            .flat_map(|page| page.iter().rev())
            .enumerate()
        {
            rows_of(event, |row| {
                rows.insert(row);
            });
            if rows.len() > 300 {
                skip = total - index;
                self.before = Some(event.sequence + 1);
                break;
            }
        }
        let events = std::mem::take(&mut self.pages)
            .into_iter()
            .flatten()
            .skip(skip)
            .collect();
        self.count = 0;
        events
    }
}

fn rows_of<'a>(event: &'a Event, mut row: impl FnMut((u8, &'a str, &'a str))) {
    match &event.content {
        EventContent::Input(data) if data.user.is_some() => row((0, &event.id, "")),
        EventContent::Text(data) if !data.delta.is_empty() => row((1, &data.request_id, "")),
        EventContent::ToolUse(data) => row((2, &data.request_id, &data.call.id)),
        EventContent::ToolCompleted(data) => row((2, &data.request_id, &data.call_id)),
        EventContent::ToolResult(data) => {
            for result in &data.results {
                row((2, &data.request_id, &result.call_id));
            }
        }
        _ => {}
    }
}
