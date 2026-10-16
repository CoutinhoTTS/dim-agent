//! Window facts of the product profile, taken from the request's adopted
//! capability snapshot with a local token estimate (ContextCompaction §5.1).
//! The values are provisional (#128).
use crate::host::*;
use dim_agent_types::*;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Share of the input budget at which a preparation compacts.
const THRESHOLD_RATIO: f64 = 0.8;
/// Serialized UTF-8 bytes per estimated token.
const BYTES_PER_TOKEN: f64 = 4.;
/// Estimated tokens of one image part.
const IMAGE_TOKENS: f64 = 1_600.;

/// Resolves window facts from the request alone; no service is needed.
pub struct ProductContextModel;
impl ResolveContextModel for ProductContextModel {
    fn resolve(
        &self,
        request: Arc<ModelRequest>,
        _: CancellationToken,
    ) -> HostFuture<'_, Arc<dyn ContextModel>> {
        Box::pin(async move {
            let snapshot = request.capability_snapshot.as_ref().ok_or_else(|| {
                HostError::classified(
                    HostErrorKind::InvalidRequest,
                    "Model capabilities are not adopted; select the model again",
                )
            })?;
            snapshot
                .validate_for(&request.provider_id, &request.model_id)
                .map_err(|message| HostError::classified(HostErrorKind::InvalidRequest, message))?;
            let limit = snapshot.capabilities.max_output_tokens as f64;
            let context_window = snapshot.capabilities.context_window as f64;
            let output_tokens = request
                .config
                .as_ref()
                .and_then(|config| config.max_tokens)
                .map_or(limit, |max| max.min(limit));
            Ok(Arc::new(WindowFacts {
                context_window,
                output_tokens,
            }) as Arc<dyn ContextModel>)
        })
    }
}

struct WindowFacts {
    context_window: f64,
    output_tokens: f64,
}
impl ContextModel for WindowFacts {
    fn context_window(&self) -> f64 {
        self.context_window
    }
    fn output_tokens(&self) -> f64 {
        self.output_tokens
    }
    fn threshold_tokens(&self) -> f64 {
        THRESHOLD_RATIO * (self.context_window - self.output_tokens).max(0.)
    }
    fn count_tokens(&self, request: &ModelRequest) -> f64 {
        let mut bytes = size(&request.system) + size(&request.tools);
        let mut images = 0.;
        for message in &request.messages {
            match &message.content {
                MessageContent::Text(_) => bytes += size(message.as_ref()),
                MessageContent::Parts(parts) => {
                    for part in parts {
                        match part {
                            MessagePart::Image { .. } => images += 1.,
                            part => bytes += size(part),
                        }
                    }
                }
            }
        }
        bytes / BYTES_PER_TOKEN + images * IMAGE_TOKENS
    }
}

/// Serialized JSON size; request parts serialize by construction.
fn size(value: &impl serde::Serialize) -> f64 {
    serde_json::to_vec(value).map_or(0, |bytes| bytes.len()) as f64
}
