use std::{
    collections::VecDeque,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};

use aimux_core::result::Source;
use aimux_core::types::{FinishReason, InputTokenUsage, OutputTokenUsage};
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
        request: None,
        response: None,
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

/// The next chunk; a fixture stream that stalls instead of ending fails the
/// case rather than hanging the suite.
async fn next_or_stall(
    stream: &mut ModelStream,
    case: &str,
) -> Option<Result<ModelChunk, HostError>> {
    tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
        .await
        .unwrap_or_else(|_| panic!("{case}: the stream stalled instead of ending"))
}

/// Every chunk of `stream`; a fixture stream that stalls instead of ending
/// fails the case rather than hanging the suite.
async fn collect_or_stall(stream: ModelStream, case: &str) -> Vec<Result<ModelChunk, HostError>> {
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        stream.collect::<Vec<_>>(),
    )
    .await
    .unwrap_or_else(|_| panic!("{case}: the stream stalled instead of ending"))
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

fn call(id: &str, name: &str, input: &str) -> RawToolCall {
    RawToolCall {
        tool_call_id: id.into(),
        tool_name: name.into(),
        input: input.into(),
        provider_executed: None,
        dynamic: None,
        provider_metadata: None,
    }
}

fn tool(input: &str) -> StreamPart {
    StreamPart::ToolCall(call("call-1", "lookup", input))
}

