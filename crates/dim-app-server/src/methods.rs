//! Named management contributions, collected by Rpc through ordinary rutis
//! dependencies. The method table and accepted calls belong to one Rpc load.
use crate::protocol::RpcError;
use dim_agent_runtime::Contributions;
use rutis::{BoxFuture, CordisError, Ctx, TypeKey};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MethodLevel {
    Session,
    Management,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interruption {
    Report,
    Drain,
}

#[derive(Clone, Debug, Serialize)]
pub struct MethodDecl {
    pub name: String,
    pub level: MethodLevel,
    pub mutating: bool,
    #[serde(skip)]
    pub interruption: Interruption,
}
impl MethodDecl {
    pub fn management(name: &str, mutating: bool, interruption: Interruption) -> Self {
        Self {
            name: name.into(),
            level: MethodLevel::Management,
            mutating,
            interruption,
        }
    }
}

/// A process owner's wire methods. Providing this named service creates the
/// ordinary contributor -> collector reload relationship, not a second lifecycle.
pub struct ManagementMethods {
    namespace: String,
    declarations: Vec<MethodDecl>,
    notifications: Vec<String>,
    handler: Handler,
    notices: tokio::sync::broadcast::Sender<ManagementNotice>,
}

#[derive(Clone)]
pub(crate) struct ManagementNotice {
    pub method: String,
    pub params: Value,
}

impl ManagementMethods {
    pub fn provide(
        ctx: &Ctx,
        namespace: &str,
        declarations: Vec<MethodDecl>,
        notifications: Vec<String>,
        handler: Handler,
    ) -> Result<Arc<Self>, CordisError> {
        let mut unique = std::collections::HashSet::new();
        if !valid_name(namespace)
            || namespace == "session"
            || [
                "hello",
                "daemon",
                "agent",
                "approval",
                "session.state",
                "session.exec",
                "session.mcp",
                "session.model",
                "session.permission",
                "session.prompt",
                "session.skills",
                "session.input",
            ]
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
                    || decl.level != MethodLevel::Management
                    || crate::protocol::is_builtin_method(&decl.name)
                    || !unique.insert(&decl.name)
            })
            || notifications
                .iter()
                .any(|name| !valid_name(name) || !within(name, namespace))
        {
            return Err(CordisError::PluginFailed(
                "Invalid management contribution".into(),
            ));
        }
        let service = Arc::new(Self {
            namespace: namespace.into(),
            declarations,
            notifications,
            handler,
            notices: tokio::sync::broadcast::channel(64).0,
        });
        ctx.provide_as(TypeKey::keyed_dynamic::<Self>(namespace), service.clone())?;
        Ok(service)
    }

    /// Publish an owner-declared notification; undeclared names are ignored.
    pub fn notify(&self, method: &str, params: Value) {
        if self.notifications.iter().any(|declared| declared == method) {
            let _ = self.notices.send(ManagementNotice {
                method: method.into(),
                params,
            });
        }
    }
}

pub(crate) struct MethodTable {
    owners: BTreeMap<String, Arc<ManagementMethods>>,
}
impl MethodTable {
    pub fn new(
        Contributions(owners): Contributions<ManagementMethods>,
    ) -> Result<Self, CordisError> {
        let mut table = BTreeMap::new();
        for (name, owner) in owners {
            if name != owner.namespace || table.insert(name, owner).is_some() {
                return Err(CordisError::PluginFailed(
                    "Duplicate or misnamed management contribution".into(),
                ));
            }
        }
        // A nested namespace may coexist with its parent, but must never capture
        // a method that the parent declares, in either assembly order.
        for owner in table.values() {
            for declaration in &owner.declarations {
                if table.values().any(|other| {
                    other.namespace.len() > owner.namespace.len()
                        && within(&declaration.name, &other.namespace)
                }) {
                    return Err(CordisError::PluginFailed(
                        "Overlapping management contributions".into(),
                    ));
                }
            }
        }
        Ok(Self { owners: table })
    }

    pub fn methods(&self, level: MethodLevel) -> Vec<String> {
        let mut names: Vec<_> = self
            .owners
            .values()
            .flat_map(|owner| &owner.declarations)
            .filter(|decl| decl.level == level)
            .map(|decl| decl.name.clone())
            .collect();
        names.sort();
        names
    }

    pub fn notification_names(&self) -> Vec<String> {
        self.owners
            .values()
            .flat_map(|owner| owner.notifications.iter().cloned())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn subscriptions(&self) -> Vec<tokio::sync::broadcast::Receiver<ManagementNotice>> {
        self.owners
            .values()
            .filter(|owner| !owner.notifications.is_empty())
            .map(|owner| owner.notices.subscribe())
            .collect()
    }

    /// Called under ConnectionHub's Rpc admission lock. Its disposer removes
    /// that handler before draining its commands, so no accepted call is missed.
    pub fn dispatch(
        &self,
        method: String,
        params: Value,
        level: MethodLevel,
        caller_gone: CancellationToken,
        entry_stopping: CancellationToken,
        commands: &TaskTracker,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<Value, RpcError>>, RpcError> {
        let owner = self
            .owners
            .values()
            .filter(|owner| within(&method, &owner.namespace))
            .max_by_key(|owner| owner.namespace.len())
            .ok_or_else(|| RpcError::app(-32601, "unknown_method", "Unknown method"))?;
        let interruption = owner
            .declarations
            .iter()
            .find(|decl| decl.name == method && decl.level == level)
            .ok_or_else(|| RpcError::app(-32601, "unknown_method", "Unknown method"))?
            .interruption;
        let owner = owner.clone();
        let (send, receive) = tokio::sync::oneshot::channel();
        commands.spawn(async move {
            let cancel = if interruption == Interruption::Report { caller_gone.child_token() } else { CancellationToken::new() };
            let operation = (owner.handler)(method, params, cancel.clone());
            tokio::pin!(operation);
            if interruption == Interruption::Report {
                tokio::select! {
                    biased;
                    result = &mut operation => { let _ = send.send(result); },
                    _ = entry_stopping.cancelled() => {
                        cancel.cancel();
                        let _ = send.send(Err(RpcError::app(-32000, "operation_interrupted", "Method entry unloaded")));
                        let _ = operation.await;
                    },
                    _ = cancel.cancelled() => {
                        let _ = send.send(Err(RpcError::app(-32000, "operation_interrupted", "Method caller disconnected")));
                        let _ = operation.await;
                    },
                }
            } else {
                let _ = send.send(operation.await);
            }
        });
        Ok(receive)
    }
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
