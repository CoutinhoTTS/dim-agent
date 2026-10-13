use std::{
    collections::VecDeque,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};

use aimux_core::types::{FinishReason, TokenUsage};
use futures_util::Stream;
use serde_json::json;

use super::*;

struct NativeStream {
    parts: VecDeque<Result<StreamPart, AiMuxError>>,
    dropped: Arc<AtomicBool>,
}

impl Stream for NativeStream {
    type Item = Result<StreamPart, AiMuxError>;

    fn poll_next(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.parts.pop_front() {
            Some(part) => Poll::Ready(Some(part)),
            // Keep the transport open so completion must release it itself.
            None => Poll::Pending,
        }
    }
}

impl Drop for NativeStream {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

fn fixture(
    parts: Vec<Result<StreamPart, AiMuxError>>,
) -> (ModelStream, CancellationToken, AbortSignal, Arc<AtomicBool>) {
    let dropped = Arc::new(AtomicBool::new(false));
    let result = StreamResult {
        stream: Box::pin(NativeStream {
            parts: parts.into(),
            dropped: dropped.clone(),
        }),
        request_body: Some(json!({ "messages": "private request text" })),
        response_headers: None,
    };
    let cancel = CancellationToken::new();
    let abort = AbortSignal::new();
    let stream = convert(
        result,
        ToolNames::default(),
        ModelProtocol::OpenaiChat,
        cancel.clone(),
        abort.clone(),
        CancellationToken::new(),
        tokio_util::task::TaskTracker::new().token(),
    );
    (stream, cancel, abort, dropped)
}

fn start() -> StreamPart {
    StreamPart::StreamStart { warnings: vec![] }
}

fn finish(reason: FinishReasonUnified, usage: NativeUsage) -> StreamPart {
    StreamPart::Finish {
        finish_reason: FinishReason {
            unified: reason,
            raw: None,
        },
        usage,
        provider_metadata: None,
    }
}

fn tool(input: &str) -> StreamPart {
    StreamPart::ToolCall {
        tool_call_id: "call-1".into(),
        tool_name: "lookup".into(),
        input: Value::String(input.into()),
        provider_executed: None,
        dynamic: None,
        thought_signature: None,
        invalid: None,
        error: None,
        provider_metadata: None,
    }
}

#[tokio::test]
async fn native_content_and_tools_convert_in_order_to_agent_chunks() {
    let (mut stream, _, _, dropped) = fixture(vec![
        Ok(StreamPart::StreamStart {
            warnings: vec![Warning::Other {
                message: "diagnostic".into(),
            }],
        }),
        Ok(StreamPart::ResponseMetadata {
            id: Some("provider-id".into()),
            timestamp: None,
            model_id: None,
        }),
        Ok(StreamPart::ReasoningStart {
            id: "reasoning".into(),
            provider_metadata: None,
        }),
        Ok(StreamPart::ReasoningDelta {
            id: "reasoning".into(),
            delta: "thinking".into(),
            provider_metadata: Some(json!({"new_key":true})),
        }),
        Ok(StreamPart::ReasoningEnd {
            id: "reasoning".into(),
            provider_metadata: None,
        }),
        Ok(StreamPart::TextStart {
            id: "text".into(),
            provider_metadata: None,
        }),
        Ok(StreamPart::TextDelta {
            id: "text".into(),
            delta: String::new(),
            provider_metadata: None,
        }),
        Ok(StreamPart::TextDelta {
            id: "text".into(),
            delta: "answer".into(),
            provider_metadata: None,
        }),
        Ok(StreamPart::TextEnd {
            id: "text".into(),
            provider_metadata: None,
        }),
        Ok(StreamPart::ToolInputStart {
            id: "call-1".into(),
            tool_name: "lookup".into(),
            provider_executed: None,
            dynamic: Some(true),
            title: Some("display-only".into()),
            provider_metadata: None,
        }),
        Ok(StreamPart::ToolInputDelta {
            id: "call-1".into(),
            delta: "{\"query\":\"test\"}".into(),
            provider_metadata: None,
        }),
        Ok(StreamPart::ToolInputEnd {
            id: "call-1".into(),
            provider_metadata: None,
        }),
        Ok(tool("{\"query\":\"test\"}")),
        // A call delivered whole, without input streaming.
        Ok(StreamPart::ToolCall {
            tool_call_id: "call-2".into(),
            tool_name: "lookup".into(),
            input: Value::String("{}".into()),
            provider_executed: None,
            dynamic: None,
            thought_signature: None,
            invalid: None,
            error: None,
            provider_metadata: None,
        }),
        Ok(StreamPart::Raw {
            raw_value: json!({"ignored":"raw payload"}),
        }),
        Ok(finish(
            FinishReasonUnified::ToolCalls,
            NativeUsage::default(),
        )),
    ]);
    let ModelChunk::MessageStart { message_id } = stream.next().await.unwrap().unwrap() else {
        panic!("missing start")
    };
    assert_ne!(message_id, "provider-id");
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), ModelChunk::Thinking(text) if text == "thinking")
    );
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), ModelChunk::ThinkingEnd { thinking } if thinking == "thinking")
    );
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), ModelChunk::Text(text) if text == "answer")
    );
    assert!(matches!(
        stream.next().await.unwrap().unwrap(),
        ModelChunk::TextEnd
    ));
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), ModelChunk::ToolStart { id, name } if id == "call-1" && name == "lookup")
    );
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), ModelChunk::ToolArguments { id, name, delta } if id == "call-1" && name == "lookup" && delta == "{\"query\":\"test\"}")
    );
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), ModelChunk::ToolUse(call) if call.id == "call-1" && call.name == "lookup" && call.input == json!({"query":"test"}).as_object().unwrap().clone())
    );
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), ModelChunk::ToolStart { id, name } if id == "call-2" && name == "lookup")
    );
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), ModelChunk::ToolUse(call) if call.id == "call-2" && call.input.is_empty())
    );
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), ModelChunk::MessageStop { message_id: id, reason: ModelStopReason::ToolUse } if id == message_id)
    );
    assert!(dropped.load(Ordering::SeqCst));
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn finish_releases_transport_before_usage_and_survives_late_cancel() {
    let usage = NativeUsage {
        input_tokens: TokenUsage {
            total: Some(u32::MAX),
            cache_read: Some(0),
            ..Default::default()
        },
        output_tokens: TokenUsage {
            total: Some(u32::MAX),
            ..Default::default()
        },
        raw: Some(json!({"ignored":"raw usage"})),
    };
    let (mut stream, cancel, _, dropped) = fixture(vec![
        Ok(start()),
        Ok(finish(FinishReasonUnified::Stop, usage)),
    ]);
    let ModelChunk::MessageStart { message_id } = stream.next().await.unwrap().unwrap() else {
        panic!("missing start")
    };
    let ModelChunk::Usage(usage) = stream.next().await.unwrap().unwrap() else {
        panic!("missing usage")
    };
    assert!(dropped.load(Ordering::SeqCst));
    assert_eq!(usage.prompt_tokens, f64::from(u32::MAX));
    assert_eq!(usage.completion_tokens, f64::from(u32::MAX));
    assert_eq!(usage.total_tokens, 8_589_934_590.0);
    assert_eq!(usage.cache_read_tokens, Some(0.0));
    assert_eq!(usage.cache_write_tokens, None);
    assert_eq!(usage.cost, None);
    assert_eq!(usage.currency, None);
    cancel.cancel();
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), ModelChunk::MessageStop { message_id: id, reason: ModelStopReason::EndTurn } if id == message_id)
    );
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn missing_usage_is_distinct_from_zero_and_length_remains_max_tokens() {
    for total in [None, Some(0)] {
        let usage = NativeUsage {
            input_tokens: TokenUsage {
                total,
                ..Default::default()
            },
            output_tokens: TokenUsage {
                total,
                ..Default::default()
            },
            raw: None,
        };
        let (stream, _, _, _) = fixture(vec![
            Ok(start()),
            Ok(finish(FinishReasonUnified::Length, usage)),
        ]);
        let chunks = stream.collect::<Vec<_>>().await;
        let actual_usage = chunks.iter().find_map(|chunk| match chunk {
            Ok(ModelChunk::Usage(usage)) => Some(usage.total_tokens),
            _ => None,
        });
        assert_eq!(actual_usage, total.map(f64::from));
        assert!(matches!(
            chunks.last(),
            Some(Ok(ModelChunk::MessageStop {
                reason: ModelStopReason::MaxTokens,
                ..
            }))
        ));
    }
}

