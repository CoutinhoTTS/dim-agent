use dim_app_server::model_catalog::{BundledCatalog, ModelEdit, ModelSource, ProviderProjection};
use serde_json::json;

#[test]
fn bundled_lookup_preserves_explicit_asymmetric_limits_and_hidden_metadata() {
    let catalog = BundledCatalog::load().unwrap();
    let provider = ProviderProjection {
        provider_id: "my-gateway".into(),
        source: Some(ModelSource::Custom),
        ..Default::default()
    };
    let entry = catalog
        .project(
            &provider,
            &json!({
                "id": "gpt-5-nano", "limit": { "context": 777 },
                "raw": { "vendorFlag": { "nested": true } }
            }),
        )
        .unwrap();

    assert_eq!(entry.capabilities.context_window, 777);
    assert_eq!(entry.capabilities.max_output_tokens, 128_000);
    assert!(entry.metadata.as_ref().unwrap().pricing.is_some());
    assert_eq!(
        entry.metadata.as_ref().unwrap().raw["vendorFlag"]["nested"],
        true
    );
    assert_eq!(
        entry.metadata.as_ref().unwrap().match_strategy.as_deref(),
        Some("openai-exact")
    );
}

#[test]
fn effective_override_and_reset_use_the_same_bundled_projection() {
    let catalog = BundledCatalog::load().unwrap();
    let provider = ProviderProjection {
        provider_id: "custom".into(),
        source: Some(ModelSource::Custom),
        ..Default::default()
    };
    let defaults = vec![
        catalog
            .project(&provider, &json!({ "id": "gpt-5-nano" }))
            .unwrap(),
    ];
    let base = catalog.effective_models(&provider, &defaults, &[]);
    let edited = catalog.merge_visible_edit(
        &base[0],
        ModelEdit {
            display_name: Some("Mine".into()),
            ..Default::default()
        },
    );
    let overridden = catalog.effective_models(&provider, &defaults, &[edited]);
    let reset = catalog.effective_models(&provider, &defaults, &[]);

    assert_eq!(overridden[0].display_name, "Mine");
    assert_eq!(reset[0].display_name, "GPT-5 Nano");
    assert_eq!(reset[0].capabilities.max_output_tokens, 128_000);
    // Historical default limits may be corrected; explicit stored booleans may not.
    let mut stored = defaults[0].clone();
    stored.capabilities = Default::default();
    let effective = catalog.effective_models(&provider, &[stored], &[]);
    assert_eq!(effective[0].capabilities.context_window, 400_000);
    assert_eq!(effective[0].capabilities.max_output_tokens, 128_000);
    assert!(!effective[0].capabilities.vision);
    assert_eq!(effective[0].capabilities.modalities, ["text"]);
}

#[test]
fn normalized_family_lookup_matches_prefixed_ids() {
    let catalog = BundledCatalog::load().unwrap();
    let provider = ProviderProjection {
        provider_id: "custom-provider".into(),
        source: Some(ModelSource::Custom),
        ..Default::default()
    };
    let entry = catalog
        .project(
            &provider,
            &json!({
                "id": "models/vendor/DeepSeek_V4_Flash"
            }),
        )
        .unwrap();
    assert_eq!(
        entry
            .metadata
            .as_ref()
            .unwrap()
            .matched_provider_id
            .as_deref(),
        Some("custom-provider")
    );
    let official = catalog
        .project(
            &provider,
            &json!({
                "id": "models/vendor/GPT_5_Nano"
            }),
        )
        .unwrap();
    assert_eq!(
        official
            .metadata
            .as_ref()
            .unwrap()
            .matched_provider_id
            .as_deref(),
        Some("openai")
    );
    assert_eq!(official.capabilities.max_output_tokens, 128000);
    assert!(catalog.project(&provider, &json!(null)).is_err());
}

