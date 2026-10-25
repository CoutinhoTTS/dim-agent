//! The part registry and the instances' mounters
//! ([ADR 006 D3](../../../docs/adr/006-plugin-contract.md#d3-a-plugin-is-one-row-each-instance-mounts-the-parts-registered-for-it)).
//!
//! A plugin registers part generators for the Session or branch scope while
//! it runs; the registration is an effect of the caller, withdrawn when the
//! caller unloads. Every Session and branch holds two mounters. The required
//! one mounts the parts the assembly profile requires and hands them to the
//! instance, which publishes only once they are Active. The optional one
//! mounts every other registered part, which nothing waits for. Both
//! follow registrations and withdrawals while the instance is open, and a
//! mounted part closes with its instance.
use crate::{Names, SessionConfig};
use rutis::{
    BoxFuture, CordisError, Ctx, Effect, FiberState, FiberView, Keyed, Plugin, TypeKey, TypedPlugin,
};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};
use tokio::sync::watch;

/// The scope a part is registered for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PartScope {
    Session,
    Branch,
}

/// What a generator sees of the instance it builds a part for.
pub struct Mount<'a> {
    /// The names its dependencies resolve to in this instance.
    pub names: &'a Names<'a>,
    /// The Session's configuration (the enclosing Session's for a branch).
    pub session: &'a SessionConfig,
    pub session_id: &'a str,
    /// Whether the Session was restored rather than created.
    pub restored: bool,
    /// The branch, for a branch part.
    pub branch_id: Option<&'a str>,
}

/// Builds a part for one instance, or nothing when that instance does not
/// need one, such as a Session whose configuration turns the feature off.
pub type Generator =
    Arc<dyn Fn(&Mount<'_>) -> Result<Option<Box<dyn Plugin>>, CordisError> + Send + Sync>;

struct Entry {
    id: u64,
    scope: PartScope,
    name: String,
    generator: Generator,
}

#[derive(Default)]
struct State {
    next_entry: u64,
    next_mounter: u64,
    entries: Vec<Arc<Entry>>,
    /// The copies mounted from each entry, with the mounter that owns each.
    copies: HashMap<u64, Vec<(u64, FiberView)>>,
}

/// The parts plugins register for Sessions and branches. One registry serves
/// every Agent built from an [`AgentBuilder`](crate::AgentBuilder); the host
/// hands it to its process plugins ([`AgentBuilder::parts`](crate::AgentBuilder::parts)).
pub struct PartRegistry {
    state: Mutex<State>,
    version: watch::Sender<u64>,
}

impl Default for PartRegistry {
    fn default() -> Self {
        Self {
            state: Mutex::default(),
            version: watch::channel(0).0,
        }
    }
}

impl std::fmt::Debug for PartRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.lock().unwrap();
        let entries: Vec<_> = state
            .entries
            .iter()
            .map(|entry| (&entry.name, entry.scope))
            .collect();
        f.debug_struct("PartRegistry")
            .field("entries", &entries)
            .finish()
    }
}