#[tokio::test]
async fn cancellation_wins_ready_output_and_releases_pending_transport() {
    for ready in [false, true] {
        let mut parts = vec![Ok(start())];
        if ready {
            parts.push(Ok(finish(
                FinishReasonUnified::Stop,
                NativeUsage::default(),
            )));
        }
        let (mut stream, cancel, abort, dropped) = fixture(parts);
        assert!(matches!(
            stream.next().await.unwrap().unwrap(),
            ModelChunk::MessageStart { .. }
        ));
        cancel.cancel();
        assert!(matches!(
            stream.next().await,
            Some(Err(HostError::Cancelled(_)))
        ));
        assert!(abort.is_aborted());
        assert!(dropped.load(Ordering::SeqCst));
        assert!(stream.next().await.is_none());
    }
}

#[tokio::test]
async fn errors_release_transport_and_do_not_expose_request_or_tool_input() {
    let secret = "private input text";
    let mut api_error = aimux_core::error::ApiCallError::new(
        "Rate limited",
        "https://test.invalid",
        json!({"secret":secret}),
        aimux_core::ApiCallErrorOrigin::HttpResponse,
    );
    api_error.status_code = Some(429);
    api_error.provider_code = Some("rate_limit_exceeded".into());
    api_error.response_body = Some(secret.into());
    let cases = [
        (Err(AiMuxError::ApiCall(Box::new(api_error))), "429"),
        (Ok(tool(secret)), "InvalidToolInput"),
        (Ok(tool("[1, 2]")), "JSON object"),
        (
            Ok(StreamPart::Error {
                error: AiMuxError::InvalidResponseData("Truncated model stream".into()),
            }),
            "Truncated",
        ),
        (
            Err(AiMuxError::Aborted("Upstream aborted independently".into())),
            "aborted",
        ),
    ];
    for (failure, expected) in cases {
        let (mut stream, cancel, abort, dropped) = fixture(vec![
            Ok(start()),
            failure,
            Ok(finish(FinishReasonUnified::Stop, NativeUsage::default())),
        ]);
        stream.next().await.unwrap().unwrap();
        let Some(Err(HostError::Failed {
            message, budget, ..
        })) = stream.next().await
        else {
            panic!("missing failure")
        };
        assert!(message.contains(expected), "{message}");
        assert!(!message.contains(secret));
        assert!(budget.is_none());
        assert!(dropped.load(Ordering::SeqCst));
        assert!(abort.is_aborted());
        cancel.cancel();
        assert!(stream.next().await.is_none());
    }
}

