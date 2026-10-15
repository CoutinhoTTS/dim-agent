//! Named management contributions. Rpc looks up a namespace's provider on
//! each call and does not depend on it, so an inactive or reloading provider
//! affects only its own namespace. Accepted calls belong to their provider.
use crate::protocol::RpcError;
use rutis::{BoxFuture, CordisError, Ctx, Effect, FiberState, Snapshot, TypeKey};
use rutis_loader::{EntryStatus, Loader};
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
            // This generation is unloading; the next one serves the namespace.
            return Err(RpcError::reloading());
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

/// Unlike `service_reloading`, a retry fails the same way until the owner is
/// fixed or restarted.
fn unavailable() -> RpcError {
    RpcError::app(
        -32000,
        "service_unavailable",
        "Service is unavailable until it is fixed or restarted",
    )
}

/// The management namespaces assembly configures, each with the loader row
/// of the owner that provides it. Rpc reaches a namespace's provider by key on
/// each call and reads that row's status only when none is active, to tell a
/// reloading owner from a failed one and from one that serves nothing, so it
/// depends on no owner (methods contract §3.2). Adding an owner adds its pair
/// where its row is registered.
pub(crate) struct Owners {
    loader: Loader,
    /// Longest first: a name belongs to the longest namespace containing it.
    namespaces: Vec<(String, String)>,
}
impl Owners {
    pub(crate) fn new(loader: Loader, namespaces: &[(String, String)]) -> Self {
        let mut namespaces = namespaces.to_vec();
        namespaces.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
        Self { loader, namespaces }
    }

    /// The configured namespace that owns `name`, and its row. A name inside
    /// a child namespace belongs to the child even while the child serves
    /// nothing; its parent never answers it.
    fn owner(&self, name: &str) -> Option<&(String, String)> {
        self.namespaces
            .iter()
            .find(|(namespace, _)| within(name, namespace))
    }

    /// Called under ConnectionHub's Rpc admission lock, with the process root
    /// as `ctx`. The reply is tracked by the caller; the operation by its
    /// provider. A name outside the declaration grammar is unknown without
    /// any lookup.
    pub(crate) fn dispatch(
        &self,
        ctx: &Ctx,
        method: String,
        params: Value,
        entry_stopping: CancellationToken,
    ) -> Result<Accepted, RpcError> {
        if !valid_name(&method) {
            return Err(unknown());
        }
        let (namespace, row) = self.owner(&method).ok_or_else(unknown)?;
        let provider = match provider(ctx, namespace) {
            Some(provider) => provider,
            // No active provider: the owner row's state answers (methods
            // contract §3.2). Each known state is named, and a new
            // `FiberState` does not compile until it is placed.
            None => match self.loader.get(row).map(|entry| entry.status) {
                // No enabled provider.
                None | Some(EntryStatus::Disabled) => Err(unknown()),
                // Not started yet, or between generations while the loader
                // replaces its fiber. An owner that disposed itself also ends
                // here: the process loader cannot disable its row (#124).
                Some(EntryStatus::Inactive) => Err(RpcError::reloading()),
                Some(EntryStatus::Running(Snapshot { state, .. })) => match state {
                    // Waiting for its dependencies, applying, or unloading
                    // between generations. A failed dependency keeps it
                    // Pending until the dependency is fixed (#124).
                    FiberState::Pending | FiberState::Loading | FiberState::Unloading => {
                        Err(RpcError::reloading())
                    }
                    // Active between the lookup and the status read.
                    FiberState::Active => provider(ctx, namespace).ok_or_else(unknown),
                    // Its apply failed, or it was removed and cannot restart.
                    FiberState::Failed | FiberState::Disposed => Err(unavailable()),
                },
                // The row cannot run as configured; or a copy inside an
                // instance disposed itself and waits for an explicit restart
                // (`get` reads the row itself, so it never reports this).
                Some(EntryStatus::Unresolved(_) | EntryStatus::Stopped) => Err(unavailable()),
                // `EntryStatus` is non-exhaustive. A state added later stops
                // the client instead of letting it retry without end.
                Some(_) => Err(unavailable()),
            }?,
        };
        provider.accept(method, params, entry_stopping)
    }

    /// The methods `hello.features` lists: those of the configured namespaces'
    /// providers that are active now, each under the namespace that owns it.
    pub(crate) fn listed(&self, ctx: &Ctx) -> Vec<String> {
        let mut names: Vec<_> = self
            .namespaces
            .iter()
            .filter_map(|(namespace, _)| Some((namespace, provider(ctx, namespace)?)))
            .flat_map(|(namespace, provider)| {
                provider
                    .declarations
                    .iter()
                    .filter(|decl| {
                        self.owner(&decl.name)
                            .is_some_and(|(owner, _)| owner == namespace)
                    })
                    .map(|decl| decl.name.clone())
                    .collect::<Vec<_>>()
            })
            .collect();
        names.sort();
        names
    }
}

/// Fails assembly for a Session namespace recorded with
/// `AgentBuilder::session_methods` that could shadow a built-in or share a
/// name with a configured management namespace, so no name has two owners
/// (methods contract A.1). A Session namespace may lie under `session`, but
/// never equals, contains or lies inside an exact built-in, and never lies
/// under another reserved root.
pub(crate) fn check_session_namespaces(
    management: &[(String, String)],
    sessions: &[String],
) -> Result<(), CordisError> {
    for namespace in sessions {
        let overlaps = |other: &str| within(namespace, other) || within(other, namespace);
        if !valid_name(namespace)
            || ["hello", "daemon", "agent", "approval"]
                .iter()
                .any(|root| within(namespace, root))
            || crate::protocol::builtin_methods().any(overlaps)
        {
            return Err(CordisError::PluginFailed(
                format!("invalid Session namespace {namespace}").into(),
            ));
        }
        if let Some((owner, _)) = management.iter().find(|(owner, _)| overlaps(owner)) {
            return Err(CordisError::PluginFailed(
                format!("Session namespace {namespace} overlaps management namespace {owner}")
                    .into(),
            ));
        }
    }
    Ok(())
}

fn provider(ctx: &Ctx, namespace: &str) -> Option<Arc<ManagementMethods>> {
    ctx.get_as::<ManagementMethods>(TypeKey::keyed_dynamic::<ManagementMethods>(namespace))
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