impl PartRegistry {
    /// Registers a part generator for `scope` on behalf of the plugin that
    /// owns `ctx`. Every open instance of the scope mounts it, and so does
    /// every later one. `name` is the part's name, by which the assembly
    /// profile requires it ([`AgentBuilder::require_part`](crate::AgentBuilder::require_part)).
    ///
    /// The registration is withdrawn when that plugin unloads: every instance
    /// unmounts its copy, and the plugin's unload waits for those copies to
    /// unload.
    pub fn register(
        self: &Arc<Self>,
        ctx: &Ctx,
        scope: PartScope,
        name: impl Into<String>,
        generator: impl Fn(&Mount<'_>) -> Result<Option<Box<dyn Plugin>>, CordisError>
        + Send
        + Sync
        + 'static,
    ) -> Result<(), CordisError> {
        let registry = self.clone();
        let id = self.add(scope, name.into(), Arc::new(generator));
        let registered = ctx.effect(move || {
            Effect::AsyncDisposer(Box::new(move || {
                Box::pin(async move {
                    registry.withdraw(id).await;
                    Ok(())
                })
            }))
        });
        if registered.is_err() {
            // The caller is already unloading: nothing of it may stay.
            let registry = self.clone();
            tokio::spawn(async move { registry.withdraw(id).await });
        }
        registered.map(|_| ())
    }

    fn add(&self, scope: PartScope, name: String, generator: Generator) -> u64 {
        let id = {
            let mut state = self.state.lock().unwrap();
            state.next_entry += 1;
            let id = state.next_entry;
            state.entries.push(Arc::new(Entry {
                id,
                scope,
                name,
                generator,
            }));
            id
        };
        self.version.send_modify(|version| *version += 1);
        id
    }

    /// Removes the entry and unloads every copy mounted from it.
    async fn withdraw(&self, id: u64) {
        let copies = {
            let mut state = self.state.lock().unwrap();
            state.entries.retain(|entry| entry.id != id);
            state.copies.remove(&id).unwrap_or_default()
        };
        self.version.send_modify(|version| *version += 1);
        futures_util::future::join_all(copies.into_iter().map(|(_, view)| async move {
            let _ = view.dispose().await;
        }))
        .await;
    }

    /// Every registration of every scope at one moment, by name: a mounter
    /// decides and mounts from the same snapshot, in the same order each time.
    fn snapshot(&self) -> Vec<Arc<Entry>> {
        let mut entries = self.state.lock().unwrap().entries.clone();
        entries.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        entries
    }

    /// Records a mounted copy. False when its entry was withdrawn meanwhile;
    /// the caller then unloads the copy itself.
    fn attach(&self, entry: u64, mounter: u64, view: FiberView) -> bool {
        let mut state = self.state.lock().unwrap();
        if !state.entries.iter().any(|e| e.id == entry) {
            return false;
        }
        state.copies.entry(entry).or_default().push((mounter, view));
        true
    }

    /// Forgets a mounter's copies: its instance unloads them with itself.
    fn detach(&self, mounter: u64) {
        let mut state = self.state.lock().unwrap();
        for copies in state.copies.values_mut() {
            copies.retain(|(owner, _)| *owner != mounter);
        }
        state.copies.retain(|_, copies| !copies.is_empty());
    }

    fn mounter(&self) -> u64 {
        let mut state = self.state.lock().unwrap();
        state.next_mounter += 1;
        state.next_mounter
    }
}

/// Every copy an instance's mounters mounted: its close counts a copy that
/// already failed as it counts a failed row ([`crate::assembly`]).
pub(crate) type Copies = Arc<Mutex<Vec<FiberView>>>;

/// What a mounter knows of its instance.
pub(crate) struct MountSite {
    pub scope: PartScope,
    pub registry: Arc<PartRegistry>,
    /// Registered parts the profile requires, by name.
    pub required: Arc<HashSet<String>>,
    pub catalog: Arc<rutis_loader::ServiceCatalog>,
    pub build: rutis_loader::Build,
    pub session: SessionConfig,
    pub session_id: String,
    pub restored: bool,
    pub branch_id: Option<String>,
    /// Where the required mounter hands its required parts: the instance
    /// publishes only once they are Active, as its required rows.
    pub mounted: Arc<Mutex<Vec<FiberView>>>,
    pub copies: Copies,
}

impl MountSite {
    /// Calls `f` with this instance's [`Mount`].
    fn with_mount<R>(&self, f: impl FnOnce(&Mount<'_>) -> R) -> R {
        let names = Names::copy(&self.catalog, &self.build);
        f(&Mount {
            names: &names,
            session: &self.session,
            session_id: &self.session_id,
            restored: self.restored,
            branch_id: self.branch_id.as_deref(),
        })
    }
}

/// One of an instance's two mounters ([module docs](self)).
pub(crate) struct PartMounter {
    pub site: Arc<MountSite>,
    pub required: bool,
}

impl PartMounter {
    /// The module and plugin name of a mounter.
    pub(crate) fn module(scope: PartScope, required: bool) -> &'static str {
        match (scope, required) {
            (PartScope::Session, true) => "SessionParts",
            (PartScope::Session, false) => "SessionOptionalParts",
            (PartScope::Branch, true) => "BranchParts",
            (PartScope::Branch, false) => "BranchOptionalParts",
        }
    }

    /// Whether `name` is a mounter's module name.
    /// Whether the row `id` is a mounter: a module name, with the suffix
    /// the assembly adds to a name in use.
    pub(crate) fn is_row(id: &str) -> bool {
        Self::is_module(id.split_once('#').map_or(id, |(module, _)| module))
    }

    pub(crate) fn is_module(name: &str) -> bool {
        [PartScope::Session, PartScope::Branch]
            .into_iter()
            .flat_map(|scope| [Self::module(scope, true), Self::module(scope, false)])
            .any(|module| module == name)
    }

    fn owns(&self, entry: &Entry) -> bool {
        self.site.required.contains(&entry.name) == self.required
    }

    /// The required names `snapshot` has no registration of, in any scope.
    fn missing(&self, snapshot: &[Arc<Entry>]) -> Vec<String> {
        let mut missing: Vec<String> = self
            .site
            .required
            .iter()
            .filter(|name| !snapshot.iter().any(|entry| &entry.name == *name))
            .cloned()
            .collect();
        missing.sort();
        missing
    }

