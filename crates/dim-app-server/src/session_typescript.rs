//! Structural TypeScript rows; runtime and native bindings share each row's owner.
use crate::{
    Result, ServerError,
    methods::ManagementMethods,
    settings::{SourceDeclaration, WorkflowPackages},
};
use dim_agent_capabilities::{
    methods::{SessionMethods, within},
    tool_registry::valid_tool_name,
};
use dim_agent_runtime::{
    AgentBuilder, BRANCH_GROUP, SESSION_GROUP, SessionConfig, SessionIdentity, SessionLifecycle,
    SessionScope,
};
use rutis::{BoxFuture, CordisError, Ctx, Effect, Plugin, PluginFactory, TypeKey};
use rutis_bridge::{
    runtime::{LocalRuntime, Stdio},
    session::{HostDispatch, Value as RpcValue, host_key, host_key_in},
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
    /// Instanced Session and branch parts.
    pub rows: Vec<Value>,
    /// Optional process owners, independent of Session lifetime.
    pub process_rows: Vec<Value>,
}

// Existing Session and branch services exported by name (build.rs).
rutis_bridge::include_exports!(mod exports);
/// The exported branch services: a row that injects one runs in each branch.
const BRANCH_SERVICES: [&str; 4] = [
    "dispatcher",
    "branchLifecycle",
    "timeline",
    "timelineIdentity",
];

