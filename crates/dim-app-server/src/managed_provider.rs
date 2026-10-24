//! Process-owned access to the managed-provider service and its typed contract.
pub mod management;
use crate::model_catalog::ModelEntry;
use dim_agent_capabilities::host::ModelProtocol;
use dim_agent_runtime::{Capability, KeysOf};
use management::{
    BrowserLogin, BrowserLoginBegin, DeviceLogin, LoginBegin, LoginState,
    ManagedBrowserLoginComplete, ManagedLoginAttempt, Usage,
};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypeKey, TypedPlugin};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};
use tokio_util::sync::CancellationToken;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
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
#[derive(Clone)]
pub struct ManagedCatalog {
    pub account: AccountSnapshot,
    pub models: Vec<ManagedModel>,
}

/// Canonical managed-provider failures. Only fixed public diagnostics derived
/// from them cross the client boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManagedError {
    InvalidStore,
    StorageError,
    UnsupportedSchema,
    AccountRecoveryRequired,
    AccountResetBlocked,
    AccountChanged,
    IdentityRejected,
    TokenMissing,
    RefreshRejected,
    Transient,
    ConfigError,
    InvalidResponse,
    InvalidCallback,
    AttemptNotFound,
    AttemptClosed,
    ServiceReloading,
    InvalidInput,
    UnknownModel,
    LeaseNotFound,
    LeaseCapacity,
}

/// Releases a provider's lease when the last holder drops it.
pub trait LeaseOwner: Send + Sync {
    fn release(&self, lease_id: &str);
}
struct LeaseHandle {
    id: String,
    owner: Weak<dyn LeaseOwner>,
}
impl Drop for LeaseHandle {
    fn drop(&mut self) {
        if let Some(owner) = self.owner.upgrade() {
            owner.release(&self.id);
        }
    }
}
/// A one-request credential. Secrets are internal: never Debug or serialized.
pub struct CredentialLease {
    pub account: AccountSnapshot,
    pub protocol: ModelProtocol,
    pub base_url: String,
    pub headers: HashMap<String, String>,
    pub bearer: String,
    handle: LeaseHandle,
    /// The binding that issued the lease, stamped by `ManagedProviders`:
    /// recovery and final rejection go to that binding only.
    binding: u64,
}
impl CredentialLease {
    pub fn new(
        account: AccountSnapshot,
        protocol: ModelProtocol,
        base_url: String,
        headers: HashMap<String, String>,
        bearer: String,
        id: String,
        owner: Weak<dyn LeaseOwner>,
    ) -> Self {
        Self {
            account,
            protocol,
            base_url,
            headers,
            bearer,
            handle: LeaseHandle { id, owner },
            binding: 0,
        }
    }
    pub fn id(&self) -> &str {
        &self.handle.id
    }
}

pub type ManagedFuture<T> = BoxFuture<'static, Result<T, ManagedError>>;

/// Registers the process row of a managed provider plugin built outside this
/// repository, given the product root's FileStorage index, and returns the
/// row name. The row provides `Arc<dyn ManagedProvider>`.
#[derive(Clone)]
pub struct ManagedProviderRow(pub Arc<RegisterRow>);
pub type RegisterRow = dyn Fn(&mut rutis_loader::Builtins, usize) -> String + Send + Sync;
impl std::fmt::Debug for ManagedProviderRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ManagedProviderRow")
    }
}

