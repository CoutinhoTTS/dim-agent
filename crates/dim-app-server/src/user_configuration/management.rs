//! Typed configuration wire boundary; authority remains with Settings.
use crate::{
    methods::{Interruption, MethodDecl, Methods, params, reply},
    protocol::*,
    settings::{SettingsError, SettingsService},
    user_configuration::UserConfiguration,
};
use dim_agent_runtime::{Capability, KeysOf, scoped};
use rutis::{BoxFuture, CordisError, Ctx, Effect, EventKey, Listener, TypedPlugin};
use std::sync::Arc;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

struct Changes(tokio::sync::watch::Sender<()>);
impl Listener<rutis_loader::LoaderChanged> for Changes {
    fn call<'a>(
        &'a self,
        _: &'a Ctx,
        _: &'a rutis_loader::LoaderChanged,
    ) -> BoxFuture<'a, Result<Option<()>, CordisError>> {
        self.0.send_replace(());
        Box::pin(async { Ok(None) })
    }
}

fn error(error: SettingsError) -> RpcError {
    match error {
        SettingsError::Conflict => RpcError::app(
            -32000,
            "configuration_conflict",
            "Configuration changed; read its current revision",
        ),
        SettingsError::Invalid(_) => RpcError::invalid("Invalid configuration document for source"),
        SettingsError::Closed | SettingsError::Cancelled => RpcError::reloading(),
        _ => RpcError::app(
            -32000,
            "configuration_failed",
            "Configuration operation failed; read current state before retrying",
        ),
    }
}

pub struct ConfigurationManagement;
impl TypedPlugin for ConfigurationManagement {
    type Deps = (Arc<Methods>, Arc<SettingsService>, Arc<UserConfiguration>);
    fn name(&self) -> &str {
        "ConfigurationManagement"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (management, settings, owner): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let mut persistence = owner.subscribe();
            let registration = management.register(
                "configuration",
                ["configuration.catalog", "configuration.get", "configuration.update", "configuration.flush"].into_iter().map(|name| MethodDecl::management(name, matches!(name, "configuration.update" | "configuration.flush"), Interruption::Drain)).collect(),
                vec!["configuration.changed".into()],
                Arc::new(move |method, value, _stopping| {
                    let settings = settings.clone();
                    let owner = owner.clone();
                    Box::pin(async move {
                        match method.as_str() {
                            "configuration.catalog" => {
                                let _: Empty = params(value)?;
                                reply(ConfigurationCatalog { entries: owner.catalog().map_err(error)?, persistence: owner.status() })
                            }
                            "configuration.get" => {
                                let p: ConfigurationTarget = params(value)?;
                                reply(owner.get(p.source.into()).map_err(error)?)
                            }
                            "configuration.update" => {
                                let p: ConfigurationUpdate = params(value)?;
                                let source = p.source.into();
                                // Accepted edits drain on unload; client delivery and
                                // interactive-wait cancellation are not mutation cancellation.
                                let cancel = CancellationToken::new();
                                let saved = match settings.update(source, &p.expected_revision, p.document, &cancel).await {
                                    Ok(_) => true,
                                    Err(SettingsError::AppliedUnsaved) => false,
                                    Err(failure) => return Err(error(failure)),
                                };
                                reply(ConfigurationMutation { saved, current: owner.get(source).map_err(error)?, persistence: owner.status() })
                            }
                            "configuration.flush" => {
                                let _: Empty = params(value)?;
                                owner.flush().await.map_err(|_| RpcError::app(-32000, "configuration_unsaved", "Save retry failed; read current configuration and persistence status"))?;
                                reply(owner.status())
                            }
                            _ => unreachable!("registered configuration method"),
                        }
                    })
                }),
            ).map_err(|error| CordisError::PluginFailed(error.message.into()))?;
            let notifications = registration.notifications();
            let stopped = CancellationToken::new();
            let stopping = stopped.clone();
            let tasks = TaskTracker::new();
            let draining = tasks.clone();
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        stopping.cancel();
                        registration.unregister_and_drain().await;
                        draining.close();
                        draining.wait().await;
                        Ok(())
                    })
                }))
            })?;
            let (changed, mut loader) = tokio::sync::watch::channel(());
            ctx.events().on(
                ctx,
                &EventKey::<rutis_loader::LoaderChanged>::of(),
                Changes(changed),
            )?;
            tasks.spawn(async move {
                loop {
                    tokio::select! {
                        biased;
                        _ = stopped.cancelled() => break,
                        changed = loader.changed() => if changed.is_err() { break },
                        changed = persistence.changed() => if changed.is_err() { break },
                    }
                    // Invalidation describes live intent and persistence status,
                    // not acknowledgement of runtime adoption or durability.
                    notifications.notify("configuration.changed", serde_json::json!({}));
                }
            });
            Ok(Effect::Done)
        })
    }
}
impl Capability for ConfigurationManagement {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&())
    }
}
