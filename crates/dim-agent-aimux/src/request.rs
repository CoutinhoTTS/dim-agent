mod names;
mod native;
mod reasoning;

#[cfg(test)]
mod tests;

pub(crate) use names::ToolNames;

use crate::config::ProviderKind;
use aimux_core::{
    content::ContentPart,
    language_model_message::LanguageModelPromptMessage,
    message::Role as AimuxRole,
    options::{CallOptions, ToolChoice},
    tool::{FunctionTool, Tool},
};
use dim_agent_capabilities::host::HostError;
use dim_agent_types::{
    ContentBlock, ContentMetadata, ImageSourceType, MessageContent, MessagePart, ModelRequest,
    Role, SystemPrompt, SystemPromptPart, ToolContent,
};
use serde_json::{Value, json};
use std::collections::HashMap;

pub(crate) struct PreparedRequest {
    pub options: CallOptions,
    pub tool_names: ToolNames,
}

pub(crate) fn convert(
    request: &ModelRequest,
    kind: &ProviderKind,
    preset: Option<&str>,
) -> Result<PreparedRequest, HostError> {
    let mut tool_names = ToolNames::default();
    let mut prompt = Vec::new();
    match request.system.as_deref() {
        Some(SystemPrompt::Text(text)) => {
            prompt.push(message(AimuxRole::System, vec![ContentPart::text(text)]))
        }
        Some(SystemPrompt::Parts(parts)) => {
            for (
                index,
                SystemPromptPart::Text {
                    text,
                    cache_control,
                },
            ) in parts.iter().enumerate()
            {
                if cache_control.is_some() && *kind != ProviderKind::Anthropic {
                    return Err(error(
                        request,
                        &format!("system[{index}].cache_control"),
                        "cache control is only supported by Anthropic",
                    ));
                }
                let part = ContentPart::Text {
                    text: text.clone(),
                    provider_options: cache_control
                        .map(|control| json!({"anthropic":{"cacheControl":control}})),
                };
                prompt.push(message(AimuxRole::System, vec![part]));
            }
        }
        None => {}
    }
    let tools = request
        .tools
        .as_ref()
        .filter(|tools| !tools.is_empty())
        .map(|tools| {
            tools
                .iter()
                .enumerate()
                .map(|(index, tool)| {
                    let name = tool_names.register(&tool.name).map_err(|reason| {
                        error(request, &format!("tools[{index}].name"), &reason)
                    })?;
                    let schema = serde_json::to_value(&tool.input_schema).map_err(|reason| {
                        error(
                            request,
                            &format!("tools[{index}].inputSchema"),
                            &reason.to_string(),
                        )
                    })?;
                    Ok(Tool::Function(
                        FunctionTool::new(name, schema).with_description(&tool.description),
                    ))
                })
                .collect::<Result<Vec<_>, HostError>>()
        })
        .transpose()?;

    let mut calls = HashMap::new();
    for (index, source) in request.messages.iter().enumerate() {
        let path = format!("messages[{index}].content");
        let content = match &source.content {
            MessageContent::Text(text) if source.role != Role::ToolResult => {
                vec![ContentPart::text(text)]
            }
            MessageContent::Text(_) => {
                return Err(error(request, &path, "tool results require a tool call ID"));
            }
            MessageContent::Parts(parts) => {
                let mut content = Vec::with_capacity(parts.len());
                for (part_index, part) in parts.iter().enumerate() {
                    let path = format!("{path}[{part_index}]");
                    let mapped = match (source.role, part) {
                        (Role::User | Role::Assistant, MessagePart::Text { text, .. }) => {
                            Ok(ContentPart::text(text))
                        }
                        (Role::Assistant, MessagePart::Thinking { thinking, .. }) => {
                            Ok(ContentPart::reasoning(thinking))
                        }
                        (Role::User, MessagePart::Image { source }) => match source.source_type {
                            ImageSourceType::Base64 => {
                                let data = source.data.as_ref().ok_or_else(|| {
                                    error(
                                        request,
                                        &format!("{path}.source.data"),
                                        "base64 image has no data",
                                    )
                                })?;
                                Ok(ContentPart::file_base64(data, &source.media_type))
                            }
                            ImageSourceType::Url => {
                                let url = source.url.as_ref().ok_or_else(|| {
                                    error(
                                        request,
                                        &format!("{path}.source.url"),
                                        "URL image has no URL",
                                    )
                                })?;
                                Ok(ContentPart::file_url(url, &source.media_type))
                            }
                        },
                        (
                            Role::Assistant,
                            MessagePart::ToolUse {
                                id, name, input, ..
                            },
                        ) => {
                            let name = tool_names.register(name).map_err(|reason| {
                                error(request, &format!("{path}.name"), &reason)
                            })?;
                            calls.insert(id.as_str(), name.clone());
                            Ok(ContentPart::tool_call(
                                id,
                                name,
                                Value::Object(input.clone()),
                            ))
                        }
                        (
                            Role::ToolResult,
                            MessagePart::ToolResult {
                                tool_use_id,
                                content,
                                is_error,
                                ..
                            },
                        ) => {
                            let name = calls.get(tool_use_id.as_str()).ok_or_else(|| {
                                error(
                                    request,
                                    &format!("{path}.tool_use_id"),
                                    "no preceding tool call",
                                )
                            })?;
                            let result = match content {
                                ToolContent::Text(text) => text.clone(),
                                ToolContent::Blocks(blocks) => {
                                    let mut result = String::new();
                                    for block in blocks {
                                        match block {
                                            ContentBlock::Text { text } => result.push_str(text),
                                            ContentBlock::Image { .. } => {
                                                return Err(error(
                                                    request,
                                                    &format!("{path}.content"),
                                                    "images in tool results are not supported",
                                                ));
                                            }
                                        }
                                    }
                                    result
                                }
                            };
                            Ok(ContentPart::ToolResult {
                                tool_call_id: tool_use_id.clone(),
                                result: if *kind == ProviderKind::Anthropic
                                    && *is_error == Some(true)
                                {
                                    json!({"type":"error", "value":result})
                                } else {
                                    Value::String(result)
                                },
                                tool_name: Some(name.clone()),
                                is_error: *is_error,
                                preliminary: None,
                                dynamic: None,
                                provider_options: None,
                            })
                        }
                        _ => Err(error(
                            request,
                            &path,
                            "content cannot be represented for this message role",
                        )),
                    };
                    let mut mapped = mapped?;
                    continuation(request, kind, &path, part, &mut mapped)?;
                    content.push(mapped);
                }
                content
            }
        };
        let role = match source.role {
            Role::User => AimuxRole::User,
            Role::Assistant => AimuxRole::Assistant,
            Role::ToolResult => AimuxRole::Tool,
        };
        prompt.push(message(role, content));
    }

    let mut options = CallOptions {
        tool_choice: if tools.is_some() {
            ToolChoice::Auto
        } else {
            ToolChoice::None
        },
        tools,
        max_retries: Some(0),
        ..CallOptions::new(prompt)
    };
    if let Some(config) = &request.config {
        options.temperature = config.temperature;
        options.top_p = config.top_p;
        options.top_k = config.top_k;
        options.stop_sequences = config.stop_sequences.clone();
        options.max_output_tokens = config
            .max_tokens
            .map(|value| {
                if !value.is_finite()
                    || value.fract() != 0.0
                    || !(1.0..=f64::from(u32::MAX)).contains(&value)
                {
                    Err(error(
                        request,
                        "config.maxTokens",
                        "must be an integer in 1..=4294967295 for aimux",
                    ))
                } else {
                    Ok(value as u32)
                }
            })
            .transpose()?;
    }
    reasoning::apply(
        request,
        request.config.as_ref().unwrap_or(&Default::default()),
        kind,
        preset,
        &mut options,
    )?;
    if *kind == ProviderKind::Anthropic
        && options.prompt.iter().any(|message| {
            message
                .content
                .iter()
                .any(|part| matches!(part, ContentPart::Reasoning { .. }))
        })
        && !options
            .provider_options
            .as_ref()
            .and_then(|p| p.get("anthropic"))
            .and_then(|p| p.get("thinking"))
            .and_then(|p| p.get("type"))
            .and_then(Value::as_str)
            .is_some_and(|kind| matches!(kind, "enabled" | "adaptive"))
    {
        return Err(error(
            request,
            "config",
            "Anthropic thinking continuation requires enabled or adaptive thinking",
        ));
    }
    Ok(PreparedRequest {
        options,
        tool_names,
    })
}

