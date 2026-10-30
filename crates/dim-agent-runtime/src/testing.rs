//! A minimal host for tests, examples and tools.
//!
//! A host has one loader, and an Agent is built in it ([`AgentBuilder::build`]).
//! The App Server is the product's host. [`MinimalHost`] is the same assembly
//! with nothing else: one rutis root, one loader with the builder's modules
//! and plugin rows, the builder's [`AGENT_GROUP`] row and one Agent instance.
//! It owns the root's lifetime.

use crate::assembly::{Factory, loader_error, row};
use crate::*;
use rutis::host::{BoxFuture as RutisFuture, CtxHost, DynPlugin, Effect, ErrorKind};
use rutis::{Ctx, Error};
use rutis_loader::Loader;
use serde_json::{Value, json};
use std::sync::{Arc, OnceLock};
use tokio_util::sync::CancellationToken;

/// One root, its loader and one Agent built in it.
///
/// It dereferences to its [`Agent`]. [`MinimalHost::close`] closes the
/// Agent's Sessions and then shuts the root down; dropping the host does the
/// same in the background.
pub struct MinimalHost {
    agent: Agent,
    root: Ctx,
    loader: Loader,
    /// The assembly's rows, the first layer of every reconcile.
    rows: Value,
    closed: bool,
}

impl MinimalHost {
    /// Assembles a host for `builder` and builds its Agent: the builder's
    /// plugins as process rows, its modules, its [`AGENT_GROUP`] row and one
    /// Agent instance. The host's part registry is its own, so two hosts built
    /// from one builder mount only their own plugins' parts.
    pub async fn start(builder: &AgentBuilder) -> Result<Self> {
        Self::start_with(builder, |_, _| Ok(())).await
    }

    /// [`Self::start`], with `prepare` run on the new root before the loader
    /// mounts: it may provide host values there and register modules of the
    /// caller's own, for rows it adds later with [`Self::set_rows`]. Host
    /// plugins are rows ([`AgentBuilder::plugin`]), not children of the root.
    pub async fn start_with(
        builder: &AgentBuilder,
        prepare: impl FnOnce(&Ctx, &mut Modules) -> Result<()>,
    ) -> Result<Self> {
        let root = Ctx::root_with_sink(
            tokio::runtime::Handle::current(),
            Arc::new(|error| tracing::error!(%error, "Agent runtime error")),
        );
        match assemble(builder, &root, prepare).await {
            Ok((agent, loader, rows)) => Ok(Self {
                agent,
                root,
                loader,
                rows,
                closed: false,
            }),
            Err(error) => Err(match root.shutdown().await {
                Ok(()) => error,
                Err(cleanup) => error.with_cleanup(AgentError::from(cleanup)),
            }),
        }
    }

    /// The Agent built in this host.
    pub fn agent(&self) -> &Agent {
        &self.agent
    }

    /// The host's root context.
    pub fn root(&self) -> &Ctx {
        &self.root
    }

    /// The host's loader.
    pub fn loader(&self) -> &Loader {
        &self.loader
    }

    /// Replaces the caller's rows, a layer reconciled after the assembly's:
    /// `rows` are loader rows whose modules [`Self::start_with`] registered.
    /// An empty list removes them.
    pub async fn set_rows(&self, rows: Vec<Value>) -> Result<()> {
        let layers = [
            ("assembly", self.rows.clone()),
            ("host", json!([{ "insert": rows }])),
        ]
        .into_iter()
        .map(|(name, rows)| {
            serde_json::from_value(rows)
                .map(|rows| rutis_loader::Layer::new(name, rows))
                .map_err(|error| AgentError::Build(error.to_string()))
        })
        .collect::<Result<Vec<_>>>()?;
        let report = self
            .loader
            .reconcile(layers, None)
            .await
            .map_err(loader_error)?;
        if !report.failures.is_empty() || !report.issues.is_empty() {
            return Err(AgentError::Build(format!(
                "{:?} {:?}",
                report.failures, report.issues
            )));
        }
        Ok(())
    }

