use super::*;
use aimux_core::{LanguageModel, stream_part::StreamPart};
use aimux_providers::openai::{OpenAIConfig, OpenAIModel};
use futures_util::StreamExt;
use serde_json::json;
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

async fn send(options: CallOptions, provider: &str) -> Value {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        let header_end = loop {
            let mut chunk = [0; 4096];
            let count = socket.read(&mut chunk).await.unwrap();
            assert_ne!(count, 0);
            bytes.extend_from_slice(&chunk[..count]);
            if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
        assert!(headers.starts_with("POST /v1/chat/completions HTTP/1.1\r\n"));
        let length: usize = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().unwrap())
            })
            .unwrap();
        while bytes.len() < header_end + length {
            let mut chunk = [0; 4096];
            let count = socket.read(&mut chunk).await.unwrap();
            assert_ne!(count, 0);
            bytes.extend_from_slice(&chunk[..count]);
        }
        let request: Value =
            serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
        let response = concat!(
            "data: {\"id\":\"completion\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"ok\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"completion\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).await.unwrap();
        request
    });
    let model = OpenAIModel::new(
        "gpt-4o".into(),
        OpenAIConfig::new("test-key")
            .with_base_url(format!("http://{address}/v1"))
            .with_provider(provider),
    );
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let mut stream = model.do_stream(&options).await.unwrap().stream;
        let mut finished = false;
        while let Some(part) = stream.next().await {
            match part.unwrap() {
                StreamPart::Finish { .. } => finished = true,
                StreamPart::Error { error } => panic!("unexpected provider error: {error}"),
                _ => {}
            }
        }
        assert!(finished);
        server.await.unwrap()
    })
    .await
    .unwrap()
}

fn request(config: Value) -> ModelRequest {
    let mut request = ModelRequest::new(
        "application-alias",
        "gpt-4o",
        vec![dim_agent_types::Message::text(Role::User, "hello")],
    );
    if !config.is_null() {
        request.config = Some(serde_json::from_value(config).unwrap());
    }
    request
}

fn tool(name: &str) -> dim_agent_types::ToolDefinition {
    serde_json::from_value(json!({
        "name": name, "description": "", "source": "dynamic",
        "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"], "additionalProperties": false}
    })).unwrap()
}

