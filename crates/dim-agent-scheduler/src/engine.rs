//! Scheduled and manual runs (design §3.2, A.4–A.6): windows, acceptance, the
//! pre-effect recheck, launch or reuse of a Session, the first-reuse binding,
//! input, tracking to a terminal outcome, and the steer report of a tool-saved
//! job's outcome. Retention and cancellation on unload are not implemented yet.
use crate::{
    document::{Job, Mode, Origin, Schedule, canonical_instant},
    methods::{Binding, Shared, bind_job, host_bound, read_document, session_policy, window_after},
    runs::{self, ReportTo, Run, RunState, SchedulerError, SessionOrigin, stamp},
};
use chrono::{DateTime, Duration, Utc};
use dim_agent_capabilities::settings::SettingsScope;
use dim_agent_types::{AgentEvent, EventKind, PermissionPolicy, PermissionPreset, TurnOutcome};
use dim_app_server::settings::SettingsError;
use rutis::Ctx;
use rutis_bridge::session::call_service;
use serde_json::{Value, json};
use std::{collections::HashSet, sync::Arc, time::Duration as StdDuration};
use tokio_util::sync::CancellationToken;

const POLL: StdDuration = StdDuration::from_secs(1);
const SESSIONS: &str = "sessions";

const MANUAL: &str = "manual";
/// Terminal runs kept per job, newest first (A.7).
const RETAINED_TERMINAL_RUNS: usize = 100;

/// A Session service code that asks the caller to retry later: the Agents index or
/// the capability set is still loading after a restart (`reloading`, `capability reloading`).
fn is_reloading(code: &str) -> bool {
    code.ends_with("reloading")
}

fn is_launched(run: &Run) -> bool {
    run.session_origin == Some(SessionOrigin::Launch)
}
/// A window this far behind the clock is missed, not run (A.4).
const GRACE_SECONDS: i64 = 60;
/// Bounds how many stale windows one tick walks; the latest is recorded.
const MAX_STALE_WINDOWS: usize = 10_000;
const REPORT_GUIDANCE: &str = "Inspect schedule.history for this job.";
/// A run detail set when a first-reuse binding conflicts twice (A.5.1).
const BINDING_CONFLICT: &str = "binding_conflict";

/// Records a run of `window` for the job, unless one exists for that window,
/// and starts it when it is accepted. A missed window is recorded `missed`;
/// an overlapping run or a full host records `skipped`.
async fn record(
    shared: &Arc<Shared>,
    job: &Job,
    window: String,
    missed: bool,
    max: usize,
) -> Result<Option<Run>, SchedulerError> {
    let accepted = job.clone();
    let job = job.clone();
    let run = runs::update(&shared.files, move |history| {
        if window != MANUAL
            && history
                .iter()
                .any(|run| run.job_id == job.id && run.window == window)
        {
            return Ok((false, None));
        }
        let active = history
            .iter()
            .filter(|run| !run.state.is_terminal())
            .count();
        let overlapping = history
            .iter()
            .any(|run| !run.state.is_terminal() && run.job_id == job.id);
        let (state, detail) = if missed {
            (RunState::Missed, "missed")
        } else if overlapping {
            (RunState::Skipped, "overlap")
        } else if active >= max {
            (RunState::Skipped, "capacity")
        } else {
            (RunState::Launching, "")
        };
        let now = Utc::now();
        let run_id = uuid::Uuid::new_v4().to_string();
        let terminal = state.is_terminal();
        let report_to = match (&job.created_by, state) {
            (
                Origin::Tool {
                    session_id,
                    branch_id,
                },
                RunState::Launching,
            ) => Some(ReportTo {
                session_id: session_id.clone(),
                branch_id: branch_id.clone(),
            }),
            _ => None,
        };
        let run = Run {
            run_id: run_id.clone(),
            job_id: job.id.clone(),
            window,
            state,
            deadline_at: (!terminal)
                .then(|| stamp(now + Duration::minutes(i64::from(job.max_run_minutes)))),
            session_id: None,
            session_origin: None,
            policy: None,
            input_event_id: None,
            submission_id: format!("scheduler:{run_id}"),
            report_to,
            created_at: stamp(now),
            ended_at: terminal.then(|| stamp(now)),
            detail: detail.into(),
        };
        run.validate().map_err(SchedulerError::invalid_data)?;
        history.push(run.clone());
        Ok((true, Some(run)))
    })
    .await?;
    if let Some(run) = &run
        && run.state == RunState::Launching
    {
        tokio::spawn(execute(shared.clone(), accepted, run.clone()));
    }
    if run.is_some() {
        tokio::spawn(prune(shared.clone()));
    }
    Ok(run)
}

