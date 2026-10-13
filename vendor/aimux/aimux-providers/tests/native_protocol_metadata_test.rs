//! Native item identity, signed part boundaries, and replay contracts.

use std::collections::HashMap;

use aimux_core::content::ContentPart;
use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelPromptMessage;
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::result::GenerateContent;
use aimux_core::stream_part::StreamPart;
use aimux_core::types::FinishReasonUnified;
use aimux_providers::openai::responses::convert::ResponsesSystemMessageMode;
use aimux_providers::openai::responses::convert_to_responses_input;
use aimux_providers::openai::responses::responses_convert::{
    build_responses_event_stream, build_responses_generate_result,
};
use aimux_providers::xai::responses::convert::convert_to_xai_responses_input;
use aimux_providers::xai::{XAIConfig, XaiModel, XaiResponsesModel};
use futures::StreamExt;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn sse(value: Value) -> String {
    format!("data: {value}\n\n")
}

async fn xai_stream(
    path_value: &str,
    body: String,
    responses: bool,
) -> Vec<Result<StreamPart, AiMuxError>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(path_value))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .expect(1)
        .mount(&server)
        .await;
    let config = XAIConfig::new("").with_base_url(format!("{}/proxy?tenant=one", server.uri()));
    let result = if responses {
        XaiResponsesModel::new("grok-test".into(), config)
            .with_client(reqwest::Client::new())
            .do_stream(&CallOptions::new(vec![]))
            .await
    } else {
        XaiModel::new("grok-test".into(), config)
            .with_client(reqwest::Client::new())
            .do_stream(&CallOptions::new(vec![]))
            .await
    };
    match result {
        Ok(result) => result.stream.collect().await,
        Err(error) => vec![Err(error)],
    }
}

