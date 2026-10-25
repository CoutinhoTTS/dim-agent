//! Rows of other languages (Node, Python) in the process, `session` or
//! `branch` group. Each row declares its group (rutis usage §2); the
//! runtime and native bindings share each row's owner.
use crate::{
    Result, ServerError,
    methods::ManagementMethods,
    settings::{SourceDeclaration, WorkflowPackages},
};
use dim_agent_capabilities::{
    methods::{SessionMethods, within},
    tool_registry::ToolRegistry,
};
use dim_agent_runtime::{AgentBuilder, BRANCH_GROUP, SESSION_GROUP, SessionScope};
use rutis::{BoxFuture, CordisError, Ctx, Effect, Plugin, PluginFactory, TypeKey};
use rutis_bridge::{
    runtime::{LocalRuntime, Stdio},
    session::{HostDispatch, host_key, host_key_in},
};
use rutis_loader::{Build, LoaderError, Resolved, Resolver, RuntimeResolver};
use serde_json::{Map, Value};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

// The Session and branch services served by name (build.rs). Public for the
// generated catalog (`xtask graph`).
rutis_bridge::include_exports!(pub mod exports);

/// The runtimes of other languages and their loader rows, each with a
/// `RowSchema.provides` snapshot for names needed before its runtime starts.
#[derive(Clone, Debug)]
pub struct SessionTypeScriptConfig {
    pub node: PathBuf,
    pub js_runtime: PathBuf,
    pub anchor: PathBuf,
    /// The Python runtime of rows named `py:<module>`, if any.
    pub python: Option<PythonConfig>,
    /// Loader rows: a row's `group` is `"session"` or `"branch"`; a process
    /// row has none.
    pub rows: Vec<Value>,
}

