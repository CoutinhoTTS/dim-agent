use dim_agent_capabilities::host::{ModelProtocol, adopted_preferences, reasoning_config};
use dim_agent_types::*;

/// Which `metadata.reasoning` declarations configure a request, and which only
/// describe the model.
///
/// A declaration configures a request when all three hold: a field in the typed
/// configuration carries it, the request builder sends that field for some
/// protocol after validation, and the vendor adapter that receives it changes
/// the bytes it writes. The applied set is therefore the declared reasoning
/// effort, the declared service tier, and the declared reasoning summary.
/// Everything else describes the model:
///
/// - `default_enabled`, `off` and `effort_options` describe the effort control
///   and are read through it, not sent on their own.
/// - `supported` denies every control the others describe.
/// - `summaries` is applied by [`declared_summary`], which owns its own guard
///   because the summary follows the resulting effort.
/// - `mode`, `level`, `level_options`, `verbosity`, `verbosity_options` and
///   `budget` have no field on any dim request path; the first four would need
///   their own control dispatch before they could configure anything.
/// - `visibility`, `continuation` and `notes` have no reader at all.
/// - `interleaved` is not expressible on any dim protocol path. The wire field
///   that would carry it (`interleavedThinking`) is rejected before the
///   request leaves, on every protocol, and no vendor adapter reads it; the
///   Anthropic control it stands for is `thinking.type: "between_tools"`,
///   which dim's thinking type cannot spell.
///
/// The base configuration an adoption starts from: the model's declared
/// configuration with the catalog's own defaults applied.
///
/// `docs/design/appserver/provider-management.md` §3.3 says omitted
/// preferences "still start at catalog defaults", and
/// `docs/design/model-configuration.md` says a cleared field carries "the
/// value an explicit adoption of the same model with no preference for that
/// field would carry". The declared reasoning effort and service tier are
/// therefore part of that base, applied through the same mapping an explicit
/// preference uses, so the adopted configuration, every read and every
/// request agree on the value.
///
/// A declared default applies only when the model declares the control: the
/// effort is one of the picker's `effortOptions` (or `none` beside a
/// reasoning-off declaration), the tier is supported and listed, and the
/// protocol can express the value. Otherwise the base comes back unchanged.
/// Unlike an explicit preference, a default never makes an adoption fail.
pub(super) fn catalog_defaults(
    base: &ModelConfig,
    snapshot: &ModelCapabilitySnapshot,
    protocol: ModelProtocol,
) -> ModelConfig {
    let mut config = base.clone();
    let adopted = adopted_preferences(&config, protocol);
    if adopted
        .as_ref()
        .is_none_or(|adopted| adopted.reasoning_effort.is_none())
        && let Some(effort) = declared_effort(snapshot)
        && let Ok(applied) = reasoning_config(&config, snapshot, protocol, &effort)
    {
        config = applied;
    }
    if adopted
        .as_ref()
        .is_none_or(|adopted| adopted.service_tier.is_none())
        && let Some(tier) = declared_service_tier(snapshot)
    {
        let preferences = ModelPreferences {
            reasoning_effort: None,
            service_tier: Some(tier),
        };
        if let Ok(applied) = configure(&preferences, snapshot, protocol, &config) {
            config = applied;
        }
    }
    config
}

/// The model's declared default effort, when the picker would offer it.
///
/// A declaration that states reasoning is off by default (#1376) names only a
/// strength the model does not use, so it offers the declaration's own `off`
/// mapping instead of the `effort` beside it.
fn declared_effort(snapshot: &ModelCapabilitySnapshot) -> Option<String> {
    let reasoning = snapshot.reasoning.as_ref()?;
    if !reasoning.supported || !snapshot.capabilities.reasoning {
        return None;
    }
    if reasoning.default_enabled == Some(false) {
        return reasoning.off.as_ref().map(|_| "none".to_owned());
    }
    let effort = reasoning.effort.as_ref()?;
    let mut options = reasoning.effort_options.clone().unwrap_or_default();
    if reasoning.off.is_some() && !options.iter().any(|option| option == "none") {
        options.push("none".into());
    }
    options
        .iter()
        .any(|option| option == effort)
        .then(|| effort.clone())
}

/// The model's declared default service tier, when its declaration lists it.
fn declared_service_tier(snapshot: &ModelCapabilitySnapshot) -> Option<String> {
    let tier = snapshot.service_tier.as_ref()?;
    let default = tier.default.as_ref()?;
    (tier.supported && !default.is_empty() && tier.options.contains(default))
        .then(|| default.clone())
}