#[tokio::test]
async fn native_items_preserve_metadata_and_replay_without_duplicate_ids() {
    let mut user = vec![LanguageModelPromptMessage {
        role: Role::User,
        provider_options: None,
        content: vec![
            ContentPart::File {
                data: b"image".to_vec(),
                media_type: "image/jpeg".into(),
                filename: Some("picture.jpg".into()),
                provider_options: None,
            },
            ContentPart::FileBase64 {
                data: "aW1hZ2U=".into(),
                media_type: "image/png".into(),
                filename: None,
                provider_options: None,
            },
            ContentPart::File {
                data: b"document".to_vec(),
                media_type: "application/pdf".into(),
                filename: Some("note.pdf".into()),
                provider_options: None,
            },
            ContentPart::FileBase64 {
                data: "ZG9jdW1lbnQ=".into(),
                media_type: "application/pdf".into(),
                filename: None,
                provider_options: None,
            },
        ],
    }];
    let input = convert_to_responses_input(&user, ResponsesSystemMessageMode::System, false, false);
    assert_eq!(
        input.input,
        vec![json!({"role":"user","content":[
            {"type":"input_image","image_url":"data:image/jpeg;base64,aW1hZ2U="},
            {"type":"input_image","image_url":"data:image/png;base64,aW1hZ2U="},
            {"type":"input_file","filename":"note.pdf","file_data":"data:application/pdf;base64,ZG9jdW1lbnQ="},
            {"type":"input_file","filename":"part.pdf","file_data":"data:application/pdf;base64,ZG9jdW1lbnQ="},
        ]})]
    );

    user[0].content.extend([
        ContentPart::FileUrl {
            url: "https://example.test/picture.png?signature=opaque".into(),
            media_type: "image/png".into(),
            provider_options: None,
        },
        ContentPart::FileUrl {
            url: "gs://bucket/document.pdf".into(),
            media_type: "application/pdf".into(),
            provider_options: None,
        },
    ]);
    let google_user = aimux_providers::google::convert::convert_to_google_messages(&user);
    assert_eq!(
        google_user.contents,
        vec![json!({"role":"user","parts":[
            {"inlineData":{"mimeType":"image/jpeg","data":"aW1hZ2U="}},
            {"inlineData":{"mimeType":"image/png","data":"aW1hZ2U="}},
            {"inlineData":{"mimeType":"application/pdf","data":"ZG9jdW1lbnQ="}},
            {"inlineData":{"mimeType":"application/pdf","data":"ZG9jdW1lbnQ="}},
            {"fileData":{"mimeType":"image/png","fileUri":"https://example.test/picture.png?signature=opaque"}},
            {"fileData":{"mimeType":"application/pdf","fileUri":"gs://bucket/document.pdf"}},
        ]})]
    );

    let google_prompt = vec![LanguageModelPromptMessage {
        role: Role::Assistant,
        provider_options: None,
        content: vec![
            ContentPart::Reasoning {
                text: String::new(),
                signature: Some("thought".into()),
                provider_options: None,
            },
            ContentPart::Text {
                text: "before".into(),
                provider_options: Some(json!({"google":{"thoughtSignature":"sigA"}})),
            },
            ContentPart::Text {
                text: "after".into(),
                provider_options: Some(json!({"google":{"thoughtSignature":"sigB"}})),
            },
            ContentPart::Text {
                text: String::new(),
                provider_options: Some(json!({"google":{"thoughtSignature":"sigE"}})),
            },
            ContentPart::ToolCall {
                tool_call_id: "call_1".into(),
                tool_name: "lookup".into(),
                input: json!({}),
                provider_executed: None,
                thought_signature: Some("sigF".into()),
                provider_options: None,
            },
        ],
    }];
    let mut google_options = CallOptions::new(google_prompt);
    let thinking = json!({"thinkingBudget":128,"includeThoughts":true,"thinkingLevel":"low"});
    google_options.provider_options = Some(HashMap::from([(
        "google".into(),
        json!({"thinkingConfig":thinking}),
    )]));
    let google =
        aimux_providers::google::convert::build_request_body("gemini-test", &google_options);
    assert_eq!(google["generationConfig"]["thinkingConfig"], thinking);
    assert_eq!(
        google["contents"][0]["parts"],
        json!([
            {"text":"","thought":true,"thoughtSignature":"thought"},
            {"text":"before","thoughtSignature":"sigA"},
            {"text":"after","thoughtSignature":"sigB"},
            {"text":"","thoughtSignature":"sigE"},
            {"functionCall":{"id":"call_1","name":"lookup","args":{}},"thoughtSignature":"sigF"},
        ])
    );
    assert_eq!(
        aimux_providers::anthropic::AnthropicConfig::new("test")
            .with_base_url("https://example.test/proxy/v1/?tenant=/v1/")
            .base_url,
        "https://example.test/proxy?tenant=/v1/"
    );

    let output = json!([
        {"type":"reasoning","id":"rs_1","encrypted_content":"final-encrypted", "summary":[
            {"type":"summary_text","text":"first"}, {"type":"summary_text","text":""}
        ]},
        {"type":"message","id":"msg_1","phase":"final_answer","content":[
            {"type":"output_text","text":"one"}, {"type":"output_text","text":"two"}
        ]},
        {"type":"function_call","id":"fc_1","call_id":"call_1","name":"lookup","arguments":"{}"}
    ]);
    let generated = build_responses_generate_result(
        &json!({"id":"resp_1","output":output}),
        "",
        vec![],
        "openai".into(),
        "https://example.test/responses".into(),
        json!({}),
        HashMap::new(),
    )
    .unwrap();
    let mut content: Vec<ContentPart> = generated
        .content
        .into_iter()
        .map(|part| match part {
            GenerateContent::Text {
                text,
                provider_metadata,
            } => ContentPart::Text {
                text,
                provider_options: provider_metadata,
            },
            GenerateContent::Reasoning {
                text,
                provider_metadata,
            } => ContentPart::Reasoning {
                text,
                signature: None,
                provider_options: provider_metadata,
            },
            GenerateContent::ToolCall {
                tool_call_id,
                tool_name,
                input,
                provider_metadata,
                ..
            } => ContentPart::ToolCall {
                tool_call_id,
                tool_name,
                input: serde_json::from_str(&input).unwrap(),
                provider_executed: None,
                thought_signature: None,
                provider_options: provider_metadata,
            },
            part => panic!("unexpected content: {part:?}"),
        })
        .collect();
    // The explicit summary index, rather than arrival order, defines replay.
    content.swap(0, 1);
    let prompt = vec![LanguageModelPromptMessage {
        role: Role::Assistant,
        content,
        provider_options: None,
    }];
    let replay =
        convert_to_responses_input(&prompt, ResponsesSystemMessageMode::System, false, false);
    assert!(replay.warnings.is_empty());
    assert_eq!(
        replay.input,
        vec![
            json!({"type":"reasoning","id":"rs_1","encrypted_content":"final-encrypted", "summary":[
                {"type":"summary_text","text":"first"}, {"type":"summary_text","text":""}
            ]}),
            json!({"role":"assistant","id":"msg_1","phase":"final_answer","content":[
                {"type":"output_text","text":"one"}, {"type":"output_text","text":"two"}
            ]}),
            json!({"type":"function_call","id":"fc_1","call_id":"call_1","name":"lookup","arguments":"{}"}),
        ]
    );
    let stored =
        convert_to_responses_input(&prompt, ResponsesSystemMessageMode::System, true, false);
    assert_eq!(stored.input.len(), 3);
    assert_eq!(
        stored.input[0],
        json!({"type":"item_reference","id":"rs_1"})
    );
    assert_eq!(
        stored.input[1],
        json!({"type":"item_reference","id":"msg_1"})
    );
    assert!(
        convert_to_responses_input(&prompt, ResponsesSystemMessageMode::System, false, true)
            .input
            .is_empty()
    );

    let events: Vec<Value> = vec![
        json!({"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"rs_1"}}),
        json!({"type":"response.reasoning_summary_part.added","item_id":"rs_1","summary_index":0}),
        json!({"type":"response.reasoning_summary_text.delta","item_id":"rs_1","summary_index":0,"delta":"first"}),
        json!({"type":"response.reasoning_summary_part.done","item_id":"rs_1","summary_index":0}),
        json!({"type":"response.reasoning_summary_part.added","item_id":"rs_1","summary_index":1}),
        json!({"type":"response.reasoning_summary_part.done","item_id":"rs_1","summary_index":1}),
        json!({"type":"response.output_item.done","output_index":0,"item":output[0]}),
        json!({"type":"response.output_item.added","output_index":1,"item":{"type":"message","id":"msg_1","phase":"commentary"}}),
        json!({"type":"response.content_part.added","item_id":"msg_1","content_index":0,"part":{"type":"output_text"}}),
        json!({"type":"response.output_text.delta","item_id":"msg_1","content_index":0,"delta":"one"}),
        json!({"type":"response.content_part.added","item_id":"msg_1","content_index":1,"part":{"type":"output_text"}}),
        json!({"type":"response.output_text.delta","item_id":"msg_1","content_index":1,"delta":"two"}),
        json!({"type":"response.output_item.done","output_index":1,"item":output[1]}),
        json!({"type":"response.output_item.added","output_index":2,"item":output[2]}),
        json!({"type":"response.output_item.done","output_index":2,"item":output[2]}),
        json!({"type":"response.completed","response":{"id":"resp_1","usage":{"input_tokens":1,"output_tokens":1}}}),
    ];
    let parts = build_responses_event_stream(
        None,
        futures::stream::iter(events.into_iter().map(Ok)),
        "openai".into(),
        vec![],
        true,
        "https://example.test/responses".into(),
        json!({}),
        HashMap::new(),
    )
    .unwrap()
    .collect::<Vec<_>>()
    .await
    .into_iter()
    .collect::<Result<Vec<_>, _>>()
    .unwrap();
    let reasoning: Vec<_> = parts
        .iter()
        .filter_map(|part| match part {
            StreamPart::ReasoningEnd {
                id,
                provider_metadata,
            } => Some((id, provider_metadata.as_ref().unwrap())),
            _ => None,
        })
        .collect();
    assert_eq!(reasoning.len(), 2);
    for (index, (id, metadata)) in reasoning.into_iter().enumerate() {
        assert_eq!(id, &format!("rs_1:{index}"));
        assert_eq!(
            metadata["openai"],
            json!({"itemId":"rs_1","reasoningEncryptedContent":"final-encrypted","summaryIndex":index})
        );
    }
    let text: Vec<_> = parts
        .iter()
        .filter_map(|part| match part {
            StreamPart::TextEnd {
                id,
                provider_metadata,
            } => Some((id, provider_metadata.as_ref().unwrap())),
            _ => None,
        })
        .collect();
    assert_eq!(text.len(), 2);
    for (index, (id, metadata)) in text.into_iter().enumerate() {
        assert_eq!(id, &format!("msg_1:{index}"));
        assert_eq!(
            metadata["openai"],
            json!({"itemId":"msg_1","phase":"final_answer"})
        );
    }
    assert!(parts.iter().any(|part| matches!(part, StreamPart::ToolCall { provider_metadata: Some(metadata), .. } if metadata["openai"]["itemId"] == "fc_1")));

    for provider in ["openai", "azure"] {
        let refusal = build_responses_generate_result(
            &json!({"id":"resp_refusal","output":[{"type":"message","id":"msg_refusal","phase":"final_answer","content":[{"type":"refusal","refusal":"Cannot comply."}]}]}),
            "", vec![], provider.into(), "https://example.test/responses".into(), json!({}), HashMap::new(),
        ).unwrap();
        assert!(
            matches!(&refusal.content[..], [GenerateContent::Text {text, provider_metadata: Some(metadata)}] if text == "Cannot comply." && metadata[provider]["itemId"] == "msg_refusal")
        );

        for deltas in [&[][..], &[""][..], &["Cannot ", "comply."][..]] {
            let mut events = vec![
                json!({"type":"response.output_item.added","item":{"type":"message","id":"msg_refusal","phase":"final_answer"}}),
                json!({"type":"response.content_part.added","item_id":"msg_refusal","content_index":0,"part":{"type":"refusal"}}),
            ];
            for delta in deltas {
                events.push(json!({"type":"response.refusal.delta","item_id":"msg_refusal","content_index":0,"delta":delta}));
            }
            events.extend([
                json!({"type":"response.refusal.done","item_id":"msg_refusal","content_index":0,"refusal":"Cannot comply."}),
                json!({"type":"response.output_item.done","item":{"type":"message","id":"msg_refusal","phase":"final_answer"}}),
                json!({"type":"response.completed","response":{"id":"resp_refusal"}}),
            ]);
            let stream = build_responses_event_stream(
                None,
                futures::stream::iter(events.into_iter().map(Ok)).chain(futures::stream::pending()),
                provider.into(),
                vec![],
                false,
                "https://example.test/responses".into(),
                json!({}),
                HashMap::new(),
            )
            .unwrap();
            let parts = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                stream.collect::<Vec<_>>(),
            )
            .await
            .unwrap();
            let visible: String = parts
                .iter()
                .filter_map(|part| match part {
                    Ok(StreamPart::TextDelta { delta, .. }) => Some(delta.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(visible, "Cannot comply.");
            assert!(parts.iter().any(|part| matches!(part, Ok(StreamPart::TextEnd {provider_metadata: Some(metadata), ..}) if metadata[provider]["itemId"] == "msg_refusal")));
        }

        for terminal in [
            None,
            Some(json!({"type":"response.completed","response":{}})),
            Some(
                json!({"type":"response.incomplete","response":{"incomplete_details":{"reason":"content_filter"}}}),
            ),
            Some(json!({"type":"response.incomplete","response":{}})),
            Some(json!({"type":"response.failed","response":{}})),
        ] {
            let expected = match terminal.as_ref().and_then(|event| event["type"].as_str()) {
                None => None,
                Some("response.completed") => Some(FinishReasonUnified::ToolCalls),
                Some("response.incomplete")
                    if terminal.as_ref().unwrap()["response"]
                        .get("incomplete_details")
                        .is_some() =>
                {
                    Some(FinishReasonUnified::ContentFilter)
                }
                Some("response.incomplete") => Some(FinishReasonUnified::Other),
                Some("response.failed") => Some(FinishReasonUnified::Error),
                _ => unreachable!(),
            };
            let events = [
                json!({"type":"response.output_item.added","output_index":0,"item":output[2]}),
                json!({"type":"response.output_item.done","output_index":0,"item":output[2]}),
            ]
            .into_iter()
            .chain(terminal)
            .map(Ok);
            let parts = build_responses_event_stream(
                None,
                futures::stream::iter(events),
                provider.into(),
                vec![],
                false,
                "https://example.test/responses".into(),
                json!({}),
                HashMap::new(),
            )
            .unwrap()
            .collect::<Vec<_>>()
            .await;
            assert!(
                parts
                    .iter()
                    .any(|part| matches!(part, Ok(StreamPart::ToolCall { .. })))
            );
            if let Some(expected) = expected {
                assert!(
                    matches!(parts.last().unwrap(), Ok(StreamPart::Finish {finish_reason, ..}) if finish_reason.unified == expected)
                );
            } else {
                assert!(matches!(
                    parts.last().unwrap(),
                    Err(AiMuxError::InvalidResponseData(_))
                ));
                assert!(
                    !parts
                        .iter()
                        .any(|part| matches!(part, Ok(StreamPart::Finish { .. })))
                );
            }
        }
    }
}

#[tokio::test]
async fn xai_chat_preserves_delta_order_tool_identity_and_usage_tail() {
    let choice = |delta: Value, finish: Option<&str>| {
        sse(json!({"id":"chat","choices":[{"index":0,"delta":delta,"finish_reason":finish}]}))
    };
    let body = choice(json!({"reasoning_content":"same"}), None)
        + &choice(json!({"reasoning_content":"same"}), None)
        + &choice(json!({"content":"same"}), None)
        + &choice(json!({"reasoning_content":"again"}), None)
        + &choice(
            json!({"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{\"q\":"}}]}),
            None,
        )
        + &choice(
            json!({"tool_calls":[{"index":0,"type":"function","function":{"arguments":"\"x\"}"}}]}),
            Some("tool_calls"),
        )
        + &sse(
            json!({"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":4,"completion_tokens_details":{"reasoning_tokens":2}}}),
        )
        + "data: [DONE]\n\n";
    let parts = xai_stream("/proxy/chat/completions", body, false).await;
    assert!(parts.iter().all(Result::is_ok), "{parts:?}");
    let visible: Vec<_> = parts
        .iter()
        .filter_map(|part| match part {
            Ok(StreamPart::ReasoningDelta { delta, .. }) => Some(("reasoning", delta.as_str())),
            Ok(StreamPart::TextDelta { delta, .. }) => Some(("text", delta.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(
        visible,
        [
            ("reasoning", "same"),
            ("reasoning", "same"),
            ("text", "same"),
            ("reasoning", "again")
        ]
    );
    let starts: Vec<_> = parts
        .iter()
        .filter_map(|part| match part {
            Ok(StreamPart::ReasoningStart { id, .. }) => Some(("reasoning", id)),
            Ok(StreamPart::TextStart { id, .. }) => Some(("text", id)),
            _ => None,
        })
        .collect();
    assert_eq!(starts.len(), 3);
    assert_ne!(starts[0].1, starts[2].1);
    assert!(parts.iter().any(|part| matches!(part, Ok(StreamPart::ToolCall { tool_call_id, tool_name, input: Value::String(input), .. }) if tool_call_id == "call_1" && tool_name == "lookup" && input == "{\"q\":\"x\"}")));
    assert!(
        matches!(parts.last(), Some(Ok(StreamPart::Finish { usage, .. })) if usage.input_tokens.total == Some(3) && usage.output_tokens.total == Some(6) && usage.output_tokens.reasoning == Some(2))
    );
}

#[tokio::test]
async fn xai_chat_accepts_null_errors_at_any_chunk() {
    for null_chunk in 0..3 {
        let mut chunks = [
            json!({"choices":[{"index":0,"delta":{"content":"first"},"finish_reason":null}]}),
            json!({"choices":[{"index":0,"delta":{"content":" second"},"finish_reason":"stop"}]}),
            json!({"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":7}}),
        ];
        chunks[null_chunk]["error"] = Value::Null;
        let body = chunks.into_iter().map(sse).collect::<String>() + "data: [DONE]\n\n";
        let parts = xai_stream("/proxy/chat/completions", body, false).await;
        let text: String = parts
            .iter()
            .filter_map(|part| match part {
                Ok(StreamPart::TextDelta { delta, .. }) => Some(delta.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "first second", "null chunk {null_chunk}: {parts:?}");
        assert!(
            matches!(parts.last(), Some(Ok(StreamPart::Finish { finish_reason, usage, .. }))
                if finish_reason.unified == FinishReasonUnified::Stop
                    && usage.input_tokens.total == Some(3)
                    && usage.output_tokens.total == Some(7)),
            "null chunk {null_chunk}: {parts:?}"
        );
    }
}

#[tokio::test]
async fn xai_chat_rejects_truncation_malformed_identity_and_stream_errors() {
    let cases = [
        sse(
            json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{"}}]},"finish_reason":null}]}),
        ),
        sse(
            json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"","type":"function","function":{"name":"lookup","arguments":"{}"}}]},"finish_reason":"tool_calls"}]}),
        ) + "data: [DONE]\n\n",
        sse(json!({"error":{"message":"failed","type":"server_error"}})),
    ];
    for body in cases {
        let parts = xai_stream("/proxy/chat/completions", body, false).await;
        assert!(
            !parts.iter().any(|part| matches!(
                part,
                Ok(StreamPart::Finish { .. } | StreamPart::ToolCall { .. })
            )),
            "{parts:?}"
        );
        assert!(
            parts
                .iter()
                .any(|part| matches!(part, Err(_) | Ok(StreamPart::Error { .. }))),
            "{parts:?}"
        );
    }

    let body =
        sse(json!({"choices":[{"index":0,"delta":{"content":"partial"},"finish_reason":null}]}))
            + &sse(json!({"error":{"message":"Account quota exhausted","code":"quota_exhausted"}}));
    let parts = xai_stream("/proxy/chat/completions", body, false).await;
    assert!(
        parts.iter().any(
            |part| matches!(part, Ok(StreamPart::TextDelta { delta, .. }) if delta == "partial")
        )
    );
    let errors: Vec<_> = parts
        .iter()
        .filter_map(|part| match part {
            Ok(StreamPart::Error { error }) | Err(error) => Some(error),
            _ => None,
        })
        .collect();
    assert_eq!(errors.len(), 1, "{parts:?}");
    assert!(matches!(errors[0], AiMuxError::ApiCall(detail)
        if detail.message == "Account quota exhausted"
            && detail.provider_code.as_deref() == Some("quota_exhausted")));
    assert!(matches!(parts.last(), Some(Ok(StreamPart::Error { .. }))));
}

