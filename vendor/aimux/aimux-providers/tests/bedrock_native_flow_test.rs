//! Converse wire, signing and continuation contracts against a local HTTP peer.
use aimux_core::{
    content::ContentPart, language_model::LanguageModel,
    language_model_message::LanguageModelPromptMessage, message::Role, options::CallOptions,
    stream_part::StreamPart,
};
use aimux_providers::bedrock::{
    AwsCredentials, BedrockAuth, BedrockConfig, BedrockModel, event_stream::encode_message,
};
use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

fn frame(name: &str, payload: Value) -> Vec<u8> {
    encode_message("event", name, &payload.to_string())
}
fn start() -> Vec<u8> {
    frame("messageStart", json!({"role":"assistant"}))
}
fn delta(index: u64, value: Value) -> Vec<u8> {
    frame(
        "contentBlockDelta",
        json!({"contentBlockIndex":index,"delta":value}),
    )
}
fn stop(index: u64) -> Vec<u8> {
    frame("contentBlockStop", json!({"contentBlockIndex":index}))
}
fn end() -> Vec<u8> {
    [
        frame("messageStop", json!({"stopReason":"end_turn"})),
        frame(
            "metadata",
            json!({"usage":{"inputTokens":17,"outputTokens":9,"totalTokens":26}}),
        ),
    ]
    .concat()
}
fn model(base: &str, auth: BedrockAuth) -> BedrockModel {
    BedrockModel::new(
        "arn:aws:bedrock:us-east-1:123456789012:inference-profile/test".into(),
        BedrockConfig {
            base_url: base.into(),
            auth,
            retry_config: Default::default(),
            api_key_source: None,
        },
    )
    .with_client(reqwest::Client::builder().no_proxy().build().unwrap())
}

