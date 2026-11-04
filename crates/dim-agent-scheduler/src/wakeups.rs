//! The wakeup file `scheduler/wakeups.json`: one inner v1 JSON document
//! inside the host's `{version, text}` envelope, kept by the same
//! [`PluginFileTransactions`] withLock and replace transactions as the run
//! history ([design §10, A.7]). A wakeup is a one-shot scheduled `input/message`
//! into an existing Session: it captures no policy, launches nothing, has no
//! deadline, and never appears in `schedule.list` or in the run history.
use crate::{
    document::{MAX_PROMPT_BYTES, canonical_instant, format_instant},
    engine::{GRACE_SECONDS, escaped, sessions},
    methods::Shared,
    runs::SchedulerError,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

/// The file inside the host-authorized `scheduler/` directory (A.7's scope).
pub const WAKEUPS_FILE: &str = "scheduler/wakeups.json";
const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeupState {
    Scheduled,
    Fired,
    Cancelled,
    Expired,
}

impl WakeupState {
    /// The wire name, as the file and `session.wakeup.list` spell it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Scheduled => "scheduled",
            Self::Fired => "fired",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
        }
    }

    /// Terminal states carry `settled_at` and are never fired.
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Scheduled)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wakeup {
    pub wakeup_id: String,
    pub session_id: String,
    pub branch_id: String,
    /// The canonical UTC instant the input is written (A.1 timestamp form).
    pub at: String,
    pub prompt: String,
    pub state: WakeupState,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settled_at: Option<String>,
}