    /// Closes the Agent's Sessions, then shuts the root down, which unloads
    /// the Agent instance and the host's rows.
    pub async fn close(mut self) -> Result<()> {
        self.closed = true;
        let mut errors = vec![];
        errors.extend(self.agent.close().await.err());
        errors.extend(self.root.shutdown().await.err().map(AgentError::from));
        AgentError::aggregate(errors)
    }
}

impl std::ops::Deref for MinimalHost {
    type Target = Agent;
    fn deref(&self) -> &Agent {
        &self.agent
    }
}

impl Drop for MinimalHost {
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        let (close, root) = (self.agent.close(), self.root.clone());
        self.root.handle().clone().spawn(async move {
            let _ = close.await;
            let _ = root.shutdown().await;
        });
    }
}

/// Mounts the loader on `root`, reconciles the assembly and creates the
/// Agent instance, then builds the Agent in it.
async fn assemble(
    builder: &AgentBuilder,
    root: &Ctx,
    prepare: impl FnOnce(&Ctx, &mut Modules) -> Result<()>,
) -> Result<(Agent, Loader, Value)> {
    let builder = builder.with_own_registry();
    let mut builtins = Modules::new();
    builtins.register("AgentAnchor", |build| {
        let slot = build.value::<AnchorSlot>().ok_or_else(|| {
            rutis::Error::from(ErrorKind::PluginFailed(
                "Agent created without its slot".into(),
            ))
        })?;
        Factory::new(move || Box::new(AgentAnchor((*slot).clone())) as Box<dyn DynPlugin>)
    });
    prepare(root, &mut builtins)?;
    let mut rows = match builder.has_plugins() {
        true => builder.plugin_rows(&mut builtins).await?,
        false => vec![],
    };
    let session_row = builder.install(&mut builtins, []).await?;
    rows.push(builder.agent_row([row("AgentAnchor"), session_row]));
    let plugin = rutis_loader::LoaderPlugin::new(
        builtins,
        rutis_loader::LoaderOptions {
            catalog: builder.catalog().clone(),
            ..Default::default()
        },
    );
    let loader = plugin.handle();
    let view = root.plugin(plugin);
    let cancel = CancellationToken::new();
    wait_active(
        root,
        std::slice::from_ref(&view),
        builder.opening_timeout(),
        &cancel,
    )
    .await?;
    let assembly = json!([{ "insert": rows }]);
    let layer = serde_json::from_value(assembly.clone())
        .map_err(|error| AgentError::Build(error.to_string()))?;
    let report = loader
        .reconcile(vec![rutis_loader::Layer::new("assembly", layer)], None)
        .await
        .map_err(loader_error)?;
    if !report.failures.is_empty() || !report.issues.is_empty() {
        return Err(AgentError::Build(format!(
            "{:?} {:?}",
            report.failures, report.issues
        )));
    }
    let slot = AnchorSlot::default();
    loader
        .create_instance(root, AGENT_GROUP)
        .with(slot.clone())
        .await
        .map_err(loader_error)?;
    let anchor = slot
        .0
        .get()
        .cloned()
        .ok_or_else(|| AgentError::Build("Agent anchor did not load".into()))?;
    let agent = builder.build(loader.clone(), &anchor);
    Ok((agent, loader, assembly))
}

#[derive(Clone, Default)]
struct AnchorSlot(Arc<OnceLock<Ctx>>);

/// The plugin of the minimal host's Agent instance: the Agent's Sessions are
/// created from its context.
struct AgentAnchor(AnchorSlot);
impl DynPlugin for AgentAnchor {
    fn name(&self) -> &str {
        "AgentAnchor"
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> RutisFuture<'a, std::result::Result<Effect, Error>> {
        Box::pin(async move {
            self.0.0.set(ctx.clone()).map_err(|_| {
                rutis::Error::from(ErrorKind::PluginFailed("Agent anchor reloaded".into()))
            })?;
            Ok(Effect::Done)
        })
    }
}