#[tokio::test]
async fn converse_preserves_native_blocks_signs_and_replays() {
    for auth in [
        BedrockAuth::BearerToken("fixture-bearer".into()),
        BedrockAuth::SigV4(AwsCredentials {
            access_key_id: "fixture-access".into(),
            secret_access_key: "fixture-secret".into(),
            session_token: Some("fixture-session".into()),
            region: "us-east-1".into(),
        }),
    ] {
        let server = MockServer::start().await;
        let body = [
            start(),
            delta(0,json!({"text":"same"})), delta(0,json!({"text":"same"})), stop(0),
            delta(1,json!({"reasoningContent":{"text":"why"}})),
            delta(1,json!({"reasoningContent":{"signature":"sig"}})), stop(1),
            delta(2,json!({"reasoningContent":{"redactedContent":"YQ=="}})),
            delta(2,json!({"reasoningContent":{"redactedContent":"YmM="}})), stop(2),
            frame("contentBlockStart",json!({"contentBlockIndex":3,"start":{"toolUse":{"toolUseId":"call-1","name":"lookup"}}})),
            delta(3,json!({"toolUse":{"input":"{\"city\":\"Tokyo\"}"}})), stop(3),
            delta(4,json!({"reasoningContent":{"text":"unsigned"}})),stop(4),
            frame("messageStop",json!({"stopReason":"tool_use"})),
            frame("metadata",json!({"usage":{"inputTokens":17,"outputTokens":9,"totalTokens":26}})),
        ].concat();
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/vnd.amazon.eventstream")
                    .set_body_bytes(body),
            )
            .mount(&server)
            .await;
        let native = model(&server.uri(), auth.clone());
        let mut options = CallOptions::new(vec![]);
        options.max_output_tokens = Some(4096);
        options.headers = Some([("X-Test".into(), "kept".into())].into());
        options.provider_options = Some(
            [(
                "bedrock".into(),
                json!({"reasoningConfig":{"type":"enabled","budgetTokens":1024}}),
            )]
            .into(),
        );
        let parts: Vec<_> = native
            .do_stream(&options)
            .await
            .unwrap()
            .stream
            .collect()
            .await;
        assert!(parts.iter().all(Result::is_ok), "{parts:?}");
        let parts: Vec<_> = parts.into_iter().map(Result::unwrap).collect();
        let texts: Vec<_> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::TextDelta { delta, .. } => Some(delta.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, ["same", "same"]);
        let metadata: Vec<_> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::ReasoningEnd {
                    provider_metadata, ..
                } => provider_metadata.clone(),
                _ => None,
            })
            .collect();
        assert_eq!(metadata.len(), 3);
        assert_eq!(metadata[0]["bedrock"]["signature"], "sig");
        assert_eq!(metadata[1]["bedrock"]["redactedData"], "YWJj");
        assert_eq!(metadata[2]["bedrock"], json!({}));
        assert!(parts.iter().any(|p| matches!(p, StreamPart::ToolCall { tool_call_id, input,.. } if tool_call_id == "call-1" && input == &json!("{\"city\":\"Tokyo\"}"))));
        assert!(parts.iter().any(|p| matches!(p, StreamPart::Finish { usage,.. } if usage.input_tokens.total == Some(17) && usage.output_tokens.total == Some(9))));
        options.prompt = vec![LanguageModelPromptMessage {
            role: Role::Assistant,
            provider_options: None,
            content: vec![
                ContentPart::Reasoning {
                    text: "why".into(),
                    signature: None,
                    provider_options: Some(metadata[0].clone()),
                },
                ContentPart::Reasoning {
                    text: String::new(),
                    signature: None,
                    provider_options: Some(metadata[1].clone()),
                },
                ContentPart::reasoning("unsigned"),
            ],
        }];
        let _: Vec<_> = native
            .do_stream(&options)
            .await
            .unwrap()
            .stream
            .collect()
            .await;
        let requests = server.received_requests().await.unwrap();
        let req = &requests[1];
        assert!(
            req.url.path().contains("inference-profile%2Ftest"),
            "{}",
            req.url
        );
        assert_eq!(req.headers["x-test"], "kept");
        assert_eq!(req.headers.get_all("content-type").iter().count(), 1);
        match auth {
            BedrockAuth::BearerToken(_) => {
                assert_eq!(req.headers["authorization"], "Bearer fixture-bearer")
            }
            BedrockAuth::SigV4(_) => {
                assert!(
                    req.headers["authorization"]
                        .to_str()
                        .unwrap()
                        .contains("Credential=fixture-access/")
                );
                assert_eq!(req.headers["x-amz-security-token"], "fixture-session");
            }
        }
        let sent: Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(sent["inferenceConfig"]["maxTokens"], 4096);
        assert_eq!(
            sent["messages"][0]["content"],
            json!([
                {"reasoningContent":{"reasoningText":{"text":"why","signature":"sig"}}},
                {"reasoningContent":{"redactedContent":"YWJj"}}
            ])
        );
        options.top_k = Some(7.0);
        assert!(native.do_stream(&options).await.is_err());
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn malformed_converse_never_finishes_successfully() {
    let mut corrupt = delta(0, json!({"text":"bad"}));
    *corrupt.last_mut().unwrap() ^= 1;
    let tool_start = frame(
        "contentBlockStart",
        json!({"contentBlockIndex":0,"start":{"toolUse":{"toolUseId":"call-1","name":"lookup"}}}),
    );
    for body in [
        [start(), corrupt].concat(),
        [start(), delta(0, json!({"text":"x"})), end()].concat(),
        [start(), delta(0, json!({"text":"x"})), stop(2), end()].concat(),
        [
            start(),
            delta(0, json!({"toolUse":{"input":"{}"}})),
            stop(0),
            end(),
        ]
        .concat(),
        [start(), delta(0, json!({"unknown":"lost"})), stop(0), end()].concat(),
        [start(), stop(0), end()].concat(),
        [
            start(),
            tool_start.clone(),
            delta(0, json!({"toolUse":{"input":"{}"}})),
            stop(0),
            end(),
        ]
        .concat(),
        [
            start(),
            tool_start.clone(),
            delta(0, json!({"toolUse":{"input":"{}"}})),
            stop(0),
        ]
        .concat(),
        [
            start(),
            tool_start,
            delta(0, json!({"toolUse":{"input":"[]"}})),
            stop(0),
            end(),
        ]
        .concat(),
        [
            start(),
            encode_message(
                "exception",
                "modelStreamErrorException",
                "{\"message\":\"failed\"}",
            ),
        ]
        .concat(),
        [
            start(),
            encode_message("event", "contentBlockDelta", "not-json"),
        ]
        .concat(),
        [start(), end()[..12].to_vec()].concat(),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/vnd.amazon.eventstream")
                    .set_body_bytes(body.clone()),
            )
            .mount(&server)
            .await;
        let result = model(&server.uri(), BedrockAuth::BearerToken("test".into()))
            .do_stream(&CallOptions::new(vec![]))
            .await
            .unwrap();
        let parts: Vec<_> = result.stream.collect().await;
        assert!(
            parts.iter().any(Result::is_err),
            "accepted malformed stream: {parts:?}"
        );
        assert!(
            !parts
                .iter()
                .any(|p| matches!(p, Ok(StreamPart::Finish { .. }))),
            "{parts:?}"
        );
        assert!(
            !parts
                .iter()
                .any(|p| matches!(p, Ok(StreamPart::ToolCall { .. }))),
            "{parts:?}"
        );
    }
}

