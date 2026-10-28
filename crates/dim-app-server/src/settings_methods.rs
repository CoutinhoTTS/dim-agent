//! The client methods `settings.*` over declared sources (settings-service
//! §4.3). Settings stays persistence infrastructure: Rpc reaches it through
//! Agents and may not reach a management owner (methods contract §3.2), so
//! this owner depends on Settings, as PermissionRules does for `permission`.
use crate::{
    methods::ManagementMethods,
    settings::{Read, SettingsError, SettingsService, Update, Writer},
};
use dim_agent_capabilities::{
    file_storage::FileStorageError,
    methods::{Interruption, MethodDecl, RpcError, params, reply},
};
use dim_agent_runtime::Names;
use rutis::Ctx;
use rutis::host::{BoxFuture, CtxHost, DynPlugin, Effect, Key, TypeKey};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Fill {
    source: String,
    document: Value,
}

/// The client path's wire errors (§4.3).
fn wire(error: SettingsError) -> RpcError {
    let kind = match &error {
        SettingsError::InvalidScope(_) | SettingsError::Document(_) => {
            return RpcError::invalid(error.to_string());
        }
        SettingsError::Closed => return RpcError::reloading(),
        SettingsError::Cancelled => "operation_interrupted",
        SettingsError::UnknownSource => "settings_unknown_source",
        SettingsError::Conflict => "settings_conflict",
        SettingsError::Storage(FileStorageError::Unsynced(_)) => "settings_unsynced",
        _ => "settings_failed",
    };
    RpcError::app(-32000, kind, error.to_string())
}

/// Process owner of the `settings` management namespace. A client write may
/// change user-only fields.
pub struct SettingsMethodsPlugin {
    service: Key<SettingsService>,
    methods: Key<ManagementMethods>,
}
impl SettingsMethodsPlugin {
    pub fn new(names: &Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            service: names.key()?,
            methods: names.key()?,
        })
    }
}
impl DynPlugin for SettingsMethodsPlugin {
    fn name(&self) -> &str {
        "SettingsMethods"
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![self.service.erase(), self.methods.erase()]
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let service = ctx.require_at(&self.service)?;
            let methods = ctx.require_at(&self.methods)?;
            methods.register(
                ctx,
                "settings",
                vec![
                    MethodDecl::new("settings.read", false, Interruption::Report),
                    MethodDecl::new("settings.update", true, Interruption::Report),
                    MethodDecl::new("settings.fill", true, Interruption::Report),
                ],
                Arc::new(move |method, value, stopping, _| {
                    let service = service.clone();
                    Box::pin(async move {
                        match method.as_str() {
                            "settings.fill" => {
                                let Fill { source, document } = params(value)?;
                                reply(
                                    service
                                        .fill_missing(source, document, &stopping)
                                        .await
                                        .map_err(wire)?,
                                )
                            }
                            "settings.read" => {
                                let Read { source, scope } = params(value)?;
                                reply(
                                    service
                                        .read_declared(&source, scope, &stopping)
                                        .await
                                        .map_err(wire)?,
                                )
                            }
                            "settings.update" => {
                                let update: Update = params(value)?;
                                reply(
                                    service
                                        .update_declared(
                                            &update.source,
                                            update.scope,
                                            &update.expected_revision,
                                            update.document,
                                            Writer::Client,
                                            &stopping,
                                        )
                                        .await
                                        .map_err(wire)?,
                                )
                            }
                            _ => unreachable!("registered settings management method"),
                        }
                    })
                }),
            )?;
            Ok(Effect::Done)
        })
    }
}
