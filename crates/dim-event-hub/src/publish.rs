//! Publish and delivery (design §2.3–§2.5, §3.1, §3.2, A.2): one input per
//! target branch, at most eight deliveries at a time across the install, one
//! receipt, and the publish's subscription changes in one transaction after
//! its deliveries settle.
use crate::{
    error::{EventHubError, SessionsFailure},
    hub::Shared,
    model::{BODY_LIMIT, PREVIEW_BYTES, PublishRequest, Target, validate_publish},
    state::{InstanceState, MAX_DELIVERIES_IN_FLIGHT},
    store::{Document, GuideRecord, LastFailure, Store, Subscription},
    subscribe::{refresh_submission, write_live_guide},
    text::{digest, escaped, preview, stamp},
};
use chrono::Utc;
use dim_agent_capabilities::{host::SessionFilesHost, wait::EVENT_SUBMISSION_PREFIX};
use dim_agent_types::InputMode;
use futures_util::{StreamExt, stream};
use rutis::Ctx;
use rutis::host::CtxHost;
use rutis_bridge::session::{HostDispatch, settle};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

/// The file name an overflowed body is stored as (design §3.2).
const EVENT_FILE: &str = "event.txt";

/// `publish`'s reply (design A.2).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Receipt {
    pub event_id: String,
    pub deliveries: Vec<Delivery>,
}

/// One target branch's outcome (design §2.3).
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub(crate) enum Delivery {
    #[serde(rename_all = "camelCase")]
    Delivered {
        session_id: String,
        branch_id: String,
        subscription_ids: Vec<String>,
        input_id: String,
        sequence: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        truncated: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        repeated: Option<bool>,
    },
    #[serde(rename_all = "camelCase")]
    Gone {
        session_id: String,
        branch_id: String,
        subscription_ids: Vec<String>,
        code: String,
    },
    #[serde(rename_all = "camelCase")]
    Failed {
        session_id: String,
        branch_id: String,
        subscription_ids: Vec<String>,
        code: String,
        message: String,
    },
}

/// One branch a publish delivers to, merged from `to` and the matching
/// subscriptions (design §3.1).
struct PlannedTarget {
    session_id: String,
    branch_id: String,
    subscription_ids: Vec<String>,
    /// The first `to` entry's own mode for this branch, when one names it.
    explicit_mode: Option<InputMode>,
    /// The effective modes of the matching subscriptions.
    effective_modes: Vec<InputMode>,
    /// The mode the delivery uses: the `to` entry's, else the strongest
    /// effective mode, else the type's declared mode (design §3.1).
    mode: InputMode,
}

impl PlannedTarget {
    fn target(&self) -> Target {
        Target {
            session_id: self.session_id.clone(),
            branch_id: self.branch_id.clone(),
        }
    }
}

/// The guide a delivery writes before its event: the current declaration's
/// text and digest (design §3.3).
struct GuideRefresh {
    source: String,
    guide: String,
    digest: String,
}

/// One delivery to make, or one branch that already holds the event.
enum Planned {
    /// The branch holds this event's input already: a retry answers the first
    /// input's ids without writing a file or a second input (design §3.2).
    Already(Delivery),
    Dispatch(DeliveryJob),
}

