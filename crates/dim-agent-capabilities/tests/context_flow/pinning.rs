//! Class 2(a) pinning: a cut inside the running turn carries that turn's
//! consumed inputs (ContextCompaction §5.9, acceptance 13), and the record's
//! `replacement_parts` locates them (ContextCompaction §4.3).
use super::*;
use dim_agent_capabilities::history::summary_message;

/// Emits one `wait` call carrying `text` when the last message asks for the
/// tool; the wait holds the turn long enough for a steer to arrive while the
/// tool runs. Echoes otherwise.
#[derive(Default)]
struct ToolWait {
    text: std::sync::Mutex<String>,
}
impl ModelDriver for ToolWait {
    fn invoke(
        &self,
        request: Arc<ModelRequest>,
        connection: ModelConnection,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ModelStream> {
        let asks = matches!(&request.messages.last().unwrap().content,
            MessageContent::Text(text) if text == "use the tool");
        Box::pin(async move {
            if !asks {
                return EchoModel.invoke(request, connection, cancel).await;
            }
            let text = self.text.lock().unwrap().clone();
            Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(ModelChunk::MessageStart {
                    message_id: "tool".into(),
                }),
                Ok(ModelChunk::ToolStart {
                    id: "wait-call".into(),
                    name: "wait".into(),
                }),
                Ok(ModelChunk::ToolUse(ToolCall {
                    id: "wait-call".into(),
                    name: "wait".into(),
                    input: serde_json::from_value(
                        serde_json::json!({ "millis": 300, "text": text }),
                    )
                    .unwrap(),
                })),
                Ok(ModelChunk::MessageStop {
                    message_id: "tool".into(),
                    reason: ModelStopReason::ToolUse,
                }),
            ])) as ModelStream)
        })
    }
}

/// Submits the input, waits for its `wait` call to start, then submits the
/// steer while the tool still runs.
async fn turn_with_steer(session: &dim_agent_runtime::Session, steer: &str) {
    session
        .submit("use the tool".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let events = session.history("main", None).await.unwrap();
            if events
                .iter()
                .any(|event| matches!(event.kind, EventKind::ToolCall(_)))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the tool call starts");
    session
        .submit(steer.into(), InputMode::Steer, "main")
        .await
        .unwrap();
}

/// A session with the `wait` tool and full access, so a turn can continue
/// after its tool result and hold while a steer arrives.
async fn tool_session(
    model: Arc<ToolWait>,
    threshold: f64,
) -> (dim_agent_runtime::Agent, dim_agent_runtime::Session) {
    let agent = builder_with_context(
        model,
        HistoryRenderer::default(),
        Context {
            threshold: Some(threshold),
            ..Context::default()
        },
    )
    .build();
    let session = agent
        .create_session(SessionConfig {
            tools: Some(vec!["wait".into()]),
            permission_policy: dim_agent_runtime::PermissionPolicy {
                preset: dim_agent_runtime::PermissionPreset::FullAccess,
                ..Default::default()
            },
            ..Default::default()
        })
        .await
        .unwrap();
    (agent, session)
}

/// The same hosts with a configured model selection, which the Loop's
/// manual step needs to build its request.
fn configured(model: Arc<dyn ModelDriver>, context: Context) -> AgentBuilder {
    let mut builder = AgentBuilder::new(AgentConfig::default());
    builder
        .process_value(Arc::new(ModelHost(model)))
        .session_value(Arc::new(ToolHost(Arc::new(MockTools))))
        .session_value(Arc::new(HistoryRenderer::default()))
        .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
        .session_value(Arc::new(ContextModelHost(Some(Arc::new(context)))))
        .session_value(Arc::new(CompactionStrategyHost(Some(Arc::new(context)))))
        .require_feature(
            Feature::new("PrepareModelHost").session("PrepareModelHost", |names| {
                Ok(Box::new(DefaultPrepareModelPlugin::new(names)?))
            }),
        );
    defaults::install_with_prompt(
        &mut builder,
        Arc::new(EmptyPromptHost),
        Some(ConfigureUpdate {
            sources: Vec::new(),
            model: Some(ModelSelection {
                provider_id: "mock".into(),
                model_id: "echo".into(),
                capability_snapshot: None,
                config: None,
                metadata: None,
            }),
        }),
    );
    builder
}

/// The one completed compaction, with its replacement, parts and start.
fn completed(
    events: &[Arc<AgentEvent>],
) -> (
    &ContextCompactionData,
    Vec<Message>,
    Option<ReplacementParts>,
) {
    let data = events
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::ContextCompaction(data)
                if matches!(data.outcome, ContextCompactionResult::Completed { .. }) =>
            {
                Some(data)
            }
            _ => None,
        })
        .expect("no completed compaction");
    let ContextCompactionResult::Completed {
        replacement,
        replacement_parts,
        ..
    } = &data.outcome
    else {
        unreachable!()
    };
    (data, replacement.clone(), *replacement_parts)
}

