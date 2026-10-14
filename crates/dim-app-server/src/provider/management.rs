//! Provider configuration wire boundary; accepted operations drain on unload.
use super::ProviderService;
use crate::{
    methods::{Interruption, MethodDecl, Methods, params, reply},
    protocol::*,
};
use dim_agent_runtime::{Capability, KeysOf, scoped};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};
use std::sync::Arc;

pub struct ProviderManagement;
impl TypedPlugin for ProviderManagement {
    type Deps = (Arc<Methods>, Arc<ProviderService>);
    fn name(&self) -> &str {
        "ProviderManagement"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (management, provider): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let registration = management
                .register(
                    "provider",
                    [
                        "provider.catalog",
                        "provider.refresh",
                        "provider.models.set_enabled",
                        "provider.models.edit",
                        "provider.models.reset",
                        "provider.selection",
                        "provider.select",
                        "provider.list",
                        "provider.save",
                        "provider.delete",
                        "provider.import_legacy",
                    ]
                    .into_iter()
                    .map(|name| {
                        MethodDecl::management(
                            name,
                            !matches!(
                                name,
                                "provider.catalog" | "provider.selection" | "provider.list"
                            ),
                            Interruption::Drain,
                        )
                    })
                    .collect(),
                    vec![],
                    Arc::new(move |method, value, stopping| {
                        let provider = provider.clone();
                        Box::pin(async move {
                            let cancel = &stopping;
                            match method.as_str() {
                                "provider.catalog" => {
                                    let p: ProviderTarget = params(value)?;
                                    reply(CatalogResult {
                                        catalog: provider
                                            .catalog_for(p.provider_id, cancel)
                                            .await
                                            .map_err(error)?,
                                    })
                                }
                                "provider.refresh" => {
                                    let p: ProviderTarget = params(value)?;
                                    reply(
                                        provider
                                            .refresh_for(p.provider_id, cancel)
                                            .await
                                            .map_err(error)?,
                                    )
                                }
                                "provider.models.set_enabled" => {
                                    let p: SetEnabledModels = params(value)?;
                                    reply(
                                        provider
                                            .set_enabled_models(
                                                p.provider_id,
                                                p.expected_version,
                                                p.model_ids,
                                                cancel,
                                            )
                                            .await
                                            .map_err(error)?,
                                    )
                                }
                                "provider.models.edit" => {
                                    let p: EditModel = params(value)?;
                                    reply(
                                        provider
                                            .update_model_config(
                                                p.provider_id,
                                                p.expected_version,
                                                p.model_id,
                                                Some(p.edit),
                                                cancel,
                                            )
                                            .await
                                            .map_err(error)?,
                                    )
                                }
                                "provider.models.reset" => {
                                    let p: ResetModel = params(value)?;
                                    reply(
                                        provider
                                            .update_model_config(
                                                p.provider_id,
                                                p.expected_version,
                                                p.model_id,
                                                None,
                                                cancel,
                                            )
                                            .await
                                            .map_err(error)?,
                                    )
                                }
                                "provider.selection" => {
                                    let p: SelectionScope = params(value)?;
                                    let cwd = match p.cwd {
                                        Some(cwd) => Some(canonical_cwd(&cwd).await?),
                                        None => None,
                                    };
                                    reply(SelectionResult {
                                        selection: provider
                                            .selection(cwd.as_deref(), cancel)
                                            .await
                                            .map_err(error)?,
                                    })
                                }
                                "provider.select" => {
                                    let p: SelectModel = params(value)?;
                                    if p.model_id.trim().is_empty() {
                                        return Err(RpcError::invalid("modelId must not be empty"));
                                    }
                                    let cwd = match p.cwd {
                                        Some(cwd) => Some(canonical_cwd(&cwd).await?),
                                        None => None,
                                    };
                                    reply(
                                        provider
                                            .select(
                                                cwd.as_deref(),
                                                dim_agent_types::ModelId {
                                                    provider_id: p.provider_id,
                                                    model_id: p.model_id,
                                                },
                                                p.expected_version,
                                                cancel,
                                            )
                                            .await
                                            .map_err(error)?,
                                    )
                                }
                                "provider.list" => {
                                    let _: Empty = params(value)?;
                                    reply(provider.list(cancel).await.map_err(error)?)
                                }
                                "provider.save" => {
                                    let p: SaveProvider = params(value)?;
                                    reply(
                                        provider
                                            .save_static(p.provider, p.expected_version, cancel)
                                            .await
                                            .map_err(error)?,
                                    )
                                }
                                "provider.import_legacy" => {
                                    let p: ImportLegacyProviders = params(value)?;
                                    reply(
                                        provider
                                            .import_legacy(p.source_path, cancel)
                                            .await
                                            .map_err(error)?,
                                    )
                                }
                                "provider.delete" => {
                                    let p: DeleteProvider = params(value)?;
                                    provider
                                        .delete_static(p.provider_id, p.expected_version, cancel)
                                        .await
                                        .map_err(error)?;
                                    reply(Empty {})
                                }
                                _ => unreachable!("registered provider method"),
                            }
                        })
                    }),
                )
                .map_err(|error| CordisError::PluginFailed(error.message.into()))?;
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        registration.unregister_and_drain().await;
                        Ok(())
                    })
                }))
            })?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for ProviderManagement {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&())
    }
}

fn error(error: dim_agent_capabilities::host::HostError) -> RpcError {
    let kind = match &error {
        dim_agent_capabilities::host::HostError::Cancelled(_) => "provider_cancelled",
        _ => "provider_failed",
    };
    RpcError::app(-32000, kind, error.to_string())
}
