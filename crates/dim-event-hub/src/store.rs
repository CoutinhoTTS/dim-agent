//! The subscription file `eventhub/subscriptions.json` (design §2.2, A.5):
//! one inner v1 JSON document inside the host's `{version, text}` envelope,
//! which [`PluginFileTransactions`] reads and writes. The part reads the file
//! once when it applies, holds the set in memory, and every change writes the
//! file under the lock and then replaces the set.
use crate::{
    error::EventHubError,
    model::{Origin, Target},
};
use dim_agent_types::InputMode;
use dim_app_server::plugin_files::{PluginFileError, PluginFileTransactions};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    sync::{Arc, RwLock},
};

pub const SUBSCRIPTIONS_FILE: &str = "eventhub/subscriptions.json";
const FORMAT_VERSION: u32 = 1;

/// The newest delivery outcome that was not delivered (design §2.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LastFailure {
    pub code: String,
    pub at: String,
}

/// One durable subscription (design §2.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Subscription {
    pub id: String,
    pub types: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "match")]
    pub matches: Option<BTreeMap<String, String>>,
    pub target: Target,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<InputMode>,
    pub origin: Origin,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failure: Option<LastFailure>,
}

impl Subscription {
    /// The subscription's effective mode: its own, else the type's declared
    /// mode (design §3.1).
    pub(crate) fn effective_mode(&self, declared: InputMode) -> InputMode {
        self.mode.unwrap_or(declared)
    }

    /// Whether the subscription selects an event of `source` and `event_type`
    /// with `attributes` (design §2.2).
    pub(crate) fn selects(
        &self,
        source: &str,
        event_type: &str,
        attributes: &BTreeMap<String, String>,
    ) -> bool {
        let type_matches = self
            .types
            .iter()
            .any(|pattern| crate::model::pattern_matches(pattern, source, event_type));
        type_matches
            && self.matches.as_ref().is_none_or(|matchers| {
                matchers
                    .iter()
                    .all(|(name, value)| attributes.get(name) == Some(value))
            })
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SubscriptionFile {
    version: u32,
    subscriptions: Vec<Subscription>,
}

/// Decodes the inner text of the file; `None` is a missing file, which holds
/// no subscriptions. An unreadable or inconsistent document fails the part
/// and leaves its bytes unchanged (design A.5).
fn decode(text: Option<&str>) -> Result<Vec<Subscription>, String> {
    let Some(text) = text else {
        return Ok(Vec::new());
    };
    let file: SubscriptionFile =
        serde_json::from_str(text).map_err(|error| format!("subscriptions.json: {error}"))?;
    if file.version != FORMAT_VERSION {
        return Err(format!(
            "subscriptions.json has unknown version {}",
            file.version
        ));
    }
    let mut seen = HashSet::new();
    for subscription in &file.subscriptions {
        if subscription.id.is_empty() || !seen.insert(subscription.id.as_str()) {
            return Err(format!(
                "subscriptions.json repeats or lacks an id: {}",
                subscription.id
            ));
        }
        if subscription.target.session_id.is_empty() || subscription.target.branch_id.is_empty() {
            return Err(format!(
                "subscription {} has a target without a Session",
                subscription.id
            ));
        }
    }
    Ok(file.subscriptions)
}

fn encode(subscriptions: &[Subscription]) -> String {
    serde_json::to_string(&SubscriptionFile {
        version: FORMAT_VERSION,
        subscriptions: subscriptions.to_vec(),
    })
    .expect("subscription records contain only serializable fields")
}

/// The subscription set: the file, the lock every change takes, and the
/// in-memory copy publish reads.
pub(crate) struct Store {
    files: Arc<PluginFileTransactions>,
    /// Serializes every change: the write and the in-memory replacement. A
    /// publish reads the set without it.
    change: tokio::sync::Mutex<()>,
    state: RwLock<Vec<Subscription>>,
}

impl Store {
    pub(crate) fn new(files: Arc<PluginFileTransactions>) -> Self {
        Self {
            files,
            change: tokio::sync::Mutex::new(()),
            state: RwLock::new(Vec::new()),
        }
    }

    /// Reads the file once, when the part applies. A missing file is no
    /// subscriptions; a corrupt one is an error, and its bytes stay unchanged.
    pub(crate) async fn load(&self) -> Result<Vec<Subscription>, String> {
        self.files
            .transact(SUBSCRIPTIONS_FILE, |current| (None, decode(current)))
            .await
            .map_err(|error| error.to_string())?
    }

    /// Replaces the in-memory set, after a load or a successful change.
    pub(crate) fn replace(&self, subscriptions: Vec<Subscription>) {
        *self.state.write().expect("the subscription set") = subscriptions;
    }

    /// The set as it is now: what a publish matches against, and what
    /// `eventhub.subscriptions` lists.
    pub(crate) fn read(&self) -> Vec<Subscription> {
        self.state.read().expect("the subscription set").clone()
    }

    /// Applies `change` under the store's lock. `change` returns whether it
    /// modified the set, the value it answers with, and errors as its caller
    /// answers them. On success the in-memory set is replaced with the set
    /// the file now holds; a failed write leaves both the file and the set as
    /// they were.
    pub(crate) async fn change<R>(
        &self,
        change: impl FnOnce(&mut Vec<Subscription>) -> Result<(bool, R), EventHubError> + Send + 'static,
    ) -> Result<R, EventHubError>
    where
        R: Send + 'static,
    {
        let _guard = self.change.lock().await;
        let outcome = self
            .files
            .transact(SUBSCRIPTIONS_FILE, move |current| {
                let mut subscriptions = match decode(current) {
                    Ok(subscriptions) => subscriptions,
                    Err(error) => {
                        return (None, Err(EventHubError::storage(error)));
                    }
                };
                match change(&mut subscriptions) {
                    Ok((true, value)) => (
                        Some(encode(&subscriptions)),
                        Ok((Some(subscriptions), value)),
                    ),
                    Ok((false, value)) => (None, Ok((None, value))),
                    Err(error) => (None, Err(error)),
                }
            })
            .await;
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) => return Err(storage_error(error)),
        };
        let (subscriptions, value) = outcome?;
        if let Some(subscriptions) = subscriptions {
            self.replace(subscriptions);
        }
        Ok(value)
    }
}

/// A failed file transaction is the design's `storage`: the change did not
/// happen and the in-memory set is unchanged (design A.5, Appendix C).
fn storage_error(error: PluginFileError) -> EventHubError {
    EventHubError::storage(error.to_string())
}