/// Accepts a manual run of the job (`schedule.run_now`).
pub async fn accept(shared: Arc<Shared>, job_id: &str) -> Result<Run, SchedulerError> {
    let cancel = CancellationToken::new();
    let document = read_document(&shared, &cancel).await?.document;
    let job = document
        .jobs
        .iter()
        .find(|job| job.id == job_id)
        .cloned()
        .ok_or_else(|| SchedulerError::new("schedule_not_found", format!("no job {job_id}")))?;
    let max = document.max_concurrent_runs as usize;
    record(&shared, &job, MANUAL.into(), false, max)
        .await?
        .ok_or_else(|| SchedulerError::invalid_data("a manual run is always recorded"))
}

/// One pass of the timer: starts each enabled job's due window, records a
/// stale window `missed`, and disables an `at` job once its instant has passed.
pub async fn tick(shared: &Arc<Shared>) -> Result<(), SchedulerError> {
    let cancel = CancellationToken::new();
    let document = read_document(shared, &cancel).await?.document;
    let history = runs::read(&shared.files).await?;
    let now = Utc::now();
    let max = document.max_concurrent_runs as usize;
    for job in document.jobs.iter().filter(|job| job.enabled) {
        let Ok(mut next) = job.next_window(window_after(job, &history)) else {
            continue;
        };
        let grace = now - Duration::seconds(GRACE_SECONDS);
        let mut stale = None;
        let mut steps = 0;
        while let Some(window) = next
            && window < grace
            && steps < MAX_STALE_WINDOWS
        {
            stale = Some(window);
            next = job.next_window(window).ok().flatten();
            steps += 1;
        }
        if let Some(window) = stale {
            record(shared, job, stamp(window), true, max).await?;
        }
        if let Some(window) = next
            && window <= now
        {
            record(shared, job, stamp(window), false, max).await?;
        }
        if matches!(job.schedule, Schedule::At(_))
            && next.is_none()
            && !history
                .iter()
                .any(|run| run.job_id == job.id && !run.state.is_terminal())
        {
            disable(shared, &job.id).await?;
        }
    }
    Ok(())
}

/// Disables a job whose `at` instant has passed. A conflicting write is left
/// for the next tick, which repeats the check.
async fn disable(shared: &Arc<Shared>, job_id: &str) -> Result<(), SchedulerError> {
    let cancel = CancellationToken::new();
    let snapshot = read_document(shared, &cancel).await?;
    let mut document = snapshot.document;
    let Some(job) = document
        .jobs
        .iter_mut()
        .find(|job| job.id == job_id && job.enabled)
    else {
        return Ok(());
    };
    job.enabled = false;
    job.enabled_at = None;
    job.revision += 1;
    match shared
        .settings
        .update(SettingsScope::User, &snapshot.revision, document, &cancel)
        .await
    {
        Ok(_) | Err(SettingsError::Conflict) => Ok(()),
        Err(error) => Err(SchedulerError::new(
            "service_unavailable",
            error.to_string(),
        )),
    }
}

/// The authority of a run's launch request: the saved preset, capped by the
/// saved authority (§3.3). A management job without a preset requests
/// read-only; a tool job requests its saved cap.
fn requested_policy(job: &Job) -> PermissionPolicy {
    match (&job.created_by, job.policy) {
        (_, Some(preset)) => PermissionPolicy {
            preset,
            ..PermissionPolicy::default()
        },
        (Origin::User, None) => PermissionPolicy {
            preset: PermissionPreset::ReadOnly,
            ..PermissionPolicy::default()
        },
        (Origin::Tool { .. }, None) => job.policy_cap.clone(),
    }
}