/// The rows' `settings` entries (settings-service §3.1, §5.4): a row's
/// package declares its sources next to `provides`, and the row id is their
/// owner. Settings checks and compiles each declaration when it applies.
pub(crate) fn settings_sources(config: &SessionTypeScriptConfig) -> Result<Vec<SourceDeclaration>> {
    let mut sources = Vec::new();
    for row in config.rows.iter().chain(&config.process_rows) {
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
    /// Rows that inject a branch service: one instance per branch.
    pub branch_rows: Vec<Value>,
    pub process_rows: Vec<Value>,
    pub management: Vec<(String, String)>,
    /// Each Session namespace an enabled row provides, with that row's id:
    /// Rpc reads the row's copy in a Session where the namespace has no
    /// active provider.
    pub namespaces: Vec<(String, String)>,
}

impl Parts {
    pub fn new(
        config: &SessionTypeScriptConfig,
        builder: &mut AgentBuilder,
        catalog: &mut ServiceCatalog,
        management: &[(String, String)],
        file_roots: &[PathBuf],
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
        let mut branch_ids = HashSet::new();
        let mut namespaces = Vec::new();
        let mut process_namespaces = Vec::new();
        // Before the resolver takes the catalog.
        catalog.export_all(exports::all());
        let mut host_catalog = catalog.clone();
        host_catalog.share_registered_only();
        for (row, process) in config
            .process_rows
            .iter()
            .map(|row| (row, true))
            .chain(config.rows.iter().map(|row| (row, false)))
        {
            let id = row
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .ok_or_else(|| invalid("TypeScript row requires id"))?;
            let name = row
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| invalid("TypeScript row requires name"))?;
            if !ids.insert(id) {
                return Err(invalid("duplicate TypeScript row id"));
            }
            let disabled = match row.get("disabled") {
                None => false,
                Some(Value::Bool(disabled)) => *disabled,
                Some(_) => return Err(invalid("row disabled must be boolean")),
            };
            let provides = row
                .get("provides")
                .and_then(Value::as_object)
                .ok_or_else(|| invalid("TypeScript row requires RowSchema.provides"))?;
            if snapshots
                .get(name)
                .is_some_and(|snapshot| snapshot != provides)
            {
                return Err(invalid(
                    "rows of one module must share the same provides snapshot",
                ));
            }
            let branch = !process
                && row
                    .get("inject")
                    .and_then(Value::as_array)
                    .is_some_and(|inject| {
                        inject.iter().any(|name| {
                            name.as_str()
                                .is_some_and(|name| BRANCH_SERVICES.contains(&name))
                        })
                    });
            if branch {
                // A branch part's services live in its branch instance; the
                // Session families belong to Session rows.
                for service in provides.keys() {
                    if !identities.insert(service.clone()) {
                        return Err(invalid("duplicate Session TypeScript service identity"));
                    }
                    if [
                        "toolContribution:",
                        "sessionMethods:",
                        "sessionEvents:",
                        "managementMethods:",
                    ]
                    .iter()
                    .any(|family| service.starts_with(family))
                    {
                        return Err(invalid(
                            "a branch row cannot provide Session tools, methods or observers",
                        ));
                    }
                    catalog.register_shared_instance(service, BRANCH_GROUP);
                }
                branch_ids.insert(id);
                snapshots.insert(name.to_owned(), provides.clone());
                continue;
            }
            for (service, methods) in provides {
                if !identities.insert(service.clone()) {
                    return Err(invalid("duplicate Session TypeScript service identity"));
                }
                if process {
                    if host_catalog.is_shared(service) {
                        return Err(invalid(
                            "TypeScript process service collides with a native export",
                        ));
                    }
                    if ["toolContribution:", "sessionMethods:", "sessionEvents:"]
                        .iter()
                        .any(|family| service.starts_with(family))
                    {
                        return Err(invalid(
                            "a process row cannot provide Session tools, methods or observers",
                        ));
                    }
                    if let Some(namespace) = service.strip_prefix("managementMethods:") {
                        ManagementMethods::check_namespace(namespace)
                            .map_err(|error| ServerError::Configuration(error.to_string()))?;
                        if management
                            .iter()
                            .any(|(owner, _)| within(namespace, owner) || within(owner, namespace))
                        {
                            return Err(invalid(
                                "TypeScript management namespace overlaps a native owner",
                            ));
                        }
                        if methods != &serde_json::json!({"describe":"async", "call":"async"})
                            || process_namespaces
                                .iter()
                                .any(|(owner, _)| owner == namespace)
                        {
                            return Err(invalid("invalid or duplicate management namespace"));
                        }
                        for existing in builder.session_method_namespaces() {
                            crate::methods::SessionNamespaces::check(
                                &[(namespace.to_owned(), id.to_owned())],
                                existing,
                            )
                            .map_err(|error| ServerError::Configuration(error.to_string()))?;
                        }
                        process_namespaces.push((namespace.to_owned(), id.to_owned()));
                    }
                    catalog.register_shared(service);
                    continue;
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
                    crate::methods::SessionNamespaces::check(management, namespace)
                        .map_err(|error| ServerError::Configuration(error.to_string()))?;
                    crate::methods::SessionNamespaces::check(&process_namespaces, namespace)
                        .map_err(|error| ServerError::Configuration(error.to_string()))?;
                    if !disabled {
                        builder.session_methods(namespace);
                        namespaces.push((namespace.to_owned(), id.to_owned()));
                    }
                } else if let Some(part) = service.strip_prefix("sessionEvents:") {
                    if part.is_empty() || methods != &serde_json::json!({"bind":"sync"}) {
                        return Err(invalid("invalid sessionEvents identity or method mode"));
                    }
                } else {
                    return Err(invalid("unsupported Session TypeScript service family"));
                }
                catalog.register_shared_instance(service, SESSION_GROUP);
            }
            snapshots.insert(name.to_owned(), provides.clone());
        }
        for row in &config.process_rows {
            if let Some(injects) = row.get("inject").and_then(Value::as_array) {
                for service in injects.iter().filter_map(Value::as_str) {
                    if matches!(
                        catalog.key_in(service, &rutis_loader::Build::default()),
                        Err(LoaderError::OutsideInstance { .. })
                    ) {
                        return Err(invalid(
                            "a process row cannot inject Session or branch services",
                        ));
                    }
                }
            }
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
            file_roots: file_roots.to_vec().into(),
        });
        // `settings` is the host's input to Settings, not a loader field.
        let without_settings = |mut row: Value| {
            if let Value::Object(fields) = &mut row {
                fields.remove("settings");
            }
            row
        };
        let (branch_rows, rows) = config
            .rows
            .iter()
            .cloned()
            .map(without_settings)
            .partition(|row| row["id"].as_str().is_some_and(|id| branch_ids.contains(id)));
        Ok(Self {
            runtime,
            rows_runtime,
            resolver,
            rows,
            branch_rows,
            process_rows: config
                .process_rows
                .iter()
                .cloned()
                .map(without_settings)
                .collect(),
            management: process_namespaces,
            namespaces,
        })
    }
}