/// Applies the model's declared reasoning summary to a configuration that is
/// otherwise final.
///
/// The summary is derived, not stored: it follows the reasoning effort the
/// configuration actually sends, so [`configure`] recomputes it after the
/// preferences are applied instead of writing it once into the base. A base
/// written once would survive an update to effort `none` and reach the wire as
/// `{"effort":"none","summary":"detailed"}`, and it would go stale when the
/// effort is cleared back to the declared default.
///
/// The rule needs no record of where a value came from, because the only value
/// this function writes is `Detailed`:
///
/// 1. Off the OpenAI Responses protocol, or on a model that does not declare
///    summaries, the configuration comes back untouched. A model this function
///    does not apply to never loses a summary of its own.
/// 2. A configuration carrying a style other than `Detailed` states it itself,
///    so it is left exactly as it is.
/// 3. Otherwise the declaration decides: `Detailed` when the resulting effort
///    is present and not `none`, and no summary when it is not.
///
/// Step 2 has one consequence worth stating: on a model that declares
/// summaries, `Detailed` cannot be told apart from the value this function
/// derives, so it follows the effort. `Detailed` beside an effort of `none` is
/// therefore dropped, because that request contradicts itself. Every other
/// style survives any effort.
///
/// The effort guard is not a detail. Without an effort, a summary alone flips
/// the adapter into reasoning mode through `forceReasoning`, which changes the
/// system role to `developer` and drops temperature and top-p. The adapter's own
/// default already sends a summary exactly when an effort other than `none` is
/// set; the declaration makes that default explicit and stable rather than
/// widening it.
///
/// Only the OpenAI Responses body has a place for a summary style. Every other
/// protocol rejects the field, so nothing is applied or cleared there.
fn declared_summary(
    config: &ModelConfig,
    snapshot: &ModelCapabilitySnapshot,
    protocol: ModelProtocol,
) -> ModelConfig {
    if protocol != ModelProtocol::OpenaiResponses {
        return config.clone();
    }
    // A model that does not declare summaries is not this rule's business, so
    // whatever its configuration says about a summary stays said.
    let declares = snapshot
        .reasoning
        .as_ref()
        .is_some_and(|reasoning| reasoning.supported && reasoning.summaries == Some(true))
        && snapshot.capabilities.reasoning;
    if !declares {
        return config.clone();
    }
    let mut config = config.clone();
    let configured = config
        .provider_options
        .as_ref()
        .and_then(|options| options.openai.as_ref())
        .and_then(|options| options.summary);
    if configured.is_some_and(|style| style != SummaryStyle::Detailed) {
        return config;
    }
    let effort =
        adopted_preferences(&config, protocol).and_then(|preferences| preferences.reasoning_effort);
    let derived = effort
        .is_some_and(|effort| effort != "none")
        .then_some(SummaryStyle::Detailed);
    let options = config
        .provider_options
        .get_or_insert_default()
        .openai
        .get_or_insert_default();
    options.summary = derived;
    config
}

/// Applies preferences to a copy of `base`, validating them against the
/// model's declared capabilities.
///
/// The result also carries the model's declared reasoning summary, derived from
/// the effort it sends; see [`declared_summary`].
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
    Ok(declared_summary(&config, snapshot, protocol))
}

fn restore<T: Clone>(current: &mut Option<T>, declared: Option<&T>) {
    *current = declared.cloned();
}

/// The protocol's own option struct, created from the declared configuration
/// when it carries one and otherwise only reached when the adopted one exists.
fn options_for<'a, T: Default>(
    config: &'a mut ModelConfig,
    declared: Option<&T>,
    slot: impl FnOnce(&mut ProviderOptions) -> &mut Option<T>,
) -> Option<&'a mut T> {
    match declared {
        Some(_) => {
            Some(slot(config.provider_options.get_or_insert_default()).get_or_insert_default())
        }
        None => slot(config.provider_options.as_mut()?).as_mut(),
    }
}

