//! The daemon side of UI leaves (Tui design §3.6). A leaf is a rutis node
//! in its own process: `UiLeaf` starts it through the local transport,
//! links to it with `PeerPlugin`, and `TuiRow` loads the leaf's `Tui` plugin
//! there with `plugins.load`, unloading it in its effect. Disposing a
//! `UiLeaf` therefore unloads `Tui` (the leaf restores its terminal), then
//! closes the link, and the transport ends the process. The daemon writes
//! no terminal and runs no UI-specific control protocol.

use dim_agent_capabilities::methods::RpcError;
use rutis::{BoxFuture, CordisError, Ctx, Effect, FiberView, Plugin, TypeKey, TypedPlugin};
use rutis_bridge::{
    Features, Identity, LinkConfig, LinkState, Peer, PeerPlugin, StaticIdentity,
    channel::PeerId,
    peer_key,
    session::{Error as SessionError, settle},
    transport::local::{Handover, LocalTransport, Spawn, Stdio},
    transport_key,
};
use serde_json::{Value as Json, json};
use std::{
    ffi::OsString,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::oneshot;

/// The transport and identity keys of leaf links; distinct from any other
/// local transport the daemon assembles.
const LINKS: &str = "ui-leaf";
const DAEMON_PEER: &str = "dim-daemon";
/// The host catalog entry the leaf offers.
const HOSTED: &str = "Tui";
/// How long a leaf may take from spawn to a settled `Tui` load.
const START: Duration = Duration::from_secs(20);
/// How long an unload may take before the link is closed anyway.
const UNLOAD: Duration = Duration::from_secs(3);

/// How the daemon starts leaves: its own endpoint and model configuration
/// travel to every leaf in the hosted `Tui` config.
pub(crate) struct UiLeaves {
    ctx: Ctx,
    transport: Arc<LocalTransport>,
    program: PathBuf,
    endpoint: OsString,
    model_config: Option<PathBuf>,
}

/// One leaf's identity and the Session it shows.
pub(crate) struct LeafSpec {
    pub ui_id: String,
    pub generation: u64,
    pub token: String,
    pub session_id: String,
    pub tty: String,
    pub cwd: PathBuf,
    pub home: PathBuf,
}

impl UiLeaves {
    /// Mounts one `UiLeaf` and resolves once its `Tui` load settled.
    /// `stopped` runs when the link stops after that.
    pub async fn start(
        &self,
        spec: LeafSpec,
        stopped: Box<dyn Fn() + Send + Sync>,
    ) -> Result<FiberView, RpcError> {
        let peer = PeerId::new(spec.ui_id.clone()).map_err(leaf_failed)?;
        let spawner = format!("spawn-{}", spec.ui_id);
        let mut spawn = Spawn::new(self.program.clone(), peer.clone());
        spawn.args = vec![
            "ui-leaf".into(),
            "--leaf-peer".into(),
            peer.to_string().into(),
        ];
        spawn.cwd = Some(spec.cwd.clone());
        spawn.stdin = Stdio::Null;
        spawn.stdout = Stdio::Null;
        spawn.stderr = Stdio::Lines;
        #[cfg(unix)]
        {
            spawn.handover = Handover::DialBack;
        }
        self.transport.spawner(&spawner, spawn);
        let config = json!({
            "uiId": spec.ui_id,
            "generation": spec.generation,
            "token": spec.token,
            "sessionId": spec.session_id,
            "tty": spec.tty,
            "endpoint": PathBuf::from(&self.endpoint),
            "home": spec.home,
            "cwd": spec.cwd,
            "modelConfig": self.model_config,
        });
        let (loaded, settled) = oneshot::channel();
        let view = self.ctx.plugin(UiLeaf {
            name: format!("UiLeaf#{}", spec.ui_id),
            peer,
            address: format!("spawn:{spawner}"),
            config,
            loaded: Mutex::new(Some(loaded)),
            stopped: Mutex::new(Some(stopped)),
        });
        let result = match tokio::time::timeout(START, settled).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(leaf_failed("the leaf did not start")),
            Err(_) => Err(leaf_failed("the leaf did not start in time")),
        };
        match result {
            Ok(()) => Ok(view),
            Err(error) => {
                let _ = view.dispose().await;
                Err(error)
            }
        }
    }
}