#[tokio::test]
async fn native_content_and_tools_convert_in_order_to_agent_chunks() {
    let (mut stream, _, _, dropped) = fixture(vec![
        Ok(StreamPart::StreamStart {
            warnings: vec![Warning::Other {
                message: "diagnostic".into(),
            }],
        }),
        Ok(StreamPart::ResponseMetadata(
            aimux_core::types::ResponseMetadata {
                id: Some("provider-id".into()),
                timestamp: None,
                model_id: None,
            },
        )),
        Ok(StreamPart::ReasoningStart {
            id: "reasoning".into(),
            provider_metadata: None,
        }),
        Ok(StreamPart::ReasoningDelta {
            id: "reasoning".into(),
            delta: "thinking".into(),
            provider_metadata: Some(
                aimux_core::shared::provider_namespace("openai", json!({"new_key":true})).unwrap(),
            ),
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
        Ok(StreamPart::ToolCall(call("call-2", "lookup", "{}"))),
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
        input_tokens: InputTokenUsage {
            total: Some(u32::MAX),
            cache_read: Some(0),
            ..Default::default()
        },
        output_tokens: OutputTokenUsage {
            total: Some(u32::MAX),
            ..Default::default()
        },
        raw: Some(json!({"ignored":"raw usage"}).as_object().cloned().unwrap()),
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
            input_tokens: InputTokenUsage {
                total,
                ..Default::default()
            },
            output_tokens: OutputTokenUsage {
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
    )
    .with_origin(aimux_core::ApiCallErrorOrigin::HttpResponse);
    api_error.status_code = Some(429);
    api_error.provider_code = Some("rate_limit_exceeded".into());
    api_error.response_body = Some(secret.into());
    let cases = [
        (Err(AiMuxError::ApiCall(Box::new(api_error))), "429"),
        (Ok(tool(secret)), "InvalidToolInput"),
        (Ok(tool("[1, 2]")), "JSON object"),
        (
            Ok(StreamPart::Error {
                error: AiMuxError::invalid_response_data("Truncated model stream"),
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
    if let StreamPart::ToolCall(call) = &mut provider_tool {
        call.provider_executed = Some(true);
    }
    let cases = [
        StreamPart::StreamStart {
            warnings: vec![Warning::Unsupported {
                feature: "top_k".into(),
                details: None,
            }],
        },
        provider_tool,
        StreamPart::Source(Source::Url {
            id: "source".into(),
            url: "https://example.test/source".into(),
            title: None,
            provider_metadata: None,
        }),
        finish(FinishReasonUnified::ContentFilter, NativeUsage::default()),
    ];
    for part in cases {
        let case = format!("{part:?}");
        let (mut stream, _, _, dropped) = fixture(vec![Ok(part)]);
        assert!(matches!(
            next_or_stall(&mut stream, &case).await,
            Some(Err(HostError::Failed { .. }))
        ));
        assert!(dropped.load(Ordering::SeqCst));
        assert!(
            next_or_stall(&mut stream, &case).await.is_none(),
            "{case}: the stream did not end after the failure"
        );
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

#[tokio::test]
async fn malformed_tool_lifecycles_fail_and_release_transport() {
    // The lifecycles strict mode now rejects (a delta before its start, a
    // repeated start, a late end) are covered by the provider-level guard and
    // by `malformed_native_block_lifecycle_is_a_classified_failure`; what
    // stays here are the checks the adapter itself makes.
    let input_start = || StreamPart::ToolInputStart {
        id: "call-1".into(),
        tool_name: "lookup".into(),
        provider_executed: None,
        dynamic: None,
        title: None,
        provider_metadata: None,
    };
    let end = || StreamPart::ToolInputEnd {
        id: "call-1".into(),
        provider_metadata: None,
    };
    let cases = [
        (
            vec![tool("{}"), tool("{}")],
            "Duplicate or mismatched tool call",
        ),
        (
            vec![
                input_start(),
                finish(FinishReasonUnified::Stop, NativeUsage::default()),
            ],
            "incomplete blocks or tool calls",
        ),
        (
            vec![
                end(),
                finish(FinishReasonUnified::Stop, NativeUsage::default()),
            ],
            "incomplete blocks or tool calls",
        ),
    ];
    for (parts, diagnostic) in cases {
        let (stream, _, abort, dropped) = fixture(parts.into_iter().map(Ok).collect());
        let chunks = collect_or_stall(stream, &format!("the {diagnostic:?} lifecycle")).await;
        let Some(Err(failure)) = chunks.last() else {
            panic!("missing failure: {chunks:?}")
        };
        assert!(matches!(
            failure,
            HostError::Failed {
                kind: HostErrorKind::Other,
                ..
            }
        ));
        assert!(
            failure.to_string().contains("invalid_response_data"),
            "{failure}"
        );
        assert!(failure.to_string().contains(diagnostic), "{failure}");
        assert!(abort.is_aborted());
        assert!(dropped.load(Ordering::SeqCst));
    }
    // A tool call whose name disagrees with its started input is mismatched.
    let mut mismatched = call("call-1", "different", "{}");
    mismatched.tool_call_id = "call-1".into();
    let (stream, _, abort, _) = fixture(vec![
        Ok(input_start()),
        Ok(StreamPart::ToolCall(mismatched)),
    ]);
    let chunks = stream.collect::<Vec<_>>().await;
    let Some(Err(failure)) = chunks.last() else {
        panic!("missing failure: {chunks:?}")
    };
    assert!(
        failure
            .to_string()
            .contains("Duplicate or mismatched tool call"),
        "{failure}"
    );
    assert!(abort.is_aborted());
}

#[tokio::test]
async fn whole_tool_with_input_end_and_truncated_open_blocks_remain_valid() {
    let (stream, _, _, _) = fixture(vec![
        Ok(StreamPart::ToolInputEnd {
            id: "call-1".into(),
            provider_metadata: None,
        }),
        Ok(tool("{}")),
        Ok(finish(
            FinishReasonUnified::ToolCalls,
            NativeUsage::default(),
        )),
    ]);
    let chunks = stream.collect::<Vec<_>>().await;
    assert!(chunks.iter().all(Result::is_ok));
    assert!(matches!(&chunks[0], Ok(ModelChunk::ToolStart { .. })));
    assert!(matches!(&chunks[1], Ok(ModelChunk::ToolUse(_))));

    for reason in [FinishReasonUnified::Stop, FinishReasonUnified::Length] {
        let (stream, _, _, _) = fixture(vec![
            Ok(StreamPart::TextStart {
                id: "text".into(),
                provider_metadata: None,
            }),
            Ok(finish(reason, NativeUsage::default())),
        ]);
        let chunks = stream.collect::<Vec<_>>().await;
        if reason == FinishReasonUnified::Length {
            assert!(matches!(
                chunks.last(),
                Some(Ok(ModelChunk::MessageStop {
                    reason: ModelStopReason::MaxTokens,
                    ..
                }))
            ));
        } else {
            assert!(matches!(chunks.last(), Some(Err(_))));
        }
    }
}

#[tokio::test]
async fn a_text_block_can_reopen_after_its_end() {
    // Strict mode rejects an end for a block that never started, a repeated
    // start and a repeated end; what remains here is the adapter's own
    // behavior for a provider that reuses a block id.
    let text_start = || StreamPart::TextStart {
        id: "block".into(),
        provider_metadata: None,
    };
    let text_end = || StreamPart::TextEnd {
        id: "block".into(),
        provider_metadata: None,
    };
    let (stream, _, _, dropped) = fixture(vec![
        Ok(text_start()),
        Ok(StreamPart::TextDelta {
            id: "block".into(),
            delta: "first".into(),
            provider_metadata: None,
        }),
        Ok(text_end()),
        Ok(text_start()),
        Ok(StreamPart::TextDelta {
            id: "block".into(),
            delta: " second".into(),
            provider_metadata: None,
        }),
        Ok(text_end()),
        Ok(finish(FinishReasonUnified::Stop, NativeUsage::default())),
    ]);
    let chunks = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        stream.collect::<Vec<_>>(),
    )
    .await
    .unwrap();
    let chunks: Vec<_> = chunks.into_iter().collect::<Result<_, _>>().unwrap();
    let text: Vec<_> = chunks
        .iter()
        .filter_map(|chunk| match chunk {
            ModelChunk::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, ["first", " second"]);
    assert_eq!(
        chunks
            .iter()
            .filter(|chunk| matches!(chunk, ModelChunk::TextEnd))
            .count(),
        2
    );
    assert!(matches!(
        chunks.last(),
        Some(ModelChunk::MessageStop { .. })
    ));
    assert!(dropped.load(Ordering::SeqCst));
}
