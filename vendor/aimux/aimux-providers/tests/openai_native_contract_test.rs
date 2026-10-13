//! Native OpenAI-compatible streaming contracts over real HTTP/SSE.

use std::time::Duration;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::parse_tool_call::{RawToolCall, parse_json_input};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::FinishReasonUnified;
use aimux_providers::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIModel};
use futures::StreamExt;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

type Outcome = Result<StreamPart, AiMuxError>;

fn event(value: Value) -> String {
    format!("data: {value}\n\n")
}

fn choice(delta: Value, finish: Option<&str>) -> Value {
    json!({"choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
}

async fn stream(body: String, profile: OpenAICompatProfile) -> Vec<Outcome> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .expect(1)
        .mount(&server)
        .await;
    let model = OpenAIModel::new(
        "native-contract".into(),
        OpenAIConfig::new("test-key")
            .with_base_url(server.uri())
            .with_profile(profile),
    );
    model
        .do_stream(&CallOptions::new(vec![]))
        .await
        .unwrap()
        .stream
        .collect()
        .await
}

fn tool_start(index: usize, id: &str, name: &str, arguments: &str) -> Value {
    json!({"index": index, "id": id, "function": {"name": name, "arguments": arguments}})
}

fn assert_invalid_response(outcomes: &[Outcome]) {
    assert!(
        outcomes
            .iter()
            .any(|part| matches!(part, Err(AiMuxError::InvalidResponseData(_))))
    );
    assert!(!outcomes.iter().any(|part| matches!(
        part,
        Ok(StreamPart::Finish { .. }) | Ok(StreamPart::ToolCall { .. })
    )));
}

#[tokio::test]
async fn native_chat_distinguishes_null_errors_from_stream_failures() {
    for null_chunk in 0..3 {
        let mut chunks = [
            choice(json!({"content":"first"}), None),
            choice(json!({"content":" second"}), Some("stop")),
            json!({"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":7}}),
        ];
        chunks[null_chunk]["error"] = Value::Null;
        let body = chunks.into_iter().map(event).collect::<String>() + "data: [DONE]\n\n";
        let outcomes = stream(body, OpenAICompatProfile::full()).await;
        let text: String = outcomes
            .iter()
            .filter_map(|part| match part {
                Ok(StreamPart::TextDelta { delta, .. }) => Some(delta.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            text, "first second",
            "null chunk {null_chunk}: {outcomes:?}"
        );
        assert!(
            matches!(outcomes.last(), Some(Ok(StreamPart::Finish { finish_reason, usage, .. }))
                if finish_reason.unified == FinishReasonUnified::Stop
                    && usage.input_tokens.total == Some(3)
                    && usage.output_tokens.total == Some(7)),
            "null chunk {null_chunk}: {outcomes:?}"
        );
    }
    let outcomes = stream(
        event(choice(json!({"content":"partial"}), None))
            + &event(json!({"error":{"message":"quota exhausted","code":"quota_exhausted"}})),
        OpenAICompatProfile::full(),
    )
    .await;
    assert!(
        outcomes.iter().any(|part| matches!(part, Ok(StreamPart::Error { error: AiMuxError::ApiCall(detail) })
            if detail.message == "quota exhausted" && detail.provider_code.as_deref() == Some("quota_exhausted"))),
        "{outcomes:?}"
    );
    assert!(
        matches!(outcomes.last(), Some(Ok(StreamPart::Finish { finish_reason, .. }))
            if finish_reason.unified == FinishReasonUnified::Error),
        "{outcomes:?}"
    );
}

#[tokio::test]
async fn done_finishes_without_waiting_for_http_body_to_close() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (release, wait) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let (mut connection, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 2048];
        loop {
            let read = connection.read(&mut buffer).await.unwrap();
            assert!(read > 0);
            request.extend_from_slice(&buffer[..read]);
            if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if request.len() >= end + 4 + length {
                    break;
                }
            }
        }
        let body = event(choice(json!({"content": "complete"}), Some("stop")))
            + &event(json!({"choices": [], "usage": {"prompt_tokens": 0, "completion_tokens": 0}}))
            + "data: [DONE]\n\n";
        connection.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
        connection
            .write_all(format!("{:x}\r\n{body}\r\n", body.len()).as_bytes())
            .await
            .unwrap();
        // Keep the HTTP body open until the client has observed its terminal result.
        let _ = wait.await;
    });
    let model = OpenAIModel::new(
        "native-contract".into(),
        OpenAIConfig::new("test-key").with_base_url(url),
    );
    let outcomes = tokio::time::timeout(Duration::from_secs(3), async {
        model
            .do_stream(&CallOptions::new(vec![]))
            .await
            .unwrap()
            .stream
            .collect::<Vec<_>>()
            .await
    })
    .await;
    let _ = release.send(());
    server.await.unwrap();
    let outcomes = outcomes.expect("[DONE] must finish before HTTP EOF");
    assert!(
        matches!(outcomes.last(), Some(Ok(StreamPart::Finish { finish_reason, usage, .. }))
        if finish_reason.unified == FinishReasonUnified::Stop
        && usage.input_tokens.total == Some(0)
        && usage.output_tokens.total == Some(0)
        && usage.input_tokens.cache_read.is_none())
    );
}