/// Puts the reasoning controls [`configure`] replaces back to the model's
/// declared configuration, so a cleared effort carries what an explicit
/// adoption of the same model would.
pub(super) fn restore_reasoning(
    config: &mut ModelConfig,
    declared: &ModelConfig,
    protocol: ModelProtocol,
) {
    let declared_reasoning = declared.reasoning.as_ref();
    let reasoning = config.reasoning.get_or_insert_default();
    restore(
        &mut reasoning.effort,
        declared_reasoning.and_then(|value| value.effort.as_ref()),
    );
    restore(
        &mut reasoning.enabled,
        declared_reasoning.and_then(|value| value.enabled.as_ref()),
    );
    restore(
        &mut reasoning.budget_tokens,
        declared_reasoning.and_then(|value| value.budget_tokens.as_ref()),
    );
    if *reasoning == ProviderReasoningConfig::default() {
        config.reasoning = None;
    }
    let declared_options = declared.provider_options.as_ref();
    match protocol {
        ModelProtocol::Anthropic => {
            let declared = declared_options
                .and_then(|options| options.anthropic.as_ref())
                .filter(|value| value.effort.is_some() || value.thinking.is_some());
            if let Some(options) = options_for(config, declared, |options| &mut options.anthropic) {
                restore(
                    &mut options.effort,
                    declared.and_then(|value| value.effort.as_ref()),
                );
                restore(
                    &mut options.thinking,
                    declared.and_then(|value| value.thinking.as_ref()),
                );
            }
        }
        ModelProtocol::OpenaiCompatible => {
            let declared = declared_options
                .and_then(|options| options.openai_compatible.as_ref())
                .filter(|value| {
                    value.reasoning_effort.is_some()
                        || value.thinking.is_some()
                        || value.enable_thinking.is_some()
                });
            if let Some(options) =
                options_for(config, declared, |options| &mut options.openai_compatible)
            {
                restore(
                    &mut options.reasoning_effort,
                    declared.and_then(|value| value.reasoning_effort.as_ref()),
                );
                restore(
                    &mut options.thinking,
                    declared.and_then(|value| value.thinking.as_ref()),
                );
                restore(
                    &mut options.enable_thinking,
                    declared.and_then(|value| value.enable_thinking.as_ref()),
                );
            }
        }
        ModelProtocol::OpenaiChat | ModelProtocol::OpenaiResponses => {
            let declared = declared_options
                .and_then(|options| options.openai.as_ref())
                .filter(|value| value.reasoning_effort.is_some());
            if let Some(options) = options_for(config, declared, |options| &mut options.openai) {
                restore(
                    &mut options.reasoning_effort,
                    declared.and_then(|value| value.reasoning_effort.as_ref()),
                );
            }
        }
        ModelProtocol::Xai | ModelProtocol::XaiResponses => {
            let declared = declared_options
                .and_then(|options| options.xai.as_ref())
                .filter(|value| value.reasoning_effort.is_some());
            if let Some(options) = options_for(config, declared, |options| &mut options.xai) {
                restore(
                    &mut options.reasoning_effort,
                    declared.and_then(|value| value.reasoning_effort.as_ref()),
                );
            }
        }
        ModelProtocol::Gemini => {
            let declared = declared_options
                .and_then(|options| options.gemini.as_ref())
                .and_then(|options| options.thinking_config.as_ref())
                .and_then(|config| config.thinking_budget);
            match declared {
                Some(budget) => {
                    config
                        .provider_options
                        .get_or_insert_default()
                        .gemini
                        .get_or_insert_default()
                        .thinking_config
                        .get_or_insert_default()
                        .thinking_budget = Some(budget);
                }
                None => {
                    if let Some(options) = config
                        .provider_options
                        .as_mut()
                        .and_then(|options| options.gemini.as_mut())
                        .and_then(|options| options.thinking_config.as_mut())
                    {
                        options.thinking_budget = None;
                    }
                }
            }
        }
        ModelProtocol::Bedrock => {}
    }
}

/// Puts the service-tier control [`configure`] replaces back to the model's
/// declared configuration.
pub(super) fn restore_service_tier(
    config: &mut ModelConfig,
    declared: &ModelConfig,
    protocol: ModelProtocol,
) {
    let declared_options = declared.provider_options.as_ref();
    match protocol {
        ModelProtocol::OpenaiChat | ModelProtocol::OpenaiResponses => {
            let declared = declared_options
                .and_then(|options| options.openai.as_ref())
                .filter(|value| value.service_tier.is_some());
            if let Some(options) = options_for(config, declared, |options| &mut options.openai) {
                restore(
                    &mut options.service_tier,
                    declared.and_then(|value| value.service_tier.as_ref()),
                );
            }
        }
        ModelProtocol::OpenaiCompatible => {
            let declared = declared_options
                .and_then(|options| options.openai_compatible.as_ref())
                .filter(|value| value.service_tier.is_some());
            if let Some(options) =
                options_for(config, declared, |options| &mut options.openai_compatible)
            {
                restore(
                    &mut options.service_tier,
                    declared.and_then(|value| value.service_tier.as_ref()),
                );
            }
        }
        _ => {}
    }
}