    /// Mounts the entries of `snapshot` this mounter owns that it has not
    /// seen yet, and forgets withdrawn ones, whose copies the registry
    /// unloads. Returns the new copies. A registered part that fails to build
    /// fails a required mounter; for the optional one it affects only that
    /// part.
    fn reconcile(
        &self,
        ctx: &Ctx,
        id: u64,
        seen: &mut HashMap<u64, Option<FiberView>>,
        snapshot: &[Arc<Entry>],
    ) -> Result<Vec<FiberView>, CordisError> {
        let site = &self.site;
        let entries: Vec<_> = snapshot
            .iter()
            .filter(|entry| entry.scope == site.scope)
            .cloned()
            .collect();
        let live: HashSet<_> = entries.iter().map(|entry| entry.id).collect();
        seen.retain(|entry, _| live.contains(entry));
        let mut new = vec![];
        for entry in entries {
            if !self.owns(&entry) || seen.contains_key(&entry.id) {
                continue;
            }
            let part = match site.with_mount(|mount| (entry.generator)(mount)) {
                Ok(part) => part,
                Err(error) if self.required => return Err(error),
                Err(error) => {
                    tracing::warn!(part = %entry.name, %error, "a registered part failed to build");
                    seen.insert(entry.id, None);
                    continue;
                }
            };
            let Some(part) = part else {
                seen.insert(entry.id, None);
                continue;
            };
            let view = mount(ctx, part, &site.copies)?;
            if !site.registry.attach(entry.id, id, view.clone()) {
                tokio::spawn(async move {
                    let _ = view.dispose().await;
                });
                continue;
            }
            seen.insert(entry.id, Some(view.clone()));
            new.push(view);
        }
        Ok(new)
    }
}

/// Mounts a copy under the mounter and unloads it before rutis's own
/// cascade does (effects run last in, first out). A copy that already failed
/// showed its failure when it happened and is not reported again; a cleanup
/// error of a running copy is the mounter's, as a row's would be.
fn mount(ctx: &Ctx, part: Box<dyn Plugin>, copies: &Copies) -> Result<FiberView, CordisError> {
    let view = ctx.plugin(part);
    {
        let mut copies = copies.lock().unwrap();
        copies.retain(|copy| copy.state().state != FiberState::Disposed);
        copies.push(view.clone());
    }
    let owned = view.clone();
    let registered = ctx.effect(move || {
        Effect::AsyncDisposer(Box::new(move || {
            Box::pin(async move {
                match owned.state().state {
                    FiberState::Disposed => Ok(()),
                    FiberState::Failed => {
                        let _ = owned.dispose().await;
                        Ok(())
                    }
                    _ => owned
                        .dispose()
                        .await
                        .map_err(|error| CordisError::Aggregate {
                            errors: vec![error],
                        }),
                }
            })
        }))
    });
    if let Err(error) = registered {
        // The mounter is unloading: the copy must not outlive it.
        let late = view.clone();
        tokio::spawn(async move {
            let _ = late.dispose().await;
        });
        return Err(error);
    }
    Ok(view)
}

impl Plugin for PartMounter {
    fn name(&self) -> &str {
        Self::module(self.site.scope, self.required)
    }

    fn injects(&self) -> &[TypeKey] {
        &[]
    }

    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let site = self.site.clone();
            let id = site.registry.mounter();
            let mut waiting = vec![];
            let mut changes = site.registry.version.subscribe();
            changes.mark_unchanged();
            let mut seen = HashMap::new();
            let cancel = ctx.cancellation_token();
            // A required part nothing has registered yet holds the instance
            // as a required row does. The required mounter mounts once every
            // required part is registered, from the snapshot it checked, so
            // a registration between the check and the mounting cannot be
            // left out of what the instance waits for.
            let snapshot = loop {
                let snapshot = site.registry.snapshot();
                let missing = if self.required {
                    self.missing(&snapshot)
                } else {
                    vec![]
                };
                if missing.is_empty() {
                    break snapshot;
                }
                tokio::select! {
                    changed = changes.changed() => if changed.is_err() { break snapshot },
                    () = cancel.cancelled() => {
                        return Err(CordisError::PluginFailed(
                            format!("required parts not registered: {}", missing.join(", "))
                                .into(),
                        ));
                    }
                }
            };
            waiting.extend(self.reconcile(ctx, id, &mut seen, &snapshot)?);
            let follow = {
                let this = PartMounter {
                    site: site.clone(),
                    required: self.required,
                };
                let (ctx, cancel) = (ctx.clone(), cancel.clone());
                tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            changed = changes.changed() => if changed.is_err() { return },
                            _ = cancel.cancelled() => return,
                        }
                        // After publication a late part never fails the instance.
                        let snapshot = this.site.registry.snapshot();
                        if let Err(error) = this.reconcile(&ctx, id, &mut seen, &snapshot) {
                            tracing::warn!(%error, "a registered part failed to build");
                        }
                    }
                })
            };
            let registry = site.registry.clone();
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        cancel.cancel();
                        let _ = follow.await;
                        registry.detach(id);
                        Ok(())
                    })
                }))
            })?;
            if self.required {
                // Not awaited here: the loader waits for a Loading row, so
                // the instance's creation waits for these instead.
                site.mounted.lock().unwrap().extend(waiting);
            }
            Ok(Effect::Done)
        })
    }
}

