//! Named management contributions. Rpc looks up a namespace's provider on
//! each call and does not depend on it, so an inactive or reloading provider
//! affects only its own namespace. Accepted calls belong to their provider.
use dim_agent_capabilities::methods::{
    Accepted, Handler, MethodDecl, MethodProvider, RpcError, SessionMethods, valid_name, within,
};
use dim_agent_runtime::{AgentError, Session};
use rutis::{CordisError, Ctx, FiberState, Snapshot, TypeKey};
use rutis_loader::{EntryStatus, Loader};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashMap, ops::Deref, sync::Arc};
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
    pub async fn provide_foreign(
        ctx: &Ctx,
        namespace: &str,
        service: Arc<dyn rutis_bridge::session::HostDispatch>,
    ) -> Result<Arc<Self>, CordisError> {
        let (declarations, handler) = MethodProvider::foreign(ctx, service).await?;
        Self::provide(ctx, namespace, declarations, handler)
    }

    pub(crate) fn check_namespace(namespace: &str) -> Result<(), CordisError> {
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
        {
            return Err(CordisError::PluginFailed(
                "Invalid management namespace".into(),
            ));
        }
        Ok(())
    }

    pub fn provide(
        ctx: &Ctx,
        namespace: &str,
        declarations: Vec<MethodDecl>,
        handler: Handler,
    ) -> Result<Arc<Self>, CordisError> {
        // Management methods are about the host, never a Session. The `session`
        // root belongs to built-ins and Session-scoped methods, so contributed
        // methods cannot impersonate Session traffic.
        Self::check_namespace(namespace)?;
        if declarations
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

/// What a call answers when its namespace has no active provider, from the
/// status of the loader row that provides it (methods contract §3.2): `Ok`
/// when the row is Active, so the caller reads the provider again. Each
/// known state is named, and a new `FiberState` does not compile until it is
/// placed.
fn not_active(status: EntryStatus) -> Result<(), RpcError> {
    match status {
        // The row provides nothing.
        EntryStatus::Disabled => Err(RpcError::unknown_method()),
        // Not started yet, or between generations while the loader replaces
        // its fiber.
        EntryStatus::Inactive => Err(RpcError::reloading()),
        EntryStatus::Running(Snapshot { state, .. }) => match state {
            // Waiting for its dependencies, applying, or unloading between
            // generations.
            FiberState::Pending | FiberState::Loading | FiberState::Unloading => {
                Err(RpcError::reloading())
            }
            // Active between the lookup and the status read.
            FiberState::Active => Ok(()),
            // Its apply failed, or it was removed and cannot restart.
            FiberState::Failed | FiberState::Disposed => Err(unavailable()),
        },
        // The row cannot run as configured; or a copy inside an instance
        // disposed itself and waits for an explicit restart.
        EntryStatus::Unresolved(_) | EntryStatus::Stopped => Err(unavailable()),
        // `EntryStatus` is non-exhaustive. A state added later stops the
        // client instead of letting it retry without end.
        _ => Err(unavailable()),
    }
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
        progress: impl Fn(Value) + Send + Sync + 'static,
    ) -> Result<Accepted, RpcError> {
        if !valid_name(&method) {
            return Err(RpcError::unknown_method());
        }
        let (namespace, row) = self.owner(&method).ok_or_else(RpcError::unknown_method)?;
        let provider = match provider(ctx, namespace) {
            Some(provider) => provider,
            // No active provider: the owner row's state answers. `get` reads
            // the row itself, so it never reports Stopped. Two owners answer
            // `service_reloading` although they cannot come back without a
            // fix (#124): one Pending on a failed dependency, and one that
            // disposed itself, whose row the process loader cannot disable,
            // so it reads Inactive.
            None => {
                // An absent row provides nothing.
                let row = self.loader.get(row).ok_or_else(RpcError::unknown_method)?;
                not_active(row.status)?;
                provider(ctx, namespace).ok_or_else(RpcError::unknown_method)?
            }
        };
        provider.accept(method, params, entry_stopping, progress)
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

/// The `sessionMethods` namespaces assembly recorded with
/// `AgentBuilder::session_methods`, the counterpart of `Owners` (methods
/// contract A.1, "Known namespaces"). Every Agent of the process is built from
/// one assembly, so every Session lists all of them. They decide whether a
/// name is a Session method candidate before any Session is read. A recorded
/// namespace whose provider is not active in a Session is not absent: the row
/// that provides it answers, from its copy in that Session, as an owner row
/// does (contract §3.2).
pub(crate) struct SessionNamespaces {
    loader: Loader,
    /// Longest first: a name belongs to the longest namespace containing it.
    namespaces: Vec<String>,
    /// The Session row that provides a namespace, where assembly knows it: a
    /// TypeScript Session part declares its namespaces in `RowSchema.provides`.
    /// The Rust Session parts the builder registers are not here; each must be
    /// active before its Session is published.
    rows: HashMap<String, String>,
}
impl SessionNamespaces {
    pub(crate) fn new(
        loader: Loader,
        management: &[(String, String)],
        recorded: &[String],
        rows: &[(String, String)],
    ) -> Result<Self, CordisError> {
        for namespace in recorded {
            Self::check(management, namespace)?;
        }
        let mut namespaces = recorded.to_vec();
        namespaces.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        Ok(Self {
            loader,
            namespaces,
            rows: rows.iter().cloned().collect(),
        })
    }

    /// Fails assembly for a recorded namespace that could shadow a built-in
    /// or share a name with a configured management namespace, so no name has
    /// two owners. A Session namespace may lie under `session`, but never
    /// equals, contains or lies inside an exact built-in, and never lies under
    /// another reserved root.
    pub(crate) fn check(
        management: &[(String, String)],
        namespace: &str,
    ) -> Result<(), CordisError> {
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
        Ok(())
    }

    /// The provider of `namespace` in `session`, read by key on each call.
    /// With none active, the copy in this Session of the row that provides the
    /// namespace answers (`not_active`). A namespace whose row assembly does
    /// not know, or whose copy left with the Session's instance, keeps the
    /// facade's `Reloading`.
    pub(crate) fn provider(
        &self,
        session: &Session,
        namespace: &str,
    ) -> Result<Arc<SessionMethods>, RpcError> {
        let reloading = match session.session_named_service::<SessionMethods>(namespace) {
            Ok(provider) => return Ok(provider),
            Err(error @ AgentError::Reloading(_)) => error,
            Err(error) => return Err(error.into()),
        };
        let Some(row) = self.rows.get(namespace) else {
            return Err(reloading.into());
        };
        let instance = session.scope()?.instance;
        // A row inside the Session group runs one copy per Session; its
        // innermost instance is the Session's.
        let copy = self.loader.entries().into_iter().find(|entry| {
            entry.id == *row
                && entry
                    .instance
                    .as_ref()
                    .and_then(|copy| copy.chain.first())
                    .is_some_and(|(_, id)| *id == instance)
        });
        let Some(copy) = copy else {
            return Err(reloading.into());
        };
        not_active(copy.status)?;
        Ok(session.session_named_service::<SessionMethods>(namespace)?)
    }

    /// The recorded namespace that owns `name`, if it is a Session method
    /// candidate. A name outside the declaration grammar owns none, without
    /// any lookup.
    pub(crate) fn owner(&self, name: &str) -> Option<&str> {
        if !valid_name(name) {
            return None;
        }
        self.namespaces
            .iter()
            .find(|namespace| within(name, namespace))
            .map(String::as_str)
    }

    /// The methods `session.methods` lists: those of the namespaces whose
    /// providers are active in this Session, each under the namespace that
    /// owns it.
    pub(crate) fn listed(&self, session: &Session) -> Result<Vec<MethodDecl>, RpcError> {
        let mut methods = vec![];
        for namespace in &self.namespaces {
            let provider = match session.session_named_service::<SessionMethods>(namespace) {
                Ok(provider) => provider,
                Err(AgentError::Reloading(_)) => continue,
                Err(error) => return Err(error.into()),
            };
            methods.extend(
                provider
                    .declarations()
                    .iter()
                    .filter(|decl| self.owner(&decl.name) == Some(namespace))
                    .cloned(),
            );
        }
        methods.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(methods)
    }
}

fn provider(ctx: &Ctx, namespace: &str) -> Option<Arc<ManagementMethods>> {
    ctx.get_as::<ManagementMethods>(TypeKey::keyed_dynamic::<ManagementMethods>(namespace))
}

#[cfg(test)]
mod tests;