impl Wakeup {
    /// The record invariants of §10. A violation makes the whole file invalid,
    /// so the bytes stay unchanged.
    pub fn validate(&self) -> Result<(), String> {
        let timestamp = |text: &str| canonical_instant(text).is_some();
        if self.wakeup_id.is_empty()
            || self.session_id.is_empty()
            || self.branch_id.is_empty()
            || self.prompt.is_empty()
            || self.prompt.len() > MAX_PROMPT_BYTES
        {
            return Err(format!(
                "wakeup {} has an empty field or an oversized prompt",
                self.wakeup_id
            ));
        }
        if !timestamp(&self.at) || !timestamp(&self.created_at) {
            return Err(format!(
                "wakeup {} has a non-canonical timestamp",
                self.wakeup_id
            ));
        }
        if self.settled_at.is_some() != self.state.is_terminal() {
            return Err(format!("wakeup {} is inconsistent", self.wakeup_id));
        }
        if let Some(text) = &self.settled_at
            && !timestamp(text)
        {
            return Err(format!(
                "wakeup {} has a non-canonical settled_at",
                self.wakeup_id
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WakeupFile {
    version: u32,
    wakeups: Vec<Wakeup>,
}

/// Decodes the inner text of the file; `None` is a missing file, which holds
/// no wakeups.
fn decode(text: Option<&str>) -> Result<Vec<Wakeup>, SchedulerError> {
    let Some(text) = text else {
        return Ok(Vec::new());
    };
    let file: WakeupFile = serde_json::from_str(text)
        .map_err(|error| SchedulerError::invalid_data(format!("wakeups.json: {error}")))?;
    if file.version != FORMAT_VERSION {
        return Err(SchedulerError::invalid_data(format!(
            "wakeups.json has unknown version {}",
            file.version
        )));
    }
    let mut seen = std::collections::HashSet::new();
    for wakeup in &file.wakeups {
        if !seen.insert(wakeup.wakeup_id.as_str()) {
            return Err(SchedulerError::invalid_data(format!(
                "wakeups.json repeats wakeup {}",
                wakeup.wakeup_id
            )));
        }
        wakeup.validate().map_err(SchedulerError::invalid_data)?;
    }
    Ok(file.wakeups)
}

fn encode(wakeups: Vec<Wakeup>) -> String {
    serde_json::to_string(&WakeupFile {
        version: FORMAT_VERSION,
        wakeups,
    })
    .expect("wakeup records contain only serializable fields")
}

/// Reads the records. A file that is not valid v1 is an error, not an empty
/// list.
pub async fn read(
    files: &dim_app_server::plugin_files::PluginFileTransactions,
) -> Result<Vec<Wakeup>, SchedulerError> {
    files
        .transact(WAKEUPS_FILE, |current| (None, decode(current)))
        .await?
}

/// Applies `change` to the records under the file lock. `change` returns
/// whether it modified them; an unmodified list is not written back.
async fn update<R: Send + 'static>(
    files: &dim_app_server::plugin_files::PluginFileTransactions,
    change: impl FnOnce(&mut Vec<Wakeup>) -> Result<(bool, R), SchedulerError> + Send + 'static,
) -> Result<R, SchedulerError> {
    files
        .transact(WAKEUPS_FILE, move |current| {
            let mut wakeups = match decode(current) {
                Ok(wakeups) => wakeups,
                Err(error) => return (None, Err(error)),
            };
            match change(&mut wakeups) {
                Ok((true, value)) => (Some(encode(wakeups)), Ok(value)),
                Ok((false, value)) => (None, Ok(value)),
                Err(error) => (None, Err(error)),
            }
        })
        .await?
}

fn stamp(instant: DateTime<Utc>) -> String {
    format_instant(instant)
}

/// The record the Session part asks the process owner to store.
pub struct WakeupRequest {
    pub session_id: String,
    pub branch_id: String,
    pub at: DateTime<Utc>,
    pub prompt: String,
}

/// The one new error code: cancel names no wakeup, or one of another Session.
const WAKEUP_NOT_FOUND: &str = "wakeup_not_found";

/// The process-side service the Session part forwards to (design §10): the
/// same shape as `ScheduleJobs`, keyed by the Session part with `names.key()`.
pub struct Wakeups {
    pub(crate) shared: Arc<Shared>,
}

impl Wakeups {
    /// Records a one-shot wakeup. An instant already past the grace window is
    /// recorded `expired` at once; nothing is ever replayed (A.4's rule).
    pub async fn create(&self, request: WakeupRequest) -> Result<Wakeup, SchedulerError> {
        let now = Utc::now();
        let expired = now - request.at >= Duration::seconds(GRACE_SECONDS);
        let created = Wakeup {
            wakeup_id: uuid::Uuid::new_v4().to_string(),
            session_id: request.session_id,
            branch_id: request.branch_id,
            at: stamp(request.at),
            prompt: request.prompt,
            state: if expired {
                WakeupState::Expired
            } else {
                WakeupState::Scheduled
            },
            created_at: stamp(now),
            settled_at: expired.then(|| stamp(now)),
        };
        created.validate().map_err(SchedulerError::invalid_data)?;
        let record = created.clone();
        update(&self.shared.files, move |wakeups| {
            wakeups.push(created);
            Ok((true, record))
        })
        .await
    }

    /// The wakeups of one Session branch; the method boundary defaults
    /// `branch_id` to `main`. Newest first.
    pub async fn list(
        &self,
        session_id: &str,
        branch_id: &str,
    ) -> Result<Vec<Wakeup>, SchedulerError> {
        let mut own: Vec<Wakeup> = read(&self.shared.files)
            .await?
            .into_iter()
            .filter(|wakeup| wakeup.session_id == session_id && wakeup.branch_id == branch_id)
            .collect();
        own.sort_by(|a, b| b.at.cmp(&a.at).then(b.wakeup_id.cmp(&a.wakeup_id)));
        Ok(own)
    }

    /// Cancels a scheduled record. The reply is the record and whether this
    /// call settled it: a record that already fired, expired or was cancelled
    /// is returned with `false` instead of failing (§10) — cancellation is
    /// not an error after the fact.
    pub async fn cancel(
        &self,
        session_id: &str,
        wakeup_id: &str,
    ) -> Result<(Wakeup, bool), SchedulerError> {
        let _hold = self.shared.wakeups.lock().await;
        let session_id = session_id.to_owned();
        let wakeup_id = wakeup_id.to_owned();
        update(&self.shared.files, move |wakeups| {
            let Some(position) = wakeups
                .iter()
                .position(|wakeup| wakeup.wakeup_id == wakeup_id)
            else {
                return Err(SchedulerError::new(
                    WAKEUP_NOT_FOUND,
                    format!("no wakeup {wakeup_id}"),
                ));
            };
            if wakeups[position].session_id != session_id {
                return Err(SchedulerError::new(
                    WAKEUP_NOT_FOUND,
                    format!("no wakeup {wakeup_id}"),
                ));
            }
            let record = &mut wakeups[position];
            let settled = record.state == WakeupState::Scheduled;
            if settled {
                record.state = WakeupState::Cancelled;
                record.settled_at = Some(stamp(Utc::now()));
            }
            let record = record.clone();
            Ok((settled, (record, settled)))
        })
        .await
    }

    /// One pass of the timer (and of the restart, whose first pass runs
    /// within a second of startup): a `scheduled` record whose instant is
    /// within the grace window is fired once, one whose instant is past the
    /// window is `expired` and never replayed, and one whose Session is gone
    /// is `cancelled`. A fire is only the input write through the public
    /// `sessions.input`; there is no launch, stop or withdraw (§10).
    pub async fn evaluate(&self) -> Result<(), SchedulerError> {
        let _hold = self.shared.wakeups.lock().await;
        let now = Utc::now();
        let due: Vec<Wakeup> = read(&self.shared.files)
            .await?
            .into_iter()
            .filter(|wakeup| wakeup.state == WakeupState::Scheduled)
            .filter(|wakeup| {
                canonical_instant(&wakeup.at)
                    .is_some_and(|at| at <= now && now - at < Duration::seconds(GRACE_SECONDS))
            })
            .collect();
        for wakeup in due {
            let session_id = wakeup.session_id.clone();
            let branch_id = wakeup.branch_id.clone();
            let marker = format!(
                "<dim-scheduler kind=\"wakeup\">{}</dim-scheduler>",
                escaped(&wakeup.wakeup_id)
            );
            let input = json!({
                "sessionId": session_id,
                "branchId": branch_id,
                "mode": "queue",
                "submissionId": format!("wakeup:{}", wakeup.wakeup_id),
                "rawBlocks": [{"text": marker}],
                "content": [{"type": "text", "text": wakeup.prompt}],
            });
            match sessions(self.shared.sessions.as_ref(), "input", input).await {
                Ok(Ok(_)) => {
                    self.settle(&wakeup.wakeup_id, WakeupState::Fired, now)
                        .await?
                }
                Ok(Err(code)) if matches!(code.as_str(), "session_closed" | "branch_missing") => {
                    self.settle(&wakeup.wakeup_id, WakeupState::Cancelled, now)
                        .await?
                }
                // `reloading`, transport failures and any other code leave the
                // record scheduled: the next pass retries with the same
                // submission id, which a branch accepts once, or the grace
                // window expires the record.
                _ => {}
            }
        }
        // Records whose instant is beyond the grace are settled `expired`
        // without a fire; their Session is never restored or launched for them.
        let now = Utc::now();
        let stale: Vec<String> = read(&self.shared.files)
            .await?
            .into_iter()
            .filter(|wakeup| wakeup.state == WakeupState::Scheduled)
            .filter(|wakeup| {
                canonical_instant(&wakeup.at)
                    .is_some_and(|at| at <= now && now - at >= Duration::seconds(GRACE_SECONDS))
            })
            .map(|wakeup| wakeup.wakeup_id)
            .collect();
        for wakeup_id in stale {
            self.settle(&wakeup_id, WakeupState::Expired, now).await?;
        }
        Ok(())
    }

    /// Settles a record that is still `scheduled`; a record a cancel settled
    /// in between keeps its terminal state. `create`, `cancel` and
    /// `evaluate` share the same lock, so that cannot happen; the terminal
    /// guard keeps the transition total.
    async fn settle(
        &self,
        wakeup_id: &str,
        state: WakeupState,
        at: DateTime<Utc>,
    ) -> Result<(), SchedulerError> {
        let wakeup_id = wakeup_id.to_owned();
        update(&self.shared.files, move |wakeups| {
            let Some(record) = wakeups
                .iter_mut()
                .find(|wakeup| wakeup.wakeup_id == wakeup_id)
            else {
                return Ok((false, ()));
            };
            if record.state != WakeupState::Scheduled {
                return Ok((false, ()));
            }
            record.state = state;
            record.settled_at = Some(stamp(at));
            Ok((true, ()))
        })
        .await
    }
}