#[tokio::test]
async fn xai_responses_preserves_encrypted_reasoning_and_strict_function_calls() {
    let body = [
        json!({"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"rs_1"}}),
        json!({"type":"response.reasoning_summary_part.added","item_id":"rs_1","summary_index":0}),
        json!({"type":"response.reasoning_summary_text.delta","item_id":"rs_1","summary_index":0,"delta":"same"}),
        json!({"type":"response.reasoning_summary_text.delta","item_id":"rs_1","summary_index":0,"delta":"same"}),
        json!({"type":"response.output_item.done","output_index":0,"item":{"type":"reasoning","id":"rs_1","encrypted_content":"cipher"}}),
        json!({"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"lookup"}}),
        json!({"type":"response.function_call_arguments.delta","output_index":1,"delta":"{\"q\":"}),
        json!({"type":"response.function_call_arguments.delta","output_index":1,"delta":"\"x\"}"}),
        json!({"type":"response.output_item.done","output_index":1,"item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"lookup","arguments":"{\"q\":\"x\"}"}}),
        json!({"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":5,"output_tokens":6,"output_tokens_details":{"reasoning_tokens":2},"cost_in_usd_ticks":7}}}),
    ].into_iter().map(sse).collect::<String>();
    let parts = xai_stream("/proxy/responses", body, true).await;
    assert!(parts.iter().all(Result::is_ok), "{parts:?}");
    assert!(parts.iter().any(|part| matches!(part, Ok(StreamPart::ReasoningEnd { provider_metadata: Some(meta), .. }) if meta["xai"]["itemId"] == "rs_1" && meta["xai"]["reasoningEncryptedContent"] == "cipher")));
    assert!(parts.iter().any(|part| matches!(part, Ok(StreamPart::ToolCall { tool_call_id, input: Value::String(input), provider_metadata: Some(meta), .. }) if tool_call_id == "call_1" && input == "{\"q\":\"x\"}" && meta["xai"]["itemId"] == "fc_1")));
    assert!(
        matches!(parts.last(), Some(Ok(StreamPart::Finish { usage, provider_metadata: Some(meta), .. })) if usage.input_tokens.total == Some(5) && usage.output_tokens.reasoning == Some(2) && meta["xai"]["costInUsdTicks"] == 7)
    );
    let reasoning_metadata = parts
        .iter()
        .find_map(|part| match part {
            Ok(StreamPart::ReasoningEnd {
                provider_metadata, ..
            }) => provider_metadata.clone(),
            _ => None,
        })
        .unwrap();
    let replay_prompt = vec![LanguageModelPromptMessage {
        role: Role::Assistant,
        provider_options: None,
        content: vec![ContentPart::Reasoning {
            text: "samesame".into(),
            signature: None,
            provider_options: Some(reasoning_metadata),
        }],
    }];
    let (replay, warnings) = convert_to_xai_responses_input(&replay_prompt).unwrap();
    assert!(warnings.is_empty());
    assert_eq!(
        replay,
        [
            json!({"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"samesame"}],"status":"completed","encrypted_content":"cipher"})
        ]
    );

    let malformed = [
        json!({"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"lookup"}}),
        json!({"type":"response.function_call_arguments.delta","output_index":0,"delta":"{}"}),
        json!({"type":"response.output_item.done","output_index":0,"item":{"type":"function_call","id":"fc_1","call_id":"changed","name":"lookup","arguments":"{}"}}),
        json!({"type":"response.completed","response":{"status":"completed"}}),
    ].into_iter().map(sse).collect::<String>();
    let parts = xai_stream("/proxy/responses", malformed, true).await;
    assert!(
        parts
            .iter()
            .any(|part| matches!(part, Err(AiMuxError::InvalidResponseData(_))))
    );
    assert!(!parts.iter().any(|part| matches!(
        part,
        Ok(StreamPart::ToolCall { .. } | StreamPart::Finish { .. })
    )));

    // A function call must not turn a failed terminal into a successful turn.
    let failed = [
        json!({"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"lookup"}}),
        json!({"type":"response.output_item.done","output_index":0,"item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"lookup","arguments":"{}"}}),
        json!({"type":"response.done","response":{"status":"failed"}}),
    ].into_iter().map(sse).collect::<String>();
    let parts = xai_stream("/proxy/responses", failed, true).await;
    assert!(!parts.iter().any(|part| matches!(part, Ok(StreamPart::Finish { finish_reason, .. }) if matches!(finish_reason.unified, FinishReasonUnified::Stop | FinishReasonUnified::ToolCalls))));

    for terminal in [
        json!({"type":"response.failed","response":{"status":"failed","error":{"message":"Account quota exhausted","code":"quota_exhausted"}}}),
        json!({"type":"error","error":{"message":"Account quota exhausted","code":"quota_exhausted"}}),
        json!({"type":"response.done","response":{"status":"failed","error":{"message":"Account quota exhausted","code":"quota_exhausted"}}}),
    ] {
        let body = [
            json!({"type":"response.output_text.delta","item_id":"msg_partial","content_index":0,"delta":"partial"}),
            terminal.clone(),
        ].into_iter().map(sse).collect::<String>();
        let parts = xai_stream("/proxy/responses", body, true).await;
        assert!(parts.iter().any(
            |part| matches!(part, Ok(StreamPart::TextDelta { delta, .. }) if delta == "partial")
        ));
        let errors: Vec<_> = parts
            .iter()
            .filter_map(|part| match part {
                Ok(StreamPart::Error { error }) | Err(error) => Some(error),
                _ => None,
            })
            .collect();
        assert_eq!(errors.len(), 1, "{terminal}: {parts:?}");
        assert!(
            matches!(errors[0], AiMuxError::ApiCall(detail)
            if detail.message == "Account quota exhausted"
                && detail.provider_code.as_deref() == Some("quota_exhausted")),
            "{terminal}: {parts:?}"
        );
        assert!(matches!(parts.last(), Some(Ok(StreamPart::Error { .. }))));
        assert!(
            !parts
                .iter()
                .any(|part| matches!(part, Ok(StreamPart::Finish { .. })))
        );
    }
}

#[tokio::test]
async fn xai_responses_preserves_distinct_message_content_parts() {
    let cases = [
        (
            vec![],
            vec![("text-msg_1:0", "one"), ("text-msg_1:1", "two")],
        ),
        (
            vec![
                json!({"type":"response.output_text.delta","item_id":"msg_1","content_index":0,"delta":"one"}),
                json!({"type":"response.output_text.delta","item_id":"msg_1","content_index":0,"delta":"one"}),
            ],
            vec![
                ("text-msg_1:0", "one"),
                ("text-msg_1:0", "one"),
                ("text-msg_1:1", "two"),
            ],
        ),
    ];

    for (deltas, expected) in cases {
        let mut events = vec![json!({
            "type":"response.output_item.added",
            "output_index":0,
            "item":{"type":"message","id":"msg_1"}
        })];
        events.extend(deltas);
        events.extend([
            json!({
                "type":"response.output_item.done",
                "output_index":0,
                "item":{"type":"message","id":"msg_1","content":[
                    {"type":"output_text","text":"one"},
                    {"type":"output_text","text":"two"}
                ]}
            }),
            json!({"type":"response.completed","response":{"status":"completed"}}),
        ]);
        let body = events.into_iter().map(sse).collect::<String>();
        let parts = xai_stream("/proxy/responses", body, true).await;
        assert!(parts.iter().all(Result::is_ok), "{parts:?}");
        let visible: Vec<_> = parts
            .iter()
            .filter_map(|part| match part {
                Ok(StreamPart::TextDelta { id, delta, .. }) => Some((id.as_str(), delta.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(visible, expected);
    }
}

#[tokio::test]
async fn xai_responses_terminal_finishes_while_http_body_is_open() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (release, wait) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut connection, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0_u8; 2048];
        loop {
            let read = connection.read(&mut buffer).await.unwrap();
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
        let body = sse(
            json!({"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":0,"output_tokens":0}}}),
        );
        connection.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
        connection
            .write_all(format!("{:x}\r\n{body}\r\n", body.len()).as_bytes())
            .await
            .unwrap();
        let _ = wait.await;
    });
    let model = XaiResponsesModel::new("grok-test".into(), XAIConfig::new("").with_base_url(url));
    let parts = tokio::time::timeout(std::time::Duration::from_secs(3), async {
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
    let parts = parts.expect("terminal event must finish before HTTP EOF");
    assert!(
        matches!(parts.last(), Some(Ok(StreamPart::Finish { finish_reason, .. })) if finish_reason.unified == FinishReasonUnified::Stop)
    );
}
