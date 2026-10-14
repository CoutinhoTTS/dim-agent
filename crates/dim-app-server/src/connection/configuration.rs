//! Protected typed configuration management, not package installation.
use super::{ConnectionHub, RpcService};
use crate::{protocol::*, settings::SettingsError};
use rutis::{BoxFuture, CordisError, Ctx, EventKey, Listener};
use std::sync::{Arc, Weak};

pub(super) async fn management(rpc: &RpcService, call: Call) -> Result<Reply, RpcError> {
    let owner = rpc
        .configuration
        .as_ref()
        .expect("declared execution dependency");
    Ok(match call {
        Call::ConfigurationCatalog => Reply::ConfigurationCatalog(ConfigurationCatalog {
            entries: owner.catalog().map_err(error)?,
            persistence: owner.status(),
        }),
        Call::ConfigurationGet(params) => {
            Reply::ConfigurationInfo(owner.get(params.source.into()).map_err(error)?)
        }
        Call::ConfigurationUpdate(params) => {
            let source = params.source.into();
            let result = rpc
                .settings
                .as_ref()
                .expect("declared execution dependency")
                .update(
                    source,
                    &params.expected_revision,
                    params.document,
                    &rpc.unloading,
                )
                .await;
            let saved = match result {
                Ok(_) => true,
                Err(SettingsError::AppliedUnsaved) => false,
                Err(failure) => return Err(error(failure)),
            };
            Reply::ConfigurationMutation(ConfigurationMutation {
                saved,
                current: owner.get(source).map_err(error)?,
                persistence: owner.status(),
            })
        }
        Call::ConfigurationFlush => {
            owner.flush().await.map_err(|_| {
                RpcError::app(
                    -32000,
                    "configuration_unsaved",
                    "Save retry failed; read current configuration and persistence status",
                )
            })?;
            Reply::ConfigurationPersistence(owner.status())
        }
        _ => unreachable!("configuration methods are routed explicitly"),
    })
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

struct Changes(tokio::sync::watch::Sender<()>);
impl Listener<rutis_loader::LoaderChanged> for Changes {
    fn call<'a>(
        &'a self,
        _: &'a Ctx,
        _: &'a rutis_loader::LoaderChanged,
    ) -> BoxFuture<'a, Result<Option<()>, CordisError>> {
        // Never expose raw rows/errors or perform loader mutations in a listener.
        self.0.send_replace(());
        Box::pin(async { Ok(None) })
    }
}

pub(super) fn subscribe(
    ctx: &Ctx,
    hub: &Arc<ConnectionHub>,
    rpc: &Arc<RpcService>,
) -> Result<(), CordisError> {
    let Some(configuration) = &rpc.configuration else {
        return Ok(());
    };
    let mut persistence = configuration.subscribe();
    let (changed, mut loader) = tokio::sync::watch::channel(());
    ctx.events().on(
        ctx,
        &EventKey::<rutis_loader::LoaderChanged>::of(),
        Changes(changed),
    )?;
    let hub = hub.clone();
    let instance = rpc.index.server_instance_id().to_owned();
    let stopped = rpc.unloading.clone();
    rpc.commands.spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = stopped.cancelled() => break,
                changed = loader.changed() => if changed.is_err() { break },
                changed = persistence.changed() => if changed.is_err() { break },
            }
            // Coalesced invalidation, including the final save result. It neither
            // acknowledges durability nor reports successful runtime adoption.
            for connection in hub
                .connections
                .lock()
                .expect("connection index")
                .values()
                .filter_map(Weak::upgrade)
            {
                if connection.local
                    && connection.version.get() == Some(&3)
                    && connection.instance.get() == Some(&instance)
                    && !connection.closed.is_cancelled()
                {
                    let _ = connection.sender.send(
                        Outbound::notice(Notification::ConfigurationChanged(Empty {})),
                        false,
                    );
                }
            }
        }
    });
    Ok(())
}
