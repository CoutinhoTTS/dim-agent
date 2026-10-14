//! Process extension discovery and lifecycle isolation on the existing loader.
//! Consumers depend on this registry; only mounts depend on contributors.
use crate::{Capability, KeysOf, register_capability};
use rutis::{
    BoxFuture, CordisError, Ctx, DepKey, Effect, EventKey, Keyed, Listener, TypeKey, TypedPlugin,
};
use rutis_loader::{Builtins, Declared, EntryStatus, Instance, Loader, LoaderChanged};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    marker::PhantomData,
    sync::{Arc, Mutex},
};
use tokio::sync::watch;
use tokio_util::{
    sync::CancellationToken,
    task::{TaskTracker, task_tracker::TaskTrackerToken},
};

/// The consumer owns the contribution contract and validation, not its providers.
pub trait ExtensionPoint: Send + Sync + 'static {
    type Service: Send + Sync + 'static;
    const POINT: &'static str;
    fn validate(
        name: &str,
        service: &Self::Service,
        current: &RegistrySnapshot<Self>,
    ) -> Result<(), RegistryError>
    where
        Self: Sized;
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("invalid contribution {name}: {reason}")]
    Invalid { name: String, reason: String },
    #[error("contribution {0} is already registered")]
    Collision(String),
    #[error("extension registry is closed")]
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Configured {
    pub rows: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UndeterminedRow {
    pub row: String,
    pub why: String,
}

pub struct RegistrySnapshot<P: ExtensionPoint> {
    pub revision: u64,
    pub configured: BTreeMap<String, Configured>,
    pub undetermined: Vec<UndeterminedRow>,
    pub active: BTreeMap<String, Arc<Active<P>>>,
}
impl<P: ExtensionPoint> Clone for RegistrySnapshot<P> {
    fn clone(&self) -> Self {
        Self {
            revision: self.revision,
            configured: self.configured.clone(),
            undetermined: self.undetermined.clone(),
            active: self.active.clone(),
        }
    }
}
impl<P: ExtensionPoint> RegistrySnapshot<P> {
    pub fn inactive(&self) -> impl Iterator<Item = &str> {
        self.configured
            .keys()
            .filter(|name| !self.active.contains_key(*name))
            .map(String::as_str)
    }
}

pub enum Lookup<P: ExtensionPoint> {
    Active(Arc<Active<P>>),
    Inactive,
    Absent,
}

/// A retained generation never admits calls after its mount starts unloading.
pub struct Active<P: ExtensionPoint> {
    pub name: String,
    pub generation: u64,
    pub service: Arc<P::Service>,
    admission: Mutex<bool>,
    tasks: TaskTracker,
    cancel: CancellationToken,
}
pub struct CallGuard {
    pub cancel: CancellationToken,
    _task: TaskTrackerToken,
}
impl<P: ExtensionPoint> Active<P> {
    pub fn admit(&self) -> Option<CallGuard> {
        let open = self.admission.lock().expect("extension admission");
        open.then(|| CallGuard {
            cancel: self.cancel.clone(),
            _task: self.tasks.token(),
        })
    }
    fn close(&self) {
        *self.admission.lock().expect("extension admission") = false;
        self.cancel.cancel();
        self.tasks.close();
    }
}

pub struct ExtensionRegistry<P: ExtensionPoint> {
    state: Mutex<RegistrySnapshot<P>>,
    changed: watch::Sender<u64>,
    ready: watch::Sender<bool>,
}
impl<P: ExtensionPoint> Default for ExtensionRegistry<P> {
    fn default() -> Self {
        Self {
            state: Mutex::new(RegistrySnapshot {
                revision: 0,
                configured: BTreeMap::new(),
                undetermined: vec![],
                active: BTreeMap::new(),
            }),
            changed: watch::channel(0).0,
            ready: watch::channel(false).0,
        }
    }
}
impl<P: ExtensionPoint> ExtensionRegistry<P> {
    pub fn snapshot(&self) -> RegistrySnapshot<P> {
        self.state.lock().expect("extension registry").clone()
    }
    pub fn changed(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }
    /// Host startup barrier, awaited after assembly, never inside plugin apply.
    /// Initial discovery and mount attempts completed. Pending contributors are
    /// reported as inactive, not allowed to hold unrelated features at startup.
    pub async fn ready(&self) {
        let mut ready = self.ready.subscribe();
        let _ = ready.wait_for(|ready| *ready).await;
    }
    pub fn lookup(&self, name: &str) -> Lookup<P> {
        let state = self.state.lock().expect("extension registry");
        if let Some(active) = state.active.get(name) {
            Lookup::Active(active.clone())
        } else if state.configured.contains_key(name) {
            Lookup::Inactive
        } else {
            Lookup::Absent
        }
    }
    fn bump(&self, state: &mut RegistrySnapshot<P>) {
        state.revision += 1;
        self.changed.send_replace(state.revision);
    }
    fn configure(
        &self,
        configured: BTreeMap<String, Configured>,
        undetermined: Vec<UndeterminedRow>,
    ) {
        let mut state = self.state.lock().expect("extension registry");
        if state.configured == configured && state.undetermined == undetermined {
            return;
        }
        // Fence removed contributors before asynchronous instance disposal.
        state.active.retain(|name, active| {
            if configured.contains_key(name) {
                true
            } else {
                active.close();
                false
            }
        });
        state.configured = configured;
        state.undetermined = undetermined;
        self.bump(&mut state);
    }
    fn register(
        self: &Arc<Self>,
        ctx: &Ctx,
        name: String,
        service: Arc<P::Service>,
    ) -> Result<(), RegistryError> {
        let mut state = self.state.lock().expect("extension registry");
        if !state.configured.contains_key(&name) {
            return Err(RegistryError::Closed);
        }
        if state.active.contains_key(&name) {
            return Err(RegistryError::Collision(name));
        }
        P::validate(&name, &service, &state)?;
        let active = Arc::new(Active {
            name: name.clone(),
            generation: state.revision + 1,
            service,
            admission: Mutex::new(true),
            tasks: TaskTracker::new(),
            cancel: CancellationToken::new(),
        });
        // Register cleanup before making the contribution visible.
        let registry = self.clone();
        let owned = active.clone();
        ctx.effect(move || {
            Effect::AsyncDisposer(Box::new(move || {
                Box::pin(async move {
                    {
                        let mut state = registry.state.lock().expect("extension registry");
                        owned.close();
                        if state
                            .active
                            .get(&owned.name)
                            .is_some_and(|current| Arc::ptr_eq(current, &owned))
                        {
                            state.active.remove(&owned.name);
                            registry.bump(&mut state);
                        }
                    }
                    owned.tasks.wait().await;
                    Ok(())
                })
            }))
        })
        .map_err(|_| RegistryError::Closed)?;
        state.active.insert(name, active);
        self.bump(&mut state);
        Ok(())
    }
}

/// Stable, language-neutral loader declaration. The string denotes this point
/// and name; the actual injected key remains the concrete service type.
pub fn service_name<P: ExtensionPoint>(name: &str) -> String {
    format!("dim.extension/{}/{name}", P::POINT)
}
pub fn service_key<P: ExtensionPoint>(name: &str) -> TypeKey {
    TypeKey::keyed_dynamic::<P::Service>(name.to_owned())
}

/// Registers the registry and mount template, returning ordinary assembly rows.
pub fn register<P: ExtensionPoint>(builtins: &mut Builtins) -> Vec<Value> {
    let module = format!("{}-mount", P::POINT);
    builtins.register_with::<Value, _, _>(module.clone(), |build| {
        let name = build
            .value::<MountName>()
            .ok_or_else(|| CordisError::PluginFailed("missing extension name".into()))?;
        Ok(rutis::TypedFactory::with_keys(
            MountFactory::<P> {
                name: name.0.clone(),
                marker: PhantomData,
            },
            ((), DepKey::<P::Service>::dynamic(name.0.clone())),
        ))
    });
    let registry = register_capability(builtins, || RegistryPlugin::<P> {
        name: format!("{}-registry", P::POINT),
        marker: PhantomData,
    });
    for name in [&module, &registry] {
        builtins.declare(name, Arc::new(|_| Ok(Declared::default())));
    }
    vec![
        json!({"id":module,"group":true,"instanced":true,"config":[{"id":format!("{module}-entry"),"name":module}]}),
        json!({"id":registry,"name":registry}),
    ]
}

struct MountName(String);
struct MountFactory<P> {
    name: String,
    marker: PhantomData<P>,
}
impl<P: ExtensionPoint> rutis::TypedPluginFactory<Value> for MountFactory<P> {
    type Plugin = Mount<P>;
    fn name(&self) -> &str {
        P::POINT
    }
    fn build(&self, _: &Value) -> Result<Self::Plugin, CordisError> {
        Ok(Mount {
            name: self.name.clone(),
            marker: PhantomData,
        })
    }
}
struct Mount<P> {
    name: String,
    marker: PhantomData<P>,
}
impl<P: ExtensionPoint> TypedPlugin for Mount<P> {
    type Deps = (Arc<ExtensionRegistry<P>>, Keyed<P::Service>);
    fn name(&self) -> &str {
        P::POINT
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (registry, Keyed(service)): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            if let Err(error) = registry.register(ctx, self.name.clone(), service) {
                tracing::warn!(point = P::POINT, name = self.name, %error, "extension rejected");
            }
            Ok(Effect::Done)
        })
    }
}
struct Changes(watch::Sender<()>);
impl Listener<LoaderChanged> for Changes {
    fn call<'a>(
        &'a self,
        _: &'a Ctx,
        _: &'a LoaderChanged,
    ) -> BoxFuture<'a, Result<Option<()>, CordisError>> {
        self.0.send_replace(());
        Box::pin(async { Ok(None) })
    }
}
struct RegistryPlugin<P> {
    name: String,
    marker: PhantomData<P>,
}
impl<P: ExtensionPoint> TypedPlugin for RegistryPlugin<P> {
    type Deps = (Arc<Loader>,);
    fn name(&self) -> &str {
        &self.name
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (loader,): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let registry = Arc::new(ExtensionRegistry::<P>::default());
            ctx.provide_as(TypeKey::of::<ExtensionRegistry<P>>(), registry.clone())?;
            let (changed, changes) = watch::channel(());
            ctx.events()
                .on(ctx, &EventKey::<LoaderChanged>::of(), Changes(changed))?;
            let ctx = ctx.clone();
            let stop = ctx.cancellation_token();
            let worker_ctx = ctx.clone();
            ctx.effect(move || {
                let stopping = stop.clone();
                let worker =
                    tokio::spawn(discover(worker_ctx, loader, registry, changes, stopping));
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        stop.cancel();
                        worker
                            .await
                            .map_err(|error| CordisError::PluginFailed(Box::new(error)))?
                    })
                }))
            })?;
            Ok(Effect::Done)
        })
    }
}
impl<P: ExtensionPoint> Capability for RegistryPlugin<P> {
    fn keys(&self) -> KeysOf<Self> {
        ((),)
    }
}