/// Writes one run's change under the file lock.
async fn update_run(
    shared: &Shared,
    run_id: &str,
    change: impl FnOnce(&mut Run) + Send + 'static,
) -> Result<(), SchedulerError> {
    let run_id = run_id.to_owned();
    runs::update(&shared.files, move |history| {
        let run = history
            .iter_mut()
            .find(|run| run.run_id == run_id)
            .ok_or_else(|| SchedulerError::invalid_data(format!("run {run_id} is missing")))?;
        change(run);
        run.validate().map_err(SchedulerError::invalid_data)?;
        Ok((true, ()))
    })
    .await
}

async fn record_session(
    shared: &Shared,
    run_id: &str,
    session_id: String,
    origin: SessionOrigin,
    policy: PermissionPolicy,
) -> Result<(), SchedulerError> {
    update_run(shared, run_id, move |run| {
        run.session_id = Some(session_id);
        run.session_origin = Some(origin);
        run.policy = Some(policy);
    })
    .await
}

/// Settles a run to a terminal state once, then reports it when it has a
/// report target. A run that already settled keeps its outcome.
async fn finish(
    shared: &Arc<Shared>,
    run_id: &str,
    state: RunState,
    detail: String,
) -> Result<(), SchedulerError> {
    let run_id = run_id.to_owned();
    let settled = runs::update(&shared.files, move |history| {
        let run = history
            .iter_mut()
            .find(|run| run.run_id == run_id)
            .ok_or_else(|| SchedulerError::invalid_data(format!("run {run_id} is missing")))?;
        if run.state.is_terminal() {
            return Ok((false, None));
        }
        run.state = state;
        run.ended_at = Some(stamp(Utc::now()));
        run.detail = if run.detail == BINDING_CONFLICT {
            if detail.is_empty() {
                BINDING_CONFLICT.to_owned()
            } else {
                format!("{BINDING_CONFLICT}; {detail}")
            }
        } else {
            detail
        };
        run.validate().map_err(SchedulerError::invalid_data)?;
        Ok((true, Some(run.clone())))
    })
    .await?;
    match settled {
        Some(run) => {
            let outcome = report(shared, run).await;
            tokio::spawn(prune(shared.clone()));
            outcome
        }
        None => Ok(()),
    }
}

/// Steers a tool-saved run's outcome into the branch that saved the job, once.
/// A failed send is recorded in the run's detail and is not retried.
async fn report(shared: &Arc<Shared>, run: Run) -> Result<(), SchedulerError> {
    let Some(target) = run.report_to.clone() else {
        return Ok(());
    };
    let mut body = format!(
        "Scheduled job {}, run {}: {} ({}).",
        run.job_id,
        run.run_id,
        run.state.name(),
        run.detail
    );
    if let Some(session_id) = &run.session_id {
        body += &format!(" Session {session_id}.");
    }
    body += &format!("\n{REPORT_GUIDANCE}");
    let text = format!(
        "<dim-scheduler kind=\"report\" job=\"{}\" run=\"{}\">{}</dim-scheduler>",
        escaped(&run.job_id),
        escaped(&run.run_id),
        escaped(&body)
    );
    let args = json!({
        "sessionId": target.session_id,
        "branchId": target.branch_id,
        "mode": "steer",
        "submissionId": format!("scheduler-report:{}", run.run_id),
        "rawBlocks": [{"text": text}],
        "content": [],
    });
    let failure = match sessions(&shared.ctx, "input", args).await {
        Err(_) => "bridge_error".to_owned(),
        Ok(Err(code)) => code,
        Ok(Ok(_)) => return Ok(()),
    };
    let run_id = run.run_id.clone();
    update_run(shared, &run_id, move |run| {
        run.detail += &format!("; report_failed: {failure}");
    })
    .await
}