pub(crate) struct SessionResolver {
    runtime: Arc<RuntimeResolver>,
    snapshots: HashMap<String, Map<String, Value>>,
    file_roots: Arc<[PathBuf]>,
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
                .filter_map(|name| {
                    name.strip_prefix("sessionMethods:")
                        .or_else(|| name.strip_prefix("managementMethods:"))
                        .map(str::to_owned)
                })
                .collect();
            let tools: Arc<[String]> = snapshot
                .keys()
                .filter(|name| name.starts_with("toolContribution:"))
                .cloned()
                .collect();
            let events: Arc<[String]> = snapshot
                .keys()
                .filter(|name| name.starts_with("sessionEvents:"))
                .cloned()
                .collect();
            let mut resolved = Resolved::new(Arc::new(BoundFactory {
                inner: original.factory.clone(),
                scope: None,
                namespaces: namespaces.clone(),
                tools: tools.clone(),
                events: events.clone(),
                injects: original.factory.injects().to_vec(),
                file_roots: self.file_roots.clone(),
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
            let file_roots = self.file_roots.clone();
            resolved = resolved.with_scoped(Arc::new(move |build| {
                let inner = scoped(build)?;
                let scope = build
                    .instances()
                    .find(|(group, _)| *group == SESSION_GROUP)
                    .map(|(_, instance)| SessionScope { instance });
                let mut injects = inner.injects().to_vec();
                if !events.is_empty() {
                    let scope = scope.ok_or_else(|| {
                        CordisError::PluginFailed("Session observer outside a Session".into())
                    })?;
                    injects.extend([
                        scope.key::<SessionIdentity>(),
                        scope.key::<SessionConfig>(),
                        scope.key::<SessionLifecycle>(),
                    ]);
                }
                Ok(Arc::new(BoundFactory {
                    inner,
                    scope,
                    namespaces: namespaces.clone(),
                    tools: tools.clone(),
                    events: events.clone(),
                    injects,
                    file_roots: file_roots.clone(),
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
    events: Arc<[String]>,
    injects: Vec<TypeKey>,
    file_roots: Arc<[PathBuf]>,
}
impl PluginFactory<Value> for BoundFactory {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn injects(&self) -> &[TypeKey] {
        &self.injects
    }
    fn validate_config(&self, config: &Value) -> std::result::Result<(), CordisError> {
        self.inner.validate_config(config)
    }
    fn build(&self, config: &Value) -> std::result::Result<Box<dyn Plugin>, CordisError> {
        let packages = if self.scope.is_none() && self.injects.contains(&host_key("workflowFiles"))
        {
            Some(Arc::new(WorkflowPackages::from_config(
                config,
                &self.file_roots,
            )?))
        } else {
            None
        };
        Ok(Box::new(BoundRow {
            inner: self.inner.build(config)?,
            scope: self.scope,
            namespaces: self.namespaces.clone(),
            tools: self.tools.clone(),
            events: self.events.clone(),
            injects: self.injects.clone(),
            packages,
        }))
    }
}

struct BoundRow {
    inner: Box<dyn Plugin>,
    scope: Option<SessionScope>,
    namespaces: Arc<[String]>,
    tools: Arc<[String]>,
    events: Arc<[String]>,
    injects: Vec<TypeKey>,
    packages: Option<Arc<WorkflowPackages>>,
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
        _: rutis::Call<'a, ()>,
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
        &self.injects
    }
    fn validate(&self) -> std::result::Result<(), CordisError> {
        self.inner.validate()
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            if let Some(packages) = &self.packages {
                ctx.provide_as(
                    TypeKey::instance::<WorkflowPackages>(ctx.instance()),
                    packages.clone(),
                )?;
            }
            for name in self.tools.iter() {
                ctx.provide_as(
                    TypeKey::keyed_dynamic::<CancellationToken>(name.as_str())
                        .with_instance(self.scope.expect("Session tool row").instance),
                    Arc::new(ctx.cancellation_token()),
                )?;
            }
            let key = |name: &str| match self.scope {
                Some(scope) => host_key_in(name, scope.instance),
                None => host_key(name),
            };
            let keys: Arc<[TypeKey]> = self
                .namespaces
                .iter()
                .map(|namespace| {
                    let family = if self.scope.is_some() {
                        "sessionMethods"
                    } else {
                        "managementMethods"
                    };
                    key(&format!("{family}:{namespace}"))
                })
                .chain(self.tools.iter().map(|name| key(name)))
                .chain(self.events.iter().map(|name| key(name)))
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
                        _ = cancelled.cancelled() => return Err(CordisError::PluginFailed("TypeScript row binding cancelled".into())),
                    }
                };
                if let Some(namespace) = self.namespaces.get(index) {
                    match self.scope {
                        Some(scope) => {
                            SessionMethods::provide_foreign(ctx, &scope, namespace, service)
                                .await?;
                        }
                        None => {
                            ManagementMethods::provide_foreign(ctx, namespace, service).await?;
                        }
                    }
                } else if index >= self.namespaces.len() + self.tools.len() {
                    let scope = self.scope.expect("Session observer row");
                    let identity = scope.service::<SessionIdentity>(ctx)?;
                    let config = scope.service::<SessionConfig>(ctx)?;
                    let lifecycle = scope.service::<SessionLifecycle>(ctx)?;
                    let cancelled = cancelled.clone();
                    let closing = RpcValue::future(async move {
                        tokio::select! {
                            _ = lifecycle.closing() => {},
                            _ = cancelled.cancelled() => {},
                        }
                        Ok(RpcValue::Undefined)
                    });
                    let args =
                        serde_json::json!({"sessionId":identity.session_id, "config":config});
                    let reply = service
                        .invoke("bind", RpcValue::List(vec![args.into(), closing]))
                        .map_err(|error| CordisError::PluginFailed(error.into()))?;
                    if !matches!(reply, RpcValue::Data(Value::Null)) {
                        return Err(CordisError::PluginFailed(
                            "sessionEvents.bind must return null synchronously".into(),
                        ));
                    }
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
