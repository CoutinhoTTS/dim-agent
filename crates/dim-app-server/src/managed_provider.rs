//! Process-owned bridge to an optional managed-provider rutis service.
use crate::model_catalog::{MAX_MODEL_TOKENS, ModelEntry};
use dim_agent_capabilities::host::ModelProtocol;
use dim_agent_runtime::{Capability, KeysOf};
use rutis::{BoxFuture, CordisError, Ctx, DepKey, Effect, Keyed, TypedPlugin};
use rutis_bridge::session::{HostDispatch, Value as RpcValue};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[derive(Clone, Debug)]
pub struct ManagedPluginConfig {
    pub plugin_dir: PathBuf,
    pub node: PathBuf,
    pub js_runtime: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderDescriptor {
    pub provider_id: String,
    pub display_name: String,
    pub login_methods: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountSnapshot {
    pub credential_epoch: String,
    pub generation: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<AccountView>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountView {
    pub issuer: String,
    pub subject: String,
    pub granted_scopes: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedModel {
    pub entry: ModelEntry,
    pub protocol: ModelProtocol,
    pub unavailable_reason: Option<String>,
    pub zdr: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedCatalog {
    pub account: AccountSnapshot,
    pub models: Vec<ManagedModel>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireLease {
    lease_id: String,
    account: AccountSnapshot,
    protocol: ModelProtocol,
    base_url: String,
    headers: HashMap<String, String>,
    bearer: String,
}
// Secrets are internal, never Debug or serialized through terminal RPC.
pub struct CredentialLease {
    pub account: AccountSnapshot,
    pub protocol: ModelProtocol,
    pub base_url: String,
    pub headers: HashMap<String, String>,
    pub bearer: String,
    release: Arc<LeaseRelease>,
}
struct LeaseRelease {
    owner: Arc<ManagedProviders>,
    generation: uuid::Uuid,
    provider_id: String,
    lease_id: String,
}
impl Drop for LeaseRelease {
    fn drop(&mut self) {
        let owner = self.owner.clone();
        let generation = self.generation;
        let args = json!({"providerId":self.provider_id,"leaseId":self.lease_id});
        let tasks = owner.tasks.clone();
        tasks.spawn(async move {
            let _ = owner
                .call_generation::<Value>("release", args, generation, &CancellationToken::new())
                .await;
        });
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginError {
    code: String,
    message: String,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct ManagedProviderError {
    pub code: &'static str,
    pub message: &'static str,
}
fn unavailable() -> ManagedProviderError {
    ManagedProviderError {
        code: "account_provider_unavailable",
        message: "Managed provider plugin is unavailable",
    }
}
fn invalid() -> ManagedProviderError {
    ManagedProviderError {
        code: "account_invalid_response",
        message: "Managed provider returned an invalid response",
    }
}
fn plugin_error(error: PluginError) -> ManagedProviderError {
    // Only fixed public diagnostics cross the client boundary.
    let _ = error.message;
    match error.code.as_str() {
        "account_changed" => ManagedProviderError {
            code: "account_changed",
            message: "Managed account changed",
        },
        "token_missing" | "refresh_rejected" => ManagedProviderError {
            code: "account_required",
            message: "Provider login is required",
        },
        "invalid_store" => ManagedProviderError {
            code: "account_invalid_store",
            message: "Managed account store is invalid",
        },
        "unsupported_schema" => ManagedProviderError {
            code: "account_unsupported_schema",
            message: "Managed account schema is unsupported",
        },
        "account_recovery_required" => ManagedProviderError {
            code: "account_recovery_required",
            message: "Managed account requires explicit recovery",
        },
        "account_reset_blocked" => ManagedProviderError {
            code: "account_reset_blocked",
            message: "Managed account reset is blocked",
        },
        "identity_rejected" => ManagedProviderError {
            code: "account_identity_rejected",
            message: "Managed account identity was rejected",
        },
        "invalid_input" | "invalid_callback" | "config_error" => ManagedProviderError {
            code: "account_invalid_input",
            message: "Managed account input is invalid",
        },
        "attempt_not_found" => ManagedProviderError {
            code: "login_attempt_unknown",
            message: "Login attempt is unknown",
        },
        "attempt_closed" => ManagedProviderError {
            code: "login_attempt_closed",
            message: "Login attempt is closed",
        },
        "invalid_response" => invalid(),
        "service_reloading" => ManagedProviderError {
            code: "service_reloading",
            message: "Managed provider service is reloading",
        },
        _ => ManagedProviderError {
            code: "account_unavailable",
            message: "Managed account operation failed",
        },
    }
}
#[derive(Clone)]
struct Binding {
    generation: uuid::Uuid,
    dispatch: Arc<dyn HostDispatch>,
    tasks: TaskTracker,
}
pub struct ManagedProviders {
    binding: Mutex<Option<Binding>>,
    providers: Mutex<Vec<ProviderDescriptor>>,
    stop: CancellationToken,
    tasks: TaskTracker,
}
impl ManagedProviders {
    fn binding(&self) -> Result<Binding, ManagedProviderError> {
        self.binding.lock().unwrap().clone().ok_or_else(unavailable)
    }
    async fn call_generation<T: DeserializeOwned + Send + 'static>(
        self: &Arc<Self>,
        method: &'static str,
        args: Value,
        generation: uuid::Uuid,
        cancel: &CancellationToken,
    ) -> Result<T, ManagedProviderError> {
        let task = {
            let slot = self.binding.lock().unwrap();
            let binding = slot.as_ref().ok_or_else(unavailable)?;
            if binding.generation != generation
                || binding.tasks.is_closed()
                || self.stop.is_cancelled()
                || cancel.is_cancelled()
            {
                return Err(unavailable());
            }
            let dispatch = binding.dispatch.clone();
            binding.tasks.spawn(async move {
                let args = if args.is_null() {
                    json!([])
                } else {
                    json!([args])
                };
                let result = dispatch
                    .invoke(method, args.into())
                    .map_err(|_| unavailable())?;
                let result = match result {
                    RpcValue::Reference(reference) if reference.is_future() => {
                        reference.wait_async().await.map_err(|_| unavailable())?
                    }
                    value => value,
                };
                let result = result.json().map_err(|_| invalid())?;
                // A boolean tag is decoded at this sole cross-language boundary.
                if result.get("ok") == Some(&Value::Bool(true)) {
                    serde_json::from_value(result.get("value").cloned().ok_or_else(invalid)?)
                        .map_err(|_| invalid())
                } else if result.get("ok") == Some(&Value::Bool(false)) {
                    Err(plugin_error(
                        serde_json::from_value(result.get("error").cloned().ok_or_else(invalid)?)
                            .map_err(|_| invalid())?,
                    ))
                } else {
                    Err(invalid())
                }
            })
        };
        let result = task.await.map_err(|_| unavailable())?;
        if self.binding()?.generation != generation {
            return Err(unavailable());
        }
        // Drain admitted calls: a caller disappearing must not discard a rotated token.
        result
    }
    pub async fn call<T: DeserializeOwned + Send + 'static>(
        self: &Arc<Self>,
        method: &'static str,
        args: Value,
        cancel: &CancellationToken,
    ) -> Result<T, ManagedProviderError> {
        let generation = self.binding()?.generation;
        self.call_generation(method, args, generation, cancel).await
    }
    pub async fn providers(
        self: &Arc<Self>,
        cancel: &CancellationToken,
    ) -> Result<Vec<ProviderDescriptor>, ManagedProviderError> {
        if self.binding.lock().unwrap().is_none() {
            return Ok(vec![]);
        }
        if cancel.is_cancelled() {
            return Err(unavailable());
        }
        Ok(self.providers.lock().unwrap().clone())
    }
    pub async fn contains(
        self: &Arc<Self>,
        id: &str,
        cancel: &CancellationToken,
    ) -> Result<bool, ManagedProviderError> {
        if cancel.is_cancelled() {
            return Err(unavailable());
        }
        Ok(self
            .providers
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.provider_id == id))
    }
    pub async fn status(
        self: &Arc<Self>,
        id: &str,
        cancel: &CancellationToken,
    ) -> Result<AccountSnapshot, ManagedProviderError> {
        let account: AccountSnapshot = self
            .call("status", json!({"providerId":id}), cancel)
            .await?;
        validate_account(&account)?;
        Ok(account)
    }
    pub async fn catalog(
        self: &Arc<Self>,
        id: &str,
        cancel: &CancellationToken,
    ) -> Result<ManagedCatalog, ManagedProviderError> {
        let catalog: ManagedCatalog = self
            .call("catalog", json!({"providerId":id}), cancel)
            .await?;
        validate_account(&catalog.account)?;
        let mut ids = std::collections::HashSet::new();
        if catalog.account.account.is_none()
            || catalog.models.iter().any(|m| {
                m.entry.model_id.is_empty()
                    || !ids.insert(&m.entry.model_id)
                    || m.entry.display_name.trim().is_empty()
                    || !supported(m.protocol)
                    || [
                        m.entry.capabilities.context_window,
                        m.entry.capabilities.max_output_tokens,
                    ]
                    .iter()
                    .any(|n| *n == 0 || *n > MAX_MODEL_TOKENS)
            })
        {
            return Err(invalid());
        }
        Ok(catalog)
    }
    pub async fn credential(
        self: &Arc<Self>,
        id: &str,
        model: &str,
        cancel: &CancellationToken,
    ) -> Result<CredentialLease, ManagedProviderError> {
        let binding = self.binding()?;
        let lease = self
            .call_generation(
                "credential",
                json!({"providerId":id,"modelId":model}),
                binding.generation,
                cancel,
            )
            .await?;
        self.lease(id, binding.generation, lease)
    }
    fn lease(
        self: &Arc<Self>,
        id: &str,
        generation: uuid::Uuid,
        lease: WireLease,
    ) -> Result<CredentialLease, ManagedProviderError> {
        validate_account(&lease.account)?;
        let url = url::Url::parse(&lease.base_url).map_err(|_| invalid())?;
        if lease.lease_id.is_empty()
            || lease.bearer.is_empty()
            || lease.account.account.is_none()
            || !supported(lease.protocol)
            || !matches!(url.scheme(), "https" | "http")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || lease
                .headers
                .iter()
                .any(|(k, v)| k.is_empty() || k.contains(['\r', '\n']) || v.contains(['\r', '\n']))
        {
            return Err(invalid());
        }
        Ok(CredentialLease {
            account: lease.account,
            protocol: lease.protocol,
            base_url: lease.base_url,
            headers: lease.headers,
            bearer: lease.bearer,
            release: Arc::new(LeaseRelease {
                owner: self.clone(),
                generation,
                provider_id: id.into(),
                lease_id: lease.lease_id,
            }),
        })
    }
    pub async fn recover(
        self: &Arc<Self>,
        lease: &CredentialLease,
        cancel: &CancellationToken,
    ) -> Result<CredentialLease, ManagedProviderError> {
        let release = &lease.release;
        let next = self
            .call_generation(
                "recoverRejected",
                json!({"providerId":release.provider_id,"leaseId":release.lease_id}),
                release.generation,
                cancel,
            )
            .await?;
        let next = self.lease(&release.provider_id, release.generation, next)?;
        if next.account != lease.account {
            return Err(ManagedProviderError {
                code: "account_changed",
                message: "Managed account changed during recovery",
            });
        }
        Ok(next)
    }
    pub async fn finish_rejected(
        self: &Arc<Self>,
        lease: &CredentialLease,
        cancel: &CancellationToken,
    ) -> Result<(), ManagedProviderError> {
        let release = &lease.release;
        let _: Value = self
            .call_generation(
                "finishRejected",
                json!({"providerId":release.provider_id,"leaseId":release.lease_id}),
                release.generation,
                cancel,
            )
            .await?;
        Ok(())
    }
}
fn validate_account(account: &AccountSnapshot) -> Result<(), ManagedProviderError> {
    if account.credential_epoch.is_empty()
        || account.generation > MAX_MODEL_TOKENS
        || account
            .account
            .as_ref()
            .is_some_and(|a| a.issuer.is_empty() || a.subject.is_empty())
    {
        return Err(invalid());
    }
    Ok(())
}
fn supported(protocol: ModelProtocol) -> bool {
    matches!(
        protocol,
        ModelProtocol::OpenaiCompatible | ModelProtocol::OpenaiResponses | ModelProtocol::Anthropic
    )
}

pub struct ManagedProvidersPlugin;
impl TypedPlugin for ManagedProvidersPlugin {
    type Deps = ();
    fn name(&self) -> &str {
        "ManagedProviders"
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx, (): ()) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let service = Arc::new(ManagedProviders {
                binding: Mutex::new(None),
                providers: Mutex::new(vec![]),
                stop: CancellationToken::new(),
                tasks: TaskTracker::new(),
            });
            let owner = service.clone();
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        owner.stop.cancel();
                        owner.tasks.close();
                        owner.tasks.wait().await;
                        Ok(())
                    })
                }))
            })?;
            ctx.provide_as(rutis::TypeKey::of::<ManagedProviders>(), service)?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for ManagedProvidersPlugin {
    fn keys(&self) -> KeysOf<Self> {}
}
pub struct ManagedProviderBinding;
impl TypedPlugin for ManagedProviderBinding {
    type Deps = (Arc<ManagedProviders>, Keyed<dyn HostDispatch>);
    fn name(&self) -> &str {
        "ManagedProviderBinding"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (owner, Keyed(dispatch)): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let generation = uuid::Uuid::new_v4();
            let tasks = TaskTracker::new();
            *owner.binding.lock().unwrap() = Some(Binding {
                generation,
                dispatch,
                tasks: tasks.clone(),
            });
            let cleanup = owner.clone();
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        {
                            let mut slot = cleanup.binding.lock().unwrap();
                            if slot.as_ref().is_some_and(|b| b.generation == generation) {
                                tasks.close();
                                *slot = None;
                            }
                        }
                        tasks.wait().await;
                        Ok(())
                    })
                }))
            })?;
            let providers: Vec<ProviderDescriptor> = owner
                .call("providers", Value::Null, &CancellationToken::new())
                .await
                .map_err(|e| CordisError::PluginFailed(e.to_string().into()))?;
            let mut ids = std::collections::HashSet::new();
            if providers.iter().any(|p| {
                p.provider_id.is_empty()
                    || !ids.insert(&p.provider_id)
                    || p.display_name.trim().is_empty()
                    || p.login_methods
                        .iter()
                        .any(|m| !matches!(m.as_str(), "browser" | "device"))
            }) {
                return Err(CordisError::PluginFailed(
                    "Invalid managed provider description".into(),
                ));
            }
            *owner.providers.lock().unwrap() = providers;
            Ok(Effect::Done)
        })
    }
}
impl Capability for ManagedProviderBinding {
    fn keys(&self) -> KeysOf<Self> {
        ((), DepKey::dynamic("managedProvider"))
    }
}
