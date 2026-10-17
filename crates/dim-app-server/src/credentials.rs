//! The `credentials` contract (settings-service §4.4): resolves the opaque
//! string of a `*Ref` field for the plugin whose document holds it.
//!
//! It is provided by the plugin that owns the host secret store. No product
//! plugin does today (design §10 question 4): `ProviderPlugin::default` runs
//! without it, and only tests supply a store. Values are resolved per use and
//! never written to Settings, the Timeline or a reply.
use rutis::{BoxFuture, CordisError, Ctx};
use rutis_bridge::session::{Error, HostDispatch, Reply, Value as RpcValue, host_key};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Messages never contain a value: a store's own diagnostics stay inside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CredentialError {
    #[error("credential not found")]
    NotFound,
    #[error("credential lookup failed")]
    Failed,
    #[error("credential lookup cancelled")]
    Cancelled,
}
impl CredentialError {
    fn code(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Implemented by the secret store. The future must be cancellation-safe; it
/// must not detach work.
pub trait ResolveCredential: Send + Sync + 'static {
    fn resolve(
        &self,
        owner: String,
        reference: String,
        cancel: CancellationToken,
    ) -> BoxFuture<'_, Result<String, CredentialError>>;
}

/// The store for Rust consumers (Provider).
pub struct CredentialsHost(pub Arc<dyn ResolveCredential>);
impl CredentialsHost {
    /// Provides a store to Rust consumers and, as the named service
    /// `credentials`, to every other language.
    pub fn provide(ctx: &Ctx, store: Arc<dyn ResolveCredential>) -> Result<(), CordisError> {
        ctx.provide_as::<dyn HostDispatch>(
            host_key("credentials"),
            Arc::new(Dispatch(store.clone())),
        )?;
        ctx.provide(Self(store))?;
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Resolve {
    owner: String,
    reference: String,
}

/// `resolve({owner, reference}) -> {value}`, with the `{ok, value}` /
/// `{ok: false, error: {code, message}}` envelope of the other host services.
struct Dispatch(Arc<dyn ResolveCredential>);
impl HostDispatch for Dispatch {
    fn methods(&self) -> Option<Value> {
        Some(json!({"resolve": "async"}))
    }
    fn invoke(&self, method: &str, args: RpcValue) -> Reply {
        let invalid = || Error::Value("expected one credentials request object".into());
        if method != "resolve" {
            return Err(Error::Value(format!("unknown credentials method {method}")));
        }
        let Value::Array(args) = args.json()? else {
            return Err(invalid());
        };
        let [request] = <[Value; 1]>::try_from(args).map_err(|_| invalid())?;
        let Resolve { owner, reference } =
            serde_json::from_value(request).map_err(|_| invalid())?;
        let store = self.0.clone();
        Ok(RpcValue::future(async move {
            // Dropping the call cancels the lookup.
            let cancel = CancellationToken::new();
            let _cancel = cancel.clone().drop_guard();
            Ok(match store.resolve(owner, reference, cancel).await {
                Ok(value) => json!({"ok": true, "value": {"value": value}}),
                Err(error) => json!({"ok": false, "error": {"code": error.code(), "message": error.to_string()}}),
            }
            .into())
        }))
    }
}