fn message(role: AimuxRole, content: Vec<ContentPart>) -> LanguageModelPromptMessage {
    LanguageModelPromptMessage {
        role,
        content,
        provider_options: None,
    }
}

fn error(request: &ModelRequest, path: &str, reason: &str) -> HostError {
    HostError::failed(format!(
        "{}/{}: {path}: {reason}",
        request.provider_id, request.model_id
    ))
}

fn continuation(
    request: &ModelRequest,
    kind: &ProviderKind,
    path: &str,
    part: &MessagePart,
    native: &mut ContentPart,
) -> Result<(), HostError> {
    let metadata = match part {
        MessagePart::Text {
            provider_metadata, ..
        }
        | MessagePart::Thinking {
            provider_metadata, ..
        }
        | MessagePart::ToolUse {
            provider_metadata, ..
        } => provider_metadata.as_ref(),
        _ => None,
    };
    let Some(metadata) = metadata else {
        if matches!(part, MessagePart::Thinking { .. })
            && matches!(
                kind,
                ProviderKind::Anthropic
                    | ProviderKind::Bedrock
                    | ProviderKind::OpenaiResponses
                    | ProviderKind::XaiResponses
            )
        {
            return Err(error(
                request,
                path,
                "thinking requires native continuation metadata",
            ));
        }
        return Ok(());
    };
    let options = match (kind, metadata) {
        (ProviderKind::Bedrock, ContentMetadata::Bedrock { signature })
            if matches!(part, MessagePart::Thinking { .. }) =>
        {
            match signature {
                Some(signature) => json!({"bedrock":{"signature":signature}}),
                None => json!({"bedrock":{}}),
            }
        }
        (ProviderKind::Bedrock, ContentMetadata::BedrockRedacted { data })
            if matches!(part, MessagePart::Thinking { .. }) =>
        {
            json!({"bedrock":{"redactedData":data}})
        }
        (ProviderKind::Anthropic, ContentMetadata::Anthropic { signature })
            if matches!(part, MessagePart::Thinking { .. }) =>
        {
            json!({"anthropic":{"signature":signature}})
        }
        (ProviderKind::Anthropic, ContentMetadata::AnthropicRedacted { data })
            if matches!(part, MessagePart::Thinking { .. }) =>
        {
            json!({"anthropic":{"redactedData":data}})
        }
        (ProviderKind::Gemini, ContentMetadata::Gemini { thought_signature }) => {
            json!({"google":{"thoughtSignature":thought_signature}})
        }
        (
            ProviderKind::XaiResponses,
            ContentMetadata::XaiResponses {
                item_id,
                encrypted_content,
            },
        ) => {
            if matches!(part, MessagePart::Thinking { .. })
                && encrypted_content.as_ref().is_none_or(String::is_empty)
            {
                return Err(error(
                    request,
                    path,
                    "thinking requires encrypted continuation metadata",
                ));
            }
            let mut options = serde_json::Map::from_iter([("itemId".into(), json!(item_id))]);
            if let Some(encrypted) = encrypted_content {
                options.insert("reasoningEncryptedContent".into(), json!(encrypted));
            }
            json!({"xai": options})
        }
        (
            ProviderKind::OpenaiResponses,
            ContentMetadata::OpenaiResponses {
                item_id,
                encrypted_content,
                summary_index,
                phase,
            },
        ) => {
            if matches!(part, MessagePart::Thinking { .. })
                && encrypted_content.as_ref().is_none_or(String::is_empty)
            {
                return Err(error(
                    request,
                    path,
                    "thinking requires encrypted continuation metadata",
                ));
            }
            let mut options = serde_json::Map::from_iter([("itemId".into(), json!(item_id))]);
            if let Some(encrypted) = encrypted_content {
                options.insert("reasoningEncryptedContent".into(), json!(encrypted));
            }
            if let Some(index) = summary_index {
                options.insert("summaryIndex".into(), json!(index));
            }
            if let Some(phase) = phase {
                options.insert("phase".into(), json!(phase));
            }
            json!({"openai":options})
        }
        _ => {
            return Err(error(
                request,
                &format!("{path}.providerMetadata"),
                "continuation metadata belongs to a different protocol or content kind",
            ));
        }
    };
    match native {
        ContentPart::Text {
            provider_options, ..
        }
        | ContentPart::Reasoning {
            provider_options, ..
        }
        | ContentPart::ToolCall {
            provider_options, ..
        } => *provider_options = Some(options),
        _ => unreachable!(),
    }
    Ok(())
}
