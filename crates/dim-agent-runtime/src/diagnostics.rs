//! Observation only: listener failures never enter the business fault channel.
use crate::{TimelineTrace, core::unix_millis};
use dim_agent_types::AgentEvent;
use rutis::{Ctx, Event, EventKey};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
    time::Instant,
};
use tracing::{
    Dispatch, Span, Subscriber,
    field::{Field, Visit},
    span::{Attributes, Id, Record},
};
use tracing_subscriber::{Layer, layer::Context, prelude::*, registry::LookupSpan};

#[derive(Clone, Debug, Serialize)]
pub struct DiagnosticRecord {
    pub session_id: String,
    pub branch_id: Option<String>,
    pub timestamp: u64,
    #[serde(flatten)]
    pub event: DiagnosticEvent,
}
/// Raw records may contain secrets. Redaction belongs at the archive boundary.
impl Event for DiagnosticRecord {
    const NAME: &'static str = "session.diagnostics";
    type Value = ();
}

/// The rutis channel carrying one Session's diagnostic records.
pub fn diagnostics_channel(session_id: &str) -> EventKey<DiagnosticRecord> {
    EventKey::dynamic(format!("diagnostics/{session_id}"))
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ComponentPhase {
    Ready,
    Released,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionPhase {
    Registered,
    Released,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DiagnosticEvent {
    Component {
        name: String,
        phase: ComponentPhase,
    },
    Subscription {
        name: String,
        id: String,
        keys: Vec<String>,
        phase: SubscriptionPhase,
    },
    Span {
        id: u64,
        phase: SpanPhase,
        fields: BTreeMap<String, String>,
        duration_ms: Option<u64>,
    },
    Timeline {
        event: Arc<AgentEvent>,
        trace: Option<Box<TimelineTrace>>,
    },
    /// The last record of a Session: its cleanup or failed build has finished.
    Closed,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpanPhase {
    Start,
    End,
}

/// Emits one Session's records on its channel once enabled. Records are
/// only built while enabled, so a Session without trace pays nothing.
#[derive(Default)]
struct Emitter {
    host: Option<Ctx>,
    channel: EventKey<DiagnosticRecord>,
    session_id: String,
    dispatch: OnceLock<Dispatch>,
}
impl Emitter {
    fn emit(&self, branch_id: Option<String>, event: DiagnosticEvent) {
        let Some(host) = &self.host else { return };
        let _ = host.events().emit(
            host,
            &self.channel,
            Arc::new(DiagnosticRecord {
                session_id: self.session_id.clone(),
                branch_id,
                timestamp: unix_millis(),
                event,
            }),
        );
    }
}

#[derive(Clone, Default)]
pub struct Diagnostics {
    branch_id: Option<String>,
    emitter: Arc<Emitter>,
}
impl Diagnostics {
    /// Records are emitted with the Agent's host context, which outlives the Session.
    pub(crate) fn session(session_id: String, host: Ctx) -> Self {
        Self {
            branch_id: None,
            emitter: Arc::new(Emitter {
                host: Some(host),
                channel: diagnostics_channel(&session_id),
                session_id,
                dispatch: OnceLock::new(),
            }),
        }
    }
    pub(crate) fn branch(&self, branch_id: String) -> Self {
        Self {
            branch_id: Some(branch_id),
            ..self.clone()
        }
    }
    /// Enables records once, before the main branch is built; listeners
    /// must already be registered on the channel. False when already enabled.
    pub fn enable(&self) -> bool {
        if self.emitter.host.is_none() {
            return false;
        }
        let layer = DiagnosticLayer {
            emitter: Arc::downgrade(&self.emitter),
        };
        self.emitter
            .dispatch
            .set(Dispatch::new(tracing_subscriber::registry().with(layer)))
            .is_ok()
    }
    pub(crate) fn is_enabled(&self) -> bool {
        self.emitter.dispatch.get().is_some()
    }
    pub(crate) fn emit(&self, event: DiagnosticEvent) {
        if self.is_enabled() {
            self.emitter.emit(self.branch_id.clone(), event);
        }
    }
    /// Ends the channel whether or not records were enabled, so a listener
    /// registered before a failed enable still observes the end.
    pub(crate) fn close(&self) {
        self.emitter.emit(None, DiagnosticEvent::Closed);
    }
    /// Ends the channel when dropped, including when Session close panics,
    /// so archives waiting for the end record never hang.
    pub(crate) fn end_on_drop(&self) -> ChannelEnd {
        ChannelEnd(self.clone())
    }
    pub(crate) fn component(&self, name: &str, phase: ComponentPhase) {
        self.emit(DiagnosticEvent::Component {
            name: name.into(),
            phase,
        });
    }
    /// Registry owns span IDs and close detection; no global subscriber is installed.
    /// Keep the returned span alive until the actual operation and cleanup finish.
    pub fn span(
        &self,
        name: &str,
        turn: Option<&str>,
        request: Option<&str>,
        call: Option<&str>,
    ) -> Span {
        if !self.is_enabled() {
            return Span::none();
        }
        let Some(dispatch) = self.emitter.dispatch.get() else {
            return Span::none();
        };
        tracing::dispatcher::with_default(dispatch, || {
            tracing::info_span!(
                parent: None, "agent.operation", operation = name, branch_id = self.branch_id.as_deref(),
                turn_id = turn, request_id = request, call_id = call, outcome = tracing::field::Empty,
            )
        })
    }

    /// Create a prompt source load span. `prompt_source` and `prompt_scope` are
    /// raw diagnostics only; metadata-only archives deliberately discard them.
    /// Record `prompt_version` and `outcome` on the returned span when known.
    pub fn prompt_load_span(&self, source: &str, scope: &str) -> Span {
        if !self.is_enabled() {
            return Span::none();
        }
        let Some(dispatch) = self.emitter.dispatch.get() else {
            return Span::none();
        };
        tracing::dispatcher::with_default(dispatch, || {
            tracing::info_span!(
                parent: None,
                "agent.operation",
                operation = "prompt.load",
                branch_id = self.branch_id.as_deref(),
                prompt_source = source,
                prompt_scope = scope,
                prompt_version = tracing::field::Empty,
                outcome = tracing::field::Empty,
            )
        })
    }
}

pub(crate) struct ChannelEnd(Diagnostics);
impl Drop for ChannelEnd {
    fn drop(&mut self) {
        self.0.close();
    }
}

/// Holds the emitter weakly: the emitter owns the dispatch holding this layer.
struct DiagnosticLayer {
    emitter: std::sync::Weak<Emitter>,
}
struct SpanData {
    start: Instant,
    fields: BTreeMap<String, String>,
}
impl Visit for SpanData {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields.insert(field.name().into(), value.into());
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.fields
            .insert(field.name().into(), format!("{value:?}"));
    }
}
impl DiagnosticLayer {
    fn emit(&self, id: &Id, data: &SpanData, phase: SpanPhase) {
        let Some(emitter) = self.emitter.upgrade() else {
            return;
        };
        emitter.emit(
            data.fields.get("branch_id").cloned(),
            DiagnosticEvent::Span {
                id: id.into_u64(),
                phase,
                fields: data.fields.clone(),
                duration_ms: (phase == SpanPhase::End)
                    .then(|| data.start.elapsed().as_millis().min(u64::MAX as u128) as u64),
            },
        );
    }
}
impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for DiagnosticLayer {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut data = SpanData {
            start: Instant::now(),
            fields: BTreeMap::new(),
        };
        attrs.record(&mut data);
        self.emit(id, &data, SpanPhase::Start);
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(data);
        }
    }
    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id)
            && let Some(data) = span.extensions_mut().get_mut::<SpanData>()
        {
            values.record(data);
        }
    }
    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(&id)
            && let Some(mut data) = span.extensions_mut().remove::<SpanData>()
        {
            data.fields
                .entry("outcome".into())
                .or_insert_with(|| "defect".into());
            self.emit(&id, &data, SpanPhase::End);
        }
    }
}
