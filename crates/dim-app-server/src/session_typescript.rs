//! Structural Session rows; runtime and native bindings share each row's owner.
use crate::{Result, ServerError, settings::SourceDeclaration};
use dim_agent_capabilities::{methods::SessionMethods, tool_registry::valid_tool_name};
use dim_agent_runtime::{AgentBuilder, SESSION_GROUP, SessionScope};
use rutis::{BoxFuture, CordisError, Ctx, Effect, Plugin, PluginFactory, TypeKey};
use rutis_bridge::{
    runtime::{LocalRuntime, Stdio},
    session::{HostDispatch, host_key_in},
};
use rutis_loader::{LoaderError, Resolved, Resolver, RuntimeResolver, ServiceCatalog};
use serde_json::{Map, Value};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};
use tokio_util::sync::CancellationToken;

/// Local Node runtime arguments and ordinary loader rows, with a
/// `RowSchema.provides` snapshot for names needed before Node starts.
#[derive(Clone, Debug)]
pub struct SessionTypeScriptConfig {
    pub node: PathBuf,
    pub js_runtime: PathBuf,
    pub anchor: PathBuf,
    pub rows: Vec<Value>,
}

/// The rows' `settings` entries (settings-service §3.1, §5.4): a row's
/// package declares its sources next to `provides`, and the row id is their
/// owner. Settings checks and compiles each declaration when it applies.
pub(crate) fn settings_sources(config: &SessionTypeScriptConfig) -> Result<Vec<SourceDeclaration>> {
    let mut sources = Vec::new();
    for row in &config.rows {
        let Some(entries) = row.get("settings") else {
            continue;
        };
        let owner = row
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                ServerError::Configuration("Session TypeScript row requires id".into())
            })?;
        let entries: Vec<SourceDeclaration> =
            serde_json::from_value(entries.clone()).map_err(|error| {
                ServerError::Configuration(format!(
                    "settings of Session TypeScript row {owner}: {error}"
                ))
            })?;
        sources.extend(entries.into_iter().map(|mut entry| {
            entry.owner = owner.to_owned();
            entry
        }));
    }
    Ok(sources)
}

pub(crate) struct Parts {
    pub runtime: Arc<LocalRuntime>,
    pub rows_runtime: Arc<RuntimeResolver>,
    pub resolver: Arc<SessionResolver>,
    pub rows: Vec<Value>,
}

