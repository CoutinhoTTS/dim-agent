use dim_agent_capabilities::{
    adapters::{DefaultPrepareModelPlugin, EchoModel, MockTools},
    history::HistoryRenderer,
    host::*,
};
use dim_agent_runtime::*;
use dim_agent_types::BranchSource;
use dim_agent_types::{EventKind, InputMode};
use rutis::TypedPlugin;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

/// Raw records as a host listener receives them on the diagnostics channel.
#[derive(Default)]
struct Recorder {
    records: Mutex<Vec<DiagnosticRecord>>,
    fail: AtomicBool,
    changed: tokio::sync::Notify,
}
impl Recorder {
    /// Records are emitted asynchronously; each Session's channel ends with `Closed`.
    async fn closed(&self, session_id: &str) {
        loop {
            let notified = self.changed.notified();
            if self.records.lock().unwrap().iter().any(|record| {
                record.session_id == session_id && matches!(record.event, DiagnosticEvent::Closed)
            }) {
                return;
            }
            notified.await;
        }
    }
}
struct Listen(Arc<Recorder>);
impl rutis::Listener<DiagnosticRecord> for Listen {
    fn call<'a>(
        &'a self,
        _ctx: &'a rutis::Ctx,
        record: &'a DiagnosticRecord,
    ) -> rutis::BoxFuture<'a, std::result::Result<Option<()>, rutis::CordisError>> {
        if self.0.fail.load(Ordering::Relaxed) {
            panic!("broken recorder");
        }
        self.0.records.lock().unwrap().push(record.clone());
        self.0.changed.notify_waiters();
        Box::pin(async { Ok(None) })
    }
}
/// Listens on the host root, then enables the Session's diagnostics.
struct Trace {
    recorder: Arc<Recorder>,
    root: rutis::Ctx,
    scope: SessionScope,
}
impl TypedPlugin for Trace {
    type Deps = (InSession<Diagnostics>, InSession<SessionIdentity>);
    fn name(&self) -> &str {
        "Trace"
    }
    fn apply<'a>(
        &'a self,
        _ctx: &'a rutis::Ctx,
        (InSession(diagnostics), InSession(identity)): Self::Deps,
    ) -> rutis::BoxFuture<'a, std::result::Result<rutis::Effect, rutis::CordisError>> {
        Box::pin(async move {
            let _listener = self.root.events().on(
                &self.root,
                &diagnostics_channel(&identity.session_id),
                Listen(self.recorder.clone()),
            )?;
            assert!(diagnostics.enable());
            Ok(rutis::Effect::Done)
        })
    }
}
impl Capability for Trace {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&self.scope)
    }
}
fn install(builder: &mut AgentBuilder, root: &rutis::Ctx, recorder: &Arc<Recorder>) {
    let (root, recorder) = (root.clone(), recorder.clone());
    builder.session_plugin(move |scope| Trace {
        recorder: recorder.clone(),
        root: root.clone(),
        scope: *scope,
    });
}
fn root() -> rutis::Ctx {
    rutis::Ctx::root_with(tokio::runtime::Handle::current())
}
/// An echo Agent whose Sessions are recorded by the host archive.
fn archived(
    root: &rutis::Ctx,
    archive: &dim_agent_capabilities::trace::archive::TraceArchive,
) -> Agent {
    let mut builder = AgentBuilder::new(AgentConfig::default());
    // This archive fixture compares retained events after runtime close.
    builder.timeline_store(Arc::new(dim_agent_runtime::MemoryTimelineStore::default()));
    dim_agent_capabilities::defaults::mock_hosts(&mut builder);
    dim_agent_capabilities::defaults::install(&mut builder);
    trace_host(&mut builder, root, archive);
    builder.build_in(root)
}
fn trace_host(
    builder: &mut AgentBuilder,
    root: &rutis::Ctx,
    archive: &dim_agent_capabilities::trace::archive::TraceArchive,
) {
    dim_agent_capabilities::trace::install(
        builder,
        dim_agent_capabilities::trace::TraceHost {
            ctx: root.clone(),
            recorder: archive.recorder(),
        },
    );
}
/// Ends a host-emitted channel the way Session close does.
fn emit(root: &rutis::Ctx, session_id: &str, event: DiagnosticEvent) {
    root.events()
        .emit(
            root,
            &diagnostics_channel(session_id),
            Arc::new(DiagnosticRecord {
                session_id: session_id.into(),
                branch_id: Some("main".into()),
                timestamp: 1,
                event,
            }),
        )
        .unwrap();
}
fn agent(root: &rutis::Ctx, recorder: &Arc<Recorder>) -> Agent {
    model_agent(root, recorder, Arc::new(EchoModel))
}
fn model_agent(root: &rutis::Ctx, recorder: &Arc<Recorder>, model: Arc<dyn ModelDriver>) -> Agent {
    let mut builder = AgentBuilder::new(AgentConfig::default());
    // This archive fixture compares retained events after runtime close.
    builder.timeline_store(Arc::new(dim_agent_runtime::MemoryTimelineStore::default()));
    builder
        .session_value(Arc::new(ModelHost(model)))
        .session_value(Arc::new(ToolHost(Arc::new(MockTools))))
        .session_value(Arc::new(HistoryRenderer::default()))
        .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
        .session_value(Arc::new(ContextModelHost(None)))
        .session_plugin(DefaultPrepareModelPlugin::new);
    dim_agent_capabilities::defaults::install(&mut builder);
    install(&mut builder, root, recorder);
    builder.build_in(root)
}