/// Publishes one event for the row that owns `ctx` (design A.2).
pub(crate) async fn publish(
    shared: &Shared,
    ctx: &Ctx,
    request: PublishRequest,
) -> Result<Value, EventHubError> {
    validate_publish(&request)?;
    let owner = ctx.instance();
    let declaration = shared
        .state
        .catalog()
        .held_by(&request.source, owner)
        .ok_or_else(|| {
            EventHubError::unknown_source(format!(
                "the calling row holds no source {}",
                request.source
            ))
        })?;
    let declared_mode = declaration
        .types
        .get(&request.r#type)
        .map(|declared| declared.mode)
        .ok_or_else(|| {
            EventHubError::unknown_type(format!(
                "source {} declares no type {}",
                request.source, request.r#type
            ))
        })?;
    let event_id = request
        .id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let submission = submission_id(&request.source, &event_id);
    // Two publishes of one id never interleave: the second finds the first's
    // inputs and writes no file for a branch that holds one (design §2.4).
    let _event = shared.state.event_lock(&submission).await;
    let document = shared.store.document();
    let plans = plan_targets(&document.subscriptions, &request, declared_mode);
    let truncated = request.body.len() > BODY_LIMIT;
    let guide_digest = digest(&declaration.guide);
    let repeated = if truncated {
        looked_up(shared, &plans, &submission).await
    } else {
        HashMap::new()
    };
    let pending: Vec<&PlannedTarget> = plans
        .iter()
        .filter(|plan| !repeated.contains_key(&(plan.session_id.clone(), plan.branch_id.clone())))
        .collect();
    let files = if truncated {
        overflow_files(shared, &pending, &request.body).await
    } else {
        HashMap::new()
    };
    let planned: Vec<Planned> = plans
        .iter()
        .map(|plan| {
            if let Some((input_id, sequence)) =
                repeated.get(&(plan.session_id.clone(), plan.branch_id.clone()))
            {
                return Planned::Already(Delivery::Delivered {
                    session_id: plan.session_id.clone(),
                    branch_id: plan.branch_id.clone(),
                    subscription_ids: plan.subscription_ids.clone(),
                    input_id: input_id.clone(),
                    sequence: *sequence,
                    truncated: None,
                    repeated: Some(true),
                });
            }
            let file = files.get(&plan.session_id).and_then(Option::as_ref);
            let text = event_text(request.title.as_deref(), &request.body, truncated);
            Planned::Dispatch(DeliveryJob {
                session_id: plan.session_id.clone(),
                branch_id: plan.branch_id.clone(),
                subscription_ids: plan.subscription_ids.clone(),
                mode: plan.mode,
                submission_id: submission.clone(),
                marker: marker(
                    &request.source,
                    &request.r#type,
                    &event_id,
                    &request.attributes,
                    file,
                    truncated,
                    &text,
                ),
                truncated: truncated && file.is_none(),
                refresh: guide_refresh(
                    &document,
                    plan,
                    &request.source,
                    &declaration.guide,
                    &guide_digest,
                ),
            })
        })
        .collect();
    let service = shared.sessions.clone();
    let store = shared.store.clone();
    let state = shared.state.clone();
    let width = planned.len().max(1);
    // Every pending delivery waits for one of the install's eight permits, so
    // concurrent publishes share the budget (design §2.5); `buffered` keeps
    // the receipt in target order.
    let outcomes: Vec<(Delivery, Option<GuideRecord>)> = stream::iter(planned)
        .map(|planned| {
            let service = service.clone();
            let store = store.clone();
            let state = state.clone();
            async move {
                match planned {
                    Planned::Already(delivery) => (delivery, None),
                    Planned::Dispatch(job) => {
                        let _permit = state.delivery_permit().await;
                        deliver(service, store, state, job).await
                    }
                }
            }
        })
        .buffered(width)
        .collect()
        .await;
    let (deliveries, refreshes): (Vec<Delivery>, Vec<Option<GuideRecord>>) =
        outcomes.into_iter().unzip();
    let refreshes: Vec<GuideRecord> = refreshes.into_iter().flatten().collect();
    // A Session Agents says is gone takes its subscriptions with it (design
    // §3.4); the notice stays the prompt path.
    reconcile_closed(shared, &deliveries).await;
    if let Err(error) = apply_changes(shared, &deliveries, &refreshes).await {
        // The receipt stands and the in-memory set keeps its old value; the
        // next publish whose outcome changes it writes again (Appendix B).
        tracing::warn!(%error, "EventHub could not record a publish's subscription changes");
    }
    Ok(json!(Receipt {
        event_id,
        deliveries,
    }))
}

/// Builds one target per branch: the `to` targets plus the in-memory matches
/// as they stand when the publish starts, in that order (design §3.1).
fn plan_targets(
    subscriptions: &[Subscription],
    request: &PublishRequest,
    declared_mode: InputMode,
) -> Vec<PlannedTarget> {
    let mut plans: Vec<PlannedTarget> = Vec::new();
    let find = |plans: &mut Vec<PlannedTarget>, session_id: &str, branch_id: &str| {
        if let Some(index) = plans
            .iter()
            .position(|plan| plan.session_id == session_id && plan.branch_id == branch_id)
        {
            return index;
        }
        plans.push(PlannedTarget {
            session_id: session_id.to_owned(),
            branch_id: branch_id.to_owned(),
            subscription_ids: Vec::new(),
            explicit_mode: None,
            effective_modes: Vec::new(),
            mode: InputMode::Queue,
        });
        plans.len() - 1
    };
    for target in &request.to {
        let index = find(
            &mut plans,
            &target.session_id,
            target.branch_id.as_deref().unwrap_or("main"),
        );
        if plans[index].explicit_mode.is_none() {
            plans[index].explicit_mode = target.mode;
        }
    }
    for subscription in subscriptions {
        if !subscription.selects(&request.source, &request.r#type, &request.attributes) {
            continue;
        }
        let index = find(
            &mut plans,
            &subscription.target.session_id,
            &subscription.target.branch_id,
        );
        plans[index].subscription_ids.push(subscription.id.clone());
        plans[index]
            .effective_modes
            .push(subscription.effective_mode(declared_mode));
    }
    for plan in &mut plans {
        plan.mode = plan
            .explicit_mode
            .or_else(|| strongest(&plan.effective_modes))
            .unwrap_or(declared_mode);
    }
    plans
}

/// The strongest of the effective modes, `steer` over `queue` over `append`
/// (design §3.1).
fn strongest(modes: &[InputMode]) -> Option<InputMode> {
    modes.iter().copied().max_by_key(|mode| match mode {
        InputMode::Steer => 3,
        InputMode::Queue => 2,
        InputMode::Append => 1,
    })
}

/// The branches that already hold this event's input, by `sessions.lookup`
/// (design §2.4, §3.2). Only an overflowed body asks: a body that fits writes
/// no file, so a repeated input is already answered by the branch itself.
async fn looked_up(
    shared: &Shared,
    plans: &[PlannedTarget],
    submission: &str,
) -> HashMap<(String, String), (String, u64)> {
    let mut held = HashMap::new();
    for plan in plans {
        let args = json!({
            "sessionId": plan.session_id,
            "branchId": plan.branch_id,
            "submissionId": submission,
        });
        if let Ok(Ok(value)) = sessions(shared.sessions.as_ref(), "lookup", args).await
            && value["status"] == json!("accepted")
            && let (Some(input_id), Some(sequence)) =
                (value["inputId"].as_str(), value["sequence"].as_u64())
        {
            held.insert(
                (plan.session_id.clone(), plan.branch_id.clone()),
                (input_id.to_owned(), sequence),
            );
        }
    }
    held
}

/// Stores an overflowed body once per target Session (design §3.2), and
/// answers the file each Session got, if any. A Session whose every branch
/// already holds the input gets no file.
async fn overflow_files(
    shared: &Shared,
    plans: &[&PlannedTarget],
    body: &str,
) -> HashMap<String, Option<dim_agent_capabilities::host::StoredSessionFile>> {
    let mut sessions: Vec<String> = Vec::new();
    for plan in plans {
        if !sessions.contains(&plan.session_id) {
            sessions.push(plan.session_id.clone());
        }
    }
    let Some(service) = shared.ctx.get::<SessionFilesHost>() else {
        // No native store: the markers carry `truncated="true"` (design §3.2).
        return sessions
            .into_iter()
            .map(|session| (session, None))
            .collect();
    };
    let body = body.to_owned();
    stream::iter(sessions)
        .map(|session_id| {
            let service = service.clone();
            let body = body.clone();
            let cancel = shared.stop.clone();
            async move {
                let stored = service
                    .0
                    .put(
                        session_id.clone(),
                        EVENT_FILE.to_owned(),
                        body.into_bytes(),
                        cancel,
                    )
                    .await
                    .ok();
                (session_id, stored)
            }
        })
        .buffered(MAX_DELIVERIES_IN_FLIGHT)
        .collect()
        .await
}

/// The refresh a delivery writes before its event, when the branch's recorded
/// guide differs from the current declaration (design §3.3). A branch reached
/// only through `to` has no record and gets no guide write.
fn guide_refresh(
    document: &Document,
    plan: &PlannedTarget,
    source: &str,
    guide: &str,
    digest: &str,
) -> Option<GuideRefresh> {
    if plan.subscription_ids.is_empty() {
        return None;
    }
    match document.guide(&plan.target(), source) {
        Some(record) if record.digest == digest => None,
        _ => Some(GuideRefresh {
            source: source.to_owned(),
            guide: guide.to_owned(),
            digest: digest.to_owned(),
        }),
    }
}

/// One delivery's inputs: the branch, the mode, the marker, the submission id
/// and, when the recorded guide is stale, the guide to write first (design
/// §2.4, §3.3).
struct DeliveryJob {
    session_id: String,
    branch_id: String,
    subscription_ids: Vec<String>,
    mode: InputMode,
    submission_id: String,
    marker: String,
    truncated: bool,
    refresh: Option<GuideRefresh>,
}

/// Delivers the event to one branch and answers its outcome, plus the guide
/// record the delivery refreshed (design §2.3, §3.3).
async fn deliver(
    service: Arc<dyn HostDispatch>,
    store: Arc<Store>,
    state: Arc<InstanceState>,
    job: DeliveryJob,
) -> (Delivery, Option<GuideRecord>) {
    let refreshed = match &job.refresh {
        Some(refresh) => refresh_guide(&service, &store, &state, &job, refresh).await,
        None => None,
    };
    let (session_id, branch_id, subscription_ids) = (
        job.session_id.clone(),
        job.branch_id.clone(),
        job.subscription_ids.clone(),
    );
    let write = json!({
        "sessionId": session_id.clone(),
        "branchId": branch_id.clone(),
        "mode": job.mode,
        "submissionId": job.submission_id,
        "rawBlocks": [{ "text": job.marker }],
        "content": [],
    });
    let delivery = match sessions(service.as_ref(), "input", write).await {
        Ok(Ok(value)) => match (value["inputId"].as_str(), value["sequence"].as_u64()) {
            (Some(input_id), Some(sequence)) => Delivery::Delivered {
                session_id: session_id.clone(),
                branch_id: branch_id.clone(),
                subscription_ids: subscription_ids.clone(),
                input_id: input_id.to_owned(),
                sequence,
                truncated: job.truncated.then_some(true),
                repeated: None,
            },
            _ => Delivery::Failed {
                session_id: session_id.clone(),
                branch_id: branch_id.clone(),
                subscription_ids: subscription_ids.clone(),
                code: "failed".into(),
                message: "sessions.input answered without an input".into(),
            },
        },
        Ok(Err(failure)) => {
            let gone = matches!(failure.code.as_str(), "branch_missing" | "branch_ended");
            if gone {
                Delivery::Gone {
                    session_id: session_id.clone(),
                    branch_id: branch_id.clone(),
                    subscription_ids: subscription_ids.clone(),
                    code: failure.code,
                }
            } else {
                Delivery::Failed {
                    session_id: session_id.clone(),
                    branch_id: branch_id.clone(),
                    subscription_ids: subscription_ids.clone(),
                    code: failure.code,
                    message: failure.message,
                }
            }
        }
        Err(message) => Delivery::Failed {
            session_id: session_id.clone(),
            branch_id: branch_id.clone(),
            subscription_ids: subscription_ids.clone(),
            code: "failed".into(),
            message,
        },
    };
    (delivery, refreshed)
}

/// Writes the current guide to the branch before its event, under that
/// Session's lock, and only while a stored subscription on that branch still
/// names the source (design §3.3). A refresh that cannot be written leaves the
/// record stale, so the next delivery tries again.
async fn refresh_guide(
    service: &Arc<dyn HostDispatch>,
    store: &Arc<Store>,
    state: &Arc<InstanceState>,
    job: &DeliveryJob,
    refresh: &GuideRefresh,
) -> Option<GuideRecord> {
    let target = Target {
        session_id: job.session_id.clone(),
        branch_id: job.branch_id.clone(),
    };
    let _session = state.session_lock(&job.session_id).await;
    if !store.document().names_source(&target, &refresh.source) {
        return None;
    }
    match write_live_guide(
        service.as_ref(),
        &target,
        refresh_submission(&refresh.source),
        &refresh.source,
        &refresh.guide,
    )
    .await
    {
        Ok(()) => Some(GuideRecord {
            target,
            source: refresh.source.clone(),
            digest: refresh.digest.clone(),
        }),
        Err(_) => None,
    }
}

/// Applies a publish's subscription changes in one transaction after its
/// deliveries settle (design §2.3): `gone` removes each listed subscription,
/// `failed` sets `lastFailure`, `delivered` clears it, and a refreshed guide
/// records its digest, each only when the value changes.
async fn apply_changes(
    shared: &Shared,
    deliveries: &[Delivery],
    refreshes: &[GuideRecord],
) -> Result<(), EventHubError> {
    let mut removed: HashSet<String> = HashSet::new();
    let mut cleared: HashSet<String> = HashSet::new();
    let mut failed: HashMap<String, String> = HashMap::new();
    for delivery in deliveries {
        match delivery {
            Delivery::Delivered {
                subscription_ids, ..
            } => cleared.extend(subscription_ids.iter().cloned()),
            Delivery::Gone {
                subscription_ids, ..
            } => removed.extend(subscription_ids.iter().cloned()),
            Delivery::Failed {
                subscription_ids,
                code,
                ..
            } => failed.extend(subscription_ids.iter().map(|id| (id.clone(), code.clone()))),
        }
    }
    if removed.is_empty() && cleared.is_empty() && failed.is_empty() && refreshes.is_empty() {
        return Ok(());
    }
    let now = stamp(Utc::now());
    let refreshes = refreshes.to_vec();
    shared
        .store
        .change(move |document| {
            let before = document.subscriptions.len();
            document
                .subscriptions
                .retain(|subscription| !removed.contains(subscription.id.as_str()));
            let mut changed = document.subscriptions.len() != before;
            for subscription in &mut document.subscriptions {
                if let Some(code) = failed.get(subscription.id.as_str()) {
                    if subscription
                        .last_failure
                        .as_ref()
                        .map(|failure| failure.code.as_str())
                        != Some(code.as_str())
                    {
                        subscription.last_failure = Some(LastFailure {
                            code: (*code).to_owned(),
                            at: now.clone(),
                        });
                        changed = true;
                    }
                } else if cleared.contains(subscription.id.as_str())
                    && subscription.last_failure.is_some()
                {
                    subscription.last_failure = None;
                    changed = true;
                }
            }
            for record in &refreshes {
                if document.names_source(&record.target, &record.source) {
                    changed |= document.set_guide(&record.target, &record.source, &record.digest);
                }
            }
            Ok((changed, ()))
        })
        .await
}

/// Asks Agents whether each Session a delivery failed `session_closed` for
/// still exists, and removes the subscriptions of one it says is gone (design
/// §3.4). An error keeps them.
async fn reconcile_closed(shared: &Shared, deliveries: &[Delivery]) {
    let mut sessions: Vec<String> = Vec::new();
    for delivery in deliveries {
        if let Delivery::Failed {
            session_id, code, ..
        } = delivery
            && code == "session_closed"
            && !sessions.contains(session_id)
        {
            sessions.push(session_id.clone());
        }
    }
    for session_id in sessions {
        if session_exists(shared, &session_id).await == Some(false) {
            shared.remove_session(&session_id).await;
        }
    }
}

/// The reconciliation an `apply` runs: for each Session the stored
/// subscriptions target, `sessions.exists`; a `false` removes that Session's
/// subscriptions in one transaction, at most eight reads at a time (design
/// §3.4). Any error keeps them.
pub(crate) async fn reconcile(shared: Arc<Shared>) {
    let mut sessions: Vec<String> = Vec::new();
    for subscription in shared.store.read() {
        if !sessions.contains(&subscription.target.session_id) {
            sessions.push(subscription.target.session_id);
        }
    }
    stream::iter(sessions)
        .map(|session_id| {
            let shared = shared.clone();
            async move {
                if session_exists(&shared, &session_id).await == Some(false) {
                    shared.remove_session(&session_id).await;
                }
            }
        })
        .buffered(MAX_DELIVERIES_IN_FLIGHT)
        .collect::<Vec<()>>()
        .await;
}

/// `sessions.exists` (design §3.4, §9.2). An error or a malformed reply is
/// `None`, which keeps the subscriptions.
async fn session_exists(shared: &Shared, session_id: &str) -> Option<bool> {
    match sessions(
        shared.sessions.as_ref(),
        "exists",
        json!({ "sessionId": session_id }),
    )
    .await
    {
        Ok(Ok(value)) => value["exists"].as_bool(),
        _ => None,
    }
}

/// The submission id every delivery of one event uses (design §2.4). It
/// starts with the prefix Wait matches, so a delivery ends a wait on the
/// branch (design §9.3); a guide write starts `eventhub-guide`, so a guide
/// never does.
pub(crate) fn submission_id(source: &str, event_id: &str) -> String {
    format!("{EVENT_SUBMISSION_PREFIX}{source}:{event_id}")
}

/// The marker's text: the title line and the body, or the start of the body
/// when it was stored as a file (design §3.2).
fn event_text(title: Option<&str>, body: &str, truncated: bool) -> String {
    let body = if truncated {
        preview(body, PREVIEW_BYTES)
    } else {
        body
    };
    match title {
        Some(title) => format!("{title}\n{body}"),
        None => body.to_owned(),
    }
}

/// The `dim-event` marker (design A.2, §7): every attribute value, the title
/// and the body escaped, so no producer text can open or close a tag.
fn marker(
    source: &str,
    event_type: &str,
    event_id: &str,
    attributes: &BTreeMap<String, String>,
    file: Option<&dim_agent_capabilities::host::StoredSessionFile>,
    truncated: bool,
    text: &str,
) -> String {
    let mut marker = format!(
        "<dim-event source=\"{}\" type=\"{}\" id=\"{}\"",
        escaped(source),
        escaped(event_type),
        escaped(event_id)
    );
    for (name, value) in attributes {
        marker.push_str(&format!(" {name}=\"{}\"", escaped(value)));
    }
    if let Some(file) = file {
        marker.push_str(&format!(
            " file=\"{}\" bytes=\"{}\"",
            escaped(&file.path.to_string_lossy()),
            file.bytes
        ));
    }
    if truncated && file.is_none() {
        marker.push_str(" truncated=\"true\"");
    }
    marker.push('>');
    marker.push_str(&escaped(text));
    marker.push_str("</dim-event>");
    marker
}

/// A `sessions` call. The outer error is a transport failure, whose outcome
/// is unknown; the inner error is the service's own code and message.
pub(crate) async fn sessions(
    service: &dyn HostDispatch,
    method: &str,
    args: Value,
) -> Result<Result<Value, SessionsFailure>, String> {
    let call = service
        .invoke(method, json!([args]).into())
        .map_err(|error| error.to_string())?;
    let envelope = settle(call)
        .await
        .map_err(|error| error.to_string())?
        .json()
        .map_err(|error| error.to_string())?;
    if envelope["ok"] == json!(true) {
        Ok(Ok(envelope["value"].clone()))
    } else {
        Ok(Err(SessionsFailure::new(
            envelope["error"]["code"]
                .as_str()
                .unwrap_or("failed")
                .to_owned(),
            envelope["error"]["message"]
                .as_str()
                .unwrap_or("the sessions service refused the input")
                .to_owned(),
        )))
    }
}