/// A cut that replaces the running turn's input pins it verbatim, after the
/// strategy's messages (ContextCompaction §5.9).
#[tokio::test]
async fn a_cut_inside_the_running_turn_pins_its_input_verbatim() {
    let model = Arc::new(ToolWait::default());
    *model.text.lock().unwrap() = "t".repeat(2_000);
    let (agent, session) = tool_session(model, 3_000.).await;
    session
        .submit("use the tool".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    let events = session.history("main", None).await.unwrap();
    let (_, replacement, parts) = completed(&events);
    assert_eq!(replacement.len(), 2, "{replacement:?}");
    assert_eq!(replacement[0], summary_message("short"));
    assert_eq!(
        replacement[1],
        Message::text(Role::User, "use the tool"),
        "the running turn's input is pinned verbatim"
    );
    assert_eq!(
        parts,
        Some(ReplacementParts {
            strategy: 1,
            digest: 0
        })
    );
    let at = events
        .iter()
        .position(|event| {
            matches!(&event.kind, EventKind::ContextCompaction(data)
                if matches!(data.outcome, ContextCompactionResult::Completed { .. }))
        })
        .unwrap();
    let call = events[at..]
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::ModelCall(data) => Some(&data.request),
            _ => None,
        })
        .expect("the request after the compaction");
    assert_eq!(
        call.messages[0].as_ref(),
        &summary_message("short"),
        "the pin follows the summary"
    );
    assert_eq!(
        call.messages[1].as_ref(),
        &Message::text(Role::User, "use the tool")
    );
    agent.close().await.unwrap();
}

/// A steer that joined the turn is one of its consumed inputs, pinned in
/// event order after the input it continued (ContextCompaction §5.9).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_steer_that_joined_the_turn_is_pinned_in_order() {
    let model = Arc::new(ToolWait::default());
    // Small enough that the first tool cycle's request fits the input
    // budget, so the compaction runs on the second cycle's request, when
    // both inputs are consumed (ContextCompaction §5.3, §5.9).
    *model.text.lock().unwrap() = "t".repeat(1_200);
    let (agent, session) = tool_session(model, 3_000.).await;
    turn_with_steer(&session, "use the tool").await;
    session.idle("main").await.unwrap();
    let events = session.history("main", None).await.unwrap();
    let (_, replacement, parts) = completed(&events);
    assert_eq!(replacement.len(), 3, "{replacement:?}");
    assert_eq!(replacement[0], summary_message("short"));
    assert_eq!(
        replacement[1],
        Message::text(Role::User, "use the tool"),
        "the turn's first input comes first"
    );
    assert_eq!(
        replacement[2],
        Message::text(Role::User, "use the tool"),
        "the steer that joined the turn follows it"
    );
    assert_eq!(
        parts,
        Some(ReplacementParts {
            strategy: 1,
            digest: 0
        })
    );
    agent.close().await.unwrap();
}

/// An input consumed at the cut stays in the kept tail and is not repeated
/// in the replacement (ContextCompaction §5.9).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_input_consumed_at_the_cut_is_not_duplicated() {
    let model = Arc::new(ToolWait::default());
    *model.text.lock().unwrap() = "t".repeat(2_000);
    // The first tool cycle's request already reaches the threshold; the cut
    // is the boundary that consumes the steer, which stays in the tail.
    let (agent, session) = tool_session(model, 3_000.).await;
    turn_with_steer(&session, "STEER-MARKER").await;
    session.idle("main").await.unwrap();
    let events = session.history("main", None).await.unwrap();
    let (_, replacement, parts) = completed(&events);
    assert_eq!(
        replacement,
        vec![
            summary_message("short"),
            Message::text(Role::User, "use the tool"),
        ],
        "the input before the cut is pinned; the steer at it is not"
    );
    assert_eq!(
        parts,
        Some(ReplacementParts {
            strategy: 1,
            digest: 0
        })
    );
    let at = events
        .iter()
        .position(|event| {
            matches!(&event.kind, EventKind::ContextCompaction(data)
                if matches!(data.outcome, ContextCompactionResult::Completed { .. }))
        })
        .unwrap();
    let call = events[at..]
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::ModelCall(data) => Some(&data.request),
            _ => None,
        })
        .expect("the request after the compaction");
    let occurrences = call
        .messages
        .iter()
        .filter(|message| {
            serde_json::to_string(message.as_ref())
                .unwrap()
                .contains("STEER-MARKER")
        })
        .count();
    assert_eq!(occurrences, 1, "{:?}", call.messages);
    agent.close().await.unwrap();
}

/// A manual step leaves no running turn, so it pins nothing
/// (ContextCompaction §5.9): the replacement is the strategy's messages.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_compaction_with_no_running_turn_pins_nothing() {
    let agent = configured(Arc::new(EchoModel), Context::default()).build();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    for text in ["x".repeat(1_500), "second".into()] {
        session
            .submit(text, InputMode::Queue, "main")
            .await
            .unwrap();
        session.idle("main").await.unwrap();
    }
    let command = session
        .service::<dim_agent_capabilities::agent_loop::AgentLoop>("main")
        .unwrap()
        .compact()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let events = session.history("main", None).await.unwrap();
            if events.iter().any(|event| {
                matches!(&event.kind,
                    EventKind::ContextCompaction(data) if data.command == Some(command))
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let events = session.history("main", None).await.unwrap();
    let (data, replacement, parts) = completed(&events);
    assert_eq!(data.command, Some(command));
    assert_eq!(replacement, vec![summary_message("short")]);
    assert_eq!(parts, None, "no pinned inputs, so no parts field");
    agent.close().await.unwrap();
}
