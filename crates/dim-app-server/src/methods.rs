//! Stable consumer-owned method port. Contributors depend on this port, never
//! the other way round. Registration and admission share one lifecycle lock.
use crate::protocol::RpcError;
use dim_agent_runtime::{Capability, KeysOf, scoped};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypeKey, TypedPlugin};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, Weak},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

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

struct Generation {
    namespace: String,
    declarations: Vec<MethodDecl>,
    notifications: Vec<String>,
    handler: Handler,
    tasks: TaskTracker,
    stopping: CancellationToken,
}

/// A stable process service; it never depends on contributors.
pub struct Methods {
    methods: Mutex<BTreeMap<String, Arc<Generation>>>,
    revision: std::sync::atomic::AtomicU64,
    notices: tokio::sync::broadcast::Sender<ManagementNotice>,
}
impl Default for Methods {
    fn default() -> Self {
        Self {
            methods: Mutex::new(BTreeMap::new()),
            revision: std::sync::atomic::AtomicU64::new(0),
            notices: tokio::sync::broadcast::channel(64).0,
        }
    }
}
#[derive(Clone)]
pub(crate) struct ManagementNotice {
    pub method: String,
    pub params: Value,
    generation: Weak<Generation>,
}
/// Generic owner notification port. It knows neither connections nor versions.
#[derive(Clone)]
pub struct NotificationPort {
    service: Arc<Methods>,
    generation: Weak<Generation>,
}
impl NotificationPort {
    pub fn notify(&self, method: impl Into<String>, params: Value) {
        let method = method.into();
        if !self
            .generation
            .upgrade()
            .is_some_and(|generation| generation.notifications.contains(&method))
        {
            return;
        }
        let notice = ManagementNotice {
            method,
            params,
            generation: self.generation.clone(),
        };
        self.service.with_current(&notice, || {
            let _ = self.service.notices.send(notice.clone());
        });
    }
}

/// Own this handle in the contributor's immediately registered async effect.
/// Removing the catalog entries fences admission before draining accepted work.
pub struct Registration {
    service: Arc<Methods>,
    generation: Arc<Generation>,
}