#[tokio::test]
async fn truncated_native_stream_never_synthesizes_stop_or_tool_call() {
    for delta in [
        json!({"content": "partial"}),
        json!({"tool_calls": [tool_start(0, "call-0", "lookup", "{}") ]}),
    ] {
        for tail in ["", "data: [DONE]\n\n"] {
            let outcomes = stream(
                event(choice(delta.clone(), None)) + tail,
                OpenAICompatProfile::full(),
            )
            .await;
            assert_invalid_response(&outcomes);
        }
    }
}

#[tokio::test]
async fn one_native_candidate_does_not_mix_text_tools_or_finish_reasons() {
    let body = event(json!({"choices": [
        {"index": 1, "delta": {"content": "wrong", "tool_calls": [tool_start(0, "other", "lookup", "{}") ]}, "finish_reason": "tool_calls"},
        {"index": 0, "delta": {"content": "selected"}, "finish_reason": null}
    ]})) + &event(
        json!({"choices": [{"index": 1, "delta": {"content": "also wrong"}, "finish_reason": "length"}]}),
    ) + &event(choice(json!({}), Some("stop")))
        + "data: [DONE]\n\n";
    let outcomes = stream(body, OpenAICompatProfile::full()).await;
    let text: String = outcomes
        .iter()
        .filter_map(|part| match part {
            Ok(StreamPart::TextDelta { delta, .. }) => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "selected");
    assert!(
        !outcomes
            .iter()
            .any(|part| matches!(part, Err(_) | Ok(StreamPart::ToolInputStart { .. })))
    );
    assert!(
        matches!(outcomes.last(), Some(Ok(StreamPart::Finish { finish_reason, .. })) if finish_reason.unified == FinishReasonUnified::Stop)
    );
}

#[tokio::test]
async fn native_usage_preserves_absent_and_zero_counts_in_compatible_profiles() {
    for (usage, input, output) in [
        (json!({}), None, None),
        (json!({"prompt_tokens": 2}), Some(2), None),
        (json!({"completion_tokens": 0}), None, Some(0)),
    ] {
        for profile in [OpenAICompatProfile::full(), OpenAICompatProfile::groq()] {
            let usage_chunk = if profile.stream_usage_key.is_some() {
                json!({"choices": [], "x_groq": {"usage": usage}})
            } else {
                json!({"choices": [], "usage": usage})
            };
            let outcomes = stream(
                event(choice(json!({}), Some("stop"))) + &event(usage_chunk) + "data: [DONE]\n\n",
                profile,
            )
            .await;
            let Some(Ok(StreamPart::Finish { usage, .. })) = outcomes.last() else {
                panic!("missing finish: {outcomes:?}");
            };
            assert_eq!(usage.input_tokens.total, input);
            assert_eq!(usage.output_tokens.total, output);
            assert_eq!(usage.input_tokens.cache_read, None);
            assert_eq!(usage.output_tokens.reasoning, None);
        }
    }
}

#[tokio::test]
async fn native_tool_arguments_use_shared_aimux_parser_without_schema_or_repair() {
    let arguments = [r#"{"city":"東京"}"#, r#""a string""#, "{broken", ""];
    let calls: Vec<_> = arguments
        .iter()
        .enumerate()
        .map(|(i, input)| tool_start(i, &format!("call-{i}"), "lookup", input))
        .collect();
    let outcomes = stream(
        event(choice(json!({"tool_calls": calls}), Some("tool_calls"))) + "data: [DONE]\n\n",
        OpenAICompatProfile::full(),
    )
    .await;
    let parsed: Vec<_> = outcomes
        .into_iter()
        .filter_map(|part| {
            let Ok(StreamPart::ToolCall {
                tool_call_id,
                tool_name,
                input,
                provider_executed,
                dynamic,
                thought_signature,
                provider_metadata,
                ..
            }) = part
            else {
                return None;
            };
            let Value::String(input) = input else {
                panic!("native arguments must retain the raw contract");
            };
            Some(parse_json_input(&RawToolCall {
                tool_call_id,
                tool_name,
                input,
                provider_executed,
                dynamic,
                thought_signature,
                provider_metadata,
            }))
        })
        .collect();
    assert_eq!(parsed.len(), 4);
    assert_eq!(parsed[0].as_ref().unwrap(), &json!({"city": "東京"}));
    assert_eq!(parsed[1].as_ref().unwrap(), &json!("a string"));
    assert!(
        matches!(&parsed[2], Err(AiMuxError::InvalidToolInput { tool_name, .. }) if tool_name == "lookup")
    );
    assert_eq!(parsed[3].as_ref().unwrap(), &json!({}));
}

#[tokio::test]
async fn malformed_tool_identities_do_not_become_native_tool_calls() {
    let first = tool_start(0, "call-0", "lookup", "{}");
    for calls in [
        vec![tool_start(0, "", "lookup", "{}")],
        vec![tool_start(0, "call-0", "", "{}")],
        vec![first.clone(), tool_start(1, "call-0", "lookup", "{}")],
        vec![first.clone(), tool_start(0, "changed", "lookup", "")],
        vec![first.clone(), tool_start(0, "call-0", "renamed", "")],
    ] {
        let outcomes = stream(
            event(choice(json!({"tool_calls": calls}), Some("tool_calls"))) + "data: [DONE]\n\n",
            OpenAICompatProfile::full(),
        )
        .await;
        assert_invalid_response(&outcomes);
    }
}

#[tokio::test]
async fn interrupted_tool_generation_does_not_expose_executable_calls() {
    for (raw, reason) in [
        ("length", FinishReasonUnified::Length),
        ("content_filter", FinishReasonUnified::ContentFilter),
    ] {
        let outcomes = stream(
            event(choice(
                json!({"tool_calls": [tool_start(0, "call-0", "lookup", "{partial")]}),
                Some(raw),
            )) + "data: [DONE]\n\n",
            OpenAICompatProfile::full(),
        )
        .await;
        assert!(
            !outcomes
                .iter()
                .any(|part| matches!(part, Err(_) | Ok(StreamPart::ToolCall { .. })))
        );
        assert!(
            matches!(outcomes.last(), Some(Ok(StreamPart::Finish { finish_reason, .. })) if finish_reason.unified == reason)
        );
    }
}

#[tokio::test]
async fn native_finish_reason_must_agree_with_tool_output() {
    for (delta, finish) in [
        (json!({}), "tool_calls"),
        (
            json!({"tool_calls": [tool_start(0, "call-0", "lookup", "{}")]}),
            "stop",
        ),
    ] {
        let outcomes = stream(
            event(choice(delta, Some(finish))) + "data: [DONE]\n\n",
            OpenAICompatProfile::full(),
        )
        .await;
        assert_invalid_response(&outcomes);
    }
}

#[tokio::test]
async fn failed_native_finish_cannot_be_reclassified_as_tool_calls() {
    for reason in ["length", "content_filter"] {
        let outcomes = stream(
            event(choice(
                json!({"tool_calls": [tool_start(0, "call-0", "echo", r#"{"text":"must not execute"}"#)]}),
                Some(reason),
            )) + &event(choice(json!({}), Some("tool_calls")))
                + "data: [DONE]\n\n",
            OpenAICompatProfile::full(),
        ).await;
        assert_invalid_response(&outcomes);
    }
}

#[tokio::test]
async fn native_stop_rejects_later_content_and_tool_deltas() {
    for delta in [
        json!({"content": "late text"}),
        json!({"refusal": "late refusal"}),
        json!({"reasoning_content": "late reasoning"}),
        json!({"tool_calls": [tool_start(0, "call-0", "echo", "{}")] }),
    ] {
        let outcomes = stream(
            event(choice(json!({"content": "finished"}), Some("stop")))
                + &event(choice(delta, None))
                + "data: [DONE]\n\n",
            OpenAICompatProfile::full(),
        )
        .await;
        assert_invalid_response(&outcomes);
        let text: Vec<_> = outcomes
            .iter()
            .filter_map(|part| match part {
                Ok(StreamPart::TextDelta { delta, .. }) => Some(delta.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, ["finished"]);
        assert!(!outcomes.iter().any(|part| matches!(
            part,
            Ok(StreamPart::ReasoningDelta { .. } | StreamPart::ToolInputStart { .. })
        )));
    }
}

#[tokio::test]
async fn native_finish_accepts_usage_tail_without_reopening_content() {
    for tail_choices in [
        json!([]),
        json!([{"index": 0, "delta": {"content": ""}, "finish_reason": "stop"}]),
    ] {
        let outcomes = stream(
            event(choice(json!({"content": "finished"}), Some("stop")))
                + &event(json!({"choices": tail_choices, "usage": {"prompt_tokens": 3, "completion_tokens": 4}}))
                + "data: [DONE]\n\n",
            OpenAICompatProfile::full(),
        ).await;
        assert!(outcomes.iter().all(Result::is_ok));
        assert_eq!(
            outcomes
                .iter()
                .filter(|part| matches!(part, Ok(StreamPart::TextStart { .. })))
                .count(),
            1
        );
        assert!(
            matches!(outcomes.last(), Some(Ok(StreamPart::Finish { finish_reason, usage, .. }))
            if finish_reason.unified == FinishReasonUnified::Stop
            && usage.input_tokens.total == Some(3)
            && usage.output_tokens.total == Some(4))
        );
    }
}

#[tokio::test]
async fn native_refusal_remains_visible_text() {
    let refusal = "I cannot help with that request.";
    let outcomes = stream(
        event(choice(json!({"refusal": refusal}), Some("stop"))) + "data: [DONE]\n\n",
        OpenAICompatProfile::full(),
    )
    .await;
    assert!(outcomes.iter().all(Result::is_ok));
    let text: Vec<_> = outcomes
        .iter()
        .filter_map(|part| match part {
            Ok(StreamPart::TextDelta { delta, .. }) => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, [refusal]);
    assert!(
        matches!(outcomes.last(), Some(Ok(StreamPart::Finish { finish_reason, .. }))
        if finish_reason.unified == FinishReasonUnified::Stop)
    );
}
