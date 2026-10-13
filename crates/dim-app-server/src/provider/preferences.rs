use super::ProviderModel;
use dim_agent_capabilities::host::ModelProtocol;
use dim_agent_types::*;
use serde::Deserialize;

/// A complete choice for one adoption, not a patch to the previous model.
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
    ) -> Result<ModelConfig, &'static str> {
        let mut config = model.config.clone();
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
            if effort == "none" {
                match (&reasoning.off, protocol) {
                    (Some(ReasoningOff::ThinkingType { value }), ModelProtocol::Anthropic)
                        if value == "disabled" =>
                    {
                        config.provider_options.get_or_insert_default().anthropic =
                            Some(AnthropicProviderOptions {
                                thinking: Some(AnthropicThinking {
                                    thinking_type: Some(AnthropicThinkingType::Disabled),
                                    ..Default::default()
                                }),
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
                            .openai_compatible = Some(OpenAiCompatibleProviderOptions {
                            thinking: Some(CompatibleThinking {
                                thinking_type: Some(ThinkingType::Disabled),
                            }),
                            ..Default::default()
                        });
                    }
                    (Some(ReasoningOff::ThinkingBudget { value: 0 }), ModelProtocol::Gemini) => {
                        config.provider_options.get_or_insert_default().gemini =
                            Some(GeminiProviderOptions {
                                thinking_config: Some(GeminiThinkingConfig {
                                    thinking_budget: Some(0.),
                                    ..Default::default()
                                }),
                            });
                    }
                    (
                        Some(ReasoningOff::EnableThinking { value: false }),
                        ModelProtocol::OpenaiCompatible,
                    ) => {
                        config
                            .provider_options
                            .get_or_insert_default()
                            .openai_compatible = Some(OpenAiCompatibleProviderOptions {
                            enable_thinking: Some(false),
                            ..Default::default()
                        });
                    }
                    (None, _) if native => {
                        config.reasoning = Some(ProviderReasoningConfig {
                            effort: Some(effort.clone()),
                            ..Default::default()
                        });
                    }
                    _ => {
                        return Err(
                            "Model reasoning-off declaration is not supported by this protocol",
                        );
                    }
                }
            } else if native && !effort.is_empty() {
                config.reasoning = Some(ProviderReasoningConfig {
                    effort: Some(effort.clone()),
                    ..Default::default()
                });
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