/// The managed-provider service. Implementations own every accepted
/// operation: a caller dropping a returned future never abandons a token
/// rotation or a persisted login.
pub trait ManagedProvider: Send + Sync + 'static {
    fn providers(&self) -> Vec<ProviderDescriptor>;
    fn status(&self, provider: &str) -> ManagedFuture<AccountSnapshot>;
    fn recover(&self, provider: &str) -> ManagedFuture<AccountSnapshot>;
    fn begin_browser(&self, login: BrowserLoginBegin) -> ManagedFuture<BrowserLogin>;
    fn complete_browser(
        &self,
        login: ManagedBrowserLoginComplete,
    ) -> ManagedFuture<AccountSnapshot>;
    fn begin_device(&self, login: LoginBegin) -> ManagedFuture<DeviceLogin>;
    fn attempt_status(&self, attempt: &ManagedLoginAttempt) -> Result<LoginState, ManagedError>;
    fn cancel_attempt(&self, attempt: ManagedLoginAttempt) -> ManagedFuture<()>;
    fn logout(&self, provider: &str) -> ManagedFuture<AccountSnapshot>;
    fn usage(&self, provider: &str) -> ManagedFuture<Usage>;
    fn catalog(&self, provider: &str) -> ManagedFuture<ManagedCatalog>;
    fn credential(&self, provider: &str, model: &str) -> ManagedFuture<CredentialLease>;
    /// One controlled recovery after the relay rejected `lease`.
    fn recover_rejected(&self, lease: &CredentialLease) -> ManagedFuture<CredentialLease>;
    /// Settles a rejection of the recovered credential.
    fn finish_rejected(&self, lease: &CredentialLease) -> ManagedFuture<()>;
    /// Stops admission and cancels lock waits and login attempts, without
    /// abandoning a refresh that already holds the account lock.
    fn quiesce(&self) -> BoxFuture<'static, ()>;
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
impl From<ManagedError> for ManagedProviderError {
    fn from(error: ManagedError) -> Self {
        let (code, message) = match error {
            ManagedError::AccountChanged => ("account_changed", "Managed account changed"),
            ManagedError::TokenMissing | ManagedError::RefreshRejected => {
                ("account_required", "Provider login is required")
            }
            ManagedError::InvalidStore => {
                ("account_invalid_store", "Managed account store is invalid")
            }
            ManagedError::UnsupportedSchema => (
                "account_unsupported_schema",
                "Managed account schema is unsupported",
            ),
            ManagedError::AccountRecoveryRequired => (
                "account_recovery_required",
                "Managed account requires explicit recovery",
            ),
            ManagedError::AccountResetBlocked => {
                ("account_reset_blocked", "Managed account reset is blocked")
            }
            ManagedError::IdentityRejected => (
                "account_identity_rejected",
                "Managed account identity was rejected",
            ),
            ManagedError::InvalidInput
            | ManagedError::InvalidCallback
            | ManagedError::ConfigError => {
                ("account_invalid_input", "Managed account input is invalid")
            }
            ManagedError::AttemptNotFound => ("login_attempt_unknown", "Login attempt is unknown"),
            ManagedError::AttemptClosed => ("login_attempt_closed", "Login attempt is closed"),
            ManagedError::InvalidResponse => (
                "account_invalid_response",
                "Managed provider returned an invalid response",
            ),
            ManagedError::ServiceReloading => {
                ("service_reloading", "Managed provider service is reloading")
            }
            ManagedError::StorageError
            | ManagedError::Transient
            | ManagedError::UnknownModel
            | ManagedError::LeaseNotFound
            | ManagedError::LeaseCapacity => {
                ("account_unavailable", "Managed account operation failed")
            }
        };
        Self { code, message }
    }
}

