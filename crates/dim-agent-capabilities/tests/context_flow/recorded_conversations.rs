use super::*;

fn record(id: &str, message: Message, in_context: bool) -> RecordedMessage {
    RecordedMessage {
        id: id.into(),
        created_at: Some("2026-01-01T00:00:00Z".into()),
        message,
        in_context,
    }
}

fn conversation(messages: Vec<RecordedMessage>, context_error: Option<String>) -> InputMessageData {
    InputMessageData {
        submission_id: Some("import:stable-chunk".into()),
        blocks: vec![InputBlock::Messages {
            source: "Imported conversation from legacy Session old-session".into(),
            messages,
            context_error,
        }],
        mode: Some(InputMode::Append),
        ..InputMessageData::default()
    }
}

fn tool_group() -> Vec<RecordedMessage> {
    vec![
        record(
            "old-user",
            Message::text(Role::User, "previous request"),
            true,
        ),
        record(
            "old-assistant",
            Message {
                role: Role::Assistant,
                content: MessageContent::Parts(vec![MessagePart::ToolUse {
                    provider_metadata: None,
                    id: "old-call".into(),
                    name: "echo".into(),
                    input: serde_json::from_value(serde_json::json!({"text":"already executed"}))
                        .unwrap(),
                    start_time: None,
                }]),
            },
            true,
        ),
        record(
            "old-result",
            Message {
                role: Role::ToolResult,
                content: MessageContent::Parts(vec![MessagePart::ToolResult {
                    tool_use_id: "old-call".into(),
                    content: "already executed".into(),
                    structured_content: None,
                    is_error: None,
                }]),
            },
            true,
        ),
    ]
}

#[tokio::test]
async fn recorded_conversation_restores_native_roles_without_replaying_execution() {
    let store = Arc::new(MemoryTimelineStore::default());
    let mut build = builder(Arc::new(EchoModel));
    build.timeline_store(store.clone());
    let agent = build.build();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    let id = session.id().to_owned();
    let mut records = vec![
        record(
            "compacted-original",
            Message::text(Role::User, "excluded compressed original"),
            false,
        ),
        record(
            "reverted",
            Message::text(Role::Assistant, "excluded reverted answer"),
            false,
        ),
        record(
            "summary",
            Message::text(Role::User, "Existing legacy summary"),
            true,
        ),
    ];
    records.extend(tool_group());
    let input = conversation(records.clone(), None);
    let accepted = session.input(input.clone(), "main").await.unwrap();
    assert_eq!(session.input(input, "main").await.unwrap().id, accepted.id);
    session.idle("main").await.unwrap();
    let before = session.history("main", None).await.unwrap();
    assert!(
        !before
            .iter()
            .any(|event| matches!(event.kind, EventKind::ModelCall(_) | EventKind::ToolCall(_)))
    );
    agent.close().await.unwrap();

    let mut build = builder(Arc::new(EchoModel));
    build.timeline_store(store);
    let agent = build.build();
    let session = agent.restore_session(id).await.unwrap();
    session
        .submit("continue here".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    let events = session.history("main", None).await.unwrap();
    let requests: Vec<_> = events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::ModelCall(data) => Some(&data.request),
            _ => None,
        })
        .collect();
    assert_eq!(requests.len(), 1);
    let messages = &requests[0].messages;
    let marker = messages.iter().position(|message| matches!(&message.content, MessageContent::Text(text) if text.contains("Imported conversation"))).unwrap();
    let effective: Vec<_> = records
        .iter()
        .filter(|record| record.in_context)
        .map(|record| record.message.clone())
        .collect();
    assert_eq!(
        messages[marker + 1..marker + 1 + effective.len()]
            .iter()
            .map(|message| &**message)
            .collect::<Vec<_>>(),
        effective.iter().collect::<Vec<_>>()
    );
    let serialized = serde_json::to_string(messages).unwrap();
    assert!(!serialized.contains("excluded compressed original"));
    assert!(!serialized.contains("excluded reverted answer"));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.kind, EventKind::ToolCall(_) | EventKind::ToolStart(_)))
    );
    assert!(events.iter().any(|event| matches!(&event.kind, EventKind::InputMessage(data) if matches!(&data.blocks[..], [InputBlock::Messages { messages, .. }] if messages == &records))));
    let child = session
        .branch(
            "main".into(),
            session.sequence("main").unwrap(),
            BranchOptions {
                source: BranchSource::Host {
                    operation_id: "recorded-fork".into(),
                },
                prompt: "continue on child".into(),
            },
        )
        .await
        .unwrap();
    session.idle(&child).await.unwrap();
    let child_events = session
        .context(&child, session.sequence(&child).unwrap())
        .await
        .unwrap();
    let child_messages = project_context_history(&child_events).unwrap();
    assert_eq!(
        child_messages
            .iter()
            .filter(|entry| entry.message == effective[1])
            .count(),
        1
    );
    assert!(
        !serde_json::to_string(
            &child_messages
                .iter()
                .map(|entry| &entry.message)
                .collect::<Vec<_>>()
        )
        .unwrap()
        .contains("excluded reverted answer")
    );
    agent.close().await.unwrap();
}

#[tokio::test]
async fn recorded_context_errors_preserve_history_and_prevent_a_substitute_request() {
    let agent = builder(Arc::new(EchoModel)).build();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    let incomplete = conversation(vec![tool_group().remove(1)], None);
    assert!(session.input(incomplete, "main").await.is_err());
    let history = vec![record(
        "original",
        Message::text(Role::User, "retained original"),
        false,
    )];
    session
        .input(
            conversation(
                history.clone(),
                Some("Provider continuation material is incompatible".into()),
            ),
            "main",
        )
        .await
        .unwrap();
    session
        .submit("continue".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    let events = session.history("main", None).await.unwrap();
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.kind, EventKind::ModelCall(_)))
    );
    assert!(events.iter().any(|event| matches!(&event.kind, EventKind::TurnEnd(data) if matches!(data.outcome, TurnOutcome::Failed { .. }))));
    assert!(events.iter().any(|event| matches!(&event.kind, EventKind::InputMessage(data) if matches!(&data.blocks[..], [InputBlock::Messages { messages, .. }] if messages == &history))));
    agent.close().await.unwrap();
}