/// Python 3.10 or later with the `rutis` package, running the modules of
/// `project`.
#[derive(Clone, Debug)]
pub struct PythonConfig {
    pub interpreter: PathBuf,
    pub project: PathBuf,
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

/// A runtime of other languages and the resolver of its rows.
pub(crate) struct ForeignRuntime {
    /// Its name, which its rows gate on.
    pub name: &'static str,
    pub runtime: Arc<LocalRuntime>,
    pub rows: Arc<RuntimeResolver>,
}

pub(crate) struct Parts {
    pub node: ForeignRuntime,
    pub python: Option<ForeignRuntime>,
    pub resolver: Arc<SessionResolver>,
    /// The rows of each group, without the host's fields.
    pub session_rows: Vec<Value>,
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
        management: &[(String, String)],
        file_roots: &[PathBuf],
    ) -> Result<Self> {
        let invalid = |message: &str| ServerError::Configuration(message.into());
        if [&config.node, &config.js_runtime, &config.anchor]
            .into_iter()
            .chain(
                config
                    .python
                    .iter()
                    .flat_map(|python| [&python.interpreter, &python.project]),
            )
            .any(|path| !path.is_absolute())
        {
            return Err(invalid("plugin runtime paths must be absolute"));
        }
        let mut ids = HashSet::new();
        let mut snapshots = HashMap::new();
        let mut namespaces = Vec::new();
        let mut process_namespaces = Vec::new();
        let (mut session_rows, mut branch_rows, mut process_rows) =
            (Vec::new(), Vec::new(), Vec::new());
        // Before the rows register their names and the resolvers take the
        // catalog. A part of any language registers its tools with
        // `toolRegistry`, which provides its own dispatcher in each Session.
        builder
            .catalog_mut()
            .export_all(exports::all())
            .register_shared_instance(
                dim_agent_runtime::names::name_of::<ToolRegistry>(),
                SESSION_GROUP,
            );
        for row in &config.rows {
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
            if name.starts_with("py:") && config.python.is_none() {
                return Err(invalid("a py: row needs the Python runtime"));
            }
            let disabled = match row.get("disabled") {
                None => false,
                Some(Value::Bool(disabled)) => *disabled,
                Some(_) => return Err(invalid("row disabled must be boolean")),
            };
            let group = match row.get("group") {
                None => None,
                Some(Value::String(group)) if [SESSION_GROUP, BRANCH_GROUP].contains(&&**group) => {
                    Some(group.as_str())
                }
                Some(_) => {
                    return Err(invalid(
                        "row group must be \"session\" or \"branch\"; a process row has none",
                    ));
                }
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
            for (service, methods) in provides {
                // A name the catalog does not hold yet belongs to this row's
                // group, so each instance has its own (rutis usage §3.5).
                if let Err(LoaderError::UnknownService(_)) =
                    builder.catalog().key_in(service, &Build::default())
                {
                    match group {
                        None => builder.catalog_mut().register_shared(service),
                        Some(group) => builder
                            .catalog_mut()
                            .register_shared_instance(service, group),
                    };
                }
                // Host-side registration of the families Rpc and
                // ToolRegistry read, until they find providers by name.
                if group.is_none()
                    && let Some(namespace) = service.strip_prefix("managementMethods:")
                {
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
                if group != Some(SESSION_GROUP) {
                    continue;
                }
                if let Some(namespace) = service.strip_prefix("sessionMethods:") {
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
                }
            }
            snapshots.insert(name.to_owned(), provides.clone());
            // `settings` is the host's input to Settings and `group` says
            // where the row goes: neither is a loader field.
            let mut row = row.clone();
            if let Value::Object(fields) = &mut row {
                fields.remove("settings");
                fields.remove("group");
            }
            match group {
                None => process_rows.push(row),
                Some(SESSION_GROUP) => session_rows.push(row),
                Some(_) => branch_rows.push(row),
            }
        }
        let node = LocalRuntime::node(&config.js_runtime, &config.anchor)
            .interpreter(config.node.clone())
            .named("session-node");
        let node = ForeignRuntime::new("session-node", node, RuntimeResolver::node, builder);
        // Named `py`, so its rows are named `py:<module>`.
        let python = config.python.as_ref().map(|python| {
            let runtime =
                LocalRuntime::python(&python.project).interpreter(python.interpreter.clone());
            ForeignRuntime::new("py", runtime, RuntimeResolver::modules, builder)
        });
        let resolver = Arc::new(SessionResolver {
            node: node.rows.clone(),
            python: python.as_ref().map(|python| python.rows.clone()),
            snapshots,
            file_roots: file_roots.to_vec().into(),
        });
        Ok(Self {
            node,
            python,
            resolver,
            session_rows,
            branch_rows,
            process_rows,
            management: process_namespaces,
            namespaces,
        })
    }
    /// The runtimes the rows run in: Node, then Python if configured.
    pub fn runtimes(&self) -> impl Iterator<Item = &ForeignRuntime> {
        std::iter::once(&self.node).chain(&self.python)
    }
}

impl ForeignRuntime {
    fn new(
        name: &'static str,
        runtime: LocalRuntime,
        rows: impl FnOnce(rutis_bridge::runtime::RuntimeHandle) -> RuntimeResolver,
        builder: &AgentBuilder,
    ) -> Self {
        // Its standard error reaches the process log, line by line (ADR 005).
        let runtime = Arc::new(
            runtime
                .stdin(Stdio::Null)
                .stdout(Stdio::Null)
                .stderr(Stdio::Lines),
        );
        let rows = Arc::new(rows(runtime.handle()).with_catalog(builder.catalog()));
        Self {
            name,
            runtime,
            rows,
        }
    }
}

pub(crate) struct SessionResolver {
    node: Arc<RuntimeResolver>,
    python: Option<Arc<RuntimeResolver>>,
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
            let runtime = match &self.python {
                Some(python) if name.starts_with("py:") => python,
                _ => &self.node,
            };
            let original = runtime.resolve(name).await?;
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
            let names = |prefix: &str| -> Arc<[String]> {
                snapshot
                    .keys()
                    .filter_map(|name| name.strip_prefix(prefix).map(str::to_owned))
                    .collect()
            };
            let families = Families {
                session_methods: names("sessionMethods:"),
                management: names("managementMethods:"),
            };
            let mut resolved = Resolved::new(Arc::new(BoundFactory {
                inner: original.factory.clone(),
                place: Place::Process,
                families: families.clone(),
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
                // Where the row is placed: its innermost instance.
                let place = match build.instances().next() {
                    None => Place::Process,
                    Some((group, instance)) if group == SESSION_GROUP => {
                        Place::Session(SessionScope { instance })
                    }
                    Some(_) => Place::Branch,
                };
                Ok(Arc::new(BoundFactory {
                    inner: scoped(build)?,
                    place,
                    families: families.clone(),
                    file_roots: file_roots.clone(),
                }))
            }));
            Ok(Arc::new(resolved))
        })
    }
}

/// The provided names of the families the host binds: Session methods in a
/// Session, management methods at the process.
#[derive(Clone)]
struct Families {
    session_methods: Arc<[String]>,
    management: Arc<[String]>,
}