/// Process-owned access to the bound provider. Without a binding the process
/// has no managed providers.
pub struct ManagedProviders {
    provider: Mutex<Option<Bound>>,
    stop: CancellationToken,
}
/// A published provider and the identity of the binding that published it.
/// A replacement, even of an equal provider, is a new binding.
#[derive(Clone)]
struct Bound {
    provider: Arc<dyn ManagedProvider>,
    binding: u64,
}
static NEXT_BINDING: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
fn retired() -> ManagedProviderError {
    ManagedProviderError {
        code: "service_reloading",
        message: "Managed provider service is reloading",
    }
}
impl ManagedProviders {
    /// Called before Agent/RPC settlement, not only during plugin disposal.
    pub async fn quiesce(&self) {
        self.stop.cancel();
        let bound = self.provider.lock().unwrap().clone();
        if let Some(bound) = bound {
            bound.provider.quiesce().await;
        }
    }
    fn bound(&self, cancel: &CancellationToken) -> Result<Bound, ManagedProviderError> {
        if self.stop.is_cancelled() || cancel.is_cancelled() {
            return Err(unavailable());
        }
        self.provider
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(unavailable)
    }
    fn provider(
        &self,
        cancel: &CancellationToken,
    ) -> Result<Arc<dyn ManagedProvider>, ManagedProviderError> {
        Ok(self.bound(cancel)?.provider)
    }
    /// The binding that issued `lease`, if it is still the published one.
    fn issuer(
        &self,
        lease: &CredentialLease,
        cancel: &CancellationToken,
    ) -> Result<Bound, ManagedProviderError> {
        let bound = self.bound(cancel)?;
        if bound.binding != lease.binding {
            return Err(retired());
        }
        Ok(bound)
    }
    pub async fn providers(
        &self,
        cancel: &CancellationToken,
    ) -> Result<Vec<ProviderDescriptor>, ManagedProviderError> {
        let Some(Bound { provider, .. }) = self.provider.lock().unwrap().clone() else {
            return Ok(vec![]);
        };
        if cancel.is_cancelled() {
            return Err(unavailable());
        }
        Ok(provider.providers())
    }
    pub async fn contains(
        &self,
        id: &str,
        cancel: &CancellationToken,
    ) -> Result<bool, ManagedProviderError> {
        Ok(self
            .providers(cancel)
            .await?
            .iter()
            .any(|p| p.provider_id == id))
    }
    pub async fn status(
        &self,
        id: &str,
        cancel: &CancellationToken,
    ) -> Result<AccountSnapshot, ManagedProviderError> {
        Ok(self.provider(cancel)?.status(id).await?)
    }
    pub async fn recover_account(
        &self,
        id: &str,
        cancel: &CancellationToken,
    ) -> Result<AccountSnapshot, ManagedProviderError> {
        Ok(self.provider(cancel)?.recover(id).await?)
    }
    pub async fn begin_browser(
        &self,
        login: BrowserLoginBegin,
        cancel: &CancellationToken,
    ) -> Result<BrowserLogin, ManagedProviderError> {
        Ok(self.provider(cancel)?.begin_browser(login).await?)
    }
    pub async fn complete_browser(
        &self,
        login: ManagedBrowserLoginComplete,
        cancel: &CancellationToken,
    ) -> Result<AccountSnapshot, ManagedProviderError> {
        Ok(self.provider(cancel)?.complete_browser(login).await?)
    }
    pub async fn begin_device(
        &self,
        login: LoginBegin,
        cancel: &CancellationToken,
    ) -> Result<DeviceLogin, ManagedProviderError> {
        Ok(self.provider(cancel)?.begin_device(login).await?)
    }
    pub fn attempt_status(
        &self,
        attempt: &ManagedLoginAttempt,
        cancel: &CancellationToken,
    ) -> Result<LoginState, ManagedProviderError> {
        Ok(self.provider(cancel)?.attempt_status(attempt)?)
    }
    pub async fn cancel_attempt(
        &self,
        attempt: ManagedLoginAttempt,
        cancel: &CancellationToken,
    ) -> Result<(), ManagedProviderError> {
        Ok(self.provider(cancel)?.cancel_attempt(attempt).await?)
    }
    pub async fn logout(
        &self,
        id: &str,
        cancel: &CancellationToken,
    ) -> Result<AccountSnapshot, ManagedProviderError> {
        Ok(self.provider(cancel)?.logout(id).await?)
    }
    pub async fn usage(
        &self,
        id: &str,
        cancel: &CancellationToken,
    ) -> Result<Usage, ManagedProviderError> {
        Ok(self.provider(cancel)?.usage(id).await?)
    }
    pub async fn catalog(
        &self,
        id: &str,
        cancel: &CancellationToken,
    ) -> Result<ManagedCatalog, ManagedProviderError> {
        Ok(self.provider(cancel)?.catalog(id).await?)
    }
    pub async fn credential(
        &self,
        id: &str,
        model: &str,
        cancel: &CancellationToken,
    ) -> Result<CredentialLease, ManagedProviderError> {
        let bound = self.bound(cancel)?;
        let mut lease = bound.provider.credential(id, model).await?;
        lease.binding = bound.binding;
        Ok(lease)
    }
    pub async fn recover(
        &self,
        lease: &CredentialLease,
        cancel: &CancellationToken,
    ) -> Result<CredentialLease, ManagedProviderError> {
        let bound = self.issuer(lease, cancel)?;
        let mut next = bound.provider.recover_rejected(lease).await?;
        next.binding = bound.binding;
        if next.account != lease.account {
            return Err(ManagedProviderError {
                code: "account_changed",
                message: "Managed account changed during recovery",
            });
        }
        Ok(next)
    }
    pub async fn finish_rejected(
        &self,
        lease: &CredentialLease,
        cancel: &CancellationToken,
    ) -> Result<(), ManagedProviderError> {
        Ok(self
            .issuer(lease, cancel)?
            .provider
            .finish_rejected(lease)
            .await?)
    }
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
                provider: Mutex::new(None),
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
            ctx.provide_as(TypeKey::of::<ManagedProviders>(), service)?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for ManagedProvidersPlugin {
    fn keys(&self) -> KeysOf<Self> {}
}

/// Publishes a provider to ManagedProviders while both are active.
pub struct ManagedProviderBinding;
impl TypedPlugin for ManagedProviderBinding {
    type Deps = (Arc<ManagedProviders>, Arc<dyn ManagedProvider>);
    fn name(&self) -> &str {
        "ManagedProviderBinding"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (owner, provider): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let binding = NEXT_BINDING.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            *owner.provider.lock().unwrap() = Some(Bound {
                provider: provider.clone(),
                binding,
            });
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        {
                            let mut slot = owner.provider.lock().unwrap();
                            if slot.as_ref().is_some_and(|bound| bound.binding == binding) {
                                *slot = None;
                            }
                        }
                        // The provider's own disposer joins its accepted work.
                        provider.quiesce().await;
                        Ok(())
                    })
                }))
            })?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for ManagedProviderBinding {
    fn keys(&self) -> KeysOf<Self> {
        ((), ())
    }
}