/// Retention (A.7): keeps the newest terminal runs of each job, every active run,
/// and the launch record of each Session that a kept run or a reuse job still
/// references. Expired launch-owned Sessions are deleted first, and their records
/// stay when the delete fails, so the next pass retries.
pub async fn prune(shared: Arc<Shared>) {
    let _pass = shared.pruning.lock().await;
    if let Err(error) = prune_once(&shared).await {
        tracing::warn!(%error, "scheduler retention failed");
    }
}

async fn prune_once(shared: &Arc<Shared>) -> Result<(), SchedulerError> {
    let cancel = CancellationToken::new();
    let document = read_document(shared, &cancel).await?.document;
    let runs = runs::read(&shared.files).await?;
    let mut keep: HashSet<String> = runs
        .iter()
        .filter(|run| !run.state.is_terminal())
        .map(|run| run.run_id.clone())
        .collect();
    for job in &document.jobs {
        let mut own: Vec<&Run> = runs
            .iter()
            .filter(|run| run.job_id == job.id && run.state.is_terminal())
            .collect();
        own.sort_by(|a, b| b.ended_at.cmp(&a.ended_at).then(b.run_id.cmp(&a.run_id)));
        keep.extend(
            own.iter()
                .take(RETAINED_TERMINAL_RUNS)
                .map(|run| run.run_id.clone()),
        );
    }
    let mut references: HashSet<String> = document
        .jobs
        .iter()
        .filter(|job| job.mode == Mode::Reuse)
        .filter_map(|job| job.session_id.clone())
        .collect();
    references.extend(
        runs.iter()
            .filter(|run| keep.contains(&run.run_id))
            .filter_map(|run| run.session_id.clone()),
    );
    for session_id in &references {
        let owner = runs
            .iter()
            .filter(|run| {
                run.session_id.as_deref() == Some(session_id)
                    && run.session_origin == Some(SessionOrigin::Launch)
            })
            .max_by(|a, b| {
                a.created_at
                    .cmp(&b.created_at)
                    .then(a.run_id.cmp(&b.run_id))
            });
        if let Some(owner) = owner {
            keep.insert(owner.run_id.clone());
        }
    }
    let expired: Vec<&Run> = runs
        .iter()
        .filter(|run| !keep.contains(&run.run_id))
        .collect();
    let deletions: HashSet<String> = expired
        .iter()
        .filter(|run| is_launched(run))
        .filter_map(|run| run.session_id.clone())
        .filter(|session_id| !references.contains(session_id))
        .collect();
    for session_id in deletions {
        match sessions(&shared.ctx, "delete", json!({"sessionId": session_id})).await {
            Ok(Ok(_)) => {}
            _ => keep.extend(
                expired
                    .iter()
                    .filter(|run| {
                        is_launched(run) && run.session_id.as_deref() == Some(session_id.as_str())
                    })
                    .map(|run| run.run_id.clone()),
            ),
        }
    }
    let remove: HashSet<String> = runs
        .iter()
        .filter(|run| !keep.contains(&run.run_id))
        .map(|run| run.run_id.clone())
        .collect();
    if remove.is_empty() {
        return Ok(());
    }
    runs::update(&shared.files, move |history| {
        let before = history.len();
        history.retain(|run| !remove.contains(&run.run_id));
        Ok((history.len() != before, ()))
    })
    .await
}

/// Starts a tracker for each run that was active when the previous process
/// stopped (A.5). Nothing is launched or submitted again.
pub fn recover(shared: &Arc<Shared>, active: Vec<Run>) {
    for run in active {
        let shared = shared.clone();
        tokio::spawn(async move {
            if let Err(error) = resume(&shared, &run).await {
                let _ = finish(&shared, &run.run_id, RunState::Lost, error.message).await;
            }
        });
    }
}

