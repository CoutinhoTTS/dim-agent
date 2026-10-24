//! The Rust binding shared by method contributions (methods contract A.1,
//! Appendix F): declarations, the handler, wire errors and the call admission
//! that both `ManagementMethods` (App Server, process scope) and
//! `SessionMethods` (Session scope) build on, so the two cannot diverge.
use dim_agent_runtime::{AgentError, SessionScope};
use rutis::{BoxFuture, CordisError, Ctx, Effect};
use rutis_bridge::session::{HostDispatch, Value as BridgeValue, settle};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    ops::Deref,
    sync::{Arc, Mutex},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

/// A JSON-RPC error as the client receives it; `data.appError` is the stable
/// application code.
#[derive(Debug, Clone, Serialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<ErrorData>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorData {
    pub app_error: String,
}
impl RpcError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::app(-32602, "invalid_params", message)
    }
    pub fn app(code: i32, app_error: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: Some(ErrorData {
                app_error: app_error.into(),
            }),
        }
    }
    pub fn wire(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
    /// The server service is reloading; the request did not run and may be retried.
    pub fn reloading() -> Self {
        Self::app(
            -32000,
            "service_reloading",
            "Server service is reloading; retry",
        )
    }
    /// No owner serves the name.
    pub fn unknown_method() -> Self {
        Self::app(-32601, "unknown_method", "Unknown method")
    }
    /// An admitted operation was interrupted by an unload; it may already
    /// have committed.
    pub fn interrupted() -> Self {
        Self::app(
            -32000,
            "operation_interrupted",
            "Service reloaded; the operation may already have committed. Read the current state before retrying",
        )
    }
    pub fn is_closed(&self) -> bool {
        self.data
            .as_ref()
            .is_some_and(|data| data.app_error == "session_closed")
    }
    pub fn closed() -> Self {
        Self::app(
            -32000,
            "session_closed",
            "Session is closed or does not exist",
        )
    }
}
impl From<AgentError> for RpcError {
    fn from(error: AgentError) -> Self {
        let app = match &error {
            AgentError::InvalidInput(_) | AgentError::UnknownBranch(_) => "session_invalid_input",
            AgentError::Closed(_) => "session_closed",
            AgentError::Build(_) => {
                return Self::app(
                    -32000,
                    "session_build_failed",
                    "Session initialization failed",
                );
            }
            AgentError::Reloading(_) => return Self::reloading(),
            _ => return Self::wire(-32603, "Session operation failed"),
        };
        Self::app(-32000, app, error.to_string())
    }
}

/// Decodes a method's raw params at the wire boundary.
pub fn params<T: DeserializeOwned>(value: Value) -> Result<T, RpcError> {
    serde_json::from_value(value).map_err(|_| RpcError::invalid("Invalid request parameters"))
}

/// Encodes a method's reply.
pub fn reply(value: impl Serialize) -> Result<Value, RpcError> {
    serde_json::to_value(value)
        .map_err(|_| RpcError::wire(-32603, "Method reply serialization failed"))
}

/// The name grammar of namespaces and methods (A.1): dot-separated segments
/// `[a-z][a-z0-9_]*`, at most 128 bytes.
pub fn valid_name(name: &str) -> bool {
    name.len() <= 128
        && name.split('.').all(|part| {
            part.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
}

/// Whether `name` equals `namespace` or lies inside it.
pub fn within(name: &str, namespace: &str) -> bool {
    name == namespace
        || name
            .strip_prefix(namespace)
            .is_some_and(|suffix| suffix.starts_with('.'))
}

/// Serves one call: `(method, params, cancellation token, progress)`. The
/// token fires when the provider's generation or the calling load unloads.
pub type Handler = Arc<
    dyn Fn(
            String,
            Value,
            CancellationToken,
            Progress,
        ) -> BoxFuture<'static, Result<Value, RpcError>>
        + Send
        + Sync,
>;

type Sink = Box<dyn Fn(Value) + Send + Sync>;

/// The progress channel of one call (methods contract A.2, `progress`): the
/// provider sends progress data on the call it serves, and the caller's entry
/// delivers it to the calling connection only, before the call's reply. Data
/// sent on a method that does not declare `progress`, or once the reply has
/// been handed over, is dropped. Delivery is best effort: the entry may also
/// drop progress its connection cannot take, which never affects the reply.
#[derive(Clone)]
pub struct Progress(Arc<Mutex<Option<Sink>>>);
impl Progress {
    pub fn send(&self, data: Value) {
        if let Some(sink) = &*self.0.lock().unwrap() {
            sink(data);
        }
    }
    /// Ends the channel. A send in flight completes first, so everything the
    /// caller received precedes the reply handed over next.
    fn close(&self) {
        self.0.lock().unwrap().take();
    }
}

/// How the provider replies to an admitted call that an unload interrupts:
/// `Report` replies `operation_interrupted` at once and settles afterwards;
/// `Drain` replies with the call's own result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interruption {
    Report,
    Drain,
}

/// One declared method. Serialized as the listing entry `{name, mutating}`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MethodDecl {
    /// Full wire name inside the provider's namespace.
    pub name: String,
    pub mutating: bool,
    #[serde(skip_serializing)]
    pub interruption: Interruption,
    /// Whether the caller receives the progress the provider sends.
    #[serde(skip_serializing)]
    pub progress: bool,
}
impl MethodDecl {
    /// A method without progress.
    pub fn new(name: &str, mutating: bool, interruption: Interruption) -> Self {
        Self {
            name: name.into(),
            mutating,
            interruption,
            progress: false,
        }
    }
}