#[derive(Clone, Copy)]
enum Place {
    Process,
    Session(SessionScope),
    Branch,
}

struct BoundFactory {
    inner: Arc<dyn PluginFactory<Value>>,
    place: Place,
    families: Families,
    file_roots: Arc<[PathBuf]>,
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
        let packages = match self.place {
            Place::Process if self.injects().contains(&host_key("workflowFiles")) => Some(
                Arc::new(WorkflowPackages::from_config(config, &self.file_roots)?),
            ),
            _ => None,
        };
        let namespaces = match self.place {
            Place::Process => self.families.management.clone(),
            Place::Session(_) => self.families.session_methods.clone(),
            Place::Branch => Arc::default(),
        };
        Ok(Box::new(BoundRow {
            inner: self.inner.build(config)?,
            place: self.place,
            namespaces,
            packages,
        }))
    }
}

struct BoundRow {
    inner: Box<dyn Plugin>,
    place: Place,
    namespaces: Arc<[String]>,
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
            if let Some(packages) = &self.packages {
                ctx.provide_as(
                    TypeKey::instance::<WorkflowPackages>(ctx.instance()),
                    packages.clone(),
                )?;
            }
            let session = match self.place {
                Place::Session(scope) => Some(scope),
                Place::Process | Place::Branch => None,
            };
            let keys: Arc<[TypeKey]> = self
                .namespaces
                .iter()
                .map(|namespace| match session {
                    Some(scope) => {
                        host_key_in(&format!("sessionMethods:{namespace}"), scope.instance)
                    }
                    None => host_key(&format!("managementMethods:{namespace}")),
                })
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
            for (namespace, key) in self.namespaces.iter().zip(keys.iter()) {
                let service = loop {
                    if let Some(service) = ctx.get_as::<dyn HostDispatch>(key.clone()) {
                        break service;
                    }
                    tokio::select! {
                        _ = changed.notified() => {},
                        _ = cancelled.cancelled() => return Err(CordisError::PluginFailed("TypeScript row binding cancelled".into())),
                    }
                };
                match session {
                    Some(scope) => {
                        SessionMethods::provide_foreign(ctx, &scope, namespace, service).await?;
                    }
                    None => {
                        ManagementMethods::provide_foreign(ctx, namespace, service).await?;
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

#[cfg(test)]
mod tests {
    use super::exports;
    use dim_agent_runtime::{AgentBuilder, AgentConfig, names::name_of};
    use rutis_loader::{Build, LoaderError};

    /// A Rust part depends on a service that crosses languages by its
    /// type's name: each listed export is named as the binding names its
    /// type and grouped as the catalog groups that name, and the marked
    /// process services are shared under their types' names.
    #[test]
    fn crossing_services_are_named_and_grouped_as_the_rust_binding_names_them() {
        let listed = [
            name_of::<dim_agent_capabilities::timeline_tree::TreeCommands>(),
            name_of::<dim_agent_runtime::SessionIdentity>(),
            name_of::<dim_agent_runtime::SessionLifecycle>(),
            name_of::<dim_agent_runtime::SessionConfig>(),
            name_of::<dim_agent_runtime::Dispatcher>(),
            name_of::<dim_agent_runtime::BranchLifecycle>(),
            name_of::<dim_agent_runtime::Timeline>(),
            name_of::<dim_agent_runtime::TimelineIdentity>(),
        ];
        let mut builder = AgentBuilder::new(AgentConfig::default());
        dim_agent_capabilities::register_names(&mut builder);
        let catalog = builder.catalog().clone();
        let exports = exports::all();
        let mut exported: Vec<_> = exports.iter().map(|export| export.name()).collect();
        exported.sort_unstable();
        let mut expected: Vec<_> = listed.iter().map(String::as_str).collect();
        expected.sort_unstable();
        assert_eq!(exported, expected);
        for export in &exports {
            let registered = match catalog.key_in(export.name(), &Build::default()) {
                Err(LoaderError::OutsideInstance { group, .. }) => Some(group),
                _ => None,
            };
            assert_eq!(registered.as_deref(), export.group(), "{}", export.name());
        }
        builder.catalog_mut().export_all(exports);
        for name in [
            name_of::<crate::agents::SessionPermissions>(),
            name_of::<crate::settings::WorkflowFiles>(),
        ] {
            assert!(rutis::service::is_exported(&name), "{name}");
            assert!(builder.catalog().is_shared(&name), "{name}");
            assert!(builder.catalog().key(&name).is_some(), "{name}");
        }
    }
}
