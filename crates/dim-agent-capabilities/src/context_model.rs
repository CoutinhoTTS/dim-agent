//! Window facts of a profile that compacts, from the model's declared limits
//! with a local token estimate (ContextCompaction §5.1).
//! The trigger reserve is a provisional heuristic (#128).
use crate::host::*;
use dim_agent_types::*;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// The trigger leaves `RESERVE_RATIO` of the input budget, at least
/// `RESERVE_MIN_TOKENS` and at most `RESERVE_MAX_RATIO` of it, for the
/// estimation error of the next request.
const RESERVE_RATIO: f64 = 0.10;
const RESERVE_MIN_TOKENS: f64 = 16_000.;
const RESERVE_MAX_RATIO: f64 = 0.20;
/// No reply needs more than this, however much the model declares.
const OUTPUT_CEILING: u64 = 131_072;
/// Serialized UTF-8 bytes per estimated token.
const BYTES_PER_TOKEN: f64 = 4.;
/// Estimated tokens of one image part.
const IMAGE_TOKENS: f64 = 1_600.;

/// The adopted output allowance of a model: its declared maximum output,
/// bounded by [`OUTPUT_CEILING`] and by the least input budget a Session
/// keeps, so `B = context_window − allowance` stays positive even when a
/// source declares an output at or above the window (ContextCompaction §5.1).
/// The declared maximum stays the capability fact it is.
pub fn output_allowance(context_window: u64, declared_max_output: u64) -> u64 {
    let input_floor = (RESERVE_MIN_TOKENS as u64).min(context_window / 2);
    let room = context_window.saturating_sub(input_floor);
    declared_max_output.min(OUTPUT_CEILING).min(room).max(1)
}

/// The configuration a stored adoption yields at a request boundary: the
/// stored output allowance, repaired to [`output_allowance`] when it is
/// absent or above the rule. The stored fact is not rewritten, so a Session
/// adopted before the rule existed stays readable and usable.
pub fn adopted_config(
    snapshot: Option<&ModelCapabilitySnapshot>,
    config: Option<&ModelConfig>,
) -> Option<ModelConfig> {
    let Some(capabilities) = snapshot.map(|snapshot| &snapshot.capabilities) else {
        return config.cloned();
    };
    let allowance =
        output_allowance(capabilities.context_window, capabilities.max_output_tokens) as f64;
    let mut config = config.cloned().unwrap_or_default();
    config.max_tokens = Some(
        config
            .max_tokens
            .filter(|stored| *stored > 0.)
            .map_or(allowance, |stored| stored.min(allowance)),
    );
    Some(config)
}

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
            let allowance = request
                .config
                .as_ref()
                .and_then(|config| config.max_tokens)
                .ok_or_else(|| {
                    HostError::classified(
                        HostErrorKind::InvalidRequest,
                        "Model output allowance is not adopted; select the model again",
                    )
                })?;
            Ok(window_facts(
                snapshot.capabilities.context_window,
                allowance,
                &request,
            ))
        })
    }
}

/// Window facts for a model whose adopted output allowance is `output_tokens`:
/// its context window and the allowance every request reserves.
pub fn window_facts(
    context_window: u64,
    output_tokens: f64,
    request: &ModelRequest,
) -> Arc<dyn ContextModel> {
    Arc::new(WindowFacts {
        context_window: context_window as f64,
        output_tokens,
        auto_compact_window: request
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.startup.as_ref())
            .and_then(|startup| startup.auto_compact_window),
    })
}

struct WindowFacts {
    context_window: f64,
    output_tokens: f64,
    auto_compact_window: Option<u64>,
}
impl ContextModel for WindowFacts {
    fn context_window(&self) -> f64 {
        self.context_window
    }
    fn output_tokens(&self) -> f64 {
        self.output_tokens
    }
    fn threshold_tokens(&self) -> f64 {
        let budget = (self.context_window - self.output_tokens).max(0.);
        let threshold = budget
            - (RESERVE_RATIO * budget)
                .max(RESERVE_MIN_TOKENS)
                .min(RESERVE_MAX_RATIO * budget);
        self.auto_compact_window.map_or(threshold, |window| {
            threshold.min((window as f64 - self.output_tokens).max(0.))
        })
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
                            MessagePart::ToolResult {
                                content: ToolContent::Blocks(blocks),
                                ..
                            } => {
                                for block in blocks {
                                    match block {
                                        ContentBlock::Image { .. } => images += 1.,
                                        block => bytes += size(block),
                                    }
                                }
                            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_allowance_reserves_an_input_budget_without_shrinking_a_healthy_limit() {
        let cases = [
            (32_768, 32_768, 16_768),
            (32_768, 65_536, 16_768),
            (8_192, 8_192, 4_096),
            (4_000, 5_000, 2_000),
            (2, 2, 1),
            (1, 1, 1),
            (128_000, 128_000, 112_000),
            (262_144, 262_144, 131_072),
            (500_000, 500_000, 131_072),
            (131_072, 131_072, 115_072),
            (8_192, 4_096, 4_096),
            (200_000, 100_000, 100_000),
            (400_000, 128_000, 128_000),
        ];
        for (window, declared, expected) in cases {
            assert_eq!(
                output_allowance(window, declared),
                expected,
                "window {window}, declared output {declared}"
            );
        }
    }
}