/// One part a [`Feature`] registers.
#[derive(Clone)]
struct FeaturePart {
    scope: PartScope,
    name: String,
    generator: Generator,
}

/// A plugin that owns nothing for the process: while it runs it registers
/// its Session and branch parts ([ADR 006 D3](../../../docs/adr/006-plugin-contract.md#d3-a-plugin-is-one-row-each-instance-mounts-the-parts-registered-for-it)).
/// A feature that holds process state writes its own plugin and registers
/// with [`PartRegistry::register`]. Add it with
/// [`AgentBuilder::feature`](crate::AgentBuilder::feature); the host's
/// profile requires parts by name
/// ([`AgentBuilder::require_part`](crate::AgentBuilder::require_part)).
#[derive(Clone)]
pub struct Feature {
    name: String,
    parts: Vec<FeaturePart>,
}

impl Feature {
    /// A feature whose plugin is named `name`.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            parts: vec![],
        }
    }

    /// A part of every Session named `name`: `make` builds a Session's copy
    /// from the names its dependencies resolve to there, usually
    /// `|names| names.part(MyPart)`.
    pub fn session(
        self,
        name: impl Into<String>,
        make: impl Fn(&Names<'_>) -> Result<Box<dyn Plugin>, CordisError> + Send + Sync + 'static,
    ) -> Self {
        self.when(PartScope::Session, name, move |mount| {
            make(mount.names).map(Some)
        })
    }

    /// A part of every branch named `name`, as [`Self::session`].
    pub fn branch(
        self,
        name: impl Into<String>,
        make: impl Fn(&Names<'_>) -> Result<Box<dyn Plugin>, CordisError> + Send + Sync + 'static,
    ) -> Self {
        self.when(PartScope::Branch, name, move |mount| {
            make(mount.names).map(Some)
        })
    }

    /// A part of `scope` that `generator` builds or not for each instance,
    /// from its identity and configuration ([`Mount`]).
    pub fn when(
        mut self,
        scope: PartScope,
        name: impl Into<String>,
        generator: impl Fn(&Mount<'_>) -> Result<Option<Box<dyn Plugin>>, CordisError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.parts.push(FeaturePart {
            scope,
            name: name.into(),
            generator: Arc::new(generator),
        });
        self
    }

    /// The name of its plugin.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The names of the parts it registers.
    pub fn part_names(&self) -> impl Iterator<Item = &str> {
        self.parts.iter().map(|part| part.name.as_str())
    }

    /// Its plugin, keyed from `names`.
    pub fn plugin(&self, names: &Names<'_>) -> Result<Box<dyn Plugin>, CordisError> {
        names.part(FeaturePlugin(self.clone()))
    }
}

struct FeaturePlugin(Feature);

impl TypedPlugin for FeaturePlugin {
    type Deps = (Keyed<PartRegistry>,);
    fn name(&self) -> &str {
        &self.0.name
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (Keyed(registry),): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            for part in &self.0.parts {
                let generator = part.generator.clone();
                registry.register(ctx, part.scope, part.name.clone(), move |mount| {
                    generator(mount)
                })?;
            }
            Ok(Effect::Done)
        })
    }
}

/// Provides a placement's registry to its plugins, at the process name
/// `partRegistry`.
pub(crate) struct PartRegistryPlugin {
    pub registry: Arc<PartRegistry>,
    pub key: rutis::DepKey<PartRegistry>,
}

impl TypedPlugin for PartRegistryPlugin {
    type Deps = ();
    fn name(&self) -> &str {
        "PartRegistry"
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx, (): ()) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            ctx.provide_as(self.key.key().clone(), self.registry.clone())?;
            Ok(Effect::Done)
        })
    }
}
