use crate::{
    context_preparation::{ContextOpError, ContextUsage as UsageRead},
    history::HistoryRenderer,
    host::{ContextContribution as Contribution, ContextModel, ContextModelHost, HostError},
};
use dim_agent_runtime::{
    AgentError, BranchLifecycle, Dispatcher, Names, QueryFilter, QueryOperand, QueryOutput,
    QueryPage, QueryPick, QuerySelect, QuerySpec, QueryValue, Result, Timeline, TimelineQuery,
    TimelineQueryScope,
};
use dim_agent_types::*;
use futures_util::FutureExt;
use rutis::{
    Ctx,
    host::{BoxFuture, CtxHost, DynPlugin, Effect, Key as ServiceKey, TypeKey},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    num::NonZeroU64,
    panic::AssertUnwindSafe,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::{Notify, watch};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

mod numeric;
mod sources;
use numeric::{Entry, Numeric, Reference, Summary};

pub(crate) struct UsageEvidence(sources::RequestBase);

const RESULT: &str = "context-usage/recovered";
const SCHEMA: &str = "context-usage/v1";
const SOURCES: &[&str] = &[
    "context/configured",
    "input/consumed",
    "step/model-output-end",
    "step/tool-result",
    "context/compaction",
    "model/usage",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Key {
    branch: u32,
    sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Limits {
    window: f64,
    output: f64,
    threshold: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CounterConfiguration {
    event_id: String,
    request_identity: String,
    algorithm: String,
    fixed: Contribution,
    limits: Limits,
}

impl CounterConfiguration {
    fn identity(&self) -> Result<String> {
        fingerprint(&(&self.request_identity, &self.limits))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum Configuration {
    Ready { counter: Box<CounterConfiguration> },
    Unsupported { event_id: String },
    Unavailable { event_id: String, message: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Compression {
    event_id: String,
    cut: Key,
    removed: Contribution,
    replacement: Contribution,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Calibration {
    request_id: String,
    request_at: Key,
    anchor_at: Key,
    configuration: String,
    compression: Option<String>,
    local: f64,
    prompt_tokens: f64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    branches: Vec<String>,
    initialized: Key,
    root: Option<Reference>,
}

impl State {
    fn key(&self, position: &TimelinePosition) -> Result<Key> {
        let branch = self
            .branches
            .iter()
            .position(|branch| *branch == position.branch_id)
            .ok_or_else(|| fault("source is outside the numeric branch chain"))?;
        Ok(Key {
            branch: branch as u32,
            sequence: position.sequence,
        })
    }

    fn at(&self, sequence: u64) -> Key {
        Key {
            branch: self.branches.len() as u32 - 1,
            sequence,
        }
    }

    fn summary(&self) -> Summary {
        self.root
            .as_ref()
            .map_or_else(Summary::default, |root| root.summary.clone())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Recovered {
    schema: String,
    source_through: u64,
    state: State,
}

struct Machine {
    state: State,
    source_through: u64,
    result: u64,
    model: Option<Arc<dyn ContextModel>>,
    numeric: Numeric,
}

#[derive(Clone, Default)]
struct Progress {
    state: Option<Arc<State>>,
    covered: u64,
    result: u64,
    error: Option<AgentError>,
}

pub(crate) struct ContextUsage {
    timeline: Arc<Timeline>,
    resolver: Arc<ContextModelHost>,
    history: Arc<HistoryRenderer>,
    dispatcher: Arc<Dispatcher>,
    branch: Arc<BranchLifecycle>,
    wanted: AtomicU64,
    wake: Notify,
    progress: watch::Sender<Progress>,
    tasks: TaskTracker,
    cancel: CancellationToken,
    errors: Mutex<Vec<AgentError>>,
}

fn fault(message: impl Into<String>) -> AgentError {
    AgentError::fault("ContextUsage", message)
}

fn add(left: Contribution, right: Contribution) -> Result<Contribution> {
    Ok(Contribution {
        serialized_bytes: left
            .serialized_bytes
            .checked_add(right.serialized_bytes)
            .ok_or_else(|| fault("numeric byte contribution overflow"))?,
        images: left
            .images
            .checked_add(right.images)
            .ok_or_else(|| fault("numeric image contribution overflow"))?,
    })
}

fn subtract(left: Contribution, right: Contribution) -> Result<Contribution> {
    Ok(Contribution {
        serialized_bytes: left
            .serialized_bytes
            .checked_sub(right.serialized_bytes)
            .ok_or_else(|| fault("numeric byte prefix exceeds its total"))?,
        images: left
            .images
            .checked_sub(right.images)
            .ok_or_else(|| fault("numeric image prefix exceeds its total"))?,
    })
}

fn total(summary: &Summary) -> Result<Contribution> {
    let messages = match &summary.compression {
        Some(compression) => add(
            subtract(summary.ordinary, compression.removed)?,
            compression.replacement,
        )?,
        None => summary.ordinary,
    };
    match &summary.configuration {
        Some(Configuration::Ready { counter }) => add(counter.fixed, messages),
        _ => Ok(messages),
    }
}

fn fingerprint<T: Serialize>(value: &T) -> Result<String> {
    let mut hash = Sha256::new();
    serde_json::to_writer(&mut hash, value).map_err(|error| fault(error.to_string()))?;
    Ok(format!("{:x}", hash.finalize()))
}

fn request(
    scope: TimelineQueryScope,
    through: u64,
    kinds: &[&str],
    fields: &[&str],
) -> TimelineQuery {
    TimelineQuery {
        scope,
        from: None,
        through: Some(through),
        include_publishing: false,
        query: QuerySpec {
            rows: None,
            filter: Some(QueryFilter::In {
                value: QueryOperand::Field {
                    path: "kind".into(),
                },
                values: kinds.iter().map(|kind| (*kind).into()).collect(),
            }),
            select: QuerySelect::Fields {
                fields: fields.iter().map(|field| (*field).into()).collect(),
            },
            group_by: vec![],
            pick: None,
            order_by: vec![],
        },
        page: Some(QueryPage {
            limit: NonZeroU64::new(128).unwrap(),
            after: None,
        }),
    }
}

fn reading(through: u64, summary: &Summary) -> std::result::Result<UsageRead, ContextOpError> {
    let configuration = match &summary.configuration {
        Some(Configuration::Ready { counter }) => counter,
        Some(Configuration::Unsupported { .. }) => {
            return Err(ContextOpError::Model(HostError::failed(
                "The context model does not support persistent usage statistics",
            )));
        }
        Some(Configuration::Unavailable { message, .. }) => {
            return Err(ContextOpError::Model(HostError::failed(message)));
        }
        None => return Err(ContextOpError::NotConfigured),
    };
    let local = summary
        .local
        .ok_or_else(|| ContextOpError::Read(fault("missing local estimate")))?;
    let identity = configuration.identity().map_err(ContextOpError::Read)?;
    let compatible = summary.calibration.as_ref().filter(|baseline| {
        baseline.configuration == identity
            && baseline.compression.as_deref()
                == summary
                    .compression
                    .as_ref()
                    .map(|compression| compression.event_id.as_str())
    });
    let (input, source) = match compatible {
        Some(baseline) => (
            baseline.prompt_tokens + local - baseline.local,
            BaselineSource::ProviderBaseline,
        ),
        None => (local, BaselineSource::FullEstimate),
    };
    if !input.is_finite() || input < 0. {
        return Err(ContextOpError::Read(fault(
            "invalid calibrated input estimate",
        )));
    }
    Ok(UsageRead {
        through,
        context_window: configuration.limits.window,
        output_tokens: configuration.limits.output,
        threshold_tokens: configuration.limits.threshold,
        estimated_input_tokens: input,
        baseline_source: source,
    })
}

impl ContextUsage {
    pub(crate) async fn reference(
        &self,
        request_id: &str,
        through: u64,
        request_value: &ModelRequest,
        compaction: Option<&str>,
        cancel: CancellationToken,
    ) -> std::result::Result<Option<UsageEvidence>, HostError> {
        let view = match self.at(through, cancel).await {
            Ok(view) => view,
            Err(ContextOpError::Model(_) | ContextOpError::NotConfigured) => return Ok(None),
            Err(ContextOpError::Read(error)) => return Err(HostError::failed(error.to_string())),
        };
        let state = view.state.expect("ready usage state");
        if state.at(through) < state.initialized {
            return Ok(None);
        }
        let numeric = Numeric::new(self.timeline.clone(), view.result);
        let summary = numeric
            .prefix(state.root.as_ref(), state.at(through), true)
            .await
            .map_err(|error| HostError::failed(error.to_string()))?;
        let Some(Configuration::Ready { counter }) = summary.configuration else {
            return Ok(None);
        };
        if compaction.is_some_and(|id| {
            summary
                .compression
                .as_ref()
                .is_none_or(|current| current.event_id != id)
        }) {
            return Ok(None);
        }
        let control = ModelRequest {
            provider_id: request_value.provider_id.clone(),
            model_id: request_value.model_id.clone(),
            capability_snapshot: request_value.capability_snapshot.clone(),
            messages: Vec::new(),
            tools: None,
            system: None,
            config: request_value.config.clone(),
            metadata: request_value.metadata.clone(),
        };
        let identity = fingerprint(&(&request_value.system, &request_value.tools))
            .and_then(|fixed| fingerprint(&(&control, fixed, &counter.algorithm)))
            .map_err(|error| HostError::failed(error.to_string()))?;
        if identity != counter.request_identity {
            return Ok(None);
        }
        Ok(Some(UsageEvidence(sources::RequestBase {
            schema: "context-usage/request-base/v1".into(),
            request_id: request_id.into(),
            source: TimelinePosition {
                branch_id: self.timeline.branch_id(),
                sequence: through,
            },
            algorithm: counter.algorithm.clone(),
            configuration: counter
                .identity()
                .map_err(|error| HostError::failed(error.to_string()))?,
            compression: summary.compression.map(|compression| compression.event_id),
            local: summary
                .local
                .ok_or_else(|| HostError::failed("Missing request local estimate"))?,
            root: state.root.clone(),
        })))
    }

    pub(crate) async fn confirm(
        &self,
        evidence: UsageEvidence,
        cancel: CancellationToken,
    ) -> std::result::Result<(), HostError> {
        let data = serde_json::to_value(evidence.0)
            .map_err(|error| HostError::failed(error.to_string()))?;
        let event = self
            .timeline
            .append(
                EventKind::Extension(
                    ExtensionEvent::new("context-usage", "request-base", data)
                        .expect("usage request base key"),
                )
                .into(),
            )
            .map_err(|error| HostError::failed(error.to_string()))?;
        tokio::select! {
            result = self.publication(event.sequence) => result.map_err(|error| HostError::failed(error.to_string())),
            _ = cancel.cancelled() => Err(HostError::Cancelled("Request base confirmation cancelled".into())),
        }
    }

    pub(crate) async fn read(
        &self,
        through: Option<u64>,
        cancel: CancellationToken,
    ) -> std::result::Result<UsageRead, ContextOpError> {
        let through = match through {
            Some(through) => through,
            None => {
                self.at(0, cancel.clone()).await?;
                self.timeline.sequence()
            }
        };
        let view = self.at(through, cancel).await?;
        let state = view.state.expect("ready usage state");
        let end = state.at(through);
        if end < state.initialized {
            return Err(ContextOpError::Model(HostError::failed(
                "Usage statistics were initialized after this request boundary",
            )));
        }
        let numeric = Numeric::new(self.timeline.clone(), view.result);
        let summary = numeric
            .prefix(state.root.as_ref(), end, true)
            .await
            .map_err(ContextOpError::Read)?;
        reading(through, &summary)
    }

    async fn at(
        &self,
        through: u64,
        cancel: CancellationToken,
    ) -> std::result::Result<Progress, ContextOpError> {
        if self.resolver.0.is_none() {
            return Err(ContextOpError::Model(HostError::failed(
                "Context compaction is not available",
            )));
        }
        if through > self.timeline.sequence() {
            return Err(ContextOpError::Read(AgentError::InvalidInput(
                "future usage boundary".into(),
            )));
        }
        self.wanted.fetch_max(through, Ordering::SeqCst);
        self.wake.notify_one();
        let mut progress = self.progress.subscribe();
        loop {
            let view = progress.borrow_and_update().clone();
            if let Some(error) = &view.error {
                return Err(ContextOpError::Read(error.clone()));
            }
            if view.state.is_some() && view.covered >= through {
                return Ok(view);
            }
            tokio::select! {
                changed = progress.changed() => changed.map_err(|_| ContextOpError::Read(AgentError::Closed("ContextUsage".into())))?,
                _ = cancel.cancelled() => return Err(ContextOpError::Model(HostError::Cancelled("Usage read cancelled".into()))),
                _ = self.cancel.cancelled() => return Err(ContextOpError::Read(AgentError::Closed("ContextUsage".into()))),
                _ = self.branch.closing() => return Err(ContextOpError::Read(AgentError::Closed("ContextUsage".into()))),
            }
        }
    }

    async fn publication(&self, sequence: u64) -> Result<()> {
        let mut published = self.timeline.watch();
        while published.borrow_and_update().0 < sequence {
            if let Some(error) = self.timeline.failure() {
                return Err(error);
            }
            tokio::select! {
                changed = published.changed() => changed.map_err(|_| fault("publication channel closed"))?,
                _ = self.cancel.cancelled() => return Err(AgentError::Closed("ContextUsage".into())),
                _ = self.branch.closing() => return Err(AgentError::Closed("ContextUsage".into())),
            }
        }
        Ok(())
    }

    async fn save(&self, machine: &mut Machine) -> Result<()> {
        let mut drafts = machine.numeric.drafts(machine.state.root.as_ref())?;
        drafts.push(
            EventKind::Extension(
                ExtensionEvent::new(
                    "context-usage",
                    "recovered",
                    serde_json::to_value(Recovered {
                        schema: SCHEMA.into(),
                        source_through: machine.source_through,
                        state: machine.state.clone(),
                    })
                    .map_err(|error| fault(error.to_string()))?,
                )
                .expect("usage result key"),
            )
            .into(),
        );
        let written = self.timeline.append_batch_registered(drafts, |_| Ok(()))?;
        machine.result = written.last().expect("usage result").sequence;
        self.publication(machine.result).await?;
        machine.numeric.published(machine.result);
        Ok(())
    }

    fn publish(&self, machine: &Machine, covered: u64) {
        self.progress.send_replace(Progress {
            state: Some(Arc::new(machine.state.clone())),
            covered,
            result: machine.result,
            error: None,
        });
    }
}

pub(crate) struct ContextUsagePlugin {
    usage: ServiceKey<ContextUsage>,
    timeline: ServiceKey<Timeline>,
    resolver: ServiceKey<ContextModelHost>,
    history: ServiceKey<HistoryRenderer>,
    dispatcher: ServiceKey<Dispatcher>,
    branch: ServiceKey<BranchLifecycle>,
}

impl ContextUsagePlugin {
    pub(crate) fn new(names: &Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            usage: names.key()?,
            timeline: names.key()?,
            resolver: names.key()?,
            history: names.key()?,
            dispatcher: names.key()?,
            branch: names.key()?,
        })
    }
}

impl DynPlugin for ContextUsagePlugin {
    fn name(&self) -> &str {
        "ContextUsage"
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![
            self.timeline.erase(),
            self.resolver.erase(),
            self.history.erase(),
            self.dispatcher.erase(),
            self.branch.erase(),
        ]
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let owner = Arc::new(ContextUsage {
                timeline: ctx.require_at(&self.timeline)?,
                resolver: ctx.require_at(&self.resolver)?,
                history: ctx.require_at(&self.history)?,
                dispatcher: ctx.require_at(&self.dispatcher)?,
                branch: ctx.require_at(&self.branch)?,
                wanted: AtomicU64::new(0),
                wake: Notify::new(),
                progress: watch::channel(Progress::default()).0,
                tasks: TaskTracker::new(),
                cancel: ctx.cancellation_token(),
                errors: Mutex::new(Vec::new()),
            });
            let cleanup = owner.clone();
            ctx.effect(move || async move {
                cleanup.cancel.cancel();
                cleanup.tasks.close();
                cleanup.tasks.wait().await;
                AgentError::aggregate(cleanup.errors.lock().unwrap().clone()).map_err(Into::into)
            })?;
            if owner.resolver.0.is_some() {
                let notify = owner.clone();
                owner
                    .dispatcher
                    .subscribe(ctx, "ContextUsage", SOURCES, move |event| {
                        let owner = notify.clone();
                        Box::pin(async move {
                            owner.wanted.fetch_max(event.sequence, Ordering::SeqCst);
                            owner.wake.notify_one();
                            Ok(())
                        })
                    })?;
                let task = owner.clone();
                owner.tasks.spawn(async move {
                    let started = tokio::select! {
                        started = task.branch.activated() => started,
                        _ = task.cancel.cancelled() => false,
                    };
                    if !started {
                        return;
                    }
                    let result = tokio::select! {
                        result = AssertUnwindSafe(task.run()).catch_unwind() => result
                            .map_err(|panic| fault(crate::host::panic_message(panic)))
                            .and_then(|result| result),
                        _ = task.cancel.cancelled() => return,
                        _ = task.branch.closing() => return,
                    };
                    if let Err(error) = result {
                        if matches!(error, AgentError::Closed(_))
                            && (task.cancel.is_cancelled()
                                || task.branch.phase() >= dim_agent_runtime::BranchPhase::Closing)
                        {
                            return;
                        }
                        task.errors.lock().unwrap().push(error.clone());
                        task.progress
                            .send_modify(|progress| progress.error = Some(error.clone()));
                        task.dispatcher.fail(error);
                    }
                });
            }
            ctx.provide_at(self.usage.clone(), owner)?;
            Ok(Effect::Done)
        })
    }
}