async fn discover<P: ExtensionPoint>(
    ctx: Ctx,
    loader: Arc<Loader>,
    registry: Arc<ExtensionRegistry<P>>,
    mut changes: watch::Receiver<()>,
    stop: CancellationToken,
) -> Result<(), CordisError> {
    let mut mounts: BTreeMap<String, Instance> = BTreeMap::new();
    let group = format!("{}-mount", P::POINT);
    let mut failures = vec![];
    // Startup discovery must not label a still-loading contributor unavailable.
    // The registry's apply does not await this worker, so it participates in
    // normal assembly without adding a dependency cycle.
    tokio::select! {
        biased;
        _ = stop.cancelled() => return Ok(()),
        _ = loader.settled() => {}
    }
    loop {
        changes.borrow_and_update();
        let mut configured: BTreeMap<String, Configured> = BTreeMap::new();
        let mut undetermined = vec![];
        for entry in loader.entries() {
            if entry.instance.is_some()
                || matches!(entry.status, EntryStatus::Disabled | EntryStatus::Inactive)
            {
                continue;
            }
            let Some(module) = entry.options.get("name").and_then(Value::as_str) else {
                continue;
            };
            let declared = match loader.resolve(module).await {
                Ok(module) => module
                    .declares
                    .as_ref()
                    .ok_or_else(|| "no declarations".to_owned())
                    .and_then(|declares| {
                        declares(entry.options.get("config").unwrap_or(&Value::Null))
                    }),
                Err(error) => Err(error.to_string()),
            };
            match declared {
                Ok(declared) => {
                    for service in declared.provides {
                        if let Some(name) = service.strip_prefix(&service_name::<P>("")) {
                            configured
                                .entry(name.to_owned())
                                .or_insert_with(|| Configured { rows: vec![] })
                                .rows
                                .push(entry.id.clone());
                        }
                    }
                }
                Err(why) => undetermined.push(UndeterminedRow { row: entry.id, why }),
            }
        }
        for entry in configured.values_mut() {
            entry.rows.sort();
        }
        undetermined.sort_by(|a, b| a.row.cmp(&b.row));
        registry.configure(configured.clone(), undetermined);
        let removed: Vec<_> = mounts
            .keys()
            .filter(|name| !configured.contains_key(*name))
            .cloned()
            .collect();
        for name in removed {
            if let Some(instance) = mounts.remove(&name)
                && let Err(error) = instance.view.shutdown().await
            {
                tracing::warn!(point = P::POINT, name, %error, "extension mount disposal failed");
                failures.push(error.to_string());
            }
        }
        for name in configured.keys() {
            if mounts.contains_key(name) {
                continue;
            }
            match loader
                .create_instance(&ctx, &group)
                .with(MountName(name.clone()))
                .await
            {
                Ok(instance) => {
                    mounts.insert(name.clone(), instance);
                }
                Err(error) => {
                    tracing::warn!(point = P::POINT, name, %error, "extension mount unavailable")
                }
            }
        }
        // Instance creation emits LoaderChanged too. Finish a stable scan before
        // allowing hello to observe the initial method set.
        if changes.has_changed().unwrap_or(false) && !stop.is_cancelled() {
            continue;
        }
        registry.ready.send_replace(true);
        tokio::select! {
            biased;
            _ = stop.cancelled() => break,
            changed = changes.changed() => if changed.is_err() { break },
        }
    }
    registry.configure(BTreeMap::new(), vec![]);
    for (_, instance) in mounts {
        if let Err(error) = instance.view.shutdown().await {
            failures.push(error.to_string());
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(CordisError::PluginFailed(failures.join("; ").into()))
    }
}