/// A `launching` run with a Session id is looked up by its submission id: an
/// accepted input resumes tracking, an unknown one is `lost`. A `launching` run
/// without a Session id is `lost`, because its launch cannot be confirmed.
async fn resume(shared: &Arc<Shared>, run: &Run) -> Result<(), SchedulerError> {
    let deadline = run
        .deadline_at
        .as_deref()
        .and_then(canonical_instant)
        .ok_or_else(|| SchedulerError::invalid_data("active run has no deadline"))?;
    let Some(session_id) = run.session_id.clone() else {
        return finish(
            shared,
            &run.run_id,
            RunState::Lost,
            "no_session_after_restart".into(),
        )
        .await;
    };
    let input_id = match &run.input_event_id {
        Some(input_id) => input_id.clone(),
        None => match lookup_input(shared, &session_id, &run.submission_id).await {
            Ok(Some(input_id)) => {
                let accepted = input_id.clone();
                update_run(shared, &run.run_id, move |run| {
                    run.state = RunState::Submitted;
                    run.input_event_id = Some(accepted);
                })
                .await?;
                input_id
            }
            Ok(None) => {
                return finish(shared, &run.run_id, RunState::Lost, "input_unknown".into()).await;
            }
            Err(code) => {
                return finish(
                    shared,
                    &run.run_id,
                    RunState::Lost,
                    format!("lookup_failed: {code}"),
                )
                .await;
            }
        },
    };
    track(shared, &run.run_id, &session_id, &input_id, deadline).await
}

/// `Ok(None)` when the Session never accepted the submission. A service error
/// other than `reloading` is returned as its code; a transport failure or a
/// reloading Session is retried until it answers.
async fn lookup_input(
    shared: &Arc<Shared>,
    session_id: &str,
    submission_id: &str,
) -> Result<Option<String>, String> {
    let args = json!({"sessionId": session_id, "branchId": "main", "submissionId": submission_id});
    loop {
        match sessions(&shared.ctx, "lookup", args.clone()).await {
            Ok(Ok(value)) => {
                return match value["status"].as_str() {
                    Some("accepted") => value["inputId"]
                        .as_str()
                        .map(|input_id| Some(input_id.to_owned()))
                        .ok_or_else(|| "invalid_data".to_owned()),
                    _ => Ok(None),
                };
            }
            Ok(Err(code)) if !is_reloading(&code) => return Err(code),
            Ok(Err(_)) | Err(_) => tokio::time::sleep(POLL).await,
        }
    }
}

/// A `sessions` call. The outer error is a transport failure, whose outcome
/// is unknown; the inner error is the service's own code.
async fn sessions(ctx: &Ctx, method: &str, args: Value) -> Result<Result<Value, String>, String> {
    let envelope: Value = call_service(ctx, SESSIONS, method, (args,))
        .await
        .map_err(|error| error.to_string())?;
    if envelope["ok"] == json!(true) {
        Ok(Ok(envelope["value"].clone()))
    } else {
        Ok(Err(envelope["error"]["code"]
            .as_str()
            .unwrap_or("unknown")
            .to_owned()))
    }
}

fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

async fn execute(shared: Arc<Shared>, job: Job, run: Run) {
    if let Err(error) = drive(&shared, &job, &run).await {
        let _ = finish(&shared, &run.run_id, RunState::Lost, error.message).await;
    }
}

/// The pre-effect recheck (A.5): the accepted job is still enabled at
/// `revision` with `binding`, and the run's deadline has not passed. A failed
/// check settles the run and returns `false`.
async fn still_eligible(
    shared: &Arc<Shared>,
    job: &Job,
    run: &Run,
    revision: u64,
    binding: Option<&str>,
) -> Result<bool, SchedulerError> {
    let cancel = CancellationToken::new();
    let document = match read_document(shared, &cancel).await {
        Ok(snapshot) => snapshot.document,
        Err(_) => {
            finish(
                shared,
                &run.run_id,
                RunState::Skipped,
                "settings_read_failed".into(),
            )
            .await?;
            return Ok(false);
        }
    };
    let deadline = run
        .deadline_at
        .as_deref()
        .and_then(canonical_instant)
        .ok_or_else(|| SchedulerError::invalid_data("accepted run has no deadline"))?;
    if Utc::now() >= deadline {
        finish(
            shared,
            &run.run_id,
            RunState::TimedOut,
            "deadline_before_input".into(),
        )
        .await?;
        return Ok(false);
    }
    let current = document.jobs.iter().find(|current| current.id == job.id);
    if let Some(current) = current
        && current.enabled
        && current.revision == revision
        && current.mode == job.mode
        && current.session_id.as_deref() == binding
    {
        return Ok(true);
    }
    let reason = if current.is_some_and(|current| !current.enabled) {
        "disabled"
    } else {
        "changed"
    };
    finish(shared, &run.run_id, RunState::Skipped, reason.into()).await?;
    Ok(false)
}

