use dim_agent_capabilities::{
    adapters::{DefaultPrepareModelPlugin, EchoModel, MockTools},
    defaults,
    host::*,
    tool_registry::definition,
};
use dim_agent_runtime::*;
use dim_agent_types::*;
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

fn builder(model: Arc<dyn ModelDriver>, tools: Arc<dyn ToolDriver>) -> AgentBuilder {
    let mut builder = AgentBuilder::new(AgentConfig::default());
    builder
        .session_value(Arc::new(ModelHost(model)))
        .session_value(Arc::new(ToolHost(tools)))
        .session_value(Arc::new(
            dim_agent_capabilities::history::HistoryRenderer::default(),
        ))
        .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
        .session_value(Arc::new(ContextModelHost(None)))
        .session_value(Arc::new(CompactionStrategyHost(None)))
        .session_part(|names| names.part(DefaultPrepareModelPlugin::new(names)?));
    defaults::install(&mut builder);
    builder
}

struct Rewrite {
    scope: BranchScope,
}
impl Rewrite {
    pub fn new(scope: &BranchScope) -> Self {
        Self { scope: *scope }
    }
}
impl TypedPlugin for Rewrite {
    type Deps = (InBranch<Timeline>,);
    fn name(&self) -> &str {
        "Rewrite"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (InBranch(timeline),): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            timeline.before(ctx, "rewrite-model", &["model/call"], |mut draft| {
                if let EventKind::ModelCall(call) = &mut draft.kind {
                    Arc::make_mut(&mut call.request).model_id = "final-model".into();
                }
                Ok(draft)
            })?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for Rewrite {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&self.scope)
    }
}
struct RecordingModel(Arc<Mutex<Vec<Arc<ModelRequest>>>>);
impl ModelDriver for RecordingModel {
    fn invoke(
        &self,
        request: Arc<ModelRequest>,
        _: dim_agent_capabilities::host::ModelConnection,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ModelStream> {
        self.0.lock().unwrap().push(request.clone());
        EchoModel.invoke(
            request,
            dim_agent_capabilities::host::ModelConnection::Local,
            cancel,
        )
    }
}

struct BoundaryTools {
    entered: Arc<Notify>,
    release: Arc<Notify>,
}
impl ToolDriver for BoundaryTools {
    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![definition(
            "boundary",
            "Hold the tool-result boundary",
            serde_json::json!({"type":"object"}),
        )]
    }
    fn execute(
        &self,
        _: ToolCall,
        _: ToolExecutionContext,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ToolResult> {
        Box::pin(async move {
            self.entered.notify_one();
            tokio::select! {
                _ = cancel.cancelled() => Err(HostError::Cancelled("boundary cancelled".into())),
                _ = self.release.notified() => Ok(ToolResult::text("boundary completed")),
            }
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn steer_and_append_enter_the_current_turn_at_tool_result_while_queue_waits() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let agent = builder(Arc::new(RecordingModel(requests.clone())), Arc::new(BoundaryTools {
            entered: entered.clone(), release: release.clone(),
        })).build();
        let session = agent.create_session(SessionConfig { tools: vec!["boundary".into()], permission_policy: dim_agent_runtime::PermissionPolicy { preset: dim_agent_runtime::PermissionPreset::FullAccess, ..Default::default() }, ..Default::default() }).await.unwrap();
        session.submit("tool:boundary {}".into(), InputMode::Queue, "main").await.unwrap();
        entered.notified().await;
        let queued = session.submit("next turn".into(), InputMode::Queue, "main").await.unwrap();
        let steered = session.submit("change current request".into(), InputMode::Steer, "main").await.unwrap();
        let appended = session.submit("extra current context".into(), InputMode::Append, "main").await.unwrap();
        // Release the tool once the steer and append inputs are applied; the
        // queue input waits for the turn to end.
        let mut dispatch = session.dispatch_watch("main").unwrap();
        loop {
            let events = session.history("main", None).await.unwrap();
            let applied = events.iter().filter(|event| matches!(&event.kind,
                EventKind::InputApplied(data) if [&steered.id, &appended.id].contains(&&data.input_id))).count();
            if applied == 2 && dispatch.borrow().sequence >= events.last().unwrap().sequence {
                break;
            }
            dispatch.changed().await.unwrap();
        }
        assert!(!session.history("main", None).await.unwrap().iter().any(|event| matches!(&event.kind,
            EventKind::InputApplied(data) if data.input_id == queued.id)));
        assert!(!session.history("main", None).await.unwrap().iter().any(|event| matches!(&event.kind,
            EventKind::InputConsumed(data) if [&queued.id, &steered.id, &appended.id].contains(&&data.input_id))));
        release.notify_one();
        session.idle("main").await.unwrap();
        let requests = requests.lock().unwrap().clone();
        assert_eq!(requests.len(), 3);
        assert!(requests[1].messages.contains(&Arc::new(Message::text(Role::User, "change current request"))));
        assert!(requests[1].messages.contains(&Arc::new(Message::text(Role::User, "extra current context"))));
        assert!(!requests[1].messages.contains(&Arc::new(Message::text(Role::User, "next turn"))));
        assert!(requests[2].messages.contains(&Arc::new(Message::text(Role::User, "next turn"))));
        let events = session.history("main", None).await.unwrap();
        let applied = |id: &str| events.iter().find(|event| matches!(&event.kind, EventKind::InputConsumed(data) if data.input_id == id)).unwrap().sequence;
        let tool_result = events.iter().find(|event| matches!(event.kind, EventKind::ToolResult(_))).unwrap().sequence;
        let turn_end = events.iter().find(|event| matches!(event.kind, EventKind::TurnEnd(_))).unwrap().sequence;
        assert!(tool_result < applied(&steered.id));
        assert!(applied(&steered.id) < applied(&appended.id));
        assert!(applied(&appended.id) < turn_end);
        assert!(turn_end < applied(&queued.id));
        assert_eq!(events.iter().filter(|event| matches!(event.kind, EventKind::TurnStart(_))).count(), 2);
        agent.close().await.unwrap();
    }).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_receives_the_committed_before_result_and_fixed_history() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut builder = builder(
            Arc::new(RecordingModel(requests.clone())),
            Arc::new(MockTools),
        );
        builder.timeline_plugin(Rewrite::new);
        let agent = builder.build();
        let session = agent
            .create_session(SessionConfig::default())
            .await
            .unwrap();
        session
            .submit("first".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        session.idle("main").await.unwrap();
        let frozen = session.history("main", None).await.unwrap();
        session
            .submit("second".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        session.idle("main").await.unwrap();
        let requests = requests.lock().unwrap().clone();
        assert_eq!(requests[0].model_id, "final-model");
        assert_eq!(
            requests[0].messages,
            vec![Arc::new(Message::text(Role::User, "first"))]
        );
        assert_eq!(requests[1].messages.len(), 3);
        assert_eq!(
            frozen
                .iter()
                .filter(|event| matches!(event.kind, EventKind::InputMessage(_)))
                .count(),
            1
        );
        let call = frozen
            .iter()
            .find(|event| matches!(event.kind, EventKind::ModelCall(_)))
            .unwrap();
        assert_eq!(
            session
                .trace("main", call.sequence)
                .await
                .unwrap()
                .unwrap()
                .steps
                .len(),
            1
        );
        agent.close().await.unwrap();
    })
    .await
    .unwrap();
}

struct DelayedTools {
    started: Arc<Notify>,
    cleaned: Arc<AtomicBool>,
}
impl ToolDriver for DelayedTools {
    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![definition(
            "delayed",
            "Test native cleanup",
            serde_json::json!({"type":"object"}),
        )]
    }
    fn execute(
        &self,
        _: ToolCall,
        _: ToolExecutionContext,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ToolResult> {
        Box::pin(async move {
            self.started.notify_one();
            cancel.cancelled().await;
            tokio::time::sleep(Duration::from_millis(25)).await;
            self.cleaned.store(true, Ordering::SeqCst);
            Ok(ToolResult::text("native operation succeeded"))
        })
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_waits_for_native_cleanup_and_keeps_a_real_success() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let started = Arc::new(Notify::new()); let cleaned = Arc::new(AtomicBool::new(false));
        let agent = builder(Arc::new(EchoModel), Arc::new(DelayedTools { started: started.clone(), cleaned: cleaned.clone() })).build();
        let session = agent.create_session(SessionConfig { tools: vec!["delayed".into()], permission_policy: dim_agent_runtime::PermissionPolicy { preset: dim_agent_runtime::PermissionPreset::FullAccess, ..Default::default() }, ..SessionConfig::default() }).await.unwrap();
        session.submit("tool:delayed {}".into(), InputMode::Queue, "main").await.unwrap();
        started.notified().await;
        session.stop("main", "stop".into()).await.unwrap();
        assert!(cleaned.load(Ordering::SeqCst));
        let events = session.history("main", None).await.unwrap();
        assert!(events.iter().any(|event| matches!(&event.kind, EventKind::ToolResult(data) if data.results[0].result.content == ToolContent::Text("native operation succeeded".into()) && data.results[0].result.is_error.is_none())));
        assert!(matches!(events.last().unwrap().kind, EventKind::TurnEnd(TurnEndData { outcome: TurnOutcome::Cancelled { .. }, .. })));
        agent.close().await.unwrap();
    }).await.unwrap();
}

struct BrokenStream {
    cleaned: Arc<AtomicBool>,
}
impl ModelDriver for BrokenStream {
    fn invoke(
        &self,
        _: Arc<ModelRequest>,
        _: dim_agent_capabilities::host::ModelConnection,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ModelStream> {
        let cleaned = self.cleaned.clone();
        Box::pin(async move {
            let stream = futures_util::stream::unfold(
                (0, cancel, cleaned),
                |(stage, cancel, cleaned)| async move {
                    match stage {
                        0 | 1 => Some((
                            Ok(ModelChunk::MessageStart {
                                message_id: "duplicate".into(),
                            }),
                            (stage + 1, cancel, cleaned),
                        )),
                        _ => {
                            cancel.cancelled().await;
                            tokio::time::sleep(Duration::from_millis(20)).await;
                            cleaned.store(true, Ordering::SeqCst);
                            None
                        }
                    }
                },
            );
            Ok(Box::pin(stream) as ModelStream)
        })
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_model_stream_is_cancelled_and_drained_before_settlement() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let cleaned = Arc::new(AtomicBool::new(false));
        let agent = builder(
            Arc::new(BrokenStream {
                cleaned: cleaned.clone(),
            }),
            Arc::new(MockTools),
        )
        .build();
        let session = agent
            .create_session(SessionConfig::default())
            .await
            .unwrap();
        session
            .submit("hello".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        session.idle("main").await.unwrap();
        assert!(cleaned.load(Ordering::SeqCst));
        assert!(matches!(
            session
                .history("main", None)
                .await
                .unwrap()
                .last()
                .unwrap()
                .kind,
            EventKind::TurnEnd(TurnEndData {
                outcome: TurnOutcome::Failed { .. },
                ..
            })
        ));
        agent.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn mock_hosts_enforce_the_declared_output_byte_limit() {
    use dim_agent_capabilities::adapters::MAX_MOCK_OUTPUT_BYTES;
    use futures_util::StreamExt;
    let request = Arc::new(ModelRequest::new(
        "mock",
        "echo",
        vec![Message::text(
            Role::User,
            "x".repeat(MAX_MOCK_OUTPUT_BYTES - 6),
        )],
    ));
    let chunks = EchoModel
        .invoke(
            request,
            dim_agent_capabilities::host::ModelConnection::Local,
            CancellationToken::new(),
        )
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert!(chunks.iter().any(
        |chunk| matches!(chunk, Ok(ModelChunk::Text(text)) if text.len() == MAX_MOCK_OUTPUT_BYTES)
    ));
    let request = Arc::new(ModelRequest::new(
        "mock",
        "echo",
        vec![Message::text(
            Role::User,
            "x".repeat(MAX_MOCK_OUTPUT_BYTES - 5),
        )],
    ));
    assert!(
        EchoModel
            .invoke(
                request,
                dim_agent_capabilities::host::ModelConnection::Local,
                CancellationToken::new()
            )
            .await
            .is_err()
    );
    for name in ["echo", "wait"] {
        for size in [MAX_MOCK_OUTPUT_BYTES, MAX_MOCK_OUTPUT_BYTES + 1] {
            let mut input = JsonObject::new();
            input.insert("text".into(), serde_json::Value::String("x".repeat(size)));
            if name == "wait" {
                input.insert("millis".into(), serde_json::Value::from(0));
            }
            let result = MockTools
                .execute(
                    ToolCall {
                        id: "limit".into(),
                        name: name.into(),
                        input,
                    },
                    ToolExecutionContext {
                        turn_id: "turn".into(),
                        request_id: "request".into(),
                        branch_id: "main".into(),
                    },
                    CancellationToken::new(),
                )
                .await;
            if size == MAX_MOCK_OUTPUT_BYTES {
                assert!(
                    matches!(result.unwrap().content, ToolContent::Text(text) if text.len() == size)
                );
            } else {
                assert!(
                    matches!(result, Err(HostError::Failed { message, .. }) if message == "Mock tool output exceeds 1 MiB")
                );
            }
        }
    }
}

#[derive(Clone)]
struct RecoveryProbe {
    mode: u8,
    calls: Arc<std::sync::atomic::AtomicUsize>,
    recovered: Arc<std::sync::atomic::AtomicUsize>,
    rejected: Arc<std::sync::atomic::AtomicUsize>,
    drained: Arc<AtomicBool>,
}
impl RecoveryProbe {
    fn connection(&self) -> ResolvedModelConnection {
        ResolvedModelConnection {
            connection: ModelConnection::Local,
            failure_policy: Some(Arc::new(self.clone())),
            recovery: (self.mode != 13)
                .then(|| Arc::new(self.clone()) as Arc<dyn RecoverModelConnection>),
        }
    }
    fn failure(&self, status: u16, retry_after: Option<Duration>) -> HostError {
        HostError::Failed {
            message: "fixture rejection".into(),
            budget: None,
            kind: HostErrorKind::Http {
                status: Some(status),
                code: None,
            },
            model: Some(Box::new(ModelFailureFacts {
                origin: ModelFailureOrigin::HttpResponse,
                error_type: Some("fixture_error".into()),
                message: Some("fixture rejection".into()),
                request_id: Some("fixture-request".into()),
                transient: status != 401,
                retry_after,
            })),
            presentation: None,
        }
    }
}
impl ResolveModelConnection for RecoveryProbe {
    fn resolve(
        &self,
        _: Arc<ModelRequest>,
        _: CancellationToken,
    ) -> HostFuture<'_, ResolvedModelConnection> {
        Box::pin(async move { Ok(self.connection()) })
    }
}
impl RecoverModelConnection for RecoveryProbe {
    fn rejection(&self, error: &HostError, phase: CredentialRecoveryPhase) -> CredentialRejection {
        if self.mode == 20 {
            return CredentialRejection::Recoverable;
        }
        if !matches!(
            error,
            HostError::Failed {
                kind: HostErrorKind::Http {
                    status: Some(401),
                    ..
                },
                ..
            }
        ) {
            return CredentialRejection::Unrelated;
        }
        match phase {
            CredentialRecoveryPhase::Initial => CredentialRejection::Recoverable,
            CredentialRecoveryPhase::Recovered => CredentialRejection::Terminal,
        }
    }
    fn recover(&self, cancel: CancellationToken) -> HostFuture<'_, RecoveredModelConnection> {
        Box::pin(async move {
            self.recovered.fetch_add(1, Ordering::SeqCst);
            if self.mode == 1 {
                assert!(self.drained.load(Ordering::SeqCst));
            }
            if self.mode == 6 {
                cancel.cancel();
            }
            Ok(RecoveredModelConnection {
                connection: ModelConnection::Local,
                recovery: Arc::new(self.clone()),
            })
        })
    }
    fn finish_rejected(&self, _: CancellationToken) -> HostFuture<'_, ()> {
        Box::pin(async move {
            self.rejected.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}
impl ModelFailurePolicy for RecoveryProbe {
    fn forbids_retry(&self, error: &HostError) -> bool {
        self.mode == 13
            || matches!(error, HostError::Failed {
            kind: HostErrorKind::Http { code: Some(code), .. }, ..
        } if code == "fixture_veto")
    }
    fn presentation(&self, _: &HostError) -> Option<ProviderPresentation> {
        Some(ProviderPresentation {
            source: "fixture_provider".into(),
            reason: "unknown_error".into(),
            action: None,
            request_id: None,
            server_message: None,
        })
    }
}
impl ModelDriver for RecoveryProbe {
    fn invoke(
        &self,
        request: Arc<ModelRequest>,
        connection: ModelConnection,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ModelStream> {
        Box::pin(async move {
            assert!(
                !cancel.is_cancelled(),
                "retry inherited failed attempt cancellation"
            );
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if call > 0 && !matches!(self.mode, 4 | 7 | 8 | 12 | 15..=20 | 23) {
                return EchoModel.invoke(request, connection, cancel).await;
            }
            let status = match self.mode {
                20 => 403,
                16 if call < 2 => 503,
                19 if call == 0 => 503,
                17 | 23 if call > 0 => 503,
                5 | 7 | 12 | 13 => 503,
                8 if call > 0 => 429,
                9..=11 | 14 => 529,
                _ => 401,
            };
            let mut error = if self.mode >= 7 {
                self.failure(
                    status,
                    match self.mode {
                        12 | 14 | 17 | 22 => Some(Duration::from_secs(40)),
                        21 => Some(Duration::from_secs(61)),
                        23 if call > 0 => Some(Duration::from_secs(60)),
                        _ => None,
                    },
                )
            } else {
                HostError::classified(
                    HostErrorKind::Http {
                        status: Some(status),
                        code: None,
                    },
                    "fixture rejection",
                )
            };
            if self.mode == 15
                && call > 0
                && let HostError::Failed {
                    kind: HostErrorKind::Http { code, .. },
                    ..
                } = &mut error
            {
                *code = Some("fixture_veto".into());
            }
            if !matches!(self.mode, 1..=3 | 9..=11) && !(self.mode == 18 && call > 0) {
                return Err(error);
            }
            let mut chunks = vec![Ok(ModelChunk::MessageStart {
                message_id: "first".into(),
            })];
            if self.mode == 2 || self.mode == 18 {
                chunks.push(Ok(ModelChunk::Text("already published".into())));
            }
            if self.mode == 3 {
                chunks.push(Ok(ModelChunk::Usage(
                    serde_json::from_value(
                        serde_json::json!({"promptTokens":7,"completionTokens":2,"totalTokens":9}),
                    )
                    .unwrap(),
                )));
            }
            if self.mode == 10 {
                chunks.push(Ok(ModelChunk::Usage(
                    serde_json::from_value(
                        serde_json::json!({"promptTokens":7,"completionTokens":0,"totalTokens":7}),
                    )
                    .unwrap(),
                )));
            }
            if self.mode == 11 {
                chunks.extend([
                    Ok(ModelChunk::ToolStart {
                        id: "partial".into(),
                        name: "echo".into(),
                    }),
                    Ok(ModelChunk::ToolArguments {
                        id: "partial".into(),
                        name: "echo".into(),
                        delta: "{\"text\":".into(),
                    }),
                ]);
            }
            chunks.push(Err(error));
            let drained = self.drained.clone();
            let stream = futures_util::stream::unfold(
                (chunks.into_iter(), cancel, drained),
                |(mut chunks, cancel, drained)| async move {
                    if let Some(chunk) = chunks.next() {
                        return Some((chunk, (chunks, cancel, drained)));
                    }
                    cancel.cancelled().await;
                    drained.store(true, Ordering::SeqCst);
                    None
                },
            );
            Ok(Box::pin(stream) as ModelStream)
        })
    }
}
struct RecoveryHosts(RecoveryProbe);
impl TypedPlugin for RecoveryHosts {
    type Deps = ();
    fn name(&self) -> &str {
        "test.recovery.hosts"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            ctx.provide(ModelHost(Arc::new(self.0.clone())))?;
            ctx.provide(ProviderConnectionResolver(Arc::new(self.0.clone())))?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for RecoveryHosts {
    fn keys(&self) -> KeysOf<Self> {}
}
#[tokio::test]
async fn process_model_recovery_respects_output_cleanup_and_cancellation() {
    tokio::time::timeout(Duration::from_secs(15), async {
        for mode in 0..=6 {
            let probe = RecoveryProbe {
                mode,
                calls: Default::default(),
                recovered: Default::default(),
                rejected: Default::default(),
                drained: Default::default(),
            };
            let root = Ctx::root().unwrap();
            root.plugin(capability_plugin(RecoveryHosts(probe.clone())));
            let mut builder = AgentBuilder::new(AgentConfig::default());
            builder
                .session_value(Arc::new(ToolHost(Arc::new(MockTools))))
                .session_value(Arc::new(
                    dim_agent_capabilities::history::HistoryRenderer::default(),
                ))
                .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
                .session_value(Arc::new(ContextModelHost(None)))
                .session_value(Arc::new(CompactionStrategyHost(None)))
                .session_part(|names| names.part(DefaultPrepareModelPlugin::new(names)?));
            defaults::install_with_process_model(&mut builder);
            let agent = builder.build_in(&root);
            let session = agent
                .create_session(SessionConfig::default())
                .await
                .unwrap();
            session
                .submit("hello".into(), InputMode::Queue, "main")
                .await
                .unwrap();
            session.idle("main").await.unwrap();
            let events = session.history("main", None).await.unwrap();
            let ends: Vec<_> = events
                .iter()
                .filter_map(|event| match &event.kind {
                    EventKind::ModelOutputEnd(end) => Some(end),
                    _ => None,
                })
                .collect();
            assert_eq!(ends.len(), 1, "mode {mode}");
            assert_eq!(
                matches!(ends[0].outcome, ModelOutputOutcome::Completed { .. }),
                mode <= 1,
                "mode {mode}"
            );
            assert_eq!(
                probe.calls.load(Ordering::SeqCst),
                if matches!(mode, 0 | 1 | 4) { 2 } else { 1 },
                "mode {mode}"
            );
            assert_eq!(
                probe.recovered.load(Ordering::SeqCst),
                usize::from(matches!(mode, 0 | 1 | 4 | 6)),
                "mode {mode}"
            );
            assert_eq!(
                probe.rejected.load(Ordering::SeqCst),
                usize::from(mode == 4),
                "mode {mode}"
            );
            if mode == 2 {
                let text: Vec<_> = events
                    .iter()
                    .filter_map(|event| match &event.kind {
                        EventKind::Text(data) => Some(data.delta.as_str()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(text, ["already published"]);
            }
            if matches!(mode, 1..=3) {
                assert!(probe.drained.load(Ordering::SeqCst));
            }
            agent.close().await.unwrap();
            root.shutdown().await.unwrap();
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn transient_retries_are_bounded_and_stop_after_irreversible_output() {
    tokio::time::timeout(Duration::from_secs(15), async {
        // 7: repeated 503; 8: 401 then 429; 9: MessageStart then 529;
        // 10: usage then 529; 11: partial tool arguments then 529; 13: business veto.
        // 15: veto after recovery; 16: first rejection on the last attempt;
        // 18: output before terminal rejection; 19: terminal rejection on attempt three.
        for (mode, expected_calls, expected_refresh, expected_rejected, completed) in [
            (7, 3, 0, 0, false),
            (8, 3, 1, 0, false),
            (9, 2, 0, 0, true),
            (10, 1, 0, 0, false),
            (11, 1, 0, 0, false),
            (13, 1, 0, 0, false),
            (15, 2, 1, 1, false),
            (16, 3, 0, 0, false),
            (18, 2, 1, 1, false),
            (19, 3, 1, 1, false),
            // Authority-specific 403 recovery, capped even if requested again.
            (20, 2, 1, 0, false),
            // No refresh when Retry-After already exceeds the whole wait budget.
            (21, 1, 0, 0, false),
        ] {
            let probe = RecoveryProbe {
                mode,
                calls: Default::default(),
                recovered: Default::default(),
                rejected: Default::default(),
                drained: Default::default(),
            };
            let root = Ctx::root().unwrap();
            root.plugin(capability_plugin(RecoveryHosts(probe.clone())));
            let mut builder = AgentBuilder::new(AgentConfig::default());
            builder
                .session_value(Arc::new(ToolHost(Arc::new(MockTools))))
                .session_value(Arc::new(
                    dim_agent_capabilities::history::HistoryRenderer::default(),
                ))
                .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
                .session_value(Arc::new(ContextModelHost(None)))
                .session_value(Arc::new(CompactionStrategyHost(None)))
                .session_part(|names| names.part(DefaultPrepareModelPlugin::new(names)?));
            defaults::install_with_process_model(&mut builder);
            let agent = builder.build_in(&root);
            let session = agent
                .create_session(SessionConfig::default())
                .await
                .unwrap();
            session
                .submit("retry contract".into(), InputMode::Queue, "main")
                .await
                .unwrap();
            session.idle("main").await.unwrap();

            assert_eq!(
                probe.calls.load(Ordering::SeqCst),
                expected_calls,
                "mode {mode}"
            );
            assert_eq!(
                probe.recovered.load(Ordering::SeqCst),
                expected_refresh,
                "mode {mode}"
            );
            assert_eq!(
                probe.rejected.load(Ordering::SeqCst),
                expected_rejected,
                "mode {mode}"
            );
            let events = session.history("main", None).await.unwrap();
            let ends: Vec<_> = events
                .iter()
                .filter_map(|event| match &event.kind {
                    EventKind::ModelOutputEnd(end) => Some(&end.outcome),
                    _ => None,
                })
                .collect();
            assert_eq!(ends.len(), 1, "mode {mode}");
            if !completed {
                assert!(
                    matches!(ends[0], ModelOutputOutcome::Failed {
                    presentation: Some(value), ..
                } if value.reason == "unknown_error"),
                    "mode {mode}"
                );
            }
            assert_eq!(
                matches!(ends[0], ModelOutputOutcome::Completed { .. }),
                completed,
                "mode {mode}"
            );
            agent.close().await.unwrap();
            root.shutdown().await.unwrap();
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn waiting_to_retry_obeys_stop_steer_and_provider_reload() {
    tokio::time::timeout(Duration::from_secs(15), async {
        for (mode, action) in [14, 22]
            .into_iter()
            .flat_map(|mode| ["stop", "steer", "reload"].map(|action| (mode, action)))
        {
            let probe = RecoveryProbe {
                mode,
                calls: Default::default(),
                recovered: Default::default(),
                rejected: Default::default(),
                drained: Default::default(),
            };
            let root = Ctx::root().unwrap();
            let provider = root.plugin(capability_plugin(RecoveryHosts(probe.clone())));
            let mut builder = AgentBuilder::new(AgentConfig::default());
            builder
                .session_value(Arc::new(ToolHost(Arc::new(MockTools))))
                .session_value(Arc::new(
                    dim_agent_capabilities::history::HistoryRenderer::default(),
                ))
                .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
                .session_value(Arc::new(ContextModelHost(None)))
                .session_value(Arc::new(CompactionStrategyHost(None)))
                .session_part(|names| names.part(DefaultPrepareModelPlugin::new(names)?));
            defaults::install_with_process_model(&mut builder);
            let agent = builder.build_in(&root);
            let session = agent
                .create_session(SessionConfig::default())
                .await
                .unwrap();
            session
                .submit("wait before retry".into(), InputMode::Queue, "main")
                .await
                .unwrap();
            while probe.calls.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
            tokio::task::yield_now().await;

            let mut steered = None;
            match action {
                "stop" => {
                    session.stop("main", "user stopped".into()).await.unwrap();
                }
                "steer" => {
                    steered = Some(
                        session
                            .submit("replacement".into(), InputMode::Steer, "main")
                            .await
                            .unwrap()
                            .id
                            .clone(),
                    );
                }
                "reload" => provider.restart().await.unwrap(),
                _ => unreachable!(),
            }
            session.idle("main").await.unwrap();
            assert_eq!(probe.recovered.load(Ordering::SeqCst), 0);
            assert_eq!(probe.rejected.load(Ordering::SeqCst), 0);
            let events = session.history("main", None).await.unwrap();
            match action {
                "stop" => {
                    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
                    let outcomes: Vec<_> = events
                        .iter()
                        .filter_map(|event| match &event.kind {
                            EventKind::ModelOutputEnd(data) => Some(&data.outcome),
                            _ => None,
                        })
                        .collect();
                    assert!(
                        matches!(
                            outcomes.as_slice(),
                            [ModelOutputOutcome::Cancelled { reason }] if reason == "user stopped"
                        ),
                        "{outcomes:?}"
                    );
                    assert!(matches!(
                        &events.last().unwrap().kind,
                        EventKind::TurnEnd(TurnEndData {
                            outcome: TurnOutcome::Cancelled { reason },
                            ..
                        }) if reason == "user stopped"
                    ));
                }
                "steer" => {
                    assert_eq!(probe.calls.load(Ordering::SeqCst), 2);
                    assert!(events.iter().any(|event| matches!(
                        &event.kind,
                        EventKind::InputConsumed(data) if Some(&data.input_id) == steered.as_ref()
                    )));
                    assert!(matches!(
                        &events.last().unwrap().kind,
                        EventKind::TurnEnd(TurnEndData {
                            outcome: TurnOutcome::Completed { .. },
                            ..
                        })
                    ));
                }
                "reload" => {
                    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
                    assert!(events.iter().any(|event| matches!(
                        &event.kind,
                        EventKind::ModelOutputEnd(ModelOutputEndData {
                            outcome: ModelOutputOutcome::Cancelled { reason },
                            ..
                        }) if reason == CAPABILITY_RELOADING
                    )));
                }
                _ => unreachable!(),
            }
            agent.close().await.unwrap();
            root.shutdown().await.unwrap();
        }
    })
    .await
    .unwrap();
}

struct CompletedThenDrained {
    draining: Arc<Notify>,
}
impl ModelDriver for CompletedThenDrained {
    fn invoke(
        &self,
        _: Arc<ModelRequest>,
        _: ModelConnection,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ModelStream> {
        let draining = self.draining.clone();
        Box::pin(async move {
            let chunks = vec![
                ModelChunk::MessageStart {
                    message_id: "complete-before-stop".into(),
                },
                ModelChunk::Text("finished".into()),
                ModelChunk::TextEnd,
                ModelChunk::MessageStop {
                    message_id: "complete-before-stop".into(),
                    reason: ModelStopReason::EndTurn,
                },
            ];
            let stream = futures_util::stream::unfold(
                (chunks.into_iter(), cancel, draining),
                |(mut chunks, cancel, draining)| async move {
                    if let Some(chunk) = chunks.next() {
                        return Some((Ok(chunk), (chunks, cancel, draining)));
                    }
                    draining.notify_one();
                    cancel.cancelled().await;
                    None
                },
            );
            Ok(Box::pin(stream) as ModelStream)
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_during_native_drain_keeps_an_already_completed_model_result() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let draining = Arc::new(Notify::new());
        let agent = builder(
            Arc::new(CompletedThenDrained {
                draining: draining.clone(),
            }),
            Arc::new(MockTools),
        )
        .build();
        let session = agent
            .create_session(SessionConfig::default())
            .await
            .unwrap();
        session
            .submit("finish before stop".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        draining.notified().await;
        session
            .stop("main", "stop during drain".into())
            .await
            .unwrap();
        let events = session.history("main", None).await.unwrap();
        assert!(events.iter().any(|event| matches!(
            &event.kind,
            EventKind::ModelOutputEnd(ModelOutputEndData {
                outcome: ModelOutputOutcome::Completed { .. },
                ..
            })
        )));
        assert!(events.iter().any(|event| matches!(
            &event.kind,
            EventKind::Text(data) if data.delta == "finished"
        )));
        agent.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test(start_paused = true)]
async fn retry_after_uses_one_real_sixty_second_budget() {
    for mode in [12, 17, 23] {
        let probe = RecoveryProbe {
            mode,
            calls: Default::default(),
            recovered: Default::default(),
            rejected: Default::default(),
            drained: Default::default(),
        };
        let root = Ctx::root().unwrap();
        root.plugin(capability_plugin(RecoveryHosts(probe.clone())));
        let mut builder = AgentBuilder::new(AgentConfig::default());
        builder
            .session_value(Arc::new(ToolHost(Arc::new(MockTools))))
            .session_value(Arc::new(
                dim_agent_capabilities::history::HistoryRenderer::default(),
            ))
            .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
            .session_value(Arc::new(ContextModelHost(None)))
            .session_value(Arc::new(CompactionStrategyHost(None)))
            .session_part(|names| names.part(DefaultPrepareModelPlugin::new(names)?));
        defaults::install_with_process_model(&mut builder);
        let agent = builder.build_in(&root);
        let session = agent
            .create_session(SessionConfig::default())
            .await
            .unwrap();
        session
            .submit("budget contract".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        if mode == 23 {
            // Immediate credential recovery must leave the full wait budget available.
            let started = tokio::time::Instant::now();
            session.idle("main").await.unwrap();
            assert!(started.elapsed() >= Duration::from_secs(60));
        } else {
            while probe.calls.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
            tokio::time::advance(Duration::from_secs(39)).await;
            tokio::task::yield_now().await;
            assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
            assert_eq!(probe.recovered.load(Ordering::SeqCst), 0);
            tokio::time::advance(Duration::from_secs(1)).await;
            session.idle("main").await.unwrap();
        }
        assert_eq!(
            probe.calls.load(Ordering::SeqCst),
            if mode == 23 { 3 } else { 2 }
        );
        assert_eq!(
            probe.recovered.load(Ordering::SeqCst),
            usize::from(mode != 12)
        );
        assert!(
            session
                .history("main", None)
                .await
                .unwrap()
                .iter()
                .any(|event| matches!(
                    &event.kind,
                    EventKind::ModelOutputEnd(ModelOutputEndData {
                        outcome: ModelOutputOutcome::Failed { .. },
                        ..
                    })
                ))
        );
        agent.close().await.unwrap();
        root.shutdown().await.unwrap();
    }
}

/// An append input written while idle starts no turn; the next driving
/// input's turn consumes it with that input.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_idle_append_starts_no_turn_and_joins_the_next_one() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let agent = builder(Arc::new(RecordingModel(requests.clone())), Arc::new(MockTools)).build();
        let session = agent.create_session(SessionConfig::default()).await.unwrap();
        let appended = session.submit("idle context".into(), InputMode::Append, "main").await.unwrap();
        session.idle("main").await.unwrap();
        let events = session.history("main", None).await.unwrap();
        assert!(events.iter().any(|event| matches!(&event.kind,
            EventKind::InputApplied(data) if data.input_id == appended.id && data.mode == Some(InputMode::Append))));
        assert!(!events.iter().any(|event| matches!(event.kind, EventKind::TurnStart(_))));
        session.submit("go".into(), InputMode::Queue, "main").await.unwrap();
        session.idle("main").await.unwrap();
        let requests = requests.lock().unwrap().clone();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].messages.contains(&Arc::new(Message::text(Role::User, "idle context"))));
        assert!(requests[0].messages.contains(&Arc::new(Message::text(Role::User, "go"))));
        let events = session.history("main", None).await.unwrap();
        assert_eq!(events.iter().filter(|event| matches!(event.kind, EventKind::TurnStart(_))).count(), 1);
        agent.close().await.unwrap();
    }).await.unwrap();
}

/// Streams one text chunk, then holds the end of its first output until
/// released; later calls echo.
struct HeldOutput {
    requests: Arc<Mutex<Vec<Arc<ModelRequest>>>>,
    release: Arc<Notify>,
}
impl ModelDriver for HeldOutput {
    fn invoke(
        &self,
        request: Arc<ModelRequest>,
        connection: dim_agent_capabilities::host::ModelConnection,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ModelStream> {
        let first = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request.clone());
            requests.len() == 1
        };
        if !first {
            return EchoModel.invoke(request, connection, cancel);
        }
        let release = self.release.clone();
        Box::pin(async move {
            use futures_util::StreamExt;
            let head = futures_util::stream::iter(vec![
                Ok(ModelChunk::MessageStart {
                    message_id: "held".into(),
                }),
                Ok(ModelChunk::Text("partial answer".into())),
                Ok(ModelChunk::TextEnd),
            ]);
            let tail = futures_util::stream::once(async move {
                release.notified().await;
                Ok(ModelChunk::MessageStop {
                    message_id: "held".into(),
                    reason: ModelStopReason::EndTurn,
                })
            });
            Ok(Box::pin(head.chain(tail)) as ModelStream)
        })
    }
}

/// A steer applied while the model is outputting waits for the output to
/// end; an output without tool calls then does not end the turn: the same
/// turn calls the model again with the steer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_steer_during_output_continues_the_turn_instead_of_ending_it() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let release = Arc::new(Notify::new());
        let agent = builder(
            Arc::new(HeldOutput { requests: requests.clone(), release: release.clone() }),
            Arc::new(MockTools),
        ).build();
        let session = agent.create_session(SessionConfig::default()).await.unwrap();
        session.submit("first".into(), InputMode::Queue, "main").await.unwrap();
        let mut dispatch = session.dispatch_watch("main").unwrap();
        loop {
            let events = session.history("main", None).await.unwrap();
            if events.iter().any(|event| matches!(event.kind, EventKind::Text(_)))
                && dispatch.borrow().sequence >= events.last().unwrap().sequence
            {
                break;
            }
            dispatch.changed().await.unwrap();
        }
        let steer = session.submit("STEER-MARKER".into(), InputMode::Steer, "main").await.unwrap();
        loop {
            let events = session.history("main", None).await.unwrap();
            if events.iter().any(|event| matches!(&event.kind, EventKind::InputApplied(data) if data.input_id == steer.id))
                && dispatch.borrow().sequence >= events.last().unwrap().sequence
            {
                break;
            }
            dispatch.changed().await.unwrap();
        }
        release.notify_one();
        session.idle("main").await.unwrap();
        let requests = requests.lock().unwrap().clone();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].messages.contains(&Arc::new(Message::text(Role::User, "STEER-MARKER"))));
        let events = session.history("main", None).await.unwrap();
        assert_eq!(events.iter().filter(|event| matches!(event.kind, EventKind::TurnStart(_))).count(), 1);
        assert_eq!(events.iter().filter(|event| matches!(event.kind, EventKind::TurnEnd(_))).count(), 1);
        assert!(events.iter().any(|event| matches!(&event.kind, EventKind::Text(data) if data.delta == "partial answer")));
        agent.close().await.unwrap();
    }).await.unwrap();
}

/// Streams the chunks a test sends to its first call; later calls echo.
struct ChannelModel(Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<ModelChunk>>>);
impl ModelDriver for ChannelModel {
    fn invoke(
        &self,
        request: Arc<ModelRequest>,
        connection: dim_agent_capabilities::host::ModelConnection,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ModelStream> {
        let Some(receiver) = self.0.lock().unwrap().take() else {
            return EchoModel.invoke(request, connection, cancel);
        };
        Box::pin(async move {
            let stream = futures_util::stream::unfold(receiver, |mut receiver| async move {
                receiver.recv().await.map(|chunk| (Ok(chunk), receiver))
            });
            Ok(Box::pin(stream) as ModelStream)
        })
    }
}

/// Output reaches the Timeline per fact: text once per block (split when a
/// block grows past the threshold), thinking once per whole block, a tool
/// call at its start and when complete. Until then it is progress state,
/// cleared as it is committed and when the request ends.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_output_is_committed_per_block_and_shown_as_progress_until_then() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let (chunks, receiver) = tokio::sync::mpsc::unbounded_channel();
        let agent = builder(
            Arc::new(ChannelModel(Mutex::new(Some(receiver)))),
            Arc::new(MockTools),
        )
        .build();
        let session = agent
            .create_session(SessionConfig::default())
            .await
            .unwrap();
        session
            .submit("go".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        let mut progress = session.on::<ModelProgress>("main").unwrap().unwrap();
        let outputs = |events: &[Arc<AgentEvent>]| -> Vec<String> {
            events
                .iter()
                .filter_map(|event| match &event.kind {
                    EventKind::Thinking(data) => Some(format!("thinking:{}", data.delta)),
                    EventKind::Text(data) => Some(format!("text:{}", data.delta.len())),
                    EventKind::ToolUseStart(data) => Some(format!("start:{}", data.call_id)),
                    EventKind::ToolUse(data) => Some(format!("use:{}", data.call.id)),
                    _ => None,
                })
                .collect()
        };
        let send = |list: Vec<ModelChunk>| {
            for chunk in list {
                chunks.send(chunk).unwrap();
            }
        };
        send(vec![
            ModelChunk::MessageStart {
                message_id: "blocks".into(),
            },
            ModelChunk::Thinking("deep ".into()),
            ModelChunk::Thinking("thought".into()),
        ]);
        progress
            .wait_for(|progress| progress.thinking == "deep thought")
            .await
            .unwrap();
        assert!(outputs(&session.history("main", None).await.unwrap()).is_empty());
        send(vec![
            ModelChunk::ThinkingEnd {
                thinking: "deep thought".into(),
            },
            ModelChunk::Text("Hel".into()),
            ModelChunk::Text("lo".into()),
        ]);
        progress
            .wait_for(|progress| progress.text == "Hello" && progress.thinking.is_empty())
            .await
            .unwrap();
        // The published thinking block now precedes the progress thinking.
        let shown = session.get::<ModelProgress>("main").unwrap().unwrap();
        assert_eq!((shown.text_offset, shown.thinking_offset), (0, 12));
        assert_eq!(
            outputs(&session.history("main", None).await.unwrap()),
            ["thinking:deep thought"]
        );
        // 3 KB pieces: the buffer passes 4 KB at the second and is written then.
        let piece = "a".repeat(3000);
        send(vec![
            ModelChunk::TextEnd,
            ModelChunk::Text(piece.clone()),
            ModelChunk::Text(piece.clone()),
            ModelChunk::Text(piece),
            ModelChunk::TextEnd,
            ModelChunk::ToolStart {
                id: "call".into(),
                name: "echo".into(),
            },
            ModelChunk::ToolArguments {
                id: "call".into(),
                name: "echo".into(),
                delta: "{\"text\":".into(),
            },
        ]);
        progress
            .wait_for(|progress| {
                progress.text.is_empty()
                    && progress.tools
                        == [ToolProgress {
                            call_id: "call".into(),
                            name: "echo".into(),
                            arguments: "{\"text\":".into(),
                        }]
            })
            .await
            .unwrap();
        assert_eq!(
            outputs(&session.history("main", None).await.unwrap()),
            [
                "thinking:deep thought",
                "text:5",
                "text:6000",
                "text:3000",
                "start:call"
            ]
        );
        let shown = session.get::<ModelProgress>("main").unwrap().unwrap();
        assert_eq!((shown.text_offset, shown.thinking_offset), (9005, 12));
        send(vec![
            ModelChunk::ToolArguments {
                id: "call".into(),
                name: "echo".into(),
                delta: "\"hi\"}".into(),
            },
            ModelChunk::ToolUse(ToolCall {
                id: "call".into(),
                name: "echo".into(),
                input: serde_json::from_value(serde_json::json!({"text":"hi"})).unwrap(),
            }),
            ModelChunk::MessageStop {
                message_id: "blocks".into(),
                reason: ModelStopReason::ToolUse,
            },
        ]);
        drop(chunks);
        session.idle("main").await.unwrap();
        let events = session.history("main", None).await.unwrap();
        assert_eq!(
            outputs(&events)[..6],
            [
                "thinking:deep thought",
                "text:5",
                "text:6000",
                "text:3000",
                "start:call",
                "use:call"
            ]
        );
        assert!(events.iter().any(|event| matches!(&event.kind,
            EventKind::ToolResult(data) if data.call_event_id.is_some())));
        assert_eq!(
            session.get::<ModelProgress>("main").unwrap().unwrap(),
            ModelProgress::default()
        );
        agent.close().await.unwrap();
    })
    .await
    .unwrap();
}

/// Text buffered under the size threshold is committed once its oldest part
/// is a second old, checked when the next text arrives; the progress offset
/// then counts the published text in Unicode scalar values.
#[tokio::test(start_paused = true)]
async fn buffered_text_is_committed_once_a_second_old() {
    // Paused time advances to this deadline only when the flow is stuck.
    tokio::time::timeout(Duration::from_secs(30), async {
        let (chunks, receiver) = tokio::sync::mpsc::unbounded_channel();
        let agent = builder(
            Arc::new(ChannelModel(Mutex::new(Some(receiver)))),
            Arc::new(MockTools),
        )
        .build();
        let session = agent
            .create_session(SessionConfig::default())
            .await
            .unwrap();
        session
            .submit("go".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        let mut progress = session.on::<ModelProgress>("main").unwrap().unwrap();
        let texts = async || -> Vec<String> {
            session
                .history("main", None)
                .await
                .unwrap()
                .iter()
                .filter_map(|event| match &event.kind {
                    EventKind::Text(data) => Some(data.delta.clone()),
                    _ => None,
                })
                .collect()
        };
        for chunk in [
            ModelChunk::MessageStart {
                message_id: "aged".into(),
            },
            ModelChunk::Text("é".into()),
            ModelChunk::Text("ü".into()),
        ] {
            chunks.send(chunk).unwrap();
        }
        progress
            .wait_for(|progress| progress.text == "éü")
            .await
            .unwrap();
        assert!(texts().await.is_empty(), "younger than a second");
        tokio::time::advance(Duration::from_secs(1)).await;
        chunks.send(ModelChunk::Text("ß".into())).unwrap();
        progress
            .wait_for(|progress| progress.text.is_empty() && progress.text_offset == 3)
            .await
            .unwrap();
        assert_eq!(texts().await, ["éüß"]);
        for chunk in [
            ModelChunk::TextEnd,
            ModelChunk::MessageStop {
                message_id: "aged".into(),
                reason: ModelStopReason::EndTurn,
            },
        ] {
            chunks.send(chunk).unwrap();
        }
        drop(chunks);
        session.idle("main").await.unwrap();
        agent.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_held_input_is_withdrawn_once_and_an_applied_one_is_not() {
    use dim_agent_capabilities::input_queue::{InputQueue, Withdrawal};
    tokio::time::timeout(Duration::from_secs(10), async {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let agent = builder(
            Arc::new(RecordingModel(requests.clone())),
            Arc::new(BoundaryTools {
                entered: entered.clone(),
                release: release.clone(),
            }),
        )
        .build();
        let session = agent
            .create_session(SessionConfig {
                tools: vec!["boundary".into()],
                permission_policy: dim_agent_runtime::PermissionPolicy {
                    preset: dim_agent_runtime::PermissionPreset::FullAccess,
                    ..Default::default()
                },
                ..Default::default()
            })
            .await
            .unwrap();
        session
            .submit("tool:boundary {}".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        entered.notified().await;
        // Both queue inputs wait for the running turn; the steer is applied.
        let first = session
            .submit("first queued".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        let second = session
            .submit("second queued".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        let steered = session
            .submit("applied steer".into(), InputMode::Steer, "main")
            .await
            .unwrap();
        let queue = session.service::<InputQueue>("main").unwrap();
        let withdraw = |input_id: &str| InputWithdrawData {
            input_id: input_id.into(),
            reason: Some("user removed it".into()),
        };

        let (answer, replacement) = queue.withdraw(withdraw(&first.id), None).await.unwrap();
        let Withdrawal::Deleted { sequence } = answer else {
            panic!("held input not deleted: {answer:?}")
        };
        assert!(replacement.is_none());
        // A second withdrawal, an unknown id and an applied input write nothing.
        assert_eq!(
            queue.withdraw(withdraw(&first.id), None).await.unwrap().0,
            Withdrawal::NotHeld
        );
        assert_eq!(
            queue.withdraw(withdraw("unknown"), None).await.unwrap().0,
            Withdrawal::NotHeld
        );
        let mut dispatch = session.dispatch_watch("main").unwrap();
        while !session
            .history("main", None)
            .await
            .unwrap()
            .iter()
            .any(|event| {
                matches!(&event.kind,
            EventKind::InputApplied(data) if data.input_id == steered.id)
            })
        {
            dispatch.changed().await.unwrap();
        }
        assert_eq!(
            queue.withdraw(withdraw(&steered.id), None).await.unwrap().0,
            Withdrawal::NotHeld
        );
        // Withdraw and re-submit as Steer: one batch, the new input joins this turn.
        let user = UserInput {
            content: vec![ContentBlock::Text {
                text: "second, now as steer".into(),
            }],
        };
        let (answer, replacement) = queue
            .withdraw(
                withdraw(&second.id),
                Some(InputMessageData {
                    user: Some(user),
                    mode: Some(InputMode::Steer),
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        assert!(matches!(answer, Withdrawal::Deleted { .. }));
        let replacement = replacement.unwrap();
        // A replacement that is not Steer is rejected before anything is written.
        assert!(
            queue
                .withdraw(
                    withdraw(&second.id),
                    Some(InputMessageData::text("x", Some(InputMode::Queue)))
                )
                .await
                .is_err()
        );

        release.notify_one();
        session.idle("main").await.unwrap();
        let events = session.history("main", None).await.unwrap();
        let deleted: Vec<_> = events
            .iter()
            .filter_map(|event| match &event.kind {
                EventKind::InputDeleted(data) => Some((event.sequence, data.input_id.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            deleted,
            [
                (sequence, first.id.clone()),
                (deleted[1].0, second.id.clone())
            ]
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event.kind, EventKind::InputWithdraw(_)))
                .count(),
            5
        );
        for id in [&first.id, &second.id] {
            assert!(!events.iter().any(|event| matches!(&event.kind,
                EventKind::InputApplied(data) if &data.input_id == id)));
            assert!(!events.iter().any(|event| matches!(&event.kind,
                EventKind::InputConsumed(data) if &data.input_id == id)));
        }
        assert!(events.iter().any(|event| matches!(&event.kind,
            EventKind::InputConsumed(data) if data.input_id == replacement.id)));
        // Deleted inputs never reach the model, and no turn is left to run them.
        let requests = requests.lock().unwrap().clone();
        assert!(
            requests
                .iter()
                .all(|request| !request.messages.iter().any(|message| [
                    Message::text(Role::User, "first queued"),
                    Message::text(Role::User, "second queued")
                ]
                .contains(message)))
        );
        assert!(
            requests
                .last()
                .unwrap()
                .messages
                .contains(&Arc::new(Message::text(Role::User, "second, now as steer")))
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event.kind, EventKind::TurnStart(_)))
                .count(),
            1
        );
        agent.close().await.unwrap();
    })
    .await
    .unwrap();
}

fn model_with_vision(model_id: &str, vision: bool) -> ModelSelection {
    ModelSelection {
        provider_id: "test".into(),
        model_id: model_id.into(),
        capability_snapshot: Some(ModelCapabilitySnapshot {
            target: ModelId {
                provider_id: "test".into(),
                model_id: model_id.into(),
            },
            capabilities: ModelCapabilities {
                modalities: vec!["text".into()],
                tool_calling: true,
                streaming: true,
                structured_output: false,
                vision,
                audio: false,
                reasoning: false,
                context_window: 32_000,
                max_output_tokens: 4096,
                provider_state: None,
            },
            reasoning: None,
            service_tier: None,
        }),
        config: None,
        metadata: None,
    }
}
fn image(media_type: &str, data: Option<&str>, url: Option<&str>) -> ContentBlock {
    ContentBlock::Image {
        source: ImageSource {
            source_type: if url.is_some() {
                ImageSourceType::Url
            } else {
                ImageSourceType::Base64
            },
            media_type: media_type.into(),
            data: data.map(str::to_owned),
            url: url.map(str::to_owned),
        },
    }
}
fn sent_parts(request: &ModelRequest) -> Vec<String> {
    request
        .messages
        .iter()
        .flat_map(|message| match &message.content {
            MessageContent::Parts(parts) => parts
                .iter()
                .map(|part| match part {
                    MessagePart::Image { source } => format!("image:{}", source.media_type),
                    MessagePart::Text { text, .. } => text.clone(),
                    _ => String::new(),
                })
                .collect(),
            MessageContent::Text(text) => vec![text.clone()],
        })
        .collect()
}

#[tokio::test]
async fn a_session_switched_to_a_text_only_model_sends_image_placeholders() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let agent = builder(Arc::new(RecordingModel(requests.clone())), Arc::new(MockTools)).build();
        let session = agent.create_session(SessionConfig::default()).await.unwrap();
        session
            .configure(ConfigureUpdate { model: Some(model_with_vision("seeing", true)), ..Default::default() }, "main")
            .await
            .unwrap();
        let png = "iVBORw0KGgo=";
        session
            .input(
                InputMessageData {
                    user: Some(UserInput {
                        content: vec![
                            ContentBlock::Text { text: "look".into() },
                            image("image/png", Some(png), None),
                            image("image/png", None, Some("https://example.test/a.png")),
                            image("image/bmp", Some(png), None),
                            image("image/png", Some("not base64!"), None),
                        ],
                    }),
                    mode: Some(InputMode::Queue),
                    ..Default::default()
                },
                "main",
            )
            .await
            .unwrap();
        session.idle("main").await.unwrap();
        // A model that takes images gets the sendable one; the others cannot be sent.
        let unsendable = "[Image omitted: the image cannot be sent to this model.]";
        assert_eq!(
            sent_parts(requests.lock().unwrap().last().unwrap()),
            ["look", "image:image/png", unsendable, unsendable, unsendable]
        );

        session
            .configure(ConfigureUpdate { model: Some(model_with_vision("text-only", false)), ..Default::default() }, "main")
            .await
            .unwrap();
        session.submit("next".into(), InputMode::Queue, "main").await.unwrap();
        session.idle("main").await.unwrap();
        // The text-only model gets placeholders for every image and still answers.
        let request = requests.lock().unwrap().last().unwrap().clone();
        assert_eq!(request.model_id, "text-only");
        let omitted = "[Image omitted: the current model does not accept images.]";
        let parts = sent_parts(&request);
        assert_eq!(parts[..5], ["look", omitted, omitted, omitted, omitted]);
        assert!(parts.contains(&"next".to_string()));
        let history = session.history("main", None).await.unwrap();
        assert!(history.iter().any(|event| matches!(&event.kind, EventKind::Text(data) if data.delta.contains("next"))));
        // The adopted input and the committed request keep their images.
        assert!(history.iter().any(|event| matches!(&event.kind,
            EventKind::InputMessage(data) if data.user.as_ref().is_some_and(|user| user.content.iter().any(|block| matches!(block, ContentBlock::Image { .. }))))));
        assert!(history.iter().any(|event| matches!(&event.kind,
            EventKind::ModelCall(call) if call.request.model_id == "text-only"
                && call.request.messages.iter().any(|message| matches!(&message.content,
                    MessageContent::Parts(parts) if parts.iter().any(|part| matches!(part, MessagePart::Image { .. })))))));
        agent.close().await.unwrap();
    })
    .await
    .unwrap();
}
