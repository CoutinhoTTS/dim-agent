use dim_agent_capabilities::host::{ModelProtocol, reasoning_config};
use dim_agent_types::*;
use serde::{Deserialize, Serialize};

/// Preferences applied to the caller's chosen model configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelPreferences {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "preference"
    )]
    pub reasoning_effort: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "preference"
    )]
    pub service_tier: Option<String>,
}

fn preference<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    String::deserialize(d).map(Some)
}

impl ModelPreferences {
    pub(super) fn configure(
        &self,
        snapshot: &ModelCapabilitySnapshot,
        protocol: ModelProtocol,
        base: &ModelConfig,
    ) -> Result<ModelConfig, &'static str> {
        if protocol == ModelProtocol::Bedrock && *self != Self::default() {
            return Err("Model preferences are not supported by Bedrock");
        }
        let mut config = match &self.reasoning_effort {
            Some(effort) => reasoning_config(base, snapshot, protocol, effort)?,
            None => base.clone(),
        };
        if let Some(tier) = &self.service_tier {
            let supported = snapshot
                .service_tier
                .as_ref()
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