/// Settles the run with `state` unless the recheck already settled it.
async fn settle_checked(
    shared: &Arc<Shared>,
    job: &Job,
    run: &Run,
    state: RunState,
    detail: &str,
) -> Result<(), SchedulerError> {
    if still_eligible(shared, job, run, job.revision, job.session_id.as_deref()).await? {
        finish(shared, &run.run_id, state, detail.into()).await?;
    }
    Ok(())
}

/// Launches the job's Session with the requested authority and records it on
/// the run before any input. `None` means the run settled.
async fn launch(
    shared: &Arc<Shared>,
    job: &Job,
    run: &Run,
    requested: &PermissionPolicy,
) -> Result<Option<String>, SchedulerError> {
    let mut args = json!({"cwd": job.cwd, "policy": requested});
    if let Some(agent) = &job.agent_id {
        args["agent"] = json!(agent);
    }
    if let Some(model) = &job.model {
        args["model"] = json!({"providerId": model.provider_id, "modelId": model.model_id});
    }
    if let Origin::Tool {
        session_id,
        branch_id,
    } = &job.created_by
    {
        args["starter"] = json!({"sessionId": session_id, "branchId": branch_id});
    }
    let launched = match sessions(&shared.ctx, "launch", args).await {
        Err(_) => {
            finish(
                shared,
                &run.run_id,
                RunState::Lost,
                "launch_reply_unknown".into(),
            )
            .await?;
            return Ok(None);
        }
        Ok(Err(code)) => {
            finish(shared, &run.run_id, RunState::Failed, code).await?;
            return Ok(None);
        }
        Ok(Ok(value)) => value,
    };
    let session_id = launched["sessionId"]
        .as_str()
        .ok_or_else(|| SchedulerError::invalid_data("launch reply has no sessionId"))?
        .to_owned();
    let effective: PermissionPolicy = serde_json::from_value(launched["policy"].clone())
        .map_err(|error| SchedulerError::invalid_data(error.to_string()))?;
    record_session(
        shared,
        &run.run_id,
        session_id.clone(),
        SessionOrigin::Launch,
        effective,
    )
    .await?;
    Ok(Some(session_id))
}

/// The live authority of the job's saving origin: the host bound for a
/// management save, or the saving branch's current policy for a tool save.
async fn live_policy(shared: &Shared, job: &Job) -> Result<PermissionPolicy, SchedulerError> {
    match &job.created_by {
        Origin::User => host_bound(&shared.ctx).await,
        Origin::Tool {
            session_id,
            branch_id,
        } => session_policy(&shared.ctx, session_id, branch_id).await,
    }
}

/// Uses the job's bound Session when its live authority is inside both the
/// requested policy and the live cap. `None` means the run settled.
async fn reuse(
    shared: &Arc<Shared>,
    job: &Job,
    run: &Run,
    requested: &PermissionPolicy,
    bound: &str,
) -> Result<bool, SchedulerError> {
    let live = match live_policy(shared, job).await {
        Ok(live) => live,
        Err(_) => {
            settle_checked(shared, job, run, RunState::Skipped, "policy_read_failed").await?;
            return Ok(false);
        }
    };
    let actual = match session_policy(&shared.ctx, bound, "main").await {
        Ok(actual) => actual,
        Err(_) => {
            settle_checked(shared, job, run, RunState::Skipped, "policy_read_failed").await?;
            return Ok(false);
        }
    };
    if !actual.within(requested) || !actual.within(&live) {
        settle_checked(shared, job, run, RunState::Skipped, "policy").await?;
        return Ok(false);
    }
    record_session(
        shared,
        &run.run_id,
        bound.to_owned(),
        SessionOrigin::Reuse,
        actual,
    )
    .await?;
    Ok(true)
}

