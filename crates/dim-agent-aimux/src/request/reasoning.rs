use super::error;
use crate::config::ProviderKind;
use aimux_core::options::CallOptions;
use dim_agent_capabilities::host::HostError;
use dim_agent_types::{ModelConfig, ModelRequest, ThinkingType};
use serde_json::{Map, Value, json};
use std::collections::HashMap;

#[derive(Default)]
struct Controls<'a> {
    path: &'a str,
    effort: Option<&'a str>,
    enabled: bool,
    thinking: Option<ThinkingType>,
    enable_thinking: Option<bool>,
}

pub(super) fn apply(
    request: &ModelRequest,
    config: &ModelConfig,
    kind: &ProviderKind,
    preset: Option<&str>,
    options: &mut CallOptions,
) -> Result<(), HostError> {
    if !matches!(
        kind,
        ProviderKind::OpenaiChat | ProviderKind::OpenaiCompatible
    ) {
        return super::native::apply(request, config, kind, options);
    }
    // The array order is also the field and intent precedence.
    let mut levels = [
        Controls::default(),
        Controls::default(),
        Controls::default(),
    ];
    if let Some(reasoning) = &config.reasoning {
        for (field, present) in [
            ("budgetTokens", reasoning.budget_tokens.is_some()),
            ("summary", reasoning.summary.is_some()),
            ("includeThoughts", reasoning.include_thoughts.is_some()),
            (
                "interleavedThinking",
                reasoning.interleaved_thinking.is_some(),
            ),
        ] {
            if present {
                return Err(error(
                    request,
                    &format!("config.reasoning.{field}"),
                    "not supported by this Chat integration",
                ));
            }
        }
        levels[2] = Controls {
            path: "config.reasoning",
            effort: reasoning.effort.as_deref(),
            enabled: reasoning.enabled.is_some(),
            ..Controls::default()
        };
    }

    let mut service_tier = None;
    if let Some(provider) = &config.provider_options {
        match kind {
            ProviderKind::OpenaiChat => {
                if let Some(openai) = &provider.openai {
                    let path = "config.providerOptions.openai";
                    for (field, present) in [
                        ("summary", openai.summary.is_some()),
                        ("interleavedThinking", openai.interleaved_thinking.is_some()),
                    ] {
                        if present {
                            return Err(error(
                                request,
                                &format!("{path}.{field}"),
                                "not supported by this Chat integration",
                            ));
                        }
                    }
                    levels[1] = Controls {
                        path,
                        effort: openai.reasoning_effort.as_deref(),
                        ..Controls::default()
                    };
                    service_tier = openai.service_tier.as_deref().map(|tier| (tier, path));
                }
            }
            ProviderKind::OpenaiCompatible => {
                if let Some(compatible) = &provider.openai_compatible {
                    let path = "config.providerOptions.openai-compatible";
                    if compatible.interleaved_thinking.is_some() {
                        return Err(error(
                            request,
                            &format!("{path}.interleavedThinking"),
                            "not supported by this Chat integration",
                        ));
                    }
                    let thinking = compatible
                        .thinking
                        .as_ref()
                        .map(|thinking| {
                            thinking.thinking_type.ok_or_else(|| {
                                error(
                                    request,
                                    &format!("{path}.thinking.type"),
                                    "required when thinking is set",
                                )
                            })
                        })
                        .transpose()?;
                    levels[1] = Controls {
                        path,
                        effort: compatible.reasoning_effort.as_deref(),
                        thinking,
                        enable_thinking: compatible.enable_thinking,
                        ..Controls::default()
                    };
                    service_tier = compatible.service_tier.as_deref().map(|tier| (tier, path));
                }
                let specific = match preset {
                    Some("deepseek") => Some((
                        provider.deepseek.as_ref(),
                        "config.providerOptions.deepseek",
                    )),
                    Some("moonshotai") => Some((
                        provider.moonshotai.as_ref(),
                        "config.providerOptions.moonshotai",
                    )),
                    Some("xai") => Some((provider.xai.as_ref(), "config.providerOptions.xai")),
                    Some("zenmux") => {
                        Some((provider.zenmux.as_ref(), "config.providerOptions.zenmux"))
                    }
                    Some("aihubmix") => Some((
                        provider.aihubmix.as_ref(),
                        "config.providerOptions.aihubmix",
                    )),
                    _ => None,
                };
                if let Some((Some(specific), path)) = specific {
                    if specific.interleaved_thinking.is_some() {
                        return Err(error(
                            request,
                            &format!("{path}.interleavedThinking"),
                            "not supported by this Chat integration",
                        ));
                    }
                    levels[0] = Controls {
                        path,
                        effort: specific.reasoning_effort.as_deref(),
                        ..Controls::default()
                    };
                }
            }
            _ => unreachable!(),
        }
    }

    let effort = levels
        .iter()
        .enumerate()
        .find_map(|(index, level)| level.effort.map(|effort| (index, effort)));
    let mut intents: [Option<bool>; 3] = [None; 3];
    for (index, level) in levels.iter().enumerate() {
        let effort_intent = effort
            .filter(|(rank, _)| *rank == index)
            .and_then(|(_, value)| match value {
                "provider-default" => None,
                "none" => Some(false),
                _ => Some(true),
            });
        for intent in [
            effort_intent,
            level.enabled.then_some(true),
            level.thinking.map(|kind| kind == ThinkingType::Enabled),
            level.enable_thinking,
        ]
        .into_iter()
        .flatten()
        {
            if intents[index].is_some_and(|existing| existing != intent) {
                return Err(error(
                    request,
                    level.path,
                    "conflicting reasoning enable and disable settings at the same priority",
                ));
            }
            intents[index] = Some(intent);
        }
    }
    let intent = intents.into_iter().flatten().next();
    let keep = |value: bool| intent.is_none_or(|intent| intent == value);
    let mut openai = Map::new();
    let mut enabled_parameter = false;
    if let Some((index, effort)) = effort {
        if effort.is_empty() {
            let field = if index == 2 {
                "effort"
            } else {
                "reasoningEffort"
            };
            return Err(error(
                request,
                &format!("{}.{field}", levels[index].path),
                "reasoning effort must not be empty",
            ));
        }
        if effort != "provider-default" && keep(effort != "none") {
            openai.insert("reasoningEffort".into(), Value::String(effort.to_owned()));
            enabled_parameter |= effort != "none";
        }
    }
    if let Some((tier, path)) = service_tier {
        if tier.is_empty() {
            return Err(error(
                request,
                &format!("{path}.serviceTier"),
                "must not be empty",
            ));
        }
        openai.insert("serviceTier".into(), Value::String(tier.to_owned()));
    }
    let mut overrides = Map::new();
    for level in &levels {
        if let Some(thinking) = level.thinking
            && keep(thinking == ThinkingType::Enabled)
        {
            overrides.insert("thinking".into(), json!({"type": thinking}));
            enabled_parameter |= thinking == ThinkingType::Enabled;
        }
        if let Some(enabled) = level.enable_thinking
            && keep(enabled)
        {
            overrides.insert("enable_thinking".into(), Value::Bool(enabled));
            enabled_parameter |= enabled;
        }
    }
    if levels[2].enabled && keep(true) && !enabled_parameter {
        return Err(error(
            request,
            "config.reasoning.enabled",
            "requires an explicit enabling effort or thinking setting",
        ));
    }
    if !openai.is_empty() {
        options.provider_options = Some(HashMap::from([("openai".into(), Value::Object(openai))]));
    }
    if !overrides.is_empty() {
        options.body_overrides = Some(Value::Object(overrides));
    }
    Ok(())
}