#[tokio::test]
async fn final_request_replays_multimodal_tool_history_and_only_model_readable_results() {
    let name = "plugin:notion/notion__notion-list-recent-pages";
    let request: ModelRequest = serde_json::from_value(json!({
        "providerId": "my-openai", "modelId": "gpt-4o",
        "system": [{"type": "text", "text": ""}, {"type": "text", "text": "system\n"}],
        "messages": [
            {"role": "user", "content": [
                {"type": "text", "text": " hello\n"},
                {"type": "image", "source": {"type": "base64", "mediaType": "image/png", "data": "aGVsbG8="}},
                {"type": "image", "source": {"type": "url", "mediaType": "image/jpeg", "url": "https://example.test/image.jpg"}}
            ]},
            {"role": "assistant", "content": [
                {"type": "thinking", "thinking": "plan", "startTime": "2026-09-21T00:00:00.000Z"},
                {"type": "text", "text": "searching"},
                {"type": "tool_use", "id": "call-1", "name": name, "input": {"query": "recent"}}
            ]},
            {"role": "tool_result", "content": [{"type": "tool_result", "tool_use_id": "call-1", "content": [{"type": "text", "text": "part 1\n"}, {"type": "text", "text": "part 2"}], "is_error": false, "structuredContent": {"private": "hidden"}}]},
            {"role": "user", "content": "continue"}
        ],
        "tools": [tool(name)],
        "config": {"maxTokens": 4096, "temperature": 0, "topP": 0, "topK": 1, "stopSequences": []},
        "metadata": {"sessionId": "session-local", "runId": "run-local", "requestId": "request-local"}
    })).unwrap();
    let prepared = convert(&request, &ProviderKind::OpenaiChat, None).unwrap();
    assert_eq!(
        prepared
            .tool_names
            .original("dim_tool_plugin_notio_ion_notion-list-recent-pages_27e3aae889c3"),
        name
    );
    assert_eq!(
        prepared.options.prompt[4].content[0],
        ContentPart::ToolResult {
            tool_call_id: "call-1".into(),
            result: json!("part 1\npart 2"),
            tool_name: Some("notion-list-recent-pages_27e3aae889c3".into()),
            is_error: Some(false),
            preliminary: None,
            dynamic: None,
            provider_options: None,
        }
    );
    let body = send(prepared.options, "openai").await;
    assert_eq!(
        body["messages"],
        json!([
            {"role": "system", "content": ""}, {"role": "system", "content": "system\n"},
            {"role": "user", "content": [
                {"type": "text", "text": " hello\n"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,aGVsbG8="}},
                {"type": "image_url", "image_url": {"url": "https://example.test/image.jpg"}}
            ]},
            {"role": "assistant", "content": "searching", "reasoning_content": "plan", "tool_calls": [{"id": "call-1", "type": "function", "function": {"name": "notion-list-recent-pages_27e3aae889c3", "arguments": "{\"query\":\"recent\"}"}}]},
            {"role": "tool", "tool_call_id": "call-1", "content": "part 1\npart 2"},
            {"role": "user", "content": "continue"}
        ])
    );
    assert_eq!(
        body["tools"][0]["function"]["name"],
        "notion-list-recent-pages_27e3aae889c3"
    );
    assert_eq!(
        body["tools"][0]["function"]["parameters"],
        serde_json::to_value(&request.tools.as_ref().unwrap()[0].input_schema).unwrap()
    );
    assert_eq!(body["tools"][0]["function"]["description"], "");
    assert_eq!(body["tool_choice"], "auto");
    assert_eq!(body["max_tokens"], 4096);
    assert_eq!(body["temperature"], 0.0);
    assert_eq!(body["top_p"], 0.0);
    assert_eq!(body["top_k"], 1.0);
    assert_eq!(body["stop"], json!([]));
    for local in [
        "metadata",
        "session_id",
        "run_id",
        "requestId",
        "structuredContent",
        "startTime",
    ] {
        assert!(!body.to_string().contains(local));
    }
}

#[tokio::test]
async fn omitted_settings_and_empty_values_keep_their_request_meaning() {
    for tools in [None, Some(vec![])] {
        let mut request = request(Value::Null);
        request.tools = tools.map(std::sync::Arc::new);
        request.system = Some(Arc::new(SystemPrompt::Parts(vec![])));
        request.messages.clear();
        let prepared = convert(&request, &ProviderKind::OpenaiChat, None).unwrap();
        let body = send(prepared.options, "openai").await;
        assert_eq!(body["messages"], json!([]));
        for key in [
            "tools",
            "tool_choice",
            "max_tokens",
            "temperature",
            "top_p",
            "top_k",
            "stop",
            "reasoning_effort",
            "service_tier",
        ] {
            assert!(body.get(key).is_none(), "unexpected {key}: {body}");
        }
    }
    let mut request = request(json!({"maxTokens": 4294967295.0, "stopSequences": ["", " \n"]}));
    request.system = Some(Arc::new(SystemPrompt::Text(String::new())));
    request.messages = vec![std::sync::Arc::new(dim_agent_types::Message::text(
        Role::User,
        "",
    ))];
    let body = send(
        convert(&request, &ProviderKind::OpenaiChat, None)
            .unwrap()
            .options,
        "openai",
    )
    .await;
    assert_eq!(
        body["messages"],
        json!([{"role": "system", "content": ""}, {"role": "user", "content": ""}])
    );
    assert_eq!(body["max_tokens"], u32::MAX);
    assert_eq!(body["stop"], json!(["", " \n"]));
}

#[tokio::test]
async fn reasoning_uses_selected_provider_fields_and_intent_precedence_on_the_wire() {
    let cases = [
        (
            Some("deepseek"),
            json!({"providerOptions": {"deepseek": {"reasoningEffort": "high"}, "openai-compatible": {"thinking": {"type": "disabled"}}}}),
            json!({"reasoning_effort": "high"}),
        ),
        (
            Some("deepseek"),
            json!({"providerOptions": {"deepseek": {"reasoningEffort": "none"}, "openai-compatible": {"thinking": {"type": "enabled"}}}}),
            json!({"reasoning_effort": "none"}),
        ),
        (
            None,
            json!({"reasoning": {"effort": "none"}, "providerOptions": {"openai-compatible": {"thinking": {"type": "enabled"}}}}),
            json!({"thinking": {"type": "enabled"}}),
        ),
        (
            None,
            json!({"reasoning": {"effort": "high"}, "providerOptions": {"openai-compatible": {"reasoningEffort": "provider-default"}}}),
            json!({}),
        ),
        (
            None,
            json!({"reasoning": {"enabled": true}, "providerOptions": {"openai-compatible": {"enable_thinking": false, "serviceTier": "auto"}}}),
            json!({"enable_thinking": false, "service_tier": "auto"}),
        ),
        (
            Some("moonshotai"),
            json!({"reasoning": {"enabled": true, "effort": "low"}, "providerOptions": {"moonshotai": {"reasoningEffort": "high"}, "deepseek": {"interleavedThinking": false}, "openai": {"summary": "auto"}, "openai-compatible": {"thinking": {"type": "enabled"}, "enable_thinking": true}}}),
            json!({"reasoning_effort": "high", "thinking": {"type": "enabled"}, "enable_thinking": true}),
        ),
    ];
    for (preset, config, expected) in cases {
        let prepared = convert(&request(config), &ProviderKind::OpenaiCompatible, preset).unwrap();
        assert!(prepared.options.reasoning.is_none());
        let body = send(prepared.options, preset.unwrap_or("openai")).await;
        for key in [
            "reasoning_effort",
            "thinking",
            "enable_thinking",
            "service_tier",
        ] {
            assert_eq!(body.get(key), expected.get(key), "key={key}; body={body}");
        }
    }
    let request = request(
        json!({"reasoning": {"effort": "low"}, "providerOptions": {"openai": {"reasoningEffort": "high", "serviceTier": "default"}, "openai-compatible": {"interleavedThinking": true}}}),
    );
    let body = send(
        convert(&request, &ProviderKind::OpenaiChat, Some("deepseek"))
            .unwrap()
            .options,
        "openai",
    )
    .await;
    assert_eq!(body["reasoning_effort"], "high");
    assert_eq!(body["service_tier"], "default");
}

#[test]
fn unsupported_request_values_fail_with_model_and_field_before_http() {
    for (config, path) in [
        (json!({"maxTokens": 4294967296.0}), "config.maxTokens"),
        (json!({"maxTokens": 0.5}), "config.maxTokens"),
        (
            json!({"providerOptions": {"openai-compatible": {"reasoningEffort": "high", "enable_thinking": false}}}),
            "config.providerOptions.openai-compatible",
        ),
        (
            json!({"reasoning": {"enabled": true, "effort": "high"}, "providerOptions": {"openai-compatible": {"reasoningEffort": "provider-default"}}}),
            "config.reasoning.enabled",
        ),
        (
            json!({"reasoning": {"budgetTokens": 0}}),
            "config.reasoning.budgetTokens",
        ),
        (
            json!({"reasoning": {"includeThoughts": false}}),
            "config.reasoning.includeThoughts",
        ),
        (
            json!({"reasoning": {"interleavedThinking": false}}),
            "config.reasoning.interleavedThinking",
        ),
        (
            json!({"providerOptions": {"openai-compatible": {"thinking": {}}}}),
            "config.providerOptions.openai-compatible.thinking.type",
        ),
        (
            json!({"providerOptions": {"openai-compatible": {"reasoningEffort": ""}}}),
            "config.providerOptions.openai-compatible",
        ),
        (
            json!({"providerOptions": {"openai-compatible": {"serviceTier": ""}}}),
            "config.providerOptions.openai-compatible.serviceTier",
        ),
    ] {
        let Err(error) = convert(&request(config), &ProviderKind::OpenaiCompatible, None) else {
            panic!("expected error at {path}");
        };
        let message = error.to_string();
        assert!(message.contains("application-alias/gpt-4o"), "{message}");
        assert!(message.contains(path), "{message}");
    }
    for message in [
        json!({"role": "tool_result", "content": "no ID"}),
        json!({"role": "tool_result", "content": [{"type": "tool_result", "tool_use_id": "missing", "content": ""}]}),
        json!({"role": "user", "content": [{"type": "thinking", "thinking": "wrong role"}]}),
    ] {
        let mut request = request(Value::Null);
        request.messages = vec![serde_json::from_value(message).unwrap()];
        assert!(convert(&request, &ProviderKind::OpenaiChat, None).is_err());
    }
    let mut request = request(Value::Null);
    request.system = Some(Arc::new(
        serde_json::from_value(
            json!([{"type": "text", "text": "system", "cache_control": {"type": "ephemeral"}}]),
        )
        .unwrap(),
    ));
    assert!(convert(&request, &ProviderKind::OpenaiChat, None).is_err());
}

#[tokio::test]
async fn tool_name_compatibility_preserves_hash_recovery_and_rejects_wire_collisions() {
    let original = "plugin:nowledge-mem/nowledge-mem__trigger_memory_catchup";
    let mut request = request(Value::Null);
    request.tools = Some(vec![
        tool(original),
        tool("other_045e29d516c3"),
        tool("read"),
        tool(&format!("插件:{}工具", "very-long-server/".repeat(8))),
    ])
    .map(std::sync::Arc::new);
    let prepared = convert(&request, &ProviderKind::OpenaiChat, None).unwrap();
    assert_eq!(
        prepared
            .tool_names
            .original("trigger_memory_catchup_045e29d516c3"),
        original
    );
    assert_eq!(
        prepared.tool_names.original("other_045e29d516c3"),
        "other_045e29d516c3"
    );
    assert_eq!(
        prepared.tool_names.original("alias_045e29d516c3"),
        "alias_045e29d516c3"
    );
    assert_eq!(prepared.tool_names.original("unknown_tool"), "unknown_tool");
    let body = send(prepared.options, "openai").await;
    assert_eq!(
        body["tools"][0]["function"]["name"],
        "trigger_memory_catchup_045e29d516c3"
    );
    assert_eq!(body["tools"][2]["function"]["name"], "read");
    assert_eq!(
        body["tools"][3]["function"]["name"].as_str().unwrap().len(),
        64
    );

    request.tools = Some(vec![
        tool(original),
        tool("trigger_memory_catchup_045e29d516c3"),
    ])
    .map(std::sync::Arc::new);
    let Err(error) = convert(&request, &ProviderKind::OpenaiChat, None) else {
        panic!("expected name collision");
    };
    assert!(error.to_string().contains("tools[1].name"));
}

// The canonical schema every protocol must send for a tool whose input
// schema arrived from MCP with a `type` array on one property.

fn mcp_shaped_tool() -> Value {
    json!({
        "name": "spaces", "description": "", "source": "dynamic",
        "inputSchema": {
            "type": "object",
            "properties": {"space_id": {"type": ["string", "null"]}},
            "required": ["space_id"]
        }
    })
}

fn canonical_shaped_schema() -> Value {
    json!({
        "type": "object",
        "properties": {"space_id": {"anyOf": [{"type": "string"}, {"type": "null"}]}},
        "required": ["space_id"]
    })
}

fn tool_request(tools: Vec<Value>) -> ModelRequest {
    serde_json::from_value(json!({
        "providerId": "application-alias", "modelId": "gpt-4o",
        "messages": [{"role": "user", "content": "hello"}],
        "tools": tools
    }))
    .unwrap()
}

fn options(kind: ProviderKind) -> CallOptions {
    options_with(mcp_shaped_tool(), kind)
}

fn options_with(tool: Value, kind: ProviderKind) -> CallOptions {
    convert(&tool_request(vec![tool]), &kind, None)
        .unwrap()
        .options
}

/// The same tool with `additionalProperties: false`, which xAI's body strips.
fn mcp_shaped_tool_closed() -> Value {
    let mut tool = mcp_shaped_tool();
    tool["inputSchema"]["additionalProperties"] = json!(false);
    tool
}

#[test]
fn the_openai_chat_and_compatible_bodies_carry_the_canonical_tool_schema() {
    for kind in [ProviderKind::OpenaiChat, ProviderKind::OpenaiCompatible] {
        let body =
            aimux_providers::openai::convert::build_request_body("gpt-4o", &options(kind), false)
                .unwrap();
        assert_eq!(
            body["tools"][0]["function"]["parameters"],
            canonical_shaped_schema(),
            "{kind:?}"
        );
    }
}

#[test]
fn the_openai_responses_body_carries_the_canonical_tool_schema() {
    let body = aimux_providers::openai::responses::convert::build_responses_request_body(
        "gpt-5",
        &options(ProviderKind::OpenaiResponses),
        false,
    )
    .body;
    assert_eq!(body["tools"][0]["parameters"], canonical_shaped_schema());
}

#[test]
fn the_anthropic_body_carries_the_canonical_tool_schema() {
    let body = aimux_providers::anthropic::convert::build_request_body(
        "claude-sonnet-4-5",
        &options(ProviderKind::Anthropic),
        false,
    )
    .unwrap();
    assert_eq!(body["tools"][0]["input_schema"], canonical_shaped_schema());
}

#[test]
fn the_xai_body_carries_the_canonical_tool_schema_without_additional_properties() {
    let body = aimux_providers::xai::convert::build_request_body_with_warnings(
        "grok-4",
        &options_with(mcp_shaped_tool_closed(), ProviderKind::Xai),
        false,
    )
    .unwrap()
    .body;
    let parameters = &body["tools"][0]["function"]["parameters"];
    assert_eq!(parameters, &canonical_shaped_schema());
    assert!(
        parameters.get("additionalProperties").is_none(),
        "xAI strips additionalProperties: false: {parameters}"
    );
}

#[test]
fn the_xai_responses_body_carries_the_canonical_tool_schema_without_additional_properties() {
    let body = aimux_providers::xai::responses::convert::build_responses_request_body(
        "grok-4",
        &options_with(mcp_shaped_tool_closed(), ProviderKind::XaiResponses),
        false,
    )
    .unwrap()
    .body;
    let parameters = &body["tools"][0]["parameters"];
    assert_eq!(parameters, &canonical_shaped_schema());
    assert!(
        parameters.get("additionalProperties").is_none(),
        "xAI strips additionalProperties: false: {parameters}"
    );
}

#[test]
fn the_bedrock_body_carries_the_canonical_tool_schema() {
    let body = aimux_providers::bedrock::convert::build_request_body(
        "anthropic.claude-sonnet-4-5-20250929-v1:0",
        &options(ProviderKind::Bedrock),
    )
    .unwrap();
    assert_eq!(
        body["toolConfig"]["tools"][0]["toolSpec"]["inputSchema"]["json"],
        canonical_shaped_schema()
    );
}

#[test]
fn the_gemini_body_folds_the_nullable_type_into_a_nullable_field() {
    let body = aimux_providers::google::convert::build_request_body(
        "gemini-2.5-pro",
        &options(ProviderKind::Gemini),
    );
    let parameters = &body["tools"][0]["functionDeclarations"][0]["parameters"];
    assert_eq!(
        parameters["properties"]["space_id"],
        json!({"type": "string", "nullable": true})
    );
    assert_eq!(parameters["required"], json!(["space_id"]));
}

/// Gemini's OpenAPI 3.0 subset drops `not`, so a property stored as `{}` (from
/// the boolean subschema `true`) and one stored as `{"not": {}}` (from `false`)
/// both reach the wire as the empty schema. Recorded here so a change in the
/// converter is visible; dim does not compensate.
#[test]
fn gemini_emits_stored_boolean_subschemas_as_empty_schemas() {
    let request = tool_request(vec![json!({
        "name": "flags", "description": "", "source": "dynamic",
        "inputSchema": {"type": "object", "properties": {"x": true, "y": false}}
    })]);
    let options = convert(&request, &ProviderKind::Gemini, None)
        .unwrap()
        .options;
    let body = aimux_providers::google::convert::build_request_body("gemini-2.5-pro", &options);
    let properties = &body["tools"][0]["functionDeclarations"][0]["parameters"]["properties"];
    assert_eq!(properties["x"], json!({}));
    assert_eq!(properties["y"], json!({}));
}