enum Bound {
    Revision(u64),
    Conflict,
}

/// The first-reuse binding (A.5.1): one retry after a stale-document conflict.
/// A second conflict continues the run unbound with `binding_conflict` in its
/// detail. `None` means the run settled.
async fn bind(
    shared: &Arc<Shared>,
    job: &Job,
    run: &Run,
    session_id: &str,
) -> Result<Option<Bound>, SchedulerError> {
    for attempt in 0..2 {
        match bind_job(shared, job, session_id).await {
            Ok(Binding::Bound(revision)) => return Ok(Some(Bound::Revision(revision))),
            Ok(Binding::Changed(reason)) => {
                finish(shared, &run.run_id, RunState::Skipped, reason.into()).await?;
                return Ok(None);
            }
            Err(error) if error.code == "schedule_conflict" && attempt == 0 => continue,
            Err(error) if error.code == "schedule_conflict" => {
                update_run(shared, &run.run_id, |run| {
                    run.detail = BINDING_CONFLICT.into();
                })
                .await?;
                return Ok(Some(Bound::Conflict));
            }
            Err(error) if error.code == "schedule_invalid" => {
                finish(
                    shared,
                    &run.run_id,
                    RunState::Failed,
                    "binding_rejected: schedule_invalid".into(),
                )
                .await?;
                return Ok(None);
            }
            Err(_) => {
                finish(
                    shared,
                    &run.run_id,
                    RunState::Skipped,
                    "binding_unconfirmed".into(),
                )
                .await?;
                return Ok(None);
            }
        }
    }
    Ok(None)
}

async fn drive(shared: &Arc<Shared>, job: &Job, run: &Run) -> Result<(), SchedulerError> {
    if !still_eligible(shared, job, run, job.revision, job.session_id.as_deref()).await? {
        return Ok(());
    }
    let requested = requested_policy(job).cap(&job.policy_cap);
    let session_id = match (&job.mode, &job.session_id) {
        (Mode::Reuse, Some(bound)) => {
            if !reuse(shared, job, run, &requested, bound).await? {
                return Ok(());
            }
            bound.clone()
        }
        _ => match launch(shared, job, run, &requested).await? {
            Some(session_id) => session_id,
            None => return Ok(()),
        },
    };
    let (revision, binding) = match (&job.mode, &job.session_id) {
        (Mode::Reuse, None) => match bind(shared, job, run, &session_id).await? {
            Some(Bound::Revision(revision)) => (revision, Some(session_id.clone())),
            Some(Bound::Conflict) => (job.revision, None),
            None => return Ok(()),
        },
        _ => (job.revision, job.session_id.clone()),
    };
    if !still_eligible(shared, job, run, revision, binding.as_deref()).await? {
        return Ok(());
    }

    let marker = format!(
        "<dim-scheduler kind=\"scheduled-run\" job=\"{}\" run=\"{}\">{}</dim-scheduler>",
        escaped(&job.id),
        escaped(&run.run_id),
        escaped(&job.name)
    );
    let input = json!({
        "sessionId": session_id,
        "branchId": "main",
        "mode": "queue",
        "submissionId": run.submission_id,
        "rawBlocks": [{"text": marker}],
        "content": [{"type": "text", "text": job.prompt}],
    });
    let input_id = match sessions(&shared.ctx, "input", input).await {
        Ok(Ok(receipt)) => receipt["inputId"]
            .as_str()
            .ok_or_else(|| SchedulerError::invalid_data("input reply has no inputId"))?
            .to_owned(),
        Ok(Err(code))
            if !is_reloading(&code)
                && !matches!(code.as_str(), "publication_unknown" | "ambiguous") =>
        {
            return finish(shared, &run.run_id, RunState::Failed, code).await;
        }
        Ok(Err(_)) | Err(_) => match lookup_input(shared, &session_id, &run.submission_id).await {
            Ok(Some(input_id)) => input_id,
            Ok(None) => {
                return finish(shared, &run.run_id, RunState::Lost, "input_unknown".into()).await;
            }
            Err(code) => {
                return finish(
                    shared,
                    &run.run_id,
                    RunState::Lost,
                    format!("lookup_failed: {code}"),
                )
                .await;
            }
        },
    };
    {
        let input_id = input_id.clone();
        update_run(shared, &run.run_id, move |run| {
            run.state = RunState::Submitted;
            run.input_event_id = Some(input_id);
        })
        .await?;
    }
    let deadline = run
        .deadline_at
        .as_deref()
        .and_then(canonical_instant)
        .ok_or_else(|| SchedulerError::invalid_data("accepted run has no deadline"))?;
    track(shared, &run.run_id, &session_id, &input_id, deadline).await
}

