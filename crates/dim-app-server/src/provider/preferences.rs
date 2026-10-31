use dim_agent_capabilities::host::{ModelProtocol, reasoning_config};
use dim_agent_types::*;

/// Applies preferences to a copy of `base`, validating them against the
/// model's declared capabilities.
pub(super) fn configure(
    preferences: &ModelPreferences,
    snapshot: &ModelCapabilitySnapshot,
    protocol: ModelProtocol,
    base: &ModelConfig,
) -> Result<ModelConfig, &'static str> {
    if protocol == ModelProtocol::Bedrock && *preferences != ModelPreferences::default() {
        return Err("Model preferences are not supported by Bedrock");
    }
    let mut config = match &preferences.reasoning_effort {
        Some(effort) => reasoning_config(base, snapshot, protocol, effort)?,
        None => base.clone(),
    };
    if let Some(tier) = &preferences.service_tier {
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