impl Parts {
    pub fn new(
        config: &SessionTypeScriptConfig,
        builder: &mut AgentBuilder,
        catalog: &mut ServiceCatalog,
        management: &[(String, String)],
    ) -> Result<Self> {
        let invalid = |message: &str| ServerError::Configuration(message.into());
        if [&config.node, &config.js_runtime, &config.anchor]
            .iter()
            .any(|path| !path.is_absolute())
        {
            return Err(invalid("Session TypeScript runtime paths must be absolute"));
        }
        let mut ids = HashSet::new();
        let mut identities = HashSet::new();
        let mut snapshots = HashMap::new();
        for row in &config.rows {
            let id = row
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .ok_or_else(|| invalid("Session TypeScript row requires id"))?;
            let name = row
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| invalid("Session TypeScript row requires name"))?;
            if !ids.insert(id) {
                return Err(invalid("duplicate Session TypeScript row id"));
            }
            let disabled = match row.get("disabled") {
                None => false,
                Some(Value::Bool(disabled)) => *disabled,
                Some(_) => return Err(invalid("row disabled must be boolean")),
            };
            let provides = row
                .get("provides")
                .and_then(Value::as_object)
                .ok_or_else(|| invalid("Session TypeScript row requires RowSchema.provides"))?;
            if snapshots
                .get(name)
                .is_some_and(|snapshot| snapshot != provides)
            {
                return Err(invalid(
                    "rows of one module must share the same provides snapshot",
                ));
            }
            for (service, methods) in provides {
                if !identities.insert(service.clone()) {
                    return Err(invalid("duplicate Session TypeScript service identity"));
                }
                if let Some(tool) = service.strip_prefix("toolContribution:") {
                    if !valid_tool_name(tool)
                        || methods != &serde_json::json!({"declaration":"sync", "execute":"async"})
                    {
                        return Err(invalid("invalid toolContribution identity or method modes"));
                    }
                    if builder.provided_tools().any(|existing| existing == tool) {
                        return Err(invalid("TypeScript tool collides with an existing tool"));
                    }
                    // Keep disabled identities known when restoring saved tool selections.
                    builder.session_tool(tool);
                } else if let Some(namespace) = service.strip_prefix("sessionMethods:") {
                    if methods != &serde_json::json!({"describe":"async", "call":"async"})
                        || builder
                            .session_method_namespaces()
                            .iter()
                            .any(|existing| existing == namespace)
                    {
                        return Err(invalid("invalid or duplicate Session method namespace"));
                    }
                    crate::methods::SessionNamespaces::new(management, &[namespace.to_owned()])
                        .map_err(|error| ServerError::Configuration(error.to_string()))?;
                    if !disabled {
                        builder.session_methods(namespace);
                    }
                } else {
                    return Err(invalid("unsupported Session TypeScript service family"));
                }
                catalog.register_shared_instance(service, SESSION_GROUP);
            }
            snapshots.insert(name.to_owned(), provides.clone());
        }
        let runtime = Arc::new(
            LocalRuntime::node(&config.js_runtime, &config.anchor)
                .interpreter(config.node.clone())
                .named("session-node")
                .stdin(Stdio::Null)
                .stdout(Stdio::Null)
                .stderr(Stdio::Null),
        );
        let rows_runtime = Arc::new(RuntimeResolver::node(runtime.handle()).with_catalog(catalog));
        let resolver = Arc::new(SessionResolver {
            runtime: rows_runtime.clone(),
            snapshots,
        });
        Ok(Self {
            runtime,
            rows_runtime,
            resolver,
            // `settings` is the host's input to Settings, not a loader field.
            rows: config
                .rows
                .iter()
                .cloned()
                .map(|mut row| {
                    if let Value::Object(fields) = &mut row {
                        fields.remove("settings");
                    }
                    row
                })
                .collect(),
        })
    }
}

pub(crate) struct SessionResolver {
    runtime: Arc<RuntimeResolver>,
    snapshots: HashMap<String, Map<String, Value>>,
}
impl Resolver for SessionResolver {
    fn resolve<'a>(
        &'a self,
        name: &'a str,
    ) -> BoxFuture<'a, std::result::Result<Arc<Resolved>, LoaderError>> {
        Box::pin(async move {
            let snapshot = self
                .snapshots
                .get(name)
                .ok_or_else(|| LoaderError::NotFound { name: name.into() })?;
            let original = self.runtime.resolve(name).await?;
            match original.meta.get("provides") {
                Some(Value::Object(actual)) if actual == snapshot => {}
                // The pinned SDK resolves this placeholder again before releasing RuntimeRows.
                None if original.meta["source"] == "runtime"
                    && original.meta["schema"]
                        == "unavailable: the Cordis runtime is not running" => {}
                _ => {
                    return Err(LoaderError::Resolve {
                        name: name.into(),
                        message: "row provides differs from its structural declaration".into(),
                    });
                }
            }
            let namespaces: Arc<[String]> = snapshot
                .keys()
                .filter_map(|name| name.strip_prefix("sessionMethods:").map(str::to_owned))
                .collect();
            let tools: Arc<[String]> = snapshot
                .keys()
                .filter(|name| name.starts_with("toolContribution:"))
                .cloned()
                .collect();
            let mut resolved = Resolved::new(Arc::new(BoundFactory {
                inner: original.factory.clone(),
                scope: None,
                namespaces: namespaces.clone(),
                tools: tools.clone(),
            }))
            .with_schema(original.schema.clone())
            .with_meta(original.meta.clone());
            if original.foreign_scope {
                resolved = resolved.with_foreign_scope();
            }
            if let Some(declares) = &original.declares {
                resolved = resolved.with_declarations(declares.clone());
            }
            let scoped = original
                .scoped
                .clone()
                .ok_or_else(|| LoaderError::Resolve {
                    name: name.into(),
                    message: "runtime row has no scoped factory".into(),
                })?;
            resolved = resolved.with_scoped(Arc::new(move |build| {
                Ok(Arc::new(BoundFactory {
                    inner: scoped(build)?,
                    scope: Some(SessionScope {
                        instance: build.instance(SESSION_GROUP)?,
                    }),
                    namespaces: namespaces.clone(),
                    tools: tools.clone(),
                }))
            }));
            Ok(Arc::new(resolved))
        })
    }
}