/// Rules 1 and 2 of [`declared_summary`] are unreachable from a flow test, so
/// they are pinned here. A configuration carrying a summary style can only come
/// from `--model-config`, and that profile resolves models through Aimux's own
/// resolver rather than `ProviderService`, so no flow drives this function with
/// a configured summary. The rules still have to be right: they are what keeps
/// this function from clearing a value it did not write.
#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(reasoning: Option<ModelReasoning>) -> ModelCapabilitySnapshot {
        ModelCapabilitySnapshot {
            target: ModelId {
                provider_id: "p".into(),
                model_id: "m".into(),
            },
            capabilities: ModelCapabilities {
                reasoning: reasoning.is_some(),
                ..ModelCapabilities::with_limits(32_000, 4_096)
            },
            reasoning,
            service_tier: None,
        }
    }

    fn declaring(summaries: Option<bool>) -> ModelReasoning {
        ModelReasoning {
            supported: true,
            summaries,
            ..Default::default()
        }
    }

    fn carrying(style: Option<SummaryStyle>) -> ModelConfig {
        ModelConfig {
            provider_options: Some(ProviderOptions {
                openai: Some(OpenAiProviderOptions {
                    summary: style,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn summary_of(config: &ModelConfig) -> Option<SummaryStyle> {
        config
            .provider_options
            .as_ref()
            .and_then(|options| options.openai.as_ref())
            .and_then(|options| options.summary)
    }

    /// Rule 1: a model that does not declare summaries is not this rule's
    /// business, so a style its configuration sets survives untouched.
    #[test]
    fn a_non_declaring_model_keeps_a_configured_summary() {
        for declared in [None, Some(false)] {
            let config = carrying(Some(SummaryStyle::Concise));
            let applied = declared_summary(
                &config,
                &snapshot(Some(declaring(declared))),
                ModelProtocol::OpenaiResponses,
            );
            assert_eq!(
                summary_of(&applied),
                Some(SummaryStyle::Concise),
                "{declared:?}"
            );
            assert_eq!(applied, config);
        }
        // And a non-declaring model with no summary stays without one.
        let applied = declared_summary(
            &carrying(None),
            &snapshot(Some(declaring(None))),
            ModelProtocol::OpenaiResponses,
        );
        assert_eq!(summary_of(&applied), None);
    }

    /// Rule 1: a protocol with no place for a summary style is never touched.
    #[test]
    fn another_protocol_is_left_alone() {
        let config = carrying(Some(SummaryStyle::Concise));
        let applied = declared_summary(
            &config,
            &snapshot(Some(declaring(Some(true)))),
            ModelProtocol::OpenaiChat,
        );
        assert_eq!(applied, config);
    }

    /// Rule 2: a style other than `Detailed` is the configuration's own, on a
    /// declaring model too, whatever the effort says.
    #[test]
    fn a_non_detailed_style_survives_on_a_declaring_model() {
        for effort in [Some("high"), Some("none"), None] {
            let mut config = carrying(Some(SummaryStyle::Auto));
            config.reasoning = effort.map(|effort| ProviderReasoningConfig {
                effort: Some(effort.into()),
                ..Default::default()
            });
            let applied = declared_summary(
                &config,
                &snapshot(Some(declaring(Some(true)))),
                ModelProtocol::OpenaiResponses,
            );
            assert_eq!(summary_of(&applied), Some(SummaryStyle::Auto), "{effort:?}");
        }
    }

    /// Rule 3, the consequence the doc comment states: on a declaring model,
    /// `Detailed` cannot be told apart from the derived value, so it follows
    /// the effort. Beside `none` it is dropped, and with no effort at all it is
    /// dropped too.
    #[test]
    fn a_configured_detailed_follows_the_effort() {
        let with_effort = |effort: Option<&str>| {
            let mut config = carrying(Some(SummaryStyle::Detailed));
            config.reasoning = effort.map(|effort| ProviderReasoningConfig {
                effort: Some(effort.into()),
                ..Default::default()
            });
            declared_summary(
                &config,
                &snapshot(Some(declaring(Some(true)))),
                ModelProtocol::OpenaiResponses,
            )
        };
        assert_eq!(
            summary_of(&with_effort(Some("high"))),
            Some(SummaryStyle::Detailed)
        );
        assert_eq!(summary_of(&with_effort(Some("none"))), None);
        assert_eq!(summary_of(&with_effort(None)), None);
    }
}