/// The methods one provider generation serves under its namespace, and the
/// calls it admitted. The generation owns them: its unload interrupts
/// `Report` calls and waits for every call to settle, so `Drain` calls hold
/// the unload.
pub struct MethodProvider {
    declarations: Vec<MethodDecl>,
    handler: Handler,
    gate: Mutex<()>,
    stopping: CancellationToken,
    calls: TaskTracker,
}

impl MethodProvider {
    /// Rejects an invalid namespace or declarations (A.1: grammar, at least
    /// one method, each unique and inside the namespace), then registers the
    /// unload effect on `ctx`. Called from the owner's `apply` after its own
    /// effects, so the effect runs first on unload and drained calls still
    /// reach the owner's services.
    pub fn new(
        ctx: &Ctx,
        namespace: &str,
        declarations: Vec<MethodDecl>,
        handler: Handler,
    ) -> Result<Arc<Self>, CordisError> {
        let mut unique = HashSet::new();
        if !valid_name(namespace)
            || declarations.is_empty()
            || declarations.iter().any(|decl| {
                !valid_name(&decl.name)
                    || !within(&decl.name, namespace)
                    || !unique.insert(&decl.name)
            })
        {
            return Err(CordisError::PluginFailed(
                format!("invalid methods for namespace {namespace}").into(),
            ));
        }
        let provider = Arc::new(Self {
            declarations,
            handler,
            gate: Mutex::new(()),
            stopping: CancellationToken::new(),
            calls: TaskTracker::new(),
        });
        let owner = provider.clone();
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
        Ok(provider)
    }

    pub fn declarations(&self) -> &[MethodDecl] {
        &self.declarations
    }

    /// Admits one call of this generation; a name it does not declare is
    /// `unknown_method`. Both this provider's unload and the calling load's
    /// unload (`entry_stopping`) interrupt a `Report` call, which replies at
    /// once and then settles; a `Drain` call replies with its own result. The
    /// handler's token fires on either unload. A caller's disconnect cancels
    /// nothing. `progress` delivers the call's progress to its caller when
    /// the method declares it; the channel closes before the reply is handed
    /// over.
    pub fn accept(
        &self,
        method: String,
        params: Value,
        entry_stopping: CancellationToken,
        progress: impl Fn(Value) + Send + Sync + 'static,
    ) -> Result<Accepted, RpcError> {
        let decl = self
            .declarations
            .iter()
            .find(|decl| decl.name == method)
            .ok_or_else(RpcError::unknown_method)?;
        let interruption = decl.interruption;
        let progress = Progress(Arc::new(Mutex::new(
            decl.progress.then(|| Box::new(progress) as Sink),
        )));
        let _guard = self.gate.lock().unwrap();
        if self.stopping.is_cancelled() {
            // This generation is unloading; the next one serves the namespace.
            return Err(RpcError::reloading());
        }
        let (send, reply) = tokio::sync::oneshot::channel();
        let (settle, settled) = tokio::sync::oneshot::channel::<()>();
        let (handler, stopping, cancel) = (
            self.handler.clone(),
            self.stopping.clone(),
            CancellationToken::new(),
        );
        self.calls.spawn(async move {
            // Dropped when the operation has settled, after any early reply.
            let _settle = settle;
            let operation = handler(method, params, cancel.clone(), progress.clone());
            tokio::pin!(operation);
            let unloading = async {
                tokio::select! {
                    _ = entry_stopping.cancelled() => {},
                    _ = stopping.cancelled() => {},
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
                        progress.close();
                        let _ = send.send(Err(RpcError::interrupted()));
                        let _ = operation.await;
                        return;
                    }
                    operation.await
                }
            };
            progress.close();
            let _ = send.send(result);
        });
        Ok(Accepted { reply, settled })
    }
}

/// An admitted call: its reply, and a signal that closes once its operation
/// has settled, which may follow an interrupted reply.
pub struct Accepted {
    pub reply: tokio::sync::oneshot::Receiver<Result<Value, RpcError>>,
    pub settled: tokio::sync::oneshot::Receiver<()>,
}

/// A Session capability's wire methods, `sessionMethods[namespace]`: the
/// Session-scoped counterpart of App Server's `ManagementMethods`, with the
/// same call admission. Rpc dispatches a call through the Session it names,
/// reading this service by namespace (`Session::session_named_service`), and
/// never names its provider.
pub struct SessionMethods(Arc<MethodProvider>);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum DirectReply {
    Value,
}

