//! `provider.management` for the `--model-config` development profile.
//!
//! The full Provider part owns a provider database and is installed only in
//! product profiles, so a development daemon had no way to list the models its
//! configured profile serves. Client model pickers still need that read, so
//! this part answers the read-only subset from the same
//! [`SelectableModelsHost`] execution resolves, rather than inventing a second
//! catalog.

use std::collections::BTreeMap;
use std::sync::Arc;

use dim_agent_capabilities::host::SelectableModelsHost;
use dim_agent_capabilities::methods::{Interruption, MethodCall, MethodDecl, RpcError, reply};
use rutis::Ctx;
use rutis::host::{BoxFuture, CtxHost, DynPlugin, Effect, Key, TypeKey};
use serde::Serialize;

use crate::methods::ManagementMethods;

pub struct ConfiguredModelsManagementPlugin {
    methods: Key<ManagementMethods>,
    selectable: Key<SelectableModelsHost>,
}

impl ConfiguredModelsManagementPlugin {
    pub fn new(names: &dim_agent_runtime::Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            methods: names.key()?,
            selectable: names.key()?,
        })
    }
}

impl DynPlugin for ConfiguredModelsManagementPlugin {
    fn name(&self) -> &str {
        "ConfiguredModelsManagement"
    }

    fn injects(&self) -> Vec<TypeKey> {
        vec![self.methods.erase(), self.selectable.erase()]
    }

    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let methods = ctx.require_at(&self.methods)?;
            let selectable = ctx.require_at(&self.selectable)?;
            methods.register(
                ctx,
                "provider",
                vec![MethodDecl::new(
                    "provider.management",
                    false,
                    Interruption::Drain,
                )],
                Arc::new(move |_call: MethodCall, stopping, _| {
                    let selectable = selectable.clone();
                    Box::pin(async move {
                        let models = selectable.0.selectable(stopping).await.map_err(|error| {
                            RpcError::app(-32000, "provider_failed", error.to_string())
                        })?;
                        reply(ManagementView::from_models(models))
                    })
                }),
            )?;
            Ok(Effect::Done)
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagementView {
    providers: Vec<ProviderView>,
    drivers: Vec<()>,
    errors: Vec<()>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderView {
    provider_id: String,
    display_name: String,
    source: &'static str,
    version: u32,
    enabled: bool,
    models: Vec<ModelView>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelView {
    model: ModelEntryView,
    enabled: bool,
    unavailable_reason: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelEntryView {
    model_id: String,
    display_name: String,
    capabilities: CapabilitiesView,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CapabilitiesView {
    context_window: Option<u64>,
    max_output_tokens: Option<u64>,
}

impl ManagementView {
    fn from_models(models: Vec<dim_agent_types::ModelCapabilitySnapshot>) -> Self {
        let mut grouped: BTreeMap<String, Vec<ModelView>> = BTreeMap::new();
        for snapshot in models {
            let provider_id = snapshot.target.provider_id.clone();
            let model_id = snapshot.target.model_id.clone();
            grouped.entry(provider_id).or_default().push(ModelView {
                model: ModelEntryView {
                    model_id: model_id.clone(),
                    display_name: model_id,
                    capabilities: CapabilitiesView {
                        context_window: Some(snapshot.capabilities.context_window),
                        max_output_tokens: Some(snapshot.capabilities.max_output_tokens),
                    },
                },
                enabled: true,
                unavailable_reason: None,
            });
        }
        Self {
            providers: grouped
                .into_iter()
                .map(|(provider_id, models)| ProviderView {
                    display_name: provider_id.clone(),
                    provider_id,
                    source: "configured",
                    version: 0,
                    enabled: true,
                    models,
                })
                .collect(),
            drivers: Vec::new(),
            errors: Vec::new(),
        }
    }
}
