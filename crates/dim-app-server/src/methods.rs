//! The management method table and Rpc's lookups through the method tables
//! (methods contract A.1, §3.2). A part registers a namespace with the table
//! of its scope until it unloads; Rpc reads the table on each call and
//! depends on no provider, so a provider that is reloading, Pending or Failed
//! affects only its own namespace. Accepted calls belong to their provider.
use dim_agent_capabilities::methods::{
    Accepted, Check, ForeignMethods, MethodDecl, MethodProvider, MethodTable, Owner, RpcError,
    SessionMethods, valid_name, within,
};
use dim_agent_runtime::Session;
use rutis::{BoxFuture, CordisError, Ctx, DepKey, Effect, FiberState, Snapshot, TypedPlugin};
use rutis_bridge::session::{HostDispatch, host_key};
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

/// The process's management methods: the namespaces process plugins
/// register, which Rpc serves to protected local connections.
pub struct ManagementMethods(pub Arc<MethodTable>);
impl Deref for ManagementMethods {
    type Target = MethodTable;
    fn deref(&self) -> &MethodTable {
        &self.0
    }
}

impl ManagementMethods {
    /// Management methods are about the host, never a Session. The `session`
    /// root belongs to built-ins and Session-scoped methods, so contributed
    /// methods cannot impersonate Session traffic.
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

    fn check() -> Check {
        Arc::new(|namespace, declarations| {
            Self::check_namespace(namespace)?;
            if declarations
                .iter()
                .any(|decl| crate::protocol::is_builtin_method(&decl.name))
            {
                return Err(CordisError::PluginFailed(
                    "Invalid management contribution".into(),
                ));
            }
            Ok(())
        })
    }
}

/// Process part: the management method table, at its native key and, for
/// other languages, at its dispatcher key.
pub struct ManagementMethodsPlugin {
    table: DepKey<ManagementMethods>,
}
impl ManagementMethodsPlugin {
    pub fn new(names: &dim_agent_runtime::Names<'_>) -> Result<Self, CordisError> {
        Ok(Self {
            table: names.key()?,
        })
    }
    /// This part keyed from `catalog`, to mount it directly.
    pub fn typed(
        catalog: &dim_agent_runtime::names::ServiceCatalog,
    ) -> Result<rutis::Typed<Self>, CordisError> {
        let build = Default::default();
        let plugin = Self::new(&dim_agent_runtime::Names::copy(catalog, &build))?;
        Ok(rutis::Typed::new(plugin))
    }
}
impl TypedPlugin for ManagementMethodsPlugin {
    type Deps = ();
    fn name(&self) -> &str {
        "ManagementMethods"
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx, (): ()) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let table = Arc::new(MethodTable::new(ManagementMethods::check()));
            ctx.provide_as::<dyn HostDispatch>(
                host_key(&dim_agent_runtime::names::name_of::<ManagementMethods>()),
                Arc::new(ForeignMethods(table.clone())),
            )?;
            ctx.provide_as(self.table.key().clone(), Arc::new(ManagementMethods(table)))?;
            Ok(Effect::Done)
        })
    }
}

/// The names a Session namespace may not take: the protocol's roots and
/// built-ins (methods contract A.1). A Session namespace may lie under
/// `session`, but never equals, contains or lies inside an exact built-in.
pub(crate) fn reserved_for_sessions() -> Check {
    Arc::new(|namespace, _| {
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
        Ok(())
    })
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

/// The provider that serves management method `method`, read on each call.
/// A namespace that was registered and is withdrawn now answers from the
/// status of the row that registered it: `service_reloading` while it
/// starts or reloads, `service_unavailable` once it cannot run. A name no
/// namespace ever held, or a disabled or absent row, is `unknown_method`.
pub(crate) fn management_provider(
    table: &ManagementMethods,
    loader: &Loader,
    method: &str,
) -> Result<Arc<MethodProvider>, RpcError> {
    match table.owner(method) {
        Owner::Active(provider) => Ok(provider),
        Owner::Left { row } => {
            let row = row
                .and_then(|row| loader.get(&row))
                .ok_or_else(RpcError::unknown_method)?;
            not_active(row.status)?;
            // Active between the lookup and the status read.
            match table.owner(method) {
                Owner::Active(provider) => Ok(provider),
                _ => Err(RpcError::unknown_method()),
            }
        }
        Owner::None => Err(RpcError::unknown_method()),
    }
}

/// Admits a management call through its provider (§3.2).
pub(crate) fn dispatch(
    table: &ManagementMethods,
    loader: &Loader,
    method: String,
    params: Value,
    entry_stopping: CancellationToken,
    progress: impl Fn(Value) + Send + Sync + 'static,
) -> Result<Accepted, RpcError> {
    let provider = management_provider(table, loader, &method)?;
    provider.accept(method, params, entry_stopping, progress)
}

/// The provider that serves Session method `method` in `session`. A
/// namespace withdrawn in this Session answers from the row that registered
/// it: its copy in this Session, if it is a row of the `session` group, or a
/// part between generations, which is `service_reloading`. A name no
/// namespace of this Session ever held is `unknown_method`.
pub(crate) fn session_provider(
    loader: &Loader,
    session: &Session,
    method: &str,
) -> Result<Arc<MethodProvider>, RpcError> {
    let table = session.session_service::<SessionMethods>()?;
    match table.owner(method) {
        Owner::Active(provider) => Ok(provider),
        Owner::Left { row } => {
            let instance = session.scope()?.instance;
            // A row inside the Session group runs one copy per Session; its
            // innermost instance is the Session's.
            let copy = row.and_then(|row| {
                loader.entries().into_iter().find(|entry| {
                    entry.id == row
                        && entry
                            .instance
                            .as_ref()
                            .and_then(|copy| copy.chain.first())
                            .is_some_and(|(_, id)| *id == instance)
                })
            });
            if let Some(copy) = copy {
                not_active(copy.status)?;
            }
            match table.owner(method) {
                Owner::Active(provider) => Ok(provider),
                _ => Err(RpcError::reloading()),
            }
        }
        Owner::None => Err(RpcError::unknown_method()),
    }
}

/// The methods `session.methods` lists: those of the namespaces registered
/// in this Session now, except names a management namespace holds, which
/// routing sends to management even when their params name a Session.
pub(crate) fn session_listed(
    management: &ManagementMethods,
    session: &Session,
) -> Result<Vec<MethodDecl>, RpcError> {
    Ok(session
        .session_service::<SessionMethods>()?
        .listed()
        .into_iter()
        .filter(|decl| !management.holds(&decl.name))
        .collect())
}

#[cfg(test)]
mod tests;
