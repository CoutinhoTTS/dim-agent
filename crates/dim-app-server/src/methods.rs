//! Named management contributions. Rpc looks up a namespace's provider on
//! each call and does not depend on it, so an inactive or reloading provider
//! affects only its own namespace. Accepted calls belong to their provider.
use crate::protocol::RpcError;
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypeKey};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectionScope {
    #[serde(default, deserialize_with = "crate::protocol::optional_non_null")]
    pub cwd: Option<String>,
}

pub(crate) fn params<T: DeserializeOwned>(value: Value) -> Result<T, RpcError> {
    serde_json::from_value(value).map_err(|_| RpcError::invalid("Invalid request parameters"))
}

pub(crate) fn reply(value: impl Serialize) -> Result<Value, RpcError> {
    serde_json::to_value(value)
        .map_err(|_| RpcError::wire(-32603, "Management serialization failed"))
}

pub type Handler = Arc<
    dyn Fn(String, Value, CancellationToken) -> BoxFuture<'static, Result<Value, RpcError>>
        + Send
        + Sync,
>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interruption {
    Report,
    Drain,
}

#[derive(Clone, Debug, Serialize)]
pub struct MethodDecl {
    pub name: String,
    pub mutating: bool,
    #[serde(skip)]
    pub interruption: Interruption,
}
impl MethodDecl {
    pub fn management(name: &str, mutating: bool, interruption: Interruption) -> Self {
        Self {
            name: name.into(),
            mutating,
            interruption,
        }
    }
}

/// A process owner's wire methods, provided under its namespace. The owner's
/// generation owns its accepted calls: its unload interrupts `Report` calls
/// and waits for every call to settle, so `Drain` calls hold the unload.
pub struct ManagementMethods {
    declarations: Vec<MethodDecl>,
    handler: Handler,
    gate: Mutex<()>,
    stopping: CancellationToken,
    calls: TaskTracker,
}

impl ManagementMethods {
    pub fn provide(
        ctx: &Ctx,
        namespace: &str,
        declarations: Vec<MethodDecl>,
        handler: Handler,
    ) -> Result<Arc<Self>, CordisError> {
        let mut unique = std::collections::HashSet::new();
        // Management methods are about the host, never a Session. The `session`
        // root belongs to built-ins and Session-scoped methods, so contributed
        // methods cannot impersonate Session traffic.
        if !valid_name(namespace)
            || ["hello", "daemon", "agent", "approval", "session"]
                .iter()
                .any(|reserved| within(namespace, reserved))
            || namespace
                .split('.')
                .scan(String::new(), |prefix, part| {
                    if !prefix.is_empty() {
                        prefix.push('.');
                    }
                    prefix.push_str(part);
                    Some(crate::protocol::is_builtin_method(prefix))
                })
                .any(|reserved| reserved)
            || declarations.is_empty()
            || declarations.iter().any(|decl| {
                !valid_name(&decl.name)
                    || !within(&decl.name, namespace)
                    || crate::protocol::is_builtin_method(&decl.name)
                    || !unique.insert(&decl.name)
            })
        {
            return Err(CordisError::PluginFailed(
                "Invalid management contribution".into(),
            ));
        }
        let service = Arc::new(Self {
            declarations,
            handler,
            gate: Mutex::new(()),
            stopping: CancellationToken::new(),
            calls: TaskTracker::new(),
        });
        // Registered after the owner's own effects, so it runs first on unload
        // and drained calls still reach the owner's services.
        let owner = service.clone();
        ctx.effect(move || {
            Effect::AsyncDisposer(Box::new(move || {
                Box::pin(async move {
                    {
                        let _guard = owner.gate.lock().unwrap();
                        owner.stopping.cancel();
                        owner.calls.close();
                    }
                    owner.calls.wait().await;
                    Ok(())
                })
            }))
        })?;
        ctx.provide_as(TypeKey::keyed_dynamic::<Self>(namespace), service.clone())?;
        Ok(service)
    }

