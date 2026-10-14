//! Process-owned bridge to an optional managed-provider rutis service.
pub(crate) mod management;
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
    binding: Binding,
    provider_id: String,
    lease_id: String,
    // A delivered or buffered result still belongs to the issuing generation.
    // Drop enqueues release before this token can let its drain finish.
    _owner: tokio_util::task::task_tracker::TaskTrackerToken,
}
impl Drop for LeaseRelease {
    fn drop(&mut self) {
        let dispatch = self.binding.dispatch.clone();
        let args = json!({"providerId":self.provider_id,"leaseId":self.lease_id});
        self.binding.tasks.spawn(async move {
            if let Ok(RpcValue::Reference(reference)) =
                dispatch.invoke("release", json!([args]).into())
                && reference.is_future()
            {
                let _ = reference.wait_async().await;
            }
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
impl Binding {
    async fn quiesce(&self) {
        // Keep the original dispatch alive. Provider disposal happens only
        // after this consumer drains; it cannot be our cancellation trigger.
        if let Ok(RpcValue::Reference(reference)) =
            self.dispatch.invoke("quiesce", json!([]).into())
            && reference.is_future()
        {
            let _ = reference.wait_async().await;
        }
    }
}
pub struct ManagedProviders {
    binding: Mutex<Option<Binding>>,
    providers: Mutex<Vec<ProviderDescriptor>>,
    stop: CancellationToken,
}
impl ManagedProviders {
    /// Called before Agent/RPC settlement, not only during plugin disposal.
    pub async fn quiesce(&self) {
        self.stop.cancel();
        let binding = self.binding.lock().unwrap().clone();
        if let Some(binding) = binding {
            binding.quiesce().await;
        }
    }

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
        self.call_adopt(method, args, generation, cancel, |value, _| Ok(value))
            .await
    }
    async fn call_adopt<T: DeserializeOwned + Send + 'static, R: Send + 'static>(
        self: &Arc<Self>,
        method: &'static str,
        args: Value,
        generation: uuid::Uuid,
        cancel: &CancellationToken,
        adopt: impl FnOnce(T, Binding) -> Result<R, ManagedProviderError> + Send + 'static,
    ) -> Result<R, ManagedProviderError> {
        let (send, receive) = tokio::sync::oneshot::channel();
        {
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
            let accepted = binding.clone();
            let owner = self.clone();
            binding.tasks.spawn(async move {
                let result = async move {
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
                        let value = serde_json::from_value(
                            result.get("value").cloned().ok_or_else(invalid)?,
                        )
                        .map_err(|_| invalid())?;
                        adopt(value, accepted)
                    } else if result.get("ok") == Some(&Value::Bool(false)) {
                        Err(plugin_error(
                            serde_json::from_value(
                                result.get("error").cloned().ok_or_else(invalid)?,
                            )
                            .map_err(|_| invalid())?,
                        ))
                    } else {
                        Err(invalid())
                    }
                }
                .await;
                let slot = owner.binding.lock().unwrap();
                let result = if slot
                    .as_ref()
                    .is_some_and(|binding| binding.generation == generation)
                {
                    result
                } else {
                    // Reload revoked the handoff. Drop any adopted lease while
                    // its original generation is still draining this task.
                    drop(result);
                    Err(unavailable())
                };
                // A dropped waiter returns ownership here, before this tracked
                // task ends; its release joins the same generation's drain.
                let _ = send.send(result);
            });
        }
        let result = receive.await.map_err(|_| unavailable())?;
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
        let id = id.to_owned();
        self.call_adopt(
            "credential",
            json!({"providerId":id,"modelId":model}),
            binding.generation,
            cancel,
            move |lease, binding| Self::lease(&id, binding, lease),
        )
        .await
    }
    fn lease(
        id: &str,
        binding: Binding,
        value: Value,
    ) -> Result<CredentialLease, ManagedProviderError> {
        // Adopt before validation so every issued lease has an owner, even
        // when validation fails or the awaiting caller disappears.
        let lease_id = value
            .get("leaseId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(invalid)?
            .to_owned();
        let release = Arc::new(LeaseRelease {
            _owner: binding.tasks.token(),
            binding,
            provider_id: id.into(),
            lease_id,
        });
        let lease: WireLease = serde_json::from_value(value).map_err(|_| invalid())?;
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
            release,
        })
    }
    pub async fn recover(
        self: &Arc<Self>,
        lease: &CredentialLease,
        cancel: &CancellationToken,
    ) -> Result<CredentialLease, ManagedProviderError> {
        let release = &lease.release;
        let id = release.provider_id.clone();
        let account = lease.account.clone();
        self.call_adopt(
            "recoverRejected",
            json!({"providerId":release.provider_id,"leaseId":release.lease_id}),
            release.binding.generation,
            cancel,
            move |wire, binding| {
                let next = Self::lease(&id, binding, wire)?;
                if next.account != account {
                    return Err(ManagedProviderError {
                        code: "account_changed",
                        message: "Managed account changed during recovery",
                    });
                }
                Ok(next)
            },
        )
        .await
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
                release.binding.generation,
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
            });
            let owner = service.clone();
            ctx.effect(move || {
                Effect::Disposer(Box::new(move || {
                    owner.stop.cancel();
                    Ok(())
                }))
            })?;
            management::provide(ctx, service.clone())?;
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
            let binding = Binding {
                generation,
                dispatch,
                tasks: tasks.clone(),
            };
            *owner.binding.lock().unwrap() = Some(binding.clone());
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
                        binding.quiesce().await;
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