impl Methods {
    fn lookup<'a>(
        methods: &'a BTreeMap<String, Arc<Generation>>,
        method: &str,
    ) -> Option<&'a Arc<Generation>> {
        methods
            .values()
            .filter(|owner| within(method, &owner.namespace))
            .max_by_key(|owner| owner.namespace.len())
    }
    pub(crate) fn subscribe(&self) -> tokio::sync::broadcast::Receiver<ManagementNotice> {
        self.notices.subscribe()
    }
    pub(crate) fn with_current(&self, notice: &ManagementNotice, deliver: impl FnOnce()) {
        let methods = self.methods.lock().expect("management catalog");
        if notice.generation.upgrade().is_some_and(|generation| {
            methods
                .values()
                .any(|current| Arc::ptr_eq(current, &generation))
        }) {
            deliver();
        }
    }
    pub fn methods(&self, level: MethodLevel) -> Vec<String> {
        let methods = self.methods.lock().expect("method catalog");
        let mut names: Vec<_> = methods
            .values()
            .flat_map(|owner| &owner.declarations)
            .filter(|decl| decl.level == level)
            .map(|decl| decl.name.clone())
            .collect();
        names.sort();
        names
    }

    pub fn catalog(&self, level: MethodLevel) -> Value {
        let methods = self.methods.lock().expect("method catalog");
        let mut declarations: Vec<_> = methods.values().flat_map(|owner| {
            owner.declarations.iter().filter(move |decl| decl.level == level).map(move |decl| {
                serde_json::json!({"name":decl.name,"namespace":owner.namespace,"mutating":decl.mutating,"progress":false,"notifications":[]})
            })
        }).collect();
        declarations.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        serde_json::json!({"revision":self.revision.load(std::sync::atomic::Ordering::Relaxed),"methods":declarations})
    }

    pub fn notification_names(&self) -> Vec<String> {
        self.methods
            .lock()
            .expect("method catalog")
            .values()
            .filter(|owner| !owner.stopping.is_cancelled())
            .flat_map(|owner| owner.notifications.iter().cloned())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn register(
        self: &Arc<Self>,
        namespace: &str,
        declarations: Vec<MethodDecl>,
        notifications: Vec<String>,
        handler: Handler,
    ) -> Result<Registration, RpcError> {
        let mut methods = self.methods.lock().expect("management catalog");
        let mut unique = std::collections::HashSet::new();
        let valid_name = |name: &str| {
            name.len() <= 128
                && name.split('.').all(|part| {
                    part.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
                        && part
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                })
        };
        if !valid_name(namespace)
            || namespace == "session"
            || [
                "hello",
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
            || methods.contains_key(namespace)
            || declarations.is_empty()
            || declarations.iter().any(|decl| {
                !valid_name(&decl.name)
                    || !within(&decl.name, namespace)
                    || crate::protocol::is_builtin_method(&decl.name)
                    || !unique.insert(&decl.name)
                    || methods.values().any(|owner| {
                        owner
                            .declarations
                            .iter()
                            .any(|current| current.name == decl.name)
                            || within(&decl.name, &owner.namespace)
                                && owner.namespace.len() > namespace.len()
                    })
            })
            || methods.values().any(|owner| {
                owner.namespace.len() < namespace.len()
                    && owner
                        .declarations
                        .iter()
                        .any(|decl| within(&decl.name, namespace))
            })
            || notifications
                .iter()
                .any(|name| !valid_name(name) || !within(name, namespace))
        {
            return Err(RpcError::invalid(
                "Duplicate or invalid management method registration",
            ));
        }
        let generation = Arc::new(Generation {
            namespace: namespace.into(),
            declarations,
            notifications,
            handler,
            tasks: TaskTracker::new(),
            stopping: CancellationToken::new(),
        });
        methods.insert(namespace.into(), generation.clone());
        self.revision
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(Registration {
            service: self.clone(),
            generation,
        })
    }

    /// The returned receiver owns delivery only, not the accepted operation.
    /// Dropping it (client disconnect) never cancels a mutation.
    pub fn dispatch(
        &self,
        method: String,
        params: Value,
        level: MethodLevel,
        caller_gone: CancellationToken,
        entry_stopping: CancellationToken,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<Value, RpcError>>, RpcError> {
        let (send, receive) = tokio::sync::oneshot::channel();
        let methods = self.methods.lock().expect("management catalog");
        let generation = Self::lookup(&methods, &method)
            .filter(|owner| {
                owner
                    .declarations
                    .iter()
                    .any(|d| d.name == method && d.level == level)
            })
            .ok_or_else(|| RpcError::app(-32601, "unknown_method", "Unknown method"))?
            .clone();
        if generation.stopping.is_cancelled() {
            return Err(RpcError::reloading());
        }
        let interruption = generation
            .declarations
            .iter()
            .find(|d| d.name == method)
            .expect("registered declaration")
            .interruption;
        let owner = generation.clone();
        // Spawn under the admission lock; unregister cannot miss this task.
        generation.tasks.spawn(async move {
            let cancel = if interruption == Interruption::Report { caller_gone.child_token() } else { CancellationToken::new() };
            let operation = (owner.handler)(method, params, cancel.clone());
            tokio::pin!(operation);
            if interruption == Interruption::Report {
                tokio::select! {
                    biased;
                    result = &mut operation => { let _ = send.send(result); },
                    _ = owner.stopping.cancelled() => {
                        cancel.cancel();
                        let _ = send.send(Err(RpcError::app(-32000, "operation_interrupted", "Method contributor unloaded")));
                        // Reporting interruption does not abandon an accepted mutation.
                        let _ = operation.await;
                    },
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

impl Registration {
    pub fn notifications(&self) -> NotificationPort {
        NotificationPort {
            service: self.service.clone(),
            generation: Arc::downgrade(&self.generation),
        }
    }
    fn fence(&self) {
        let mut methods = self.service.methods.lock().expect("management catalog");
        self.generation.stopping.cancel();
        if methods
            .get(&self.generation.namespace)
            .is_some_and(|current| Arc::ptr_eq(current, &self.generation))
        {
            methods.remove(&self.generation.namespace);
            self.service
                .revision
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    pub async fn unregister_and_drain(self) {
        self.fence();
        self.generation.tasks.close();
        self.generation.tasks.wait().await;
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        // A failed apply/effect registration must not leave callable handlers.
        // Normal disposal uses unregister_and_drain before releasing dependencies.
        self.fence();
        self.generation.tasks.close();
    }
}

fn within(name: &str, namespace: &str) -> bool {
    name == namespace
        || name
            .strip_prefix(namespace)
            .is_some_and(|suffix| suffix.starts_with('.'))
}

pub struct MethodsPlugin;
impl TypedPlugin for MethodsPlugin {
    type Deps = ();
    fn name(&self) -> &str {
        "Methods"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            ctx.provide_as(TypeKey::of::<Methods>(), Arc::new(Methods::default()))?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for MethodsPlugin {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&())
    }
}
