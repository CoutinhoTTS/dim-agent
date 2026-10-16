use super::ProviderModel;
use dim_agent_capabilities::host::ModelProtocol;
use dim_agent_types::*;
use serde::Deserialize;

/// Preferences applied to the caller's chosen model configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelPreferences {
    #[serde(default, deserialize_with = "preference")]
    pub reasoning_effort: Option<String>,
    #[serde(default, deserialize_with = "preference")]
    pub service_tier: Option<String>,
}

fn preference<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    String::deserialize(d).map(Some)
}

impl ModelPreferences {
    pub(super) fn configure(
        &self,
        model: &ProviderModel,
        protocol: ModelProtocol,
        base: &ModelConfig,
    ) -> Result<ModelConfig, &'static str> {
        if protocol == ModelProtocol::Bedrock && *self != Self::default() {
            return Err("Model preferences are not supported by Bedrock");
        }
        let mut config = base.clone();
        if let Some(effort) = &self.reasoning_effort {
            let reasoning = model
                .metadata
                .as_ref()
                .and_then(|m| m.reasoning.as_ref())
                .filter(|r| r.supported && model.capabilities.reasoning)
                .ok_or("Model does not support reasoning preferences")?;
            let native = reasoning
                .effort_options
                .as_ref()
                .is_some_and(|options| options.contains(effort));
            // Effort replaces the mutually exclusive enable/budget mode.
            if let Some(previous) = &mut config.reasoning {
                previous.effort = None;
                previous.enabled = None;
                previous.budget_tokens = None;
            }
            if let Some(options) = &mut config.provider_options {
                match protocol {
                    ModelProtocol::Anthropic => {
                        if let Some(options) = &mut options.anthropic {
                            options.effort = None;
                            options.thinking = None;
                        }
                    }
                    ModelProtocol::OpenaiCompatible => {
                        if let Some(options) = &mut options.openai_compatible {
                            options.reasoning_effort = None;
                            options.thinking = None;
                            options.enable_thinking = None;
                        }
                    }
                    ModelProtocol::Gemini => {
                        if let Some(thinking) = options
                            .gemini
                            .as_mut()
                            .and_then(|options| options.thinking_config.as_mut())
                        {
                            thinking.thinking_budget = None;
                        }
                    }
                    ModelProtocol::OpenaiChat | ModelProtocol::OpenaiResponses => {
                        if let Some(options) = &mut options.openai {
                            options.reasoning_effort = None;
                        }
                    }
                    ModelProtocol::Bedrock => unreachable!("preferences reject Bedrock"),
                    ModelProtocol::Xai | ModelProtocol::XaiResponses => {
                        if let Some(options) = &mut options.xai {
                            options.reasoning_effort = None;
                        }
                    }
                }
            }
            if effort == "none" {
                match (&reasoning.off, protocol) {
                    (Some(ReasoningOff::ThinkingType { value }), ModelProtocol::Anthropic)
                        if value == "disabled" =>
                    {
                        config
                            .provider_options
                            .get_or_insert_default()
                            .anthropic
                            .get_or_insert_default()
                            .thinking = Some(AnthropicThinking {
                            thinking_type: Some(AnthropicThinkingType::Disabled),
                            ..Default::default()
                        });
                    }
                    (
                        Some(ReasoningOff::ThinkingType { value }),
                        ModelProtocol::OpenaiCompatible,
                    ) if value == "disabled" => {
                        config
                            .provider_options
                            .get_or_insert_default()
                            .openai_compatible
                            .get_or_insert_default()
                            .thinking = Some(CompatibleThinking {
                            thinking_type: Some(ThinkingType::Disabled),
                        });
                    }
                    (Some(ReasoningOff::ThinkingBudget { value: 0 }), ModelProtocol::Gemini) => {
                        config
                            .provider_options
                            .get_or_insert_default()
                            .gemini
                            .get_or_insert_default()
                            .thinking_config
                            .get_or_insert_default()
                            .thinking_budget = Some(0.);
                    }
                    (
                        Some(ReasoningOff::EnableThinking { value: false }),
                        ModelProtocol::OpenaiCompatible,
                    ) => {
                        config
                            .provider_options
                            .get_or_insert_default()
                            .openai_compatible
                            .get_or_insert_default()
                            .enable_thinking = Some(false);
                    }
                    (None, _) if native => {
                        config.reasoning.get_or_insert_default().effort = Some(effort.clone());
                    }
                    _ => {
                        return Err(
                            "Model reasoning-off declaration is not supported by this protocol",
                        );
                    }
                }
            } else if native && !effort.is_empty() {
                config.reasoning.get_or_insert_default().effort = Some(effort.clone());
            } else {
                return Err("Reasoning effort is not supported by this model");
            }
        }
        if let Some(tier) = &self.service_tier {
            let supported = model
                .metadata
                .as_ref()
                .and_then(|m| m.service_tier.as_ref())
                .is_some_and(|s| s.supported && !tier.is_empty() && s.options.contains(tier));
            if !supported {
                return Err("Service tier is not supported by this model");
            }
            let options = config.provider_options.get_or_insert_default();
            match protocol {
                ModelProtocol::OpenaiChat | ModelProtocol::OpenaiResponses => {
                    options.openai.get_or_insert_default().service_tier = Some(tier.clone());
                }
                ModelProtocol::OpenaiCompatible => {
                    options
                        .openai_compatible
                        .get_or_insert_default()
                        .service_tier = Some(tier.clone());
                }
                _ => return Err("Service tier is not supported by this protocol"),
            }
        }
        Ok(config)
    }
}