    /// Admits one call of this generation. Both this provider's unload and the
    /// calling Rpc load's unload (`entry_stopping`) interrupt a `Report` call,
    /// which replies at once and then settles; a `Drain` call replies with its
    /// own result. The handler's token fires on either unload. A caller's
    /// disconnect cancels nothing.
    fn accept(
        self: &Arc<Self>,
        method: String,
        params: Value,
        entry_stopping: CancellationToken,
    ) -> Result<Accepted, RpcError> {
        let interruption = self
            .declarations
            .iter()
            .find(|decl| decl.name == method)
            .ok_or_else(unknown)?
            .interruption;
        let _guard = self.gate.lock().unwrap();
        if self.stopping.is_cancelled() {
            // Between generations: the next one serves the namespace.
            return Err(unknown());
        }
        let (send, reply) = tokio::sync::oneshot::channel();
        let (settle, settled) = tokio::sync::oneshot::channel::<()>();
        let (owner, cancel) = (self.clone(), CancellationToken::new());
        self.calls.spawn(async move {
            // Dropped when the operation has settled, after any early reply.
            let _settle = settle;
            let operation = (owner.handler)(method, params, cancel.clone());
            tokio::pin!(operation);
            let unloading = async {
                tokio::select! {
                    _ = entry_stopping.cancelled() => {},
                    _ = owner.stopping.cancelled() => {},
                }
            };
            // An unload wins a tie: a result racing it is not reported.
            let result = tokio::select! {
                biased;
                _ = unloading => None,
                result = &mut operation => Some(result),
            };
            let result = match result {
                Some(result) => result,
                None => {
                    cancel.cancel();
                    if interruption == Interruption::Report {
                        let _ = send.send(Err(RpcError::app(
                            -32000,
                            "operation_interrupted",
                            "Service reloaded; the operation may already have committed. Read the current state before retrying",
                        )));
                        let _ = operation.await;
                        return;
                    }
                    operation.await
                }
            };
            let _ = send.send(result);
        });
        Ok(Accepted { reply, settled })
    }
}

/// An admitted call: its reply, and a signal that closes once its operation
/// has settled, which may follow an interrupted reply.
pub(crate) struct Accepted {
    pub reply: tokio::sync::oneshot::Receiver<Result<Value, RpcError>>,
    pub settled: tokio::sync::oneshot::Receiver<()>,
}

fn unknown() -> RpcError {
    RpcError::app(-32601, "unknown_method", "Unknown method")
}

/// The active provider that owns `method`: the longest namespace, among the
/// method name's prefixes, with an active provider (methods contract §3.2).
fn owner(ctx: &Ctx, method: &str) -> Option<Arc<ManagementMethods>> {
    let mut prefix = method;
    loop {
        if let Some(owner) =
            ctx.get_as::<ManagementMethods>(TypeKey::keyed_dynamic::<ManagementMethods>(prefix))
        {
            return Some(owner);
        }
        prefix = &prefix[..prefix.rfind('.')?];
    }
}

/// Called under ConnectionHub's Rpc admission lock with that load's context.
/// The reply is tracked by the caller; the operation by its provider.
pub(crate) fn dispatch(
    ctx: &Ctx,
    method: String,
    params: Value,
    entry_stopping: CancellationToken,
) -> Result<Accepted, RpcError> {
    owner(ctx, &method)
        .ok_or_else(unknown)?
        .accept(method, params, entry_stopping)
}

/// The methods `hello.features` lists: those of the providers of `namespaces`
/// that are active now, each listed only if dispatch would reach its provider,
/// so a parent's method inside an active child namespace is not listed.
pub(crate) fn listed(ctx: &Ctx, namespaces: &[String]) -> Vec<String> {
    let mut names: Vec<_> = namespaces
        .iter()
        .filter_map(|namespace| {
            ctx.get_as::<ManagementMethods>(TypeKey::keyed_dynamic::<ManagementMethods>(
                namespace.as_str(),
            ))
        })
        .flat_map(|provider| {
            provider
                .declarations
                .iter()
                .filter(|decl| {
                    owner(ctx, &decl.name).is_some_and(|owner| Arc::ptr_eq(&owner, &provider))
                })
                .map(|decl| decl.name.clone())
                .collect::<Vec<_>>()
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

fn valid_name(name: &str) -> bool {
    name.len() <= 128
        && name.split('.').all(|part| {
            part.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
}

fn within(name: &str, namespace: &str) -> bool {
    name == namespace
        || name
            .strip_prefix(namespace)
            .is_some_and(|suffix| suffix.starts_with('.'))
}

#[cfg(test)]
mod tests;
