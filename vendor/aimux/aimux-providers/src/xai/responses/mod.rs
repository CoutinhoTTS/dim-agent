//! xAI Responses API language model.
//!
//! Implements `LanguageModel` against xAI's `/responses` endpoint.
//! Mirrors the TS `XaiResponsesLanguageModel`.
//!
//! Key differences from the chat model:
//! - Uses `input` (not `messages`) with Responses API format
//! - `reasoning: { effort, summary }` object (not `reasoning_effort` string)
//! - `text.format` for structured output (not `response_format`)
//! - Provider-executed tools (web_search, x_search, code_execution, etc.)
//! - ~65 streaming event types (response.created, response.output_text.delta, etc.)
//! - `cost_in_usd_ticks` in providerMetadata
//! - Citations via annotations → sources

pub mod convert;
pub mod types;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, FinishReasonUnified, ResponseMetadata, Usage};

use aimux_provider_utils::HttpRequest;

use super::super::XAIConfig;
use crate::openai::responses::responses_convert::build_header_list;
use convert::{
    build_responses_request_body, convert_xai_responses_usage, get_tool_input,
    map_xai_responses_finish_reason, resolve_tool_name,
};

/// Global counter for generating unique source IDs.
static SOURCE_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

fn generate_source_id() -> String {
    format!("id-{}", SOURCE_ID_COUNTER.fetch_add(1, Ordering::SeqCst))
}

/// An xAI Responses language model (Grok).
///
/// Uses the shared HTTP client unless a caller-owned pool is supplied.
pub struct XaiResponsesModel {
    model_id: String,
    config: XAIConfig,
    client: Option<reqwest::Client>,
}

impl XaiResponsesModel {
    #[must_use]
    pub fn new(model_id: String, config: XAIConfig) -> Self {
        Self {
            model_id,
            config,
            client: None,
        }
    }

    /// Use the caller-owned HTTP connection pool for every exchange.
    #[must_use]
    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = Some(client);
        self
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        if !self.config.api_key().is_empty() {
            headers.insert(
                "Authorization".to_string(),
                format!("Bearer {}", self.config.api_key()),
            );
        }
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }

    fn endpoint(&self) -> Result<String, AiMuxError> {
        let mut url = reqwest::Url::parse(self.config.base_url())
            .map_err(|error| AiMuxError::Other(format!("invalid xAI base URL: {error}")))?;
        url.path_segments_mut()
            .map_err(|()| AiMuxError::Other("xAI base URL cannot be a base".into()))?
            .pop_if_empty()
            .push("responses");
        Ok(url.into())
    }
}