#[tokio::test]
async fn unsupported_parameters_and_results_end_the_call() {
    let mut provider_tool = tool("{}");
    if let StreamPart::ToolCall {
        provider_executed, ..
    } = &mut provider_tool
    {
        *provider_executed = Some(true);
    }
    let cases = [
        StreamPart::StreamStart {
            warnings: vec![Warning::Unsupported {
                feature: "top_k".into(),
                details: None,
            }],
        },
        provider_tool,
        StreamPart::Source {
            id: "source".into(),
            source_type: "url".into(),
            url: None,
            title: None,
            provider_metadata: None,
        },
        finish(FinishReasonUnified::ContentFilter, NativeUsage::default()),
    ];
    for part in cases {
        let (mut stream, _, _, dropped) = fixture(vec![Ok(part)]);
        assert!(matches!(
            stream.next().await,
            Some(Err(HostError::Failed { .. }))
        ));
        assert!(dropped.load(Ordering::SeqCst));
        assert!(stream.next().await.is_none());
    }
}

#[tokio::test]
async fn dropping_output_stream_releases_transport_without_another_poll() {
    let (mut stream, _, _, dropped) = fixture(vec![Ok(start())]);
    stream.next().await.unwrap().unwrap();
    assert!(!dropped.load(Ordering::SeqCst));
    drop(stream);
    assert!(dropped.load(Ordering::SeqCst));
}