#[derive(Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum ForeignReply {
    Value {
        kind: DirectReply,
        #[serde(default)]
        value: Value,
    },
    Error {
        code: i32,
        #[serde(rename = "appError")]
        app_error: String,
        message: String,
    },
}
impl Deref for SessionMethods {
    type Target = MethodProvider;
    fn deref(&self) -> &MethodProvider {
        &self.0
    }
}
impl SessionMethods {
    /// Binds one foreign generation after its original row registered cleanup.
    pub async fn provide_foreign(
        ctx: &Ctx,
        scope: &SessionScope,
        namespace: &str,
        service: Arc<dyn HostDispatch>,
    ) -> Result<Arc<Self>, CordisError> {
        let (declarations, handler) = MethodProvider::foreign(ctx, service).await?;
        Self::provide(ctx, scope, namespace, declarations, handler)
    }

    /// Provides the methods of `namespace` for this Session, from the
    /// provider's `apply` after its own effects. Assembly records the
    /// namespace with `AgentBuilder::session_methods`, which Rpc checks
    /// against built-in and management names; every Session of that assembly
    /// must provide it, since a recorded namespace without an active provider
    /// answers `service_reloading`.
    pub fn provide(
        ctx: &Ctx,
        scope: &SessionScope,
        namespace: &str,
        declarations: Vec<MethodDecl>,
        handler: Handler,
    ) -> Result<Arc<Self>, CordisError> {
        let key = rutis::DepKey::dynamic(namespace.to_owned()).instance(scope.instance);
        Self::provide_at(ctx, &key, namespace, declarations, handler)
    }
    /// The key of `namespace`'s provider in the copy's Session, until Rpc
    /// finds providers by name (R6).
    pub fn key(
        names: &dim_agent_runtime::Names<'_>,
        namespace: &str,
    ) -> Result<rutis::DepKey<Self>, CordisError> {
        Ok(rutis::DepKey::dynamic(namespace.to_owned())
            .instance(names.instance(dim_agent_runtime::SESSION_GROUP)?))
    }
    /// [`Self::provide`] at the key [`Self::key`] gave the part's factory.
    pub fn provide_at(
        ctx: &Ctx,
        key: &rutis::DepKey<Self>,
        namespace: &str,
        declarations: Vec<MethodDecl>,
        handler: Handler,
    ) -> Result<Arc<Self>, CordisError> {
        let service = Arc::new(Self(MethodProvider::new(
            ctx,
            namespace,
            declarations,
            handler,
        )?));
        ctx.provide_as(key.key().clone(), service.clone())?;
        Ok(service)
    }
}

impl MethodProvider {
    /// Decodes the foreign method boundary once for Session and management owners.
    pub async fn foreign(
        ctx: &Ctx,
        service: Arc<dyn HostDispatch>,
    ) -> Result<(Vec<MethodDecl>, Handler), CordisError> {
        let declarations = service
            .invoke("describe", BridgeValue::List(vec![]))
            .map_err(|error| CordisError::PluginFailed(error.into()))?;
        let cancelled = ctx.cancellation_token();
        let declarations = tokio::select! {
            biased;
            _ = cancelled.cancelled() => return Err(CordisError::PluginFailed("Method binding cancelled".into())),
            declarations = settle(declarations) => declarations,
        }
            .and_then(|value| value.json())
            .map_err(|error| CordisError::PluginFailed(error.into()))?;
        let declarations: Vec<MethodDecl> = serde_json::from_value(declarations)
            .map_err(|error| CordisError::PluginFailed(error.into()))?;
        Ok((
            declarations,
            Arc::new(move |method, params, cancel, progress| {
                let service = service.clone();
                Box::pin(async move {
                    let _finish = cancel.clone().drop_guard();
                    let cancelled = BridgeValue::future(async move {
                        cancel.cancelled().await;
                        Ok(BridgeValue::Undefined)
                    });
                    let progress = BridgeValue::callback(move |args| {
                        let [data]: [BridgeValue; 1] = args.list()?.try_into().map_err(|_| {
                            rutis_bridge::session::Error::Value(
                                "progress needs one argument".into(),
                            )
                        })?;
                        progress.send(data.json()?);
                        Ok(BridgeValue::Undefined)
                    });
                    let call = json!({"callId": uuid::Uuid::new_v4().to_string(), "method": method, "params": params});
                    let reply = async {
                        let value = service.invoke(
                            "call",
                            BridgeValue::List(vec![call.into(), cancelled, progress]),
                        )?;
                        settle(value).await?.json()
                    }
                    .await;
                    let reply = reply
                        .and_then(|value| {
                            serde_json::from_value::<ForeignReply>(value).map_err(|error| {
                                rutis_bridge::session::Error::Value(error.to_string())
                            })
                        })
                        .map_err(|error| {
                            tracing::warn!(%error, "foreign method failed");
                            RpcError::wire(-32603, "Method binding failed")
                        })?;
                    match reply {
                        ForeignReply::Value {
                            kind: DirectReply::Value,
                            value,
                        } => Ok(value),
                        ForeignReply::Error {
                            code,
                            app_error,
                            message,
                        } => Err(RpcError::app(code, app_error, message)),
                    }
                })
            }),
        ))
    }
}
