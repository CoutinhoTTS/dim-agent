//! Named management contributions. Rpc looks up a namespace's provider on
//! each call and does not depend on it, so an inactive or reloading provider
//! affects only its own namespace. Accepted calls belong to their provider.
use dim_agent_capabilities::methods::{
    Accepted, Handler, MethodDecl, MethodProvider, RpcError, valid_name, within,
};
use rutis::{CordisError, Ctx, FiberState, Snapshot, TypeKey};
use rutis_loader::{EntryStatus, Loader};
use serde::Deserialize;
use serde_json::Value;
use std::{ops::Deref, sync::Arc};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectionScope {
    #[serde(default, deserialize_with = "crate::protocol::optional_non_null")]
    pub cwd: Option<String>,
}

/// A process owner's wire methods, provided under its namespace with the
/// shared call admission (`MethodProvider`): the owner's generation owns its
/// accepted calls.
pub struct ManagementMethods(Arc<MethodProvider>);
impl Deref for ManagementMethods {
    type Target = MethodProvider;
    fn deref(&self) -> &MethodProvider {
        &self.0
    }
}

impl ManagementMethods {
    pub fn provide(
        ctx: &Ctx,
        namespace: &str,
        declarations: Vec<MethodDecl>,
        handler: Handler,
    ) -> Result<Arc<Self>, CordisError> {
        // Management methods are about the host, never a Session. The `session`
        // root belongs to built-ins and Session-scoped methods, so contributed
        // methods cannot impersonate Session traffic.
        if ["hello", "daemon", "agent", "approval", "session"]
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
            || declarations
                .iter()
                .any(|decl| crate::protocol::is_builtin_method(&decl.name))
        {
            return Err(CordisError::PluginFailed(
                "Invalid management contribution".into(),
            ));
        }
        let service = Arc::new(Self(MethodProvider::new(
            ctx,
            namespace,
            declarations,
            handler,
        )?));
        ctx.provide_as(TypeKey::keyed_dynamic::<Self>(namespace), service.clone())?;
        Ok(service)
    }
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
            return Err(RpcError::unknown_method());
        }
        let (namespace, row) = self.owner(&method).ok_or_else(RpcError::unknown_method)?;
        let provider = match provider(ctx, namespace) {
            Some(provider) => provider,
            // No active provider: the owner row's state answers (methods
            // contract §3.2). Each known state is named, and a new
            // `FiberState` does not compile until it is placed.
            None => match self.loader.get(row).map(|entry| entry.status) {
                // No enabled provider.
                None | Some(EntryStatus::Disabled) => Err(RpcError::unknown_method()),
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
                    FiberState::Active => {
                        provider(ctx, namespace).ok_or_else(RpcError::unknown_method)
                    }
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
                    .declarations()
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

#[cfg(test)]
mod tests;
