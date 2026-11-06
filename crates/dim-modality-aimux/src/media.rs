//! One connection as an aimux provider, and one model out of it.
//!
//! The catalog names the vendor (through [`crate::vendor`]); the connection
//! supplies the endpoint, the credential and the transport. Nothing here keeps
//! either: the provider is built for one call and dropped with it.

use aimux_core::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::{FetchFunction, Resolvable};
use aimux_providers::PresetSettings;
use dim_agent_aimux::MediaTransport;
use dim_agent_capabilities::host::{ModalityConnection, ModelConnection, ModelNetwork};
use dim_modality::AdapterError;
use std::sync::Arc;

use crate::vendor::{Operation, Vendor};

/// Where a media call's transport comes from.
pub enum Transport {
    /// The product's transport: the connection's proxy, timeout and query,
    /// with the redirect policy a chat call gets.
    Shared(MediaTransport),
    /// A test's scripted transport. The connection's base URL then carries no
    /// query, because only [`MediaTransport`] splits one off.
    Fixed(FetchFunction),
}

impl Transport {
    fn resolve(&self, network: &ModelNetwork, base_url: &str) -> (String, FetchFunction) {
        match self {
            Self::Shared(transport) => transport.resolve(network, base_url),
            Self::Fixed(fetch) => (base_url.to_owned(), fetch.clone()),
        }
    }
}

/// The aimux provider that executes one connection's `operation`.
pub fn provider(
    connection: &ModalityConnection,
    vendor: &Vendor,
    operation: Operation,
    transport: &Transport,
) -> Result<Arc<dyn Provider>, AdapterError> {
    match &connection.resolved.connection {
        ModelConnection::Http(http) => {
            let (base_url, fetch) =
                transport.resolve(&http.network, &vendor.base_url(operation, &http.base_url));
            // A record may carry no base URL of its own (the bundled catalog
            // leaves google without one): the package's default then applies,
            // as it does for any other unset setting.
            let base_url = (!base_url.is_empty()).then_some(base_url);
            // The credential reaches the package the way the connection
            // resolved it: as the bearer when Provider stripped one off, or
            // already inside `headers` for a vendor's own header name. An
            // empty key leaves the header names to that vendor; a per-call
            // header of the same name then wins (the layers merge with the
            // call last).
            let credential = http
                .bearer
                .clone()
                .or_else(|| http.api_key.clone())
                .unwrap_or_default();
            let settings = PresetSettings {
                api_key: Some(Resolvable::Value(credential)),
                base_url,
                headers: Some(
                    http.headers
                        .iter()
                        .map(|(name, value)| (name.clone(), Some(value.clone())))
                        .collect(),
                ),
                fetch: Some(fetch),
                ..Default::default()
            };
            aimux_providers::create_provider(vendor.aimux, settings).map_err(|error| {
                AdapterError::unavailable(format!(
                    "The {} provider is unavailable: {error}",
                    vendor.catalog
                ))
            })
        }
        ModelConnection::Bedrock(_) => Err(AdapterError::unavailable(
            "No bundled catalog model declares media on an Amazon Bedrock connection",
        )),
        ModelConnection::Local => Err(AdapterError::unavailable(
            "The model's connection is local and serves no media call",
        )),
    }
}

/// Whether the package's image model for `model` reads the input files an
/// edit sends.
///
/// The row records whether the package reads `options.files` at all; where the
/// package also reports file input per model (`supports_file_inputs`), a model
/// that says `false` drops edit too. `None` — an unknown model — and a package
/// that cannot be built keep the row's answer.
pub fn serves_edit(vendor: &Vendor, model: &str) -> bool {
    if !vendor.kinds.image_edit {
        return false;
    }
    let Ok(provider) = aimux_providers::create_provider(vendor.aimux, PresetSettings::default())
    else {
        return true;
    };
    match provider.image_model(model) {
        Ok(image) => image.supports_file_inputs() != Some(false),
        Err(_) => true,
    }
}

/// The image model of `provider`; a package without one answers `unavailable`.
pub fn image_model(
    provider: &Arc<dyn Provider>,
    model: &str,
) -> Result<Arc<dyn ImageModel>, AdapterError> {
    match provider.image_model(model) {
        Ok(model) => Ok(model),
        Err(AiMuxError::NoSuchModel { .. }) => Err(AdapterError::unavailable(format!(
            "The provider serves no image model `{model}`"
        ))),
        Err(error) => Err(AdapterError::unavailable(format!(
            "The provider refused `{model}`: {error}"
        ))),
    }
}
