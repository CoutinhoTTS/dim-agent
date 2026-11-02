//! The one bounded read of a branch's facts ([ADR 019](../../../docs/adr/019-event-query.md),
//! [plugin contract §5.4](../../../docs/design/plugin-contract.md#54-the-event-query)).
//!
//! A capability names what it accepts; storage selects, orders and bounds the
//! events ([ADR 019 D3](../../../docs/adr/019-event-query.md#d3-the-query-addresses-the-event-contract-not-the-storage)).
//! The request, the reply and the cursor are serde data: they cross languages
//! through the `timeline` service.

use crate::{AgentError, Result};
use dim_agent_types::AgentEvent;
use serde::{Deserialize, Serialize};
use std::{num::NonZeroU64, sync::Arc};

/// Which branches a query covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimelineQueryScope {
    /// The queried branch alone.
    Branch,
    /// The branch chain: each ancestor up to its cut, root first, then the
    /// queried branch.
    Chain,
}

/// Where a query starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TimelineQueryPosition {
    /// Sequences `start..=end`. `start` may be inside an atomic batch; `end`
    /// must end one.
    Range { start: u64, end: u64 },
    /// Sequences after `cursor`, through `through`. The cursor must end an
    /// atomic batch, and `through` must end one too.
    Cursor {
        cursor: TimelineCursor,
        through: u64,
    },
}

/// An opaque position: the branch of the last event a query returned and the
/// sequence through which the query read it. The same sequence on a parent and
/// on its child are different positions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineCursor {
    pub branch_id: String,
    pub sequence: u64,
}

/// One equality against one JSON scalar of an event's data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineQueryPredicate {
    /// Field names inside the event's data joined by dots; no array indexing
    /// and no wildcards.
    pub path: String,
    /// The JSON scalar the path must equal.
    pub value: serde_json::Value,
}

impl TimelineQueryPredicate {
    /// Whether `event`'s data holds `value` at `path`. A path the event does
    /// not have matches nothing.
    pub fn matches(&self, event: &AgentEvent) -> bool {
        let Ok(kind) = serde_json::to_value(&event.kind) else {
            return false;
        };
        let Some(mut current) = kind.get("data") else {
            return false;
        };
        for segment in self.path.split('.') {
            let Some(next) = current.get(segment) else {
                return false;
            };
            current = next;
        }
        *current == self.value
    }

    /// The path as a JSON path from the root of the event's data, `None` when
    /// its spelling is not the one the contract fixes.
    pub fn json_path(&self) -> Option<String> {
        let valid = self.path.split('.').all(|segment| {
            !segment.is_empty()
                && !segment.chars().any(|c| {
                    c.is_whitespace()
                        || c.is_control()
                        || matches!(c, '.' | '[' | ']' | '*' | '"' | '\'' | '\\' | '$' | '#')
                })
        });
        valid.then(|| format!("$.{path}", path = self.path))
    }

    fn validate(&self) -> Result<()> {
        if self.json_path().is_none() {
            return Err(AgentError::InvalidInput(format!(
                "query path {:?} is not field names joined by dots",
                self.path
            )));
        }
        if matches!(
            self.value,
            serde_json::Value::Array(_) | serde_json::Value::Object(_)
        ) {
            return Err(AgentError::InvalidInput(
                "query predicate compares one JSON scalar".into(),
            ));
        }
        Ok(())
    }
}

/// One bounded read of a branch's facts: what it covers, where it starts, what
/// it accepts and how many events it targets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineQuery {
    pub scope: TimelineQueryScope,
    pub position: TimelineQueryPosition,
    /// Event kinds (`domain/name`) the read accepts; empty accepts every kind.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<String>,
    /// When named, only events whose envelope carries this render flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_render: Option<bool>,
    /// When named, only events whose envelope carries this state flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_state: Option<bool>,
    /// One equality the event's data must satisfy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predicate: Option<TimelineQueryPredicate>,
    /// The target number of events. A reply ends at an atomic batch boundary,
    /// so its final batch may take it above the limit.
    pub limit: NonZeroU64,
}

impl TimelineQuery {
    /// Checks the input a caller can get wrong before any storage read: the
    /// predicate's spelling and value, and the position's shape. Atomic batch
    /// boundaries are the store's, which sees the batches.
    pub fn validate(&self) -> Result<()> {
        if let Some(predicate) = &self.predicate {
            predicate.validate()?;
        }
        match &self.position {
            TimelineQueryPosition::Range { start, .. } => {
                if *start == 0 {
                    return Err(AgentError::InvalidInput(
                        "timeline sequences start at 1".into(),
                    ));
                }
            }
            TimelineQueryPosition::Cursor { cursor, .. } => {
                if cursor.sequence == 0 {
                    return Err(AgentError::InvalidInput(
                        "a query cursor names a committed sequence".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}

/// The facts a query read, in chain order and then sequence, and the cursor
/// after them. `next` is absent when the read reached its through.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineQueryReply {
    pub events: Vec<Arc<AgentEvent>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<TimelineCursor>,
}