#[async_trait]
impl LanguageModel for XaiResponsesModel {
    fn provider(&self) -> &str {
        "xai.responses"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn retry_config(&self) -> aimux_core::retry::RetryConfig {
        self.config.openai_config().retry_config
    }

    fn config_snapshot(&self) -> aimux_core::recording::ProviderRecord {
        // M2b: reuse the OpenAI snapshot helper with xAI's provider name.
        crate::openai::config_snapshot_from_config(
            self.provider(),
            &self.model_id,
            self.config.openai_config(),
        )
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let headers = self.build_headers(options.headers.as_ref());
        let request_result = build_responses_request_body(&self.model_id, options, false)?;
        let body = request_result.body;
        let provider_tool_names = request_result.provider_tool_names;

        let resp = aimux_provider_utils::post_json_to_api(
            HttpRequest {
                client: self.client.clone(),
                ..HttpRequest::new(self.endpoint()?, build_header_list(&headers), options)
            },
            body.clone(),
            super::xai_successful_response_handler::<types::XaiResponsesResponse>(),
            super::xai_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let _raw_value = resp.raw_value.unwrap_or(Value::Null);
        let data = resp.value;

        let mut content: Vec<GenerateContent> = Vec::new();
        let mut has_function_call = false;

        for part in &data.output {
            let part_type = part.get("type").and_then(|v| v.as_str()).unwrap_or("");

            // ── file_search_call ──
            if part_type == "file_search_call" {
                let tool_name = resolve_tool_name(part_type, None, &provider_tool_names);
                let part_id = part.get("id").and_then(|v| v.as_str()).unwrap_or("");

                content.push(GenerateContent::ToolCall {
                    tool_call_id: part_id.to_string(),
                    tool_name: tool_name.clone(),
                    input: String::new(),
                    provider_executed: Some(true),
                    dynamic: None,
                    thought_signature: None,
                    provider_metadata: None,
                });

                let queries = part
                    .get("queries")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let results =
                    part.get("results")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            json!(arr.iter().map(|r| json!({
                        "fileId": r.get("file_id").cloned().unwrap_or(Value::Null),
                        "filename": r.get("filename").cloned().unwrap_or(Value::Null),
                        "score": r.get("score").cloned().unwrap_or(Value::Null),
                        "text": r.get("text").cloned().unwrap_or(Value::Null),
                    })).collect::<Vec<_>>())
                        })
                        .unwrap_or(Value::Null);

                content.push(GenerateContent::ToolResult {
                    tool_call_id: part_id.to_string(),
                    tool_name,
                    result: json!({ "queries": queries, "results": results }),
                    is_error: None,
                    preliminary: None,
                    dynamic: None,
                    provider_metadata: None,
                });
                continue;
            }

            // ── Server-side tool calls ──
            if matches!(
                part_type,
                "web_search_call"
                    | "x_search_call"
                    | "code_interpreter_call"
                    | "code_execution_call"
                    | "view_image_call"
                    | "view_x_video_call"
                    | "custom_tool_call"
                    | "mcp_call"
            ) {
                let part_name = part.get("name").and_then(|v| v.as_str());
                let tool_name = resolve_tool_name(part_type, part_name, &provider_tool_names);
                let tool_input = get_tool_input(part_type, part);
                let part_id = part.get("id").and_then(|v| v.as_str()).unwrap_or("");

                content.push(GenerateContent::ToolCall {
                    tool_call_id: part_id.to_string(),
                    tool_name,
                    input: tool_input,
                    provider_executed: Some(true),
                    dynamic: None,
                    thought_signature: None,
                    provider_metadata: None,
                });
                continue;
            }

            match part_type {
                "message" => {
                    let content_parts = part
                        .get("content")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();
                    for cp in &content_parts {
                        if let Some(text) = cp.get("text").and_then(|v| v.as_str())
                            && !text.is_empty()
                        {
                            content.push(GenerateContent::Text {
                                text: text.to_string(),
                                provider_metadata: None,
                            });
                        }
                        if let Some(annotations) = cp.get("annotations").and_then(|v| v.as_array())
                        {
                            for ann in annotations {
                                if ann.get("type").and_then(|v| v.as_str()) == Some("url_citation")
                                {
                                    let url = ann.get("url").and_then(|v| v.as_str()).unwrap_or("");
                                    let title = ann
                                        .get("title")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or(url)
                                        .to_string();
                                    content.push(GenerateContent::Source {
                                        id: generate_source_id(),
                                        source_type: "url".to_string(),
                                        url: Some(url.to_string()),
                                        title: Some(title),
                                        provider_metadata: None,
                                    });
                                }
                            }
                        }
                    }
                }
                "function_call" => {
                    has_function_call = true;
                    let call_id = part.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                    let name = part.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let arguments = part.get("arguments").and_then(|v| v.as_str()).unwrap_or("");
                    let input = arguments.to_string();
                    content.push(GenerateContent::ToolCall {
                        tool_call_id: call_id.to_string(),
                        tool_name: name.to_string(),
                        input,
                        provider_executed: None,
                        dynamic: None,
                        thought_signature: None,
                        provider_metadata: None,
                    });
                }
                "reasoning" => {
                    let summary = part.get("summary").and_then(|v| v.as_array());
                    let reasoning_content = part.get("content").and_then(|v| v.as_array());

                    let texts: Vec<String> = if let Some(summary) = summary {
                        if !summary.is_empty() {
                            summary
                                .iter()
                                .filter_map(|s| {
                                    s.get("text")
                                        .and_then(|v| v.as_str())
                                        .map(std::string::ToString::to_string)
                                })
                                .filter(|t| !t.is_empty())
                                .collect()
                        } else {
                            reasoning_content
                                .map(|c| {
                                    c.iter()
                                        .filter_map(|s| {
                                            s.get("text")
                                                .and_then(|v| v.as_str())
                                                .map(std::string::ToString::to_string)
                                        })
                                        .filter(|t| !t.is_empty())
                                        .collect()
                                })
                                .unwrap_or_default()
                        }
                    } else {
                        Vec::new()
                    };

                    let reasoning_text = texts.join("");
                    let encrypted_content = part.get("encrypted_content").and_then(|v| v.as_str());
                    let item_id = part.get("id").and_then(|v| v.as_str());

                    if !reasoning_text.is_empty() || encrypted_content.is_some() {
                        let mut meta = json!({});
                        if let Some(ec) = encrypted_content {
                            meta["reasoningEncryptedContent"] = json!(ec);
                        }
                        if let Some(id) = item_id {
                            meta["itemId"] = json!(id);
                        }
                        content.push(GenerateContent::Reasoning {
                            text: reasoning_text,
                            provider_metadata: Some(json!({ "xai": meta })),
                        });
                    }
                }
                _ => {}
            }
        }

        let finish_reason = FinishReason {
            unified: if has_function_call {
                FinishReasonUnified::ToolCalls
            } else {
                data.status
                    .as_deref()
                    .map(map_xai_responses_finish_reason)
                    .unwrap_or(FinishReasonUnified::Other)
            },
            raw: data.status.clone(),
        };

        let (usage, provider_metadata) = if let Some(u) = &data.usage {
            let meta = if u.cost_in_usd_ticks.is_some() {
                Some(json!({
                    "xai": { "costInUsdTicks": u.cost_in_usd_ticks }
                }))
            } else {
                None
            };
            (convert_xai_responses_usage(u), meta)
        } else {
            (Usage::default(), None)
        };

        let timestamp = data
            .created_at
            .and_then(|c| chrono::DateTime::from_timestamp(c as i64, 0).map(|dt| dt.to_rfc3339()));

        Ok(GenerateResult {
            content,
            finish_reason,
            usage,
            warnings: request_result.warnings,
            provider_metadata,
            response: ResponseMetadata {
                id: data.id,
                timestamp,
                model_id: data.model,
            },
            request_body: Some(body),
            response_headers: Some(response_headers),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let headers = self.build_headers(options.headers.as_ref());
        let request_result = build_responses_request_body(&self.model_id, options, true)?;
        let body = request_result.body;
        let warnings = request_result.warnings;
        let provider_tool_names = request_result.provider_tool_names;
        let endpoint = self.endpoint()?;

        let resp = aimux_provider_utils::post_json_to_api(
            HttpRequest {
                client: self.client.clone(),
                ..HttpRequest::new(endpoint.clone(), build_header_list(&headers), options)
            },
            body.clone(),
            super::xai_event_source_response_handler::<Value>(),
            super::xai_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let mut sse_stream = resp.value;
        let first_event = match sse_stream.next().await {
            Some(Err(error @ AiMuxError::ApiCall(_))) => return Err(error),
            first_event => first_event,
        };
        if let Some(Ok(ref event)) = first_event
            && types::event_type(event) == "error"
        {
            return Err(super::xai_stream_error(
                event,
                &endpoint,
                body.clone(),
                response_headers.clone(),
            ));
        }
        let stream_error_url = endpoint;
        let stream_request_body = body.clone();
        let stream_response_headers = response_headers.clone();

        let stream = async_stream::stream! {
            yield Ok(StreamPart::StreamStart { warnings });

            let mut final_usage: Option<Usage> = None;
            let mut final_finish_reason: FinishReason = FinishReason {
                unified: FinishReasonUnified::Other,
                raw: None,
            };
            let mut cost_in_usd_ticks: Option<u64> = None;
            let mut has_function_call = false;
            let mut terminal_received = false;
            let mut is_first_chunk = true;

            // Track content blocks.
            let mut content_blocks: HashMap<String, bool> = HashMap::new(); // block_id -> ended (text blocks)
            let mut seen_tool_calls: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut active_reasoning: HashMap<String, ()> = HashMap::new();

            // Track ongoing function calls by output_index.
            // output_index -> (call_id, name, item_id, argument deltas, saw delta)
            let mut ongoing_tool_calls: HashMap<u64, (String, String, String, String, bool)> = HashMap::new();

            let mut event_iter = futures::stream::iter(first_event.into_iter()).chain(sse_stream);

            while let Some(event) = event_iter.next().await {
                match event {
                    Ok(parsed) => {
                        let event_type = types::event_type(&parsed);

                        // ── response.created / response.in_progress ──
                        if event_type == "response.created" || event_type == "response.in_progress" {
                            if is_first_chunk {
                                is_first_chunk = false;
                                let response = parsed.get("response").cloned().unwrap_or(Value::Null);
                                let id = response.get("id").and_then(|v| v.as_str()).map(std::string::ToString::to_string);
                                let model = response.get("model").and_then(|v| v.as_str()).map(std::string::ToString::to_string);
                                let created_at = response.get("created_at").and_then(serde_json::Value::as_u64);
                                let timestamp = created_at.and_then(|c| {
                                    chrono::DateTime::from_timestamp(c as i64, 0).map(|dt| dt.to_rfc3339())
                                });
                                yield Ok(StreamPart::ResponseMetadata { id, timestamp, model_id: model });
                            }
                            continue;
                        }

                        // ── reasoning_summary_part.added ──
                        if event_type == "response.reasoning_summary_part.added" {
                            let item_id = parsed.get("item_id").and_then(|v| v.as_str()).unwrap_or("");
                            let block_id = format!("reasoning-{item_id}");
                            if !active_reasoning.contains_key(item_id) {
                                active_reasoning.insert(item_id.to_string(), ());
                                yield Ok(StreamPart::ReasoningStart {
                                    id: block_id,
                                    provider_metadata: Some(json!({ "xai": { "itemId": item_id } })),
                                });
                            }
                            continue;
                        }

                        // ── reasoning_summary_text.done ──
                        if event_type == "response.reasoning_summary_text.done" {
                            continue;
                        }

                        // Both delta forms may open their reasoning item.
                        if event_type == "response.reasoning_text.delta"
                            || event_type == "response.reasoning_summary_text.delta"
                        {
                            let item_id = parsed.get("item_id").and_then(|v| v.as_str()).unwrap_or("");
                            let delta = parsed.get("delta").and_then(|v| v.as_str()).unwrap_or("");
                            let block_id = format!("reasoning-{item_id}");
                            if !active_reasoning.contains_key(item_id) {
                                active_reasoning.insert(item_id.to_string(), ());
                                yield Ok(StreamPart::ReasoningStart {
                                    id: block_id.clone(),
                                    provider_metadata: Some(json!({ "xai": { "itemId": item_id } })),
                                });
                            }
                            yield Ok(StreamPart::ReasoningDelta {
                                id: block_id,
                                delta: delta.to_string(),
                                provider_metadata: Some(json!({ "xai": { "itemId": item_id } })),
                            });
                            continue;
                        }

                        // ── reasoning_text.done ──
                        if event_type == "response.reasoning_text.done" {
                            continue;
                        }

                        // ── output_text.delta ──
                        if event_type == "response.output_text.delta" {
                            let item_id = parsed.get("item_id").and_then(|v| v.as_str()).unwrap_or("");
                            let content_index = parsed.get("content_index").and_then(|v| v.as_u64()).unwrap_or(0);
                            let delta = parsed.get("delta").and_then(|v| v.as_str()).unwrap_or("");
                            let block_id = format!("text-{item_id}:{content_index}");
                            if !content_blocks.contains_key(&block_id) {
                                content_blocks.insert(block_id.clone(), false);
                                yield Ok(StreamPart::TextStart { id: block_id.clone(), provider_metadata: None});
                            }
                            yield Ok(StreamPart::TextDelta {
                                id: block_id,
                                delta: delta.to_string(),
                                provider_metadata: None,
                            });
                            continue;
                        }

                        // ── output_text.done ──
                        if event_type == "response.output_text.done" {
                            if let Some(annotations) = parsed.get("annotations").and_then(|v| v.as_array()) {
                                for ann in annotations {
                                    if ann.get("type").and_then(|v| v.as_str()) == Some("url_citation") {
                                        let url = ann.get("url").and_then(|v| v.as_str()).unwrap_or("");
                                        let title = ann.get("title").and_then(|v| v.as_str()).unwrap_or(url).to_string();
                                        yield Ok(StreamPart::Source {
                                            id: generate_source_id(),
                                            source_type: "url".to_string(),
                                            url: Some(url.to_string()),
                                            title: Some(title),
                                            provider_metadata: None,
                                        });
                                    }
                                }
                            }
                            continue;
                        }

                        // ── output_text.annotation.added ──
                        if event_type == "response.output_text.annotation.added" {
                            let annotation = parsed.get("annotation").cloned().unwrap_or(Value::Null);
                            if annotation.get("type").and_then(|v| v.as_str()) == Some("url_citation") {
                                let url = annotation.get("url").and_then(|v| v.as_str()).unwrap_or("");
                                let title = annotation.get("title").and_then(|v| v.as_str()).unwrap_or(url).to_string();
                                yield Ok(StreamPart::Source {
                                    id: generate_source_id(),
                                    source_type: "url".to_string(),
                                    url: Some(url.to_string()),
                                    title: Some(title),
                                    provider_metadata: None,
                                });
                            }
                            continue;
                        }

                        // A provider error is terminal, not an incomplete transport.
                        if event_type == "error"
                            || event_type == "response.failed"
                            || (event_type == "response.done"
                                && parsed["response"]["status"] == "failed")
                        {
                            let error = if event_type == "error" {
                                &parsed
                            } else {
                                &parsed["response"]
                            };
                            yield Ok(StreamPart::Error {
                                error: super::xai_stream_error(
                                    error,
                                    &stream_error_url,
                                    stream_request_body.clone(),
                                    stream_response_headers.clone(),
                                ),
                            });
                            return;
                        }

                        // ── response.done / response.completed / response.incomplete ──
                        if event_type == "response.done"
                            || event_type == "response.completed"
                            || event_type == "response.incomplete"
                        {
                            let response = parsed.get("response").cloned().unwrap_or(Value::Null);
                            if let Some(usage) = response.get("usage")
                                && let Ok(u) = serde_json::from_value::<types::XaiResponsesUsage>(usage.clone()) {
                                    final_usage = Some(convert_xai_responses_usage(&u));
                                    cost_in_usd_ticks = u.cost_in_usd_ticks;
                                }

                            if event_type == "response.incomplete" {
                                let reason = response
                                    .get("incomplete_details")
                                    .and_then(|d| d.get("reason"))
                                    .and_then(|v| v.as_str());
                                final_finish_reason = FinishReason {
                                    unified: reason
                                        .map(map_xai_responses_finish_reason)
                                        .unwrap_or(FinishReasonUnified::Other),
                                    raw: reason.map(std::string::ToString::to_string).or(Some("incomplete".to_string())),
                                };
                            } else if let Some(status) = response.get("status").and_then(|v| v.as_str()) {
                                final_finish_reason = FinishReason {
                                    unified: if status == "completed" && has_function_call {
                                        FinishReasonUnified::ToolCalls
                                    } else {
                                        map_xai_responses_finish_reason(status)
                                    },
                                    raw: Some(status.to_string()),
                                };
                            }
                            if (event_type == "response.done" || event_type == "response.completed")
                                && (!active_reasoning.is_empty() || !ongoing_tool_calls.is_empty())
                            {
                                yield Err(AiMuxError::InvalidResponseData(
                                    "xAI Responses stream completed with unfinished output".to_string(),
                                ));
                                return;
                            }
                            terminal_received = true;
                            break;
                        }

                        // ── custom_tool_call_input.delta / .done ──
                        if event_type == "response.custom_tool_call_input.delta"
                            || event_type == "response.custom_tool_call_input.done"
                        {
                            continue;
                        }

                        // ── function_call_arguments.delta ──
                        if event_type == "response.function_call_arguments.delta" {
                            let output_index = parsed.get("output_index").and_then(serde_json::Value::as_u64).unwrap_or(0);
                            let delta = parsed.get("delta").and_then(|v| v.as_str()).unwrap_or("");
                            if let Some((tool_call_id, _, _, arguments, saw_delta)) = ongoing_tool_calls.get_mut(&output_index) {
                                arguments.push_str(delta);
                                *saw_delta = true;
                                yield Ok(StreamPart::ToolInputDelta {
                                    id: tool_call_id.clone(),
                                    delta: delta.to_string(),
                                    provider_metadata: None,
                                });
                            } else {
                                yield Err(AiMuxError::InvalidResponseData(
                                    "xAI Responses arguments delta has no function call".to_string(),
                                ));
                                return;
                            }
                            continue;
                        }

                        // ── function_call_arguments.done ──
                        if event_type == "response.function_call_arguments.done" {
                            continue;
                        }

                        // ── output_item.added / output_item.done ──
                        if event_type == "response.output_item.added"
                            || event_type == "response.output_item.done"
                        {
                            let item = parsed.get("item").cloned().unwrap_or(Value::Null);
                            let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            let output_index = parsed.get("output_index").and_then(serde_json::Value::as_u64).unwrap_or(0);

                            // ── reasoning item ──
                            if item_type == "reasoning" {
                                if event_type == "response.output_item.done" {
                                    let part_id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
                                    let block_id = format!("reasoning-{part_id}");
                                    let encrypted = item.get("encrypted_content").and_then(|v| v.as_str());

                                    if !active_reasoning.contains_key(part_id) {
                                        active_reasoning.insert(part_id.to_string(), ());
                                        yield Ok(StreamPart::ReasoningStart {
                                            id: block_id.clone(),
                                            provider_metadata: Some(json!({ "xai": { "itemId": part_id } })),
                                        });
                                    }

                                    let mut meta = json!({ "itemId": part_id });
                                    if let Some(ec) = encrypted {
                                        meta["reasoningEncryptedContent"] = json!(ec);
                                    }
                                    yield Ok(StreamPart::ReasoningEnd {
                                        id: block_id,
                                        provider_metadata: Some(json!({ "xai": meta })),
                                    });
                                    active_reasoning.remove(part_id);
                                }
                                continue;
                            }

                            // ── file_search_call item ──
                            if item_type == "file_search_call" {
                                let tool_name = resolve_tool_name(item_type, None, &provider_tool_names);
                                let part_id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");

                                if !seen_tool_calls.contains(part_id) {
                                    seen_tool_calls.insert(part_id.to_string());
                                    yield Ok(StreamPart::ToolInputStart {
                                        id: part_id.to_string(),
                                        tool_name: tool_name.clone(),
                                        provider_executed: Some(true),
                                        dynamic: None,
                                        title: None,
                                        provider_metadata: None,
                                    });
                                    yield Ok(StreamPart::ToolInputDelta {
                                        id: part_id.to_string(),
                                        delta: String::new(),
                                        provider_metadata: None,
                                    });
                                    yield Ok(StreamPart::ToolInputEnd {
                                        id: part_id.to_string(),
                                        provider_metadata: None,
                                    });
                                    yield Ok(StreamPart::ToolCall {
                                        tool_call_id: part_id.to_string(),
                                        tool_name: tool_name.clone(),
                                        input: Value::String(String::new()),
                                        provider_executed: Some(true),
                                        dynamic: None,
                                        thought_signature: None,
                                        invalid: None,
                                        error: None,
                                        provider_metadata: None,
                                    });
                                }

                                if event_type == "response.output_item.done" {
                                    let queries = item.get("queries").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                                    let results = item.get("results").and_then(|v| v.as_array()).map(|arr| {
                                        json!(arr.iter().map(|r| json!({
                                            "fileId": r.get("file_id").cloned().unwrap_or(Value::Null),
                                            "filename": r.get("filename").cloned().unwrap_or(Value::Null),
                                            "score": r.get("score").cloned().unwrap_or(Value::Null),
                                            "text": r.get("text").cloned().unwrap_or(Value::Null),
                                        })).collect::<Vec<_>>())
                                    }).unwrap_or(Value::Null);
                                    yield Ok(StreamPart::ToolResult {
                                        tool_call_id: part_id.to_string(),
                                        tool_name,
                                        result: json!({ "queries": queries, "results": results }),
                                        is_error: None,
                                        preliminary: None,
                                        dynamic: None,
                                        provider_metadata: None,
                                    });
                                }
                                continue;
                            }

                            // ── Server-side tool calls ──
                            if matches!(
                                item_type,
                                "web_search_call" | "x_search_call" | "code_interpreter_call"
                                    | "code_execution_call" | "view_image_call" | "view_x_video_call"
                                    | "custom_tool_call" | "mcp_call"
                            ) {
                                let item_name = item.get("name").and_then(|v| v.as_str());
                                let tool_name = resolve_tool_name(item_type, item_name, &provider_tool_names);
                                let tool_input = get_tool_input(item_type, &item);
                                let part_id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");

                                let should_emit = if item_type == "custom_tool_call" {
                                    event_type == "response.output_item.done"
                                } else {
                                    !seen_tool_calls.contains(part_id)
                                };

                                if should_emit && !seen_tool_calls.contains(part_id) {
                                    seen_tool_calls.insert(part_id.to_string());
                                    yield Ok(StreamPart::ToolInputStart {
                                        id: part_id.to_string(),
                                        tool_name: tool_name.clone(),
                                        provider_executed: Some(true),
                                        dynamic: None,
                                        title: None,
                                        provider_metadata: None,
                                    });
                                    yield Ok(StreamPart::ToolInputDelta {
                                        id: part_id.to_string(),
                                        delta: tool_input.clone(),
                                        provider_metadata: None,
                                    });
                                    yield Ok(StreamPart::ToolInputEnd {
                                        id: part_id.to_string(),
                                        provider_metadata: None,
                                    });
                                    yield Ok(StreamPart::ToolCall {
                                        tool_call_id: part_id.to_string(),
                                        tool_name: tool_name.clone(),
                                        input: Value::String(tool_input),
                                        provider_executed: Some(true),
                                        dynamic: None,
                                        thought_signature: None,
                                        invalid: None,
                                        error: None,
                                        provider_metadata: None,
                                    });
                                }

                                if event_type == "response.output_item.done" {
                                    yield Ok(StreamPart::ToolResult {
                                        tool_call_id: part_id.to_string(),
                                        tool_name,
                                        result: json!({}),
                                        is_error: None,
                                        preliminary: None,
                                        dynamic: None,
                                        provider_metadata: None,
                                    });
                                }
                                continue;
                            }

                            // ── message item ──
                            if item_type == "message" {
                                let content_parts = item
                                    .get("content")
                                    .and_then(|v| v.as_array())
                                    .cloned()
                                    .unwrap_or_default();
                                for (content_index, cp) in content_parts.iter().enumerate() {
                                    if let Some(text) = cp.get("text").and_then(|v| v.as_str())
                                        && !text.is_empty() {
                                            let part_id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
                                            let block_id = format!("text-{part_id}:{content_index}");
                                            if !content_blocks.contains_key(&block_id) {
                                                content_blocks.insert(block_id.clone(), false);
                                                yield Ok(StreamPart::TextStart { id: block_id.clone(), provider_metadata: None});
                                                yield Ok(StreamPart::TextDelta {
                                                    id: block_id,
                                                    delta: text.to_string(),
                                                    provider_metadata: None,
                                                });
                                            }
                                        }
                                    if let Some(annotations) = cp.get("annotations").and_then(|v| v.as_array()) {
                                        for ann in annotations {
                                            if ann.get("type").and_then(|v| v.as_str()) == Some("url_citation") {
                                                let url = ann.get("url").and_then(|v| v.as_str()).unwrap_or("");
                                                let title = ann.get("title").and_then(|v| v.as_str()).unwrap_or(url).to_string();
                                                yield Ok(StreamPart::Source {
                                                    id: generate_source_id(),
                                                    source_type: "url".to_string(),
                                                    url: Some(url.to_string()),
                                                    title: Some(title),
                                                    provider_metadata: None,
                                                });
                                            }
                                        }
                                    }
                                }
                                continue;
                            }

                            // ── function_call item ──
                            if item_type == "function_call" {
                                let item_id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
                                let call_id = item.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                                let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("");

                                if event_type == "response.output_item.added" {
                                    if item_id.is_empty() || call_id.is_empty() || name.is_empty()
                                        || ongoing_tool_calls.values().any(|(seen, _, seen_item, _, _)| seen == call_id || seen_item == item_id)
                                        || ongoing_tool_calls.insert(output_index, (
                                            call_id.to_string(), name.to_string(), item_id.to_string(), String::new(), false,
                                        )).is_some()
                                    {
                                        yield Err(AiMuxError::InvalidResponseData(
                                            "xAI Responses function call has a missing or duplicate identity".to_string(),
                                        ));
                                        return;
                                    }
                                    yield Ok(StreamPart::ToolInputStart {
                                        id: call_id.to_string(),
                                        tool_name: name.to_string(),
                                        provider_executed: None,
                                        dynamic: None,
                                        title: None,
                                        provider_metadata: None,
                                    });
                                } else {
                                    // output_item.done
                                    let Some((started_call_id, started_name, started_item_id, streamed_arguments, saw_delta)) =
                                        ongoing_tool_calls.remove(&output_index)
                                    else {
                                        yield Err(AiMuxError::InvalidResponseData(
                                            "xAI Responses completed an unknown function call".to_string(),
                                        ));
                                        return;
                                    };
                                    let arguments = item.get("arguments").and_then(|v| v.as_str()).unwrap_or("");
                                    if call_id != started_call_id || name != started_name || item_id != started_item_id
                                        || (saw_delta && arguments != streamed_arguments)
                                    {
                                        yield Err(AiMuxError::InvalidResponseData(
                                            "xAI Responses function call changed while streaming".to_string(),
                                        ));
                                        return;
                                    }
                                    has_function_call = true;
                                    yield Ok(StreamPart::ToolInputEnd {
                                        id: call_id.to_string(),
                                        provider_metadata: None,
                                    });
                                    let input = Value::String(arguments.to_string());
                                    yield Ok(StreamPart::ToolCall {
                                        tool_call_id: call_id.to_string(),
                                        tool_name: name.to_string(),
                                        input,
                                        provider_executed: None,
                                        dynamic: None,
                                        thought_signature: None,
                                        invalid: None,
                                        error: None,
                                        provider_metadata: Some(json!({ "xai": { "itemId": item_id } })),
                                    });
                                }
                                continue;
                            }
                        }

                        // All other event types (web_search_call.in_progress, etc.) are ignored.
                    }
                    Err(error) => {
                        let recoverable = error.is_recoverable_stream_error();
                        yield Err(error);
                        if !recoverable {
                            return;
                        }
                    }
                }
            }

            if !terminal_received {
                yield Err(AiMuxError::InvalidResponseData(
                    "xAI Responses stream ended without a terminal event".to_string(),
                ));
                return;
            }

            // Close any remaining open text blocks.
            for (block_id, ended) in &content_blocks {
                if !ended {
                    yield Ok(StreamPart::TextEnd { id: block_id.clone(), provider_metadata: None});
                }
            }

            // Final part: Finish.
            let provider_meta = cost_in_usd_ticks.map(|cost| json!({ "xai": { "costInUsdTicks": cost } }));

            yield Ok(StreamPart::Finish {
                finish_reason: final_finish_reason,
                usage: final_usage.unwrap_or_default(),
                provider_metadata: provider_meta,
            });
        };

        Ok(StreamResult {
            stream: Box::pin(stream),
            request_body: Some(body),
            response_headers: Some(response_headers),
        })
    }
}