#[tokio::test]
async fn failed_session_assembly_records_component_release_after_rollback() {
    let recorder = Arc::new(Recorder::default());
    let root = root();
    let mut builder = AgentBuilder::new(AgentConfig::default());
    // This archive fixture compares retained events after runtime close.
    builder.timeline_store(Arc::new(dim_agent_runtime::MemoryTimelineStore::default()));
    install(&mut builder, &root, &recorder);
    let agent = builder.build_in(&root);
    // Trace becomes ready, but the required TimelineTreeControl is absent.
    assert!(
        agent
            .create_session(SessionConfig::default())
            .await
            .is_err()
    );
    agent.close().await.unwrap();
    let session_id = recorder.records.lock().unwrap()[0].session_id.clone();
    recorder.closed(&session_id).await;
    let records = recorder.records.lock().unwrap();
    let phases: Vec<_> = records
        .iter()
        .filter_map(|record| match &record.event {
            DiagnosticEvent::Component { name, phase } if name == "Trace" => Some(*phase),
            _ => None,
        })
        .collect();
    assert_eq!(
        phases,
        vec![ComponentPhase::Ready, ComponentPhase::Released]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trace_observes_committed_events_and_real_close_without_changing_history() {
    let recorder = Arc::new(Recorder::default());
    let root = root();
    let agent = agent(&root, &recorder);
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    session
        .submit(
            "tool:echo {\"text\":\"hello\"}".into(),
            InputMode::Queue,
            "main",
        )
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    let child = session
        .branch(
            "main".into(),
            session.sequence("main").unwrap(),
            BranchOptions {
                prompt: "child".into(),
                source: BranchSource::Host {
                    operation_id: "parent".into(),
                },
            },
        )
        .await
        .unwrap();
    session.idle(&child).await.unwrap();
    agent.close().await.unwrap();
    recorder.closed(session.id()).await;
    let records = recorder.records.lock().unwrap().clone();
    assert!(!records.is_empty());
    assert!(records.iter().all(|r| r.session_id == session.id()));
    for branch in ["main", child.as_str()] {
        let observed: Vec<_> = records
            .iter()
            .filter(|r| r.branch_id.as_deref() == Some(branch))
            .filter_map(|r| {
                if let DiagnosticEvent::Timeline { event, .. } = &r.event {
                    Some(event.clone())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(observed, session.local(branch, ..).await.unwrap());
        assert!(
            observed
                .iter()
                .any(|e| matches!(e.kind, EventKind::ModelOutputEnd(_)))
        );
        assert!(records.iter().any(|r| r.branch_id.as_deref() == Some(branch) && matches!(&r.event,
            DiagnosticEvent::Span { fields, phase: SpanPhase::End, .. } if fields.get("operation").map(String::as_str) == Some("timeline.close") && fields.get("outcome").map(String::as_str) == Some("returned"))));
        for event in &observed {
            if let EventKind::ModelCall(call) = &event.kind {
                assert!(
                    records
                        .iter()
                        .any(|r| r.branch_id.as_deref() == Some(branch)
                            && matches!(&r.event,
                    DiagnosticEvent::Span { fields, phase: SpanPhase::End, .. }
                    if fields.get("operation").map(String::as_str) == Some("model.invoke")
                        && fields.get("request_id") == Some(&call.request_id)
                        && fields.get("turn_id") == Some(&call.turn_id)
                        && fields.get("outcome").map(String::as_str) == Some("returned")))
                );
            }
            if let EventKind::ToolResult(batch) = &event.kind {
                for call in &batch.results {
                    assert!(records.iter().any(|r| matches!(&r.event, DiagnosticEvent::Span { fields, phase: SpanPhase::End, .. }
                        if fields.get("operation").map(String::as_str) == Some("tool.execute")
                            && fields.get("call_id") == Some(&call.call_id)
                            && fields.get("request_id") == Some(&batch.request_id))));
                }
            }
        }
    }
    let registered: Vec<_> = records
        .iter()
        .filter_map(|r| match &r.event {
            DiagnosticEvent::Subscription {
                id,
                phase: SubscriptionPhase::Registered,
                ..
            } => Some(id),
            _ => None,
        })
        .collect();
    assert!(!registered.is_empty());
    for id in registered {
        assert_eq!(records.iter().filter(|r| matches!(&r.event, DiagnosticEvent::Subscription { id: released, phase: SubscriptionPhase::Released, .. } if released == id)).count(), 1);
    }
    assert!(matches!(
        records.last().unwrap().event,
        DiagnosticEvent::Closed
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn component_lifecycles_are_local_to_each_session_and_branch() {
    let recorder = Arc::new(Recorder::default());
    let root = root();
    let agent = agent(&root, &recorder);
    let a = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    let b = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    let child = a
        .branch(
            "main".into(),
            0,
            BranchOptions {
                prompt: "child".into(),
                source: BranchSource::Host {
                    operation_id: "trace".into(),
                },
            },
        )
        .await
        .unwrap();
    a.idle(&child).await.unwrap();
    a.close().await.unwrap();
    recorder.closed(a.id()).await;
    {
        let records = recorder.records.lock().unwrap();
        assert!(records.iter().any(|r| r.session_id == b.id()
            && matches!(
                r.event,
                DiagnosticEvent::Component {
                    phase: ComponentPhase::Ready,
                    ..
                }
            )));
        assert!(!records.iter().any(|r| r.session_id == b.id()
            && matches!(
                r.event,
                DiagnosticEvent::Component {
                    phase: ComponentPhase::Released,
                    ..
                }
            )));
    }
    b.submit("still running".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    b.idle("main").await.unwrap();
    agent.close().await.unwrap();
    recorder.closed(b.id()).await;
    let records = recorder.records.lock().unwrap();
    let mut phases = std::collections::BTreeMap::new();
    for record in records.iter() {
        if let DiagnosticEvent::Component { name, phase } = &record.event {
            phases
                .entry((
                    record.session_id.clone(),
                    record.branch_id.clone(),
                    name.clone(),
                ))
                .or_insert_with(Vec::new)
                .push(*phase);
        }
    }
    // Each scope is a different instance, even though scopes share their names.
    assert_eq!(
        phases
            .keys()
            .filter(|(_, _, name)| name == "SessionScope" || name == "BranchScope")
            .count(),
        5
    );
    for (scope, lifecycle) in phases {
        assert_eq!(
            lifecycle,
            vec![ComponentPhase::Ready, ComponentPhase::Released],
            "{scope:?}"
        );
    }
}

#[tokio::test]
async fn listener_panics_do_not_break_input_stop_or_close() {
    let recorder = Arc::new(Recorder::default());
    let root = root();
    let agent = agent(&root, &recorder);
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    recorder.fail.store(true, Ordering::Relaxed);
    session
        .submit("still runs".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    session.stop("main", "stop".into()).await.unwrap();
    agent.close().await.unwrap();
    assert!(
        session
            .history("main", None)
            .await
            .unwrap()
            .iter()
            .any(|e| matches!(e.kind, EventKind::ModelOutputEnd(_)))
    );
}

struct TracePromptHost;
impl LoadPrompt for TracePromptHost {
    fn load(
        &self,
        target: PromptSourceTarget,
        _: tokio_util::sync::CancellationToken,
    ) -> HostFuture<'_, Option<dim_agent_types::PromptEntry>> {
        Box::pin(async move {
            Ok(Some(dim_agent_types::PromptEntry {
                source: target.source,
                version: "safe-version-7".into(),
                body: "secret prompt body canary".into(),
                scope: target.scope,
                delivery: dim_agent_types::PromptDelivery::System,
                order: 0,
            }))
        })
    }
}

#[tokio::test]
async fn prompt_and_context_references_are_archived_without_source_or_body() {
    use dim_agent_capabilities::{defaults, trace::archive::TraceArchive};
    use dim_agent_types::{ModelSelection, PromptScope};
    use tokio::io::AsyncReadExt;

    struct TaskScope(dim_agent_runtime::BranchScope);
    impl TaskScope {
        fn new(scope: &dim_agent_runtime::BranchScope) -> Self {
            Self(*scope)
        }
    }
    impl TypedPlugin for TaskScope {
        type Deps = (InBranch<Timeline>,);
        fn name(&self) -> &str {
            "TaskScope"
        }
        fn apply<'a>(
            &'a self,
            _ctx: &'a rutis::Ctx,
            (InBranch(timeline),): Self::Deps,
        ) -> rutis::BoxFuture<'a, std::result::Result<rutis::Effect, rutis::CordisError>> {
            Box::pin(async move {
                timeline.append(
                    EventKind::ContextScope(dim_agent_types::ContextScopeData {
                        scope: PromptScope::Task {
                            timeline: "main".into(),
                            id: "task-1".into(),
                        },
                        status: dim_agent_types::ContextScopeStatus::Started,
                    })
                    .into(),
                )?;
                Ok(rutis::Effect::Done)
            })
        }
    }
    impl Capability for TaskScope {
        fn keys(&self) -> KeysOf<Self> {
            scoped::<Self::Deps, _>(&self.0)
        }
    }
    let (writer, mut reader) = tokio::io::duplex(4096);
    let reading = tokio::spawn(async move {
        let mut text = String::new();
        reader.read_to_string(&mut text).await.unwrap();
        text
    });
    let archive = TraceArchive::new(writer);
    let root = root();
    let mut builder = AgentBuilder::new(AgentConfig::default());
    // This archive fixture compares retained events after runtime close.
    builder.timeline_store(Arc::new(dim_agent_runtime::MemoryTimelineStore::default()));
    defaults::mock_hosts(&mut builder);
    defaults::install_with_prompt(&mut builder, Arc::new(TracePromptHost), None);
    dim_agent_capabilities::trace::install(
        &mut builder,
        dim_agent_capabilities::trace::TraceHost {
            ctx: root.clone(),
            recorder: archive.recorder(),
        },
    );
    builder.timeline_plugin(TaskScope::new);
    let agent = builder.build_in(&root);
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    session
        .configure(
            ConfigureUpdate {
                sources: vec![PromptSourceTarget {
                    source: "/secret/customer/source-name.md".into(),
                    scope: PromptScope::Branch {
                        timeline: "main".into(),
                    },
                }],
                model: Some(ModelSelection {
                    provider_id: "mock-provider".into(),
                    model_id: "safe-model".into(),
                    capability_snapshot: None,
                    config: None,
                    metadata: None,
                }),
            },
            "main",
        )
        .await
        .unwrap();
    session
        .submit("secret input body".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    agent.close().await.unwrap();
    let status = archive.finish().await.unwrap();
    assert!(!status.incomplete);
    let text = reading.await.unwrap();
    assert!(!text.contains("secret prompt body canary"));
    assert!(!text.contains("/secret/customer/source-name.md"));
    assert!(!text.contains("secret input body"));
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let event = |key: &str| {
        lines
            .iter()
            .find(|line| line["record"]["event"]["key"] == key)
            .unwrap_or_else(|| panic!("missing {key}"))
    };
    let changed = event("prompt/changed");
    assert_eq!(
        changed["record"]["event"]["changes"][0]["version"],
        "safe-version-7"
    );
    let configured = event("context/configured");
    assert!(configured["record"]["event"]["prompt"]["revision"].is_string());
    assert!(event("context/scope")["record"]["event"]["status"].is_string());
    for key in ["context/prepare", "model/call"] {
        let request = event(key);
        assert_eq!(
            request["record"]["event"]["configuration"],
            serde_json::json!({
                "timeline": configured["record"]["event"]["timeline"],
                "revision": configured["record"]["event"]["revision"],
            })
        );
        assert_eq!(
            request["record"]["event"]["prompt"],
            configured["record"]["event"]["prompt"]
        );
    }
}

struct SettlingModel {
    started: tokio::sync::Notify,
    cancelled: tokio::sync::Notify,
    release: tokio::sync::Notify,
    outcome: &'static str,
}
impl ModelDriver for SettlingModel {
    fn invoke(
        &self,
        _: Arc<dim_agent_types::ModelRequest>,
        _: dim_agent_capabilities::host::ModelConnection,
        cancel: tokio_util::sync::CancellationToken,
    ) -> HostFuture<'_, ModelStream> {
        Box::pin(async move {
            self.started.notify_one();
            cancel.cancelled().await;
            self.cancelled.notify_one();
            self.release.notified().await;
            match self.outcome {
                "interrupted" => Err(HostError::Cancelled("stopped".into())),
                "failure" => Err(HostError::failed("provider failed")),
                _ => panic!("provider defect"),
            }
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_spans_wait_for_native_cleanup_and_distinguish_cancel_failure_and_defect() {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        for outcome in ["interrupted", "failure", "defect"] {
            let recorder = Arc::new(Recorder::default());
            let model = Arc::new(SettlingModel { started: Default::default(), cancelled: Default::default(), release: Default::default(), outcome });
            let root = root();
            let agent = model_agent(&root, &recorder, model.clone());
            let session = agent.create_session(SessionConfig::default()).await.unwrap();
            session.submit("request".into(), InputMode::Queue, "main").await.unwrap();
            model.started.notified().await;
            let stopper = session.clone();
            let stop = tokio::spawn(async move { stopper.stop("main", "stop".into()).await });
            model.cancelled.notified().await;
            assert!(!stop.is_finished());
            assert!(!recorder.records.lock().unwrap().iter().any(|r| matches!(&r.event,
                DiagnosticEvent::Span { fields, phase: SpanPhase::End, .. } if fields.get("operation").map(String::as_str) == Some("model.invoke"))));
            model.release.notify_one();
            let result = stop.await.unwrap();
            if outcome != "defect" { result.unwrap(); }
            let _ = agent.close().await;
            recorder.closed(session.id()).await;
            assert!(recorder.records.lock().unwrap().iter().any(|r| matches!(&r.event,
                DiagnosticEvent::Span { fields, phase: SpanPhase::End, .. }
                    if fields.get("operation").map(String::as_str) == Some("model.invoke")
                        && fields.get("outcome").map(String::as_str) == Some(outcome))));
        }
    }).await.unwrap();
}

#[tokio::test]
async fn local_archive_is_private_redacted_correlated_and_never_overwritten() {
    use dim_agent_capabilities::trace::archive::TraceArchive;
    let directory = std::env::temp_dir().join(format!(
        "dim-trace-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let archive = TraceArchive::create(&directory).await.unwrap();
    let root = root();
    let mut builder = AgentBuilder::new(AgentConfig::default());
    // This archive fixture compares retained events after runtime close.
    builder.timeline_store(Arc::new(dim_agent_runtime::MemoryTimelineStore::default()));
    dim_agent_capabilities::defaults::mock_hosts(&mut builder);
    dim_agent_capabilities::defaults::install(&mut builder);
    trace_host(&mut builder, &root, &archive);
    builder.timeline_plugin(SensitiveBefore::new);
    let agent = builder.build_in(&root);
    let session = agent
        .create_session(SessionConfig {
            system_prompt: "secret-system-token".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    session
        .submit(
            "tool:echo {\"text\":\"Bearer secret-output-token\"}".into(),
            InputMode::Queue,
            "main",
        )
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    agent.close().await.unwrap();
    let expected = session.local("main", ..).await.unwrap();
    let status = archive.finish().await.unwrap();
    assert!(!status.incomplete);
    let bytes = tokio::fs::read(directory.join("trace.jsonl"))
        .await
        .unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(!text.contains("secret-system-token"));
    assert!(!text.contains("secret-output-token"));
    assert!(!text.contains("secret-before-token"));
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(lines.iter().all(|line| line["version"] == 1));
    assert_eq!(lines.first().unwrap()["kind"], "archive_start");
    assert_eq!(lines.last().unwrap()["kind"], "archive_end");
    assert_eq!(lines.last().unwrap()["incomplete"], false);
    assert_eq!(status.records as usize, lines.len() - 2);
    let events: Vec<_> = lines
        .iter()
        .filter(|line| line["record"]["kind"] == "timeline")
        .collect();
    assert_eq!(events.len(), expected.len());
    assert!(
        events
            .iter()
            .any(|line| line["record"]["before"]["steps"][0]["handler"] == "metadata-test")
    );
    for (line, event) in events.iter().zip(&expected) {
        assert_eq!(line["session_id"], session.id());
        assert_eq!(line["branch_id"], "main");
        assert_eq!(line["record"]["event_id"], event.id);
        assert_eq!(line["record"]["sequence"], event.sequence);
    }
    assert!(
        events
            .iter()
            .any(|line| line["record"]["event"]["key"] == "model/call"
                && line["record"]["event"]["model_id"] == "prepared-model")
    );
    assert!(
        events
            .iter()
            .any(|line| line["record"]["event"]["key"] == "step/tool-result"
                && line["record"]["event"]["results"][0]["call_id"].is_string())
    );
    assert!(TraceArchive::create(&directory).await.is_err());
    assert_eq!(
        tokio::fs::read(directory.join("trace.jsonl"))
            .await
            .unwrap(),
        bytes
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(directory.join("trace.jsonl"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    tokio::fs::remove_dir_all(directory).await.unwrap();
}

struct SensitiveBefore(dim_agent_runtime::BranchScope);
impl SensitiveBefore {
    fn new(scope: &dim_agent_runtime::BranchScope) -> Self {
        Self(*scope)
    }
}
impl TypedPlugin for SensitiveBefore {
    type Deps = (InBranch<Timeline>,);
    fn name(&self) -> &str {
        "SensitiveBefore"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a rutis::Ctx,
        (InBranch(timeline),): Self::Deps,
    ) -> rutis::BoxFuture<'a, std::result::Result<rutis::Effect, rutis::CordisError>> {
        Box::pin(async move {
            timeline.before(ctx, "metadata-test", &["model/call"], |mut draft| {
                draft.tags = Some(vec!["secret-before-token".into()]);
                if let EventKind::ModelCall(data) = &mut draft.kind {
                    Arc::make_mut(&mut data.request).model_id = "prepared-model".into();
                }
                Ok(draft)
            })?;
            Ok(rutis::Effect::Done)
        })
    }
}
impl Capability for SensitiveBefore {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&self.0)
    }
}

#[tokio::test]
async fn archive_write_failure_does_not_change_the_session_result() {
    use dim_agent_capabilities::trace::archive::TraceArchive;
    let (writer, reader) = tokio::io::duplex(1);
    drop(reader);
    let archive = TraceArchive::new(writer);
    let agent = archived(&root(), &archive);
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    session
        .submit("hello".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    agent.close().await.unwrap();
    assert!(archive.finish().await.is_err());
    assert!(session.history("main", None).await.unwrap().iter().any(|event| matches!(&event.kind,
        EventKind::TurnEnd(data) if matches!(data.outcome, dim_agent_types::TurnOutcome::Completed { .. }))));
}

#[tokio::test]
async fn blocked_archive_does_not_block_business_and_keeps_every_record() {
    use dim_agent_capabilities::trace::archive::TraceArchive;
    use tokio::io::AsyncReadExt;
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let (writer, mut reader) = tokio::io::duplex(1);
        let archive = TraceArchive::new(writer);
        let agent = archived(&root(), &archive);
        let session = agent
            .create_session(SessionConfig::default())
            .await
            .unwrap();
        for n in 0..30 {
            session
                .submit(format!("input {n}"), InputMode::Queue, "main")
                .await
                .unwrap();
            session.idle("main").await.unwrap();
        }
        agent.close().await.unwrap();
        archive.closed().await;
        assert!(!archive.is_incomplete());
        assert_eq!(
            session
                .history("main", None)
                .await
                .unwrap()
                .iter()
                .filter(|event| matches!(event.kind, EventKind::TurnEnd(_)))
                .count(),
            30
        );
        let reading = tokio::spawn(async move {
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).await.unwrap();
            bytes
        });
        let status = archive.finish().await.unwrap();
        assert!(!status.incomplete);
        let bytes = reading.await.unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let footer: serde_json::Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
        assert_eq!(footer["kind"], "archive_end");
        assert_eq!(footer["incomplete"], false);
        assert_eq!(footer["records"], status.records);
        // Header and footer frame exactly the queued records.
        assert_eq!(text.lines().count() as u64, status.records + 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn abandoning_archive_owner_or_finish_waiter_still_drains_without_aborting_the_writer() {
    use dim_agent_capabilities::trace::archive::TraceArchive;
    use std::{future::Future, task::Poll};
    use tokio::io::AsyncReadExt;
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        for requested_finish in [false, true] {
            let (writer, mut reader) = tokio::io::duplex(1);
            let archive = TraceArchive::new(writer);
            let agent = archived(&root(), &archive);
            let session = agent
                .create_session(SessionConfig::default())
                .await
                .unwrap();
            session
                .submit("hello".into(), InputMode::Queue, "main")
                .await
                .unwrap();
            session.idle("main").await.unwrap();
            agent.close().await.unwrap();
            archive.closed().await;
            if requested_finish {
                let mut finishing = Box::pin(archive.finish());
                std::future::poll_fn(|cx| match finishing.as_mut().poll(cx) {
                    Poll::Pending => Poll::Ready(()),
                    Poll::Ready(_) => panic!("writer must wait for the reader"),
                })
                .await;
                drop(finishing);
            } else {
                drop(archive);
            }
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).await.unwrap();
            let text = String::from_utf8(bytes).unwrap();
            let footer: serde_json::Value =
                serde_json::from_str(text.lines().last().unwrap()).unwrap();
            assert_eq!(footer["kind"], "archive_end");
            assert_eq!(footer["incomplete"], !requested_finish);
            assert!(footer["records"].as_u64().unwrap() > 0);
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn sensitive_archive_preserves_final_requests_tools_and_before_without_mutating_history() {
    use dim_agent_capabilities::trace::archive::{ArchiveContent, TraceArchive};
    use tokio::io::AsyncReadExt;
    let (writer, mut reader) = tokio::io::duplex(4096);
    let reading = tokio::spawn(async move {
        let mut text = String::new();
        reader.read_to_string(&mut text).await.unwrap();
        text
    });
    let archive = TraceArchive::with_content(writer, ArchiveContent::SensitiveContent);
    let root = root();
    let mut builder = AgentBuilder::new(AgentConfig::default());
    // This archive fixture compares retained events after runtime close.
    builder.timeline_store(Arc::new(dim_agent_runtime::MemoryTimelineStore::default()));
    dim_agent_capabilities::defaults::mock_hosts(&mut builder);
    dim_agent_capabilities::defaults::install(&mut builder);
    trace_host(&mut builder, &root, &archive);
    builder.timeline_plugin(SensitiveBefore::new);
    let agent = builder.build_in(&root);
    let session = agent
        .create_session(SessionConfig {
            system_prompt: "Inspect local data only".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    session
        .submit(
            r#"tool:echo {"text":"visible-result"}"#.into(),
            InputMode::Queue,
            "main",
        )
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    session.submit(r#"tool:echo {"text":"visible-result","api_key":"credential-canary","nested":{"Authorization":"nested-canary"}}"#.into(), InputMode::Queue, "main").await.unwrap();
    session.idle("main").await.unwrap();
    let large = "retained-message".repeat(5000);
    session
        .submit(large.clone(), InputMode::Queue, "main")
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    agent.close().await.unwrap();
    let status = archive.finish().await.unwrap();
    assert!(!status.incomplete);
    let text = reading.await.unwrap();
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(lines.iter().all(|line| line["version"] == 2));
    assert_eq!(lines[0]["content"], "sensitive_content");
    assert_eq!(lines.last().unwrap()["records"], status.records);
    let timeline: Vec<_> = lines
        .iter()
        .filter(|line| line["record"]["kind"] == "timeline")
        .collect();
    let history = session.local("main", ..).await.unwrap();
    assert_eq!(timeline.len(), history.len());
    for (record, original) in timeline.iter().zip(&history) {
        assert_eq!(record["record"]["payload"]["event"]["id"], original.id);
    }
    let call = timeline
        .iter()
        .find(|line| line["record"]["event"]["key"] == "model/call")
        .unwrap();
    assert_eq!(
        call["record"]["payload"]["event"]["data"]["request"]["modelId"],
        "prepared-model"
    );
    assert_eq!(
        call["record"]["payload"]["before"]["original"]["data"]["request"]["modelId"],
        "deterministic"
    );
    assert!(text.contains(&large));
    let tool = timeline
        .iter()
        .find(|line| {
            line["record"]["event"]["key"] == "step/tool-call"
                && line["record"]["payload"]["event"]["data"]["calls"][0]["input"]
                    .get("api_key")
                    .is_some()
        })
        .unwrap();
    let input = &tool["record"]["payload"]["event"]["data"]["calls"][0]["input"];
    assert_eq!(input["text"], "visible-result");
    assert_eq!(input["api_key"], "[REDACTED]");
    assert_eq!(input["nested"]["Authorization"], "[REDACTED]");
    assert!(history.iter().any(|event| matches!(&event.kind, EventKind::ToolCall(data) if data.calls[0].input.get("api_key").and_then(serde_json::Value::as_str) == Some("credential-canary"))));
    // Free text intentionally remains sensitive, even when it contains embedded JSON.
    assert!(text.contains("credential-canary"));
    assert!(timeline.iter().any(|line| {
        line["record"]["event"]["key"] == "step/tool-result"
            && line["record"]["payload"]["event"]["data"]["results"][0]["result"]["content"]
                .to_string()
                .contains("visible-result")
    }));
}

#[tokio::test]
async fn large_content_is_archived_whole_without_losing_later_records() {
    use dim_agent_capabilities::trace::archive::{ArchiveContent, TraceArchive};
    use dim_agent_types::{AgentEvent, InputMessageData};
    use tokio::io::AsyncReadExt;
    let (writer, mut reader) = tokio::io::duplex(4096);
    let reading = tokio::spawn(async move {
        let mut text = String::new();
        reader.read_to_string(&mut text).await.unwrap();
        text
    });
    let archive = TraceArchive::with_content(writer, ArchiveContent::SensitiveContent);
    let root = root();
    archive.recorder().listen(&root, "size-boundary").unwrap();
    for (sequence, text) in [(1, "x".repeat(4 * 1024 * 1024)), (2, "small".into())] {
        emit(
            &root,
            "size-boundary",
            DiagnosticEvent::Timeline {
                event: Arc::new(AgentEvent {
                    id: sequence.to_string(),
                    sequence,
                    create_at: 1,
                    tags: None,
                    kind: EventKind::InputMessage(InputMessageData::text(text, None)),
                }),
                trace: None,
            },
        );
    }
    emit(&root, "size-boundary", DiagnosticEvent::Closed);
    let status = archive.finish().await.unwrap();
    assert!(!status.incomplete);
    assert_eq!(status.records, 2);
    let text = reading.await.unwrap();
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        lines[1]["record"]["payload"]["event"]["data"]["user"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .len(),
        4 * 1024 * 1024
    );
    assert_eq!(
        lines[2]["record"]["payload"]["event"]["data"]["user"]["content"][0]["text"],
        "small"
    );
    assert_eq!(lines[3]["incomplete"], false);
}

/// Captures its own context so a test can unload a host scope early.
struct HostScope(Arc<Mutex<Option<rutis::Ctx>>>);
impl TypedPlugin for HostScope {
    type Deps = ();
    fn name(&self) -> &str {
        "HostScope"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a rutis::Ctx,
        (): Self::Deps,
    ) -> rutis::BoxFuture<'a, std::result::Result<rutis::Effect, rutis::CordisError>> {
        Box::pin(async move {
            *self.0.lock().unwrap() = Some(ctx.clone());
            Ok(rutis::Effect::Done)
        })
    }
}
impl Capability for HostScope {
    fn keys(&self) -> KeysOf<Self> {}
}

#[tokio::test]
async fn archive_finishes_incomplete_when_the_host_unloads_before_the_end_record() {
    use dim_agent_capabilities::trace::archive::TraceArchive;
    let archive = TraceArchive::new(tokio::io::sink());
    let root = root();
    let slot = Arc::new(Mutex::new(None));
    let view = root.plugin(capability_plugin(HostScope(slot.clone())));
    (&view).await.unwrap();
    let host = slot.lock().unwrap().clone().unwrap();
    let recorder = archive.recorder();
    recorder.listen(&host, "early-unload").unwrap();
    assert!(recorder.listen(&host, "early-unload").is_err());
    view.shutdown().await.unwrap();
    let status = tokio::time::timeout(std::time::Duration::from_secs(5), archive.finish())
        .await
        .expect("finish does not wait for a removed listener")
        .unwrap();
    assert!(status.incomplete);
}
