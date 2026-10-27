use super::error;
use crate::config::ProviderKind;
use aimux_core::options::CallOptions;
use dim_agent_capabilities::host::HostError;
use dim_agent_types::{AnthropicThinkingType, ModelConfig, ModelRequest};
use serde_json::{Map, Value, json};
use std::collections::HashMap;

pub(super) fn apply(
    request: &ModelRequest,
    config: &ModelConfig,
    kind: &ProviderKind,
    options: &mut CallOptions,
) -> Result<(), HostError> {
    let generic = config.reasoning.as_ref();
    let providers = config.provider_options.as_ref();
    let mut fields = Map::new();
    let mut thinking = Map::new();
    if let Some(reasoning) = generic {
        if reasoning.effort.as_deref() == Some("none")
            && (reasoning.enabled.is_some()
                || reasoning.budget_tokens.is_some_and(|budget| budget != 0.0))
        {
            return Err(error(
                request,
                "config.reasoning",
                "conflicting reasoning enable and disable settings",
            ));
        }
        for (field, unsupported) in [
            (
                "budgetTokens",
                reasoning.budget_tokens.is_some()
                    && matches!(
                        kind,
                        ProviderKind::OpenaiResponses
                            | ProviderKind::Xai
                            | ProviderKind::XaiResponses
                    ),
            ),
            (
                "summary",
                reasoning.summary.is_some()
                    && !matches!(
                        kind,
                        ProviderKind::OpenaiResponses | ProviderKind::XaiResponses
                    ),
            ),
            (
                "includeThoughts",
                reasoning.include_thoughts.is_some() && *kind != ProviderKind::Gemini,
            ),
            (
                "interleavedThinking",
                reasoning.interleaved_thinking.is_some(),
            ),
        ] {
            if unsupported {
                return Err(error(
                    request,
                    &format!("config.reasoning.{field}"),
                    "not supported by this protocol",
                ));
            }
        }
        if *kind == ProviderKind::Bedrock
            && reasoning
                .effort
                .as_deref()
                .is_some_and(|effort| effort != "provider-default")
        {
            return Err(error(
                request,
                "config.reasoning.effort",
                "not supported by Bedrock",
            ));
        }
        if let Some(effort) = reasoning
            .effort
            .as_deref()
            .filter(|effort| *effort != "provider-default")
        {
            if effort == "none" && *kind == ProviderKind::Anthropic {
                thinking.insert("type".into(), json!("disabled"));
            } else if effort == "none" && *kind == ProviderKind::Gemini {
                thinking.insert("thinkingBudget".into(), json!(0));
            } else {
                let field = match kind {
                    ProviderKind::OpenaiResponses
                    | ProviderKind::Xai
                    | ProviderKind::XaiResponses => "reasoningEffort",
                    ProviderKind::Anthropic => "effort",
                    ProviderKind::Gemini => "thinkingLevel",
                    ProviderKind::Bedrock => unreachable!(),
                    _ => unreachable!(),
                };
                if *kind == ProviderKind::Gemini {
                    thinking.insert(field.into(), json!(effort));
                } else {
                    fields.insert(field.into(), json!(effort));
                }
            }
        }
        let budget_overridden = match kind {
            ProviderKind::Anthropic => providers
                .and_then(|p| p.anthropic.as_ref())
                .and_then(|p| p.thinking.as_ref())
                .is_some_and(|p| {
                    p.budget_tokens.is_some()
                        || matches!(
                            p.thinking_type,
                            Some(AnthropicThinkingType::Disabled | AnthropicThinkingType::Adaptive)
                        )
                }),
            ProviderKind::Gemini => providers
                .and_then(|p| p.gemini.as_ref())
                .and_then(|p| p.thinking_config.as_ref())
                .is_some_and(|p| p.thinking_budget.is_some()),
            _ => false,
        };
        if let Some(budget) = reasoning.budget_tokens.filter(|_| !budget_overridden) {
            let minimum = if *kind == ProviderKind::Gemini {
                -1.0
            } else {
                1024.0
            };
            let budget = budget_integer(request, "config.reasoning.budgetTokens", budget, minimum)?;
            thinking.insert(
                if *kind == ProviderKind::Gemini {
                    "thinkingBudget"
                } else {
                    "budgetTokens"
                }
                .into(),
                json!(budget),
            );
        }
        if let Some(summary) = reasoning.summary {
            fields.insert("reasoningSummary".into(), json!(summary));
        }
        if let Some(include) = reasoning.include_thoughts {
            thinking.insert("includeThoughts".into(), json!(include));
        }
        if *kind == ProviderKind::Anthropic
            && (reasoning.enabled.is_some() || reasoning.budget_tokens.is_some())
        {
            thinking.insert("type".into(), json!("enabled"));
        }
    }
    match kind {
        ProviderKind::Xai | ProviderKind::XaiResponses => {
            if config.top_k.is_some() {
                return Err(error(request, "config.topK", "not supported by xAI"));
            }
            if let Some(xai) = providers.and_then(|p| p.xai.as_ref()) {
                if xai.interleaved_thinking.is_some() {
                    return Err(error(
                        request,
                        "config.providerOptions.xai.interleavedThinking",
                        "not supported by xAI",
                    ));
                }
                if let Some(effort) = xai.reasoning_effort.as_deref() {
                    fields.remove("reasoningEffort");
                    if effort != "provider-default" {
                        fields.insert("reasoningEffort".into(), json!(effort));
                    }
                }
            }
            if providers.is_some_and(|p| {
                p.openai.as_ref().is_some_and(|p| p.service_tier.is_some())
                    || p.openai_compatible
                        .as_ref()
                        .is_some_and(|p| p.service_tier.is_some())
            }) {
                return Err(error(
                    request,
                    "config.providerOptions",
                    "service tier is not supported by xAI",
                ));
            }
            if generic.is_some_and(|r| r.enabled.is_some())
                && !fields
                    .get("reasoningEffort")
                    .is_some_and(|v| v.as_str().is_some_and(|e| !e.is_empty()))
            {
                return Err(error(
                    request,
                    "config.reasoning.enabled",
                    "requires a concrete reasoning effort",
                ));
            }
            if *kind == ProviderKind::XaiResponses {
                fields.insert("store".into(), json!(false));
            }
        }
        ProviderKind::OpenaiResponses => {
            if let Some(openai) = providers.and_then(|p| p.openai.as_ref()) {
                if openai.interleaved_thinking.is_some() {
                    return Err(error(
                        request,
                        "config.providerOptions.openai.interleavedThinking",
                        "not supported by Responses",
                    ));
                }
                if let Some(effort) = openai.reasoning_effort.as_deref() {
                    fields.remove("reasoningEffort");
                    if effort != "provider-default" {
                        fields.insert("reasoningEffort".into(), json!(effort));
                    }
                }
                if let Some(summary) = openai.summary {
                    fields.insert("reasoningSummary".into(), json!(summary));
                }
                if let Some(tier) = &openai.service_tier {
                    fields.insert("serviceTier".into(), json!(tier));
                }
            }
            if generic.is_some_and(|r| r.enabled.is_some())
                && !providers
                    .and_then(|p| p.openai.as_ref())
                    .is_some_and(|p| p.reasoning_effort.as_deref() == Some("none"))
                && !fields
                    .get("reasoningEffort")
                    .is_some_and(|v| v.as_str().is_some_and(|e| e != "none"))
            {
                return Err(error(
                    request,
                    "config.reasoning.enabled",
                    "requires a concrete reasoning effort",
                ));
            }
            fields.insert("store".into(), json!(false));
            fields.insert("include".into(), json!(["reasoning.encrypted_content"]));
        }
        ProviderKind::Anthropic => {
            if let Some(anthropic) = providers.and_then(|p| p.anthropic.as_ref()) {
                if anthropic.interleaved_thinking.is_some() || anthropic.send_reasoning.is_some() {
                    return Err(error(
                        request,
                        "config.providerOptions.anthropic",
                        "sendReasoning and interleavedThinking are not supported by the native adapter",
                    ));
                }
                if let Some(effort) = anthropic.effort.as_deref() {
                    fields.remove("effort");
                    if effort != "provider-default" {
                        fields.insert("effort".into(), json!(effort));
                    }
                }
                if let Some(native) = &anthropic.thinking {
                    if native.thinking_type.is_none() {
                        return Err(error(
                            request,
                            "config.providerOptions.anthropic.thinking.type",
                            "required when thinking is set",
                        ));
                    }
                    if let Some(kind) = native.thinking_type {
                        thinking.insert("type".into(), json!(kind));
                    }
                    if let Some(budget) = native.budget_tokens {
                        thinking.insert(
                            "budgetTokens".into(),
                            json!(budget_integer(
                                request,
                                "config.providerOptions.anthropic.thinking.budgetTokens",
                                budget,
                                1024.0
                            )?),
                        );
                    }
                    if matches!(
                        native.thinking_type,
                        Some(AnthropicThinkingType::Disabled | AnthropicThinkingType::Adaptive)
                    ) {
                        thinking.remove("budgetTokens");
                    }
                    if matches!(
                        native.thinking_type,
                        Some(AnthropicThinkingType::Disabled | AnthropicThinkingType::Adaptive)
                    ) && native.budget_tokens.is_some()
                    {
                        return Err(error(
                            request,
                            "config.providerOptions.anthropic.thinking",
                            "disabled or adaptive thinking cannot have an explicit budget",
                        ));
                    }
                }
            }
            if !thinking.is_empty() {
                fields.insert("thinking".into(), Value::Object(thinking));
            }
        }
        ProviderKind::Gemini => {
            if let Some(native) = providers
                .and_then(|p| p.gemini.as_ref())
                .and_then(|p| p.thinking_config.as_ref())
            {
                if let Some(budget) = native.thinking_budget {
                    thinking.remove("thinkingLevel");
                    thinking.insert(
                        "thinkingBudget".into(),
                        json!(budget_integer(
                            request,
                            "config.providerOptions.gemini.thinkingConfig.thinkingBudget",
                            budget,
                            -1.0
                        )?),
                    );
                }
                if let Some(include) = native.include_thoughts {
                    thinking.insert("includeThoughts".into(), json!(include));
                }
            }
            if generic.is_some_and(|reasoning| reasoning.enabled.is_some())
                && thinking.get("thinkingBudget").and_then(Value::as_i64) == Some(0)
                && providers
                    .and_then(|p| p.gemini.as_ref())
                    .and_then(|p| p.thinking_config.as_ref())
                    .is_none_or(|p| p.thinking_budget.is_none())
            {
                return Err(error(
                    request,
                    "config.reasoning",
                    "conflicting reasoning enable and disable settings",
                ));
            }
            if thinking.contains_key("thinkingBudget") && thinking.contains_key("thinkingLevel") {
                return Err(error(
                    request,
                    "config.reasoning",
                    "Gemini cannot combine a thinking level and budget",
                ));
            }
            if generic.is_some_and(|r| r.enabled.is_some())
                && !thinking.contains_key("thinkingBudget")
                && !thinking.contains_key("thinkingLevel")
            {
                return Err(error(
                    request,
                    "config.reasoning.enabled",
                    "requires a thinking budget or level",
                ));
            }
            if !thinking.is_empty() {
                fields.insert("thinkingConfig".into(), Value::Object(thinking));
            }
        }
        ProviderKind::Bedrock => {
            if config.top_k.is_some() {
                return Err(error(request, "config.topK", "not supported by Bedrock"));
            }
            if providers.is_some() {
                return Err(error(
                    request,
                    "config.providerOptions",
                    "provider-specific options do not match Bedrock",
                ));
            }
            if generic.is_some_and(|reasoning| reasoning.enabled.is_some())
                && !thinking.contains_key("budgetTokens")
            {
                return Err(error(
                    request,
                    "config.reasoning.enabled",
                    "requires a reasoning budget for Bedrock",
                ));
            }
            if let Some(budget) = thinking.remove("budgetTokens") {
                fields.insert(
                    "reasoningConfig".into(),
                    json!({"type":"enabled", "budgetTokens":budget}),
                );
            }
        }
        _ => unreachable!(),
    }
    let namespace = match kind {
        ProviderKind::Xai | ProviderKind::XaiResponses => "xai",
        ProviderKind::OpenaiResponses => "openai",
        ProviderKind::Anthropic => "anthropic",
        ProviderKind::Gemini => "google",
        ProviderKind::Bedrock => "bedrock",
        _ => unreachable!(),
    };
    if !fields.is_empty() {
        options.provider_options = Some(HashMap::from([(namespace.into(), Value::Object(fields))]));
    }
    Ok(())
}

fn budget_integer(
    request: &ModelRequest,
    path: &str,
    value: f64,
    minimum: f64,
) -> Result<i32, HostError> {
    if !value.is_finite()
        || value.fract() != 0.0
        || !(minimum..=f64::from(i32::MAX)).contains(&value)
    {
        return Err(error(request, path, "invalid integer reasoning budget"));
    }
    Ok(value as i32)
}