struct BoundFactory {
    inner: Arc<dyn PluginFactory<Value>>,
    scope: Option<SessionScope>,
    namespaces: Arc<[String]>,
    tools: Arc<[String]>,
}
impl PluginFactory<Value> for BoundFactory {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn injects(&self) -> &[TypeKey] {
        self.inner.injects()
    }
    fn validate_config(&self, config: &Value) -> std::result::Result<(), CordisError> {
        self.inner.validate_config(config)
    }
    fn build(&self, config: &Value) -> std::result::Result<Box<dyn Plugin>, CordisError> {
        Ok(Box::new(BoundRow {
            inner: self.inner.build(config)?,
            scope: self.scope.ok_or_else(|| {
                CordisError::PluginFailed("TypeScript part outside a Session".into())
            })?,
            namespaces: self.namespaces.clone(),
            tools: self.tools.clone(),
        }))
    }
}

struct BoundRow {
    inner: Box<dyn Plugin>,
    scope: SessionScope,
    namespaces: Arc<[String]>,
    tools: Arc<[String]>,
}
struct ProjectedServices {
    keys: Arc<[TypeKey]>,
    changed: Arc<tokio::sync::Notify>,
}
impl rutis::Listener<rutis::ServiceChanged> for ProjectedServices {
    fn call<'a>(
        &'a self,
        _: &'a Ctx,
        event: &'a rutis::ServiceChanged,
    ) -> BoxFuture<'a, std::result::Result<Option<()>, CordisError>> {
        Box::pin(async move {
            if self.keys.contains(&event.key) {
                self.changed.notify_one();
            }
            Ok(None)
        })
    }
}
impl Plugin for BoundRow {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn injects(&self) -> &[TypeKey] {
        self.inner.injects()
    }
    fn validate(&self) -> std::result::Result<(), CordisError> {
        self.inner.validate()
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            for name in self.tools.iter() {
                ctx.provide_as(
                    TypeKey::keyed_dynamic::<CancellationToken>(name.as_str())
                        .with_instance(self.scope.instance),
                    Arc::new(ctx.cancellation_token()),
                )?;
            }
            let keys: Arc<[TypeKey]> = self
                .namespaces
                .iter()
                .map(|namespace| {
                    host_key_in(&format!("sessionMethods:{namespace}"), self.scope.instance)
                })
                .chain(
                    self.tools
                        .iter()
                        .map(|name| host_key_in(name, self.scope.instance)),
                )
                .collect();
            let changed = Arc::new(tokio::sync::Notify::new());
            // rows.load may reply before its service notification is projected.
            // Subscribe before loading, then read only this row's own services.
            let listener = if keys.is_empty() {
                None
            } else {
                Some(ctx.events().on(
                    ctx,
                    &rutis::EventKey::of(),
                    ProjectedServices {
                        keys: keys.clone(),
                        changed: changed.clone(),
                    },
                )?)
            };
            let cleanup = self.inner.apply(ctx).await?;
            ctx.effect(move || cleanup)?;
            let cancelled = ctx.cancellation_token();
            for (index, key) in keys.iter().enumerate() {
                let service = loop {
                    if let Some(service) = ctx.get_as::<dyn HostDispatch>(key.clone()) {
                        break service;
                    }
                    tokio::select! {
                        _ = changed.notified() => {},
                        _ = cancelled.cancelled() => return Err(CordisError::PluginFailed("Session part binding cancelled".into())),
                    }
                };
                if let Some(namespace) = self.namespaces.get(index) {
                    SessionMethods::provide_foreign(ctx, &self.scope, namespace, service).await?;
                }
            }
            if let Some(listener) = listener {
                listener
                    .dispose()
                    .await
                    .map_err(|error| CordisError::PluginFailed(error.to_string().into()))?;
            }
            Ok(Effect::Done)
        })
    }
}