/// Follows the run's Session history until its first turn ends after the
/// input is consumed. At the deadline an unconsumed input is withdrawn, and a
/// consumed one is stopped; either settles `timed_out`.
async fn track(
    shared: &Arc<Shared>,
    run_id: &str,
    session_id: &str,
    input_id: &str,
    deadline: DateTime<Utc>,
) -> Result<(), SchedulerError> {
    let mut after: Option<u64> = None;
    let mut consumed = false;
    let mut timing_out = false;
    loop {
        let mut args = json!({"sessionId": session_id, "branchId": "main"});
        if let Some(after) = after {
            args["after"] = json!(after);
        }
        let page = match sessions(&shared.ctx, "history", args).await {
            Ok(Err(code)) if is_reloading(&code) => {
                tokio::time::sleep(POLL).await;
                continue;
            }
            Err(_) => {
                tokio::time::sleep(POLL).await;
                continue;
            }
            Ok(Err(code)) => return finish(shared, run_id, RunState::Failed, code).await,
            Ok(Ok(value)) => value,
        };
        let events: Vec<AgentEvent> = serde_json::from_value(page["events"].clone())
            .map_err(|error| SchedulerError::invalid_data(error.to_string()))?;
        for event in events {
            match event.kind {
                EventKind::InputConsumed(data) if data.input_id == input_id => consumed = true,
                EventKind::TurnEnd(end) if consumed => {
                    let (state, detail) = if timing_out {
                        (RunState::TimedOut, "timed_out".to_owned())
                    } else {
                        match end.outcome {
                            TurnOutcome::Completed { .. } => {
                                (RunState::Completed, "completed".into())
                            }
                            TurnOutcome::Failed { error } => (RunState::Failed, error),
                            TurnOutcome::Cancelled { reason } => (RunState::Cancelled, reason),
                        }
                    };
                    return finish(shared, run_id, state, detail).await;
                }
                _ => {}
            }
        }
        after = page["through"].as_u64().or(after);
        if Utc::now() >= deadline && !timing_out {
            timing_out = true;
            if !consumed {
                let withdrawn = sessions(
                    &shared.ctx,
                    "withdraw",
                    json!({"sessionId": session_id, "branchId": "main", "inputId": input_id,
                        "reason": "scheduled run timed out"}),
                )
                .await;
                if let Ok(Ok(value)) = withdrawn
                    && value["status"] == json!("deleted")
                {
                    return finish(
                        shared,
                        run_id,
                        RunState::TimedOut,
                        "withdrawn_at_deadline".into(),
                    )
                    .await;
                }
            } else {
                let _ = sessions(
                    &shared.ctx,
                    "stop",
                    json!({"sessionId": session_id, "branchId": "main", "reason": "scheduled run timed out"}),
                )
                .await;
            }
        }
        tokio::time::sleep(POLL).await;
    }
}