#[tokio::test]
async fn converse_streams_before_eof_finishes_at_metadata_and_cancels() {
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    for cancel in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (proceed, ready) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let count = socket.read(&mut buffer).await.unwrap();
                assert_ne!(count, 0);
                request.extend_from_slice(&buffer[..count]);
                if let Some(header_end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let length = String::from_utf8_lossy(&request[..header_end])
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|s| s.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    if request.len() >= header_end + 4 + length {
                        break;
                    }
                }
            }
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/vnd.amazon.eventstream\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
            let first = [
                start(),
                frame(
                    "contentBlockStart",
                    json!({"contentBlockIndex":0,"start":{}}),
                ),
                delta(0, json!({"text":"early"})),
            ]
            .concat();
            // A chunk ending exactly at a frame prelude exercises internal decoder state.
            for bytes in [&first[..12], &first[12..]] {
                socket
                    .write_all(format!("{:x}\r\n", bytes.len()).as_bytes())
                    .await
                    .unwrap();
                socket.write_all(bytes).await.unwrap();
                socket.write_all(b"\r\n").await.unwrap();
            }
            ready.await.unwrap();
            if !cancel {
                let final_bytes = [stop(0), end()].concat();
                socket
                    .write_all(format!("{:x}\r\n", final_bytes.len()).as_bytes())
                    .await
                    .unwrap();
                socket.write_all(&final_bytes).await.unwrap();
                socket.write_all(b"\r\n").await.unwrap();
            }
            std::future::pending::<()>().await;
        });
        let abort = aimux_core::AbortSignal::new();
        let mut options = CallOptions::new(vec![]);
        options.abort_signal = Some(abort.clone());
        let native = model(&base, BedrockAuth::BearerToken("test".into()));
        let mut stream = tokio::time::timeout(Duration::from_secs(2), native.do_stream(&options))
            .await
            .unwrap()
            .unwrap()
            .stream;
        tokio::time::timeout(Duration::from_secs(2),async {
            loop {
                if matches!(stream.next().await.unwrap().unwrap(),StreamPart::TextDelta{delta,..} if delta=="early") { break; }
            }
        }).await.unwrap();
        if cancel {
            abort.abort();
        }
        proceed.send(()).unwrap();
        let rest: Vec<_> = tokio::time::timeout(Duration::from_secs(2), stream.collect())
            .await
            .unwrap();
        if cancel {
            assert!(rest.iter().any(Result::is_err), "{rest:?}");
            assert!(
                !rest
                    .iter()
                    .any(|p| matches!(p, Ok(StreamPart::Finish { .. })))
            );
        } else {
            assert!(rest.iter().all(Result::is_ok), "{rest:?}");
            assert!(
                rest.iter()
                    .any(|p| matches!(p, Ok(StreamPart::Finish { .. })))
            );
        }
        server.abort();
        let _ = server.await;
    }
}
