//! Stateless vendor routing. Provider owns configuration; Aimux owns execution.
use super::new_api;
use crate::model_catalog::{ModelEntry, NewApiEndpoint};
use dim_agent_capabilities::host::{HostError, HttpModelConnection, ModelProtocol};
use std::collections::{BTreeMap, HashMap};

pub(super) fn supported(driver: &str) -> bool {
    driver == "new-api-router" || protocol(driver, "").is_some()
}

pub(super) fn routes(
    driver: &str,
    models: &[ModelEntry],
    configured: &BTreeMap<String, NewApiEndpoint>,
) -> Result<HashMap<String, ModelProtocol>, HostError> {
    if driver == "new-api-router" {
        return new_api::routes(models, configured);
    }
    Ok(models
        .iter()
        .filter_map(|model| {
            protocol(driver, &model.model_id).map(|protocol| (model.model_id.clone(), protocol))
        })
        .collect())
}

fn protocol(driver: &str, model_id: &str) -> Option<ModelProtocol> {
    Some(match driver {
        "aihubmix" if model_id.starts_with("anthropic/") => ModelProtocol::OpenaiResponses,
        "zenmux" if model_id.starts_with("anthropic/") => ModelProtocol::Anthropic,
        "openai-compatible" | "custom" | "xai-grok-build" | "deepseek" | "moonshotai"
        | "aihubmix" | "zenmux" => ModelProtocol::OpenaiCompatible,
        "openai-responses" | "openai-codex" | "aihubmix-responses" | "azure-openai" => {
            ModelProtocol::OpenaiResponses
        }
        "xai" => ModelProtocol::Xai,
        "xai-responses" => ModelProtocol::XaiResponses,
        "anthropic" => ModelProtocol::Anthropic,
        "gemini" => ModelProtocol::Gemini,
        "amazon-bedrock" => ModelProtocol::Bedrock,
        _ => return None,
    })
}

pub(super) fn connection(
    driver: &str,
    protocol: ModelProtocol,
    base_url: &str,
    mut headers: HashMap<String, String>,
) -> HttpModelConnection {
    let base = base_url.trim_end_matches('/');
    let base_url = if driver == "new-api-router" {
        new_api::base_url(base, protocol)
    } else if driver == "azure-openai" {
        format!("{base}/v1")
    } else if driver == "zenmux" && protocol == ModelProtocol::Anthropic {
        let root = base
            .strip_suffix("/anthropic/v1")
            .or_else(|| base.strip_suffix("/v1"))
            .unwrap_or(base);
        format!("{root}/anthropic/v1")
    } else {
        base.to_owned()
    };
    if matches!(driver, "aihubmix" | "aihubmix-responses")
        && !headers
            .keys()
            .any(|key| key.eq_ignore_ascii_case("app-code"))
    {
        headers.insert("APP-Code".into(), "WHVL9885".into());
    }
    let preset = if protocol == ModelProtocol::OpenaiCompatible
        && matches!(driver, "deepseek" | "moonshotai" | "aihubmix" | "zenmux")
    {
        Some(driver.to_owned())
    } else {
        None
    };
    HttpModelConnection {
        protocol,
        base_url,
        network: Default::default(),
        headers,
        bearer: None,
        api_key: None,
        preset,
    }
}
