//! Execution settings source and the read-only host port.
//!
//! The source is a declared Settings document (`execution.json`, user scope)
//! with the two switches. The App Server owns the source; HostPlatform reads
//! the port at process preparation.
use crate::settings::{SettingsScope, SettingsService, SourceDeclaration};
use dim_agent_capabilities::execution_settings::{
    ExecutionSettings, ExecutionSettingsError, ExecutionSettingsHost, ExecutionSettingsSource,
    SOURCE,
};
use dim_agent_runtime::Names;
use rutis::Ctx;
use rutis::host::{BoxFuture, CtxHost, DynPlugin, Effect, Key, TypeKey};
use serde_json::json;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

struct Storage {
    settings: Arc<SettingsService>,
}

impl ExecutionSettingsSource for Storage {
    fn read(
        &self,
        cancel: CancellationToken,
    ) -> BoxFuture<'_, Result<ExecutionSettings, ExecutionSettingsError>> {
        Box::pin(async move {
            let snapshot = self
                .settings
                .read_declared(SOURCE, SettingsScope::User, &cancel)
                .await
                .map_err(|error| ExecutionSettingsError::Failed(error.to_string()))?;
            Ok(ExecutionSettings {
                sandbox: snapshot
                    .document
                    .get("sandbox")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                network: snapshot
                    .document
                    .get("network")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
            })
        })
    }
}

pub struct ExecutionSettingsPlugin {
    settings: Key<SettingsService>,
}

impl ExecutionSettingsPlugin {
    pub fn new(names: &Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            settings: names.key()?,
        })
    }
}

impl DynPlugin for ExecutionSettingsPlugin {
    fn name(&self) -> &str {
        "ExecutionSettings"
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![self.settings.erase()]
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let settings = ctx.require_at(&self.settings)?;
            settings.register_declared(
                ctx,
                SourceDeclaration {
                    source: SOURCE.into(),
                    user_file: "execution.json".into(),
                    project_file: None,
                    schema: json!({
                        "type": "object",
                        "properties": {
                            "sandbox": {"type": "boolean", "default": false},
                            "network": {"type": "boolean", "default": false}
                        },
                        "additionalProperties": false
                    }),
                    user_only: Vec::new(),
                },
            )?;
            let host = ExecutionSettingsHost(Arc::new(Storage { settings }));
            ctx.provide_as::<ExecutionSettingsHost>(Arc::new(host))?;
            Ok(Effect::Done)
        })
    }
}