/// One leaf: its link and the row loading `Tui` there, as children, so
/// disposing it releases both in reverse order.
struct UiLeaf {
    name: String,
    peer: PeerId,
    address: String,
    config: Json,
    loaded: Mutex<Option<oneshot::Sender<Result<(), RpcError>>>>,
    stopped: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

impl UiLeaf {
    fn report(&self, result: Result<(), RpcError>) {
        if let Some(loaded) = self.loaded.lock().expect("ui leaf").take() {
            let _ = loaded.send(result);
        }
    }
}

impl Plugin for UiLeaf {
    fn name(&self) -> &str {
        &self.name
    }

    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let mut link = LinkConfig::dial(self.peer.clone(), LINKS, LINKS, &self.address);
            // A leaf process is not restarted: its link ends with it.
            link.reconnect = false;
            let peer = PeerPlugin::new(link, Features::default())
                .map_err(|error| CordisError::PluginFailed(error.into()))?;
            let handle = peer.handle();
            let link = ctx.plugin(peer);
            if let Err(error) = (&link).await {
                self.report(Err(leaf_failed(format!(
                    "the leaf link did not start: {error}"
                ))));
                return Ok(Effect::Done);
            }
            let mut state = handle
                .link_state()
                .ok_or_else(|| CordisError::PluginFailed("the leaf link did not start".into()))?;
            if let Err(error) = ready(&mut state).await {
                self.report(Err(error));
                return Ok(Effect::Done);
            }
            let (loaded, settled) = oneshot::channel();
            ctx.plugin(TuiRow {
                injects: [peer_key(&self.peer)],
                config: self.config.clone(),
                loaded: Mutex::new(Some(loaded)),
            });
            let result = settled
                .await
                .unwrap_or_else(|_| Err(leaf_failed("the leaf link ended during the load")));
            let failed = result.is_err();
            self.report(result);
            if failed {
                return Ok(Effect::Done);
            }
            let stopped = self.stopped.lock().expect("ui leaf").take();
            let monitor = tokio::spawn(async move {
                while state.changed().await.is_ok() {
                    if matches!(&*state.borrow_and_update(), LinkState::Stopped { .. }) {
                        break;
                    }
                }
                if let Some(stopped) = stopped {
                    stopped();
                }
            });
            Ok(Effect::Disposer(Box::new(move || {
                monitor.abort();
                Ok(())
            })))
        })
    }
}

async fn ready(state: &mut tokio::sync::watch::Receiver<LinkState>) -> Result<(), RpcError> {
    loop {
        match &*state.borrow_and_update() {
            LinkState::Ready { .. } => return Ok(()),
            LinkState::Stopped { error, .. } | LinkState::Waiting { error, .. } => {
                return Err(leaf_failed(format!("the leaf link failed: {error}")));
            }
            LinkState::Idle | LinkState::Connecting => {}
        }
        if state.changed().await.is_err() {
            return Err(leaf_failed("the leaf link ended"));
        }
    }
}

/// The hosted `Tui` row: loaded on apply, unloaded in its effect. An unload
/// after the link ended counts as done: the hosted row went with the link.
/// The loader's `peer:` row does the same; it is private, and loader rows
/// are persistent configuration while a leaf exists per `ui.open`.
struct TuiRow {
    injects: [TypeKey; 1],
    config: Json,
    loaded: Mutex<Option<oneshot::Sender<Result<(), RpcError>>>>,
}

impl Plugin for TuiRow {
    fn name(&self) -> &str {
        "UiLeaf/TuiRow"
    }