#[test]
fn media_lookup_and_explicit_capabilities_survive_editing() {
    let catalog = BundledCatalog::load().unwrap();
    let provider = ProviderProjection {
        provider_id: "custom".into(),
        source: Some(ModelSource::Custom),
        ..Default::default()
    };
    let media = catalog
        .project(
            &provider,
            &json!({
                "id": "eleven_multilingual_v2"
            }),
        )
        .unwrap();
    assert_eq!(
        media
            .metadata
            .as_ref()
            .unwrap()
            .matched_provider_id
            .as_deref(),
        Some("elevenlabs")
    );
    assert_eq!(
        media.metadata.as_ref().unwrap().raw["modalityCapabilities"],
        json!(["speech.synthesize"])
    );
    let model = catalog
        .project(
            &provider,
            &json!({
                "id": "gpt-5-nano", "limit": {"context": 128000, "output": 16384},
                "vision": true, "modalities": ["text"],
                "capabilities": {"streaming": false, "providerState": {"custom": true}},
                "reasoning": {"supported": false, "effortOptions": ["off"]}
            }),
        )
        .unwrap();
    assert_eq!(model.capabilities.context_window, 128000);
    assert_eq!(model.capabilities.max_output_tokens, 16384);
    assert_eq!(model.capabilities.modalities, ["text"]);
    assert!(!model.capabilities.streaming);
    assert!(!model.capabilities.reasoning);
    assert_eq!(
        model.capabilities.provider_state.as_ref().unwrap()["custom"],
        true
    );
    let nested_limits = catalog
        .project(
            &provider,
            &json!({
                "id": "gpt-5-nano",
                "capabilities": {"contextWindow": 128000, "maxOutputTokens": 16384}
            }),
        )
        .unwrap();
    assert_eq!(nested_limits.capabilities.context_window, 128000);
    assert_eq!(nested_limits.capabilities.max_output_tokens, 16384);
    let effective = catalog.effective_models(&provider, &[nested_limits], &[]);
    assert_eq!(effective[0].capabilities.context_window, 128000);
    assert_eq!(effective[0].capabilities.max_output_tokens, 16384);
    assert_eq!(
        model
            .metadata
            .as_ref()
            .unwrap()
            .reasoning
            .as_ref()
            .unwrap()
            .effort_options
            .as_ref()
            .unwrap(),
        &["off"]
    );
    let edited = catalog.merge_visible_edit(
        &model,
        ModelEdit {
            display_name: Some("Custom".into()),
            ..Default::default()
        },
    );
    assert_eq!(edited.metadata, model.metadata);
}

#[test]
fn visible_edits_preserve_hidden_fields_and_update_legacy_media_readers() {
    let catalog = BundledCatalog::load().unwrap();
    let provider = ProviderProjection {
        provider_id: "custom".into(),
        ..Default::default()
    };
    let model = catalog.project(&provider, &json!({
        "id": "gpt-5-nano", "reasoning": {"supported": true, "mode": "effort", "defaultEnabled": true, "effort": "high"},
        "serviceTier": {"supported": true, "default": "priority", "options": ["priority"]},
        "raw": {"imageGeneration": false, "private": {"value": 73}}
    })).unwrap();
    let edit = serde_json::from_value(json!({
        "reasoning": {"supported": false, "effortOptions": ["low", "high"]},
        "modalityCapabilities": ["image.generate"]
    }))
    .unwrap();
    let edited = catalog.merge_visible_edit(&model, edit);
    let metadata = edited.metadata.as_ref().unwrap();
    assert_eq!(metadata.raw["imageGeneration"], true);
    assert_eq!(metadata.raw["private"]["value"], 73);
    assert_eq!(
        metadata.service_tier,
        model.metadata.as_ref().unwrap().service_tier
    );
    let reasoning = metadata.reasoning.as_ref().unwrap();
    assert!(!reasoning.supported);
    assert!(reasoning.effort_options.is_none());
    assert_eq!(reasoning.mode.as_deref(), Some("effort"));
    assert_eq!(reasoning.effort.as_deref(), Some("high"));
    let edited = catalog.merge_visible_edit(
        &edited,
        serde_json::from_value(json!({"modalityCapabilities": []})).unwrap(),
    );
    assert_eq!(
        edited.metadata.as_ref().unwrap().raw["imageGeneration"],
        false
    );
    for field in [
        "status",
        "streaming",
        "structuredOutput",
        "audio",
        "serviceTier",
    ] {
        assert!(
            serde_json::from_value::<ModelEdit>(json!({field: null})).is_err(),
            "{field} must not be editable"
        );
    }
}