    fn injects(&self) -> &[TypeKey] {
        &self.injects
    }

    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let session = ctx
                .require_as::<Peer>(self.injects[0].clone())?
                .connection()
                .clone();
            let key = ctx.instance().to_string();
            let load = async {
                let loaded = session
                    .invoke_async(
                        "",
                        "plugins.load",
                        json!([key, HOSTED, self.config, [], []]).into(),
                    )
                    .await?;
                settle(loaded).await
            };
            let result = load
                .await
                .map(|_| ())
                .map_err(|error| leaf_failed(format!("the leaf did not load Tui: {error}")));
            let failed = result.is_err();
            if let Some(loaded) = self.loaded.lock().expect("tui row").take() {
                let _ = loaded.send(result);
            }
            if failed {
                return Ok(Effect::Done);
            }
            Ok(Effect::AsyncDisposer(Box::new(move || {
                Box::pin(async move {
                    let unload = async {
                        let unloaded = session
                            .invoke_async("", "plugins.unload", json!([key]).into())
                            .await?;
                        settle(unloaded).await
                    };
                    match tokio::time::timeout(UNLOAD, unload).await {
                        Ok(Ok(_) | Err(SessionError::Transport(_))) => {}
                        Ok(Err(error)) => tracing::warn!(%error, "the leaf did not unload Tui"),
                        Err(_) => tracing::warn!("the leaf did not unload Tui in time"),
                    }
                    Ok(())
                })
            })))
        })
    }
}

/// The process capability owning UI leaves: the leaf transport and identity,
/// and `UiHost`, the registry. Its disposal closes every leaf.
pub(crate) struct UiHostPlugin {
    program: Option<PathBuf>,
    endpoint: Option<OsString>,
    model_config: Option<PathBuf>,
}

impl UiHostPlugin {
    pub fn new(
        program: Option<PathBuf>,
        endpoint: Option<OsString>,
        model_config: Option<PathBuf>,
    ) -> Self {
        Self {
            program,
            endpoint,
            model_config,
        }
    }
}

impl TypedPlugin for UiHostPlugin {
    type Deps = ();

    fn name(&self) -> &str {
        "UiHost"
    }

    fn apply<'a>(&'a self, ctx: &'a Ctx, (): ()) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            // Leaves need the daemon's executable and its own endpoint;
            // without them `ui.leaf` is not advertised and `ui.open` fails.
            let leaves = match (&self.program, &self.endpoint) {
                (Some(program), Some(endpoint)) => {
                    let transport = Arc::new(LocalTransport::default());
                    ctx.provide_as::<dyn rutis_bridge::Transport>(
                        transport_key(LINKS),
                        transport.clone(),
                    )?;
                    let identity: Arc<dyn Identity> = Arc::new(StaticIdentity::new(
                        PeerId::new(DAEMON_PEER).expect("valid daemon peer id"),
                    ));
                    ctx.provide_as::<dyn Identity>(rutis_bridge::identity_key(LINKS), identity)?;
                    Some(UiLeaves {
                        ctx: ctx.clone(),
                        transport,
                        program: program.clone(),
                        endpoint: endpoint.clone(),
                        model_config: self.model_config.clone(),
                    })
                }
                _ => None,
            };
            let host = crate::ui_host::UiHost::new(leaves);
            ctx.provide_as::<crate::ui_host::UiHost>(
                TypeKey::of::<crate::ui_host::UiHost>(),
                host.clone(),
            )?;
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        host.close_all().await;
                        Ok(())
                    })
                }))
            })?;
            Ok(Effect::Done)
        })
    }
}

impl dim_agent_runtime::Capability for UiHostPlugin {
    fn keys(&self) -> dim_agent_runtime::KeysOf<Self> {}
}

pub(crate) fn leaf_failed(message: impl std::fmt::Display) -> RpcError {
    RpcError::app(-32000, "ui_leaf_failed", message.to_string())
}
