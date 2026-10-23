//! Daemon-owned UI leaves. `UiHost` starts one launcher process per
//! `ui.open`; the process runs a rutis node that hosts the `UiLeaf` plugin.
//! The daemon loads that plugin over a local rutis link and owns the link,
//! the process and the attach token. The leaf's TUI remains a protocol v3
//! business client: the leaf control plane only reports readiness,
//! heartbeat and an exit reason.
//!
//! A launcher receives its channel address from the local transport and
//! keeps the terminal itself. The daemon never writes to a terminal.
//! See `docs/design/plugins/Tui.md` §3.6.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use dim_agent_capabilities::methods::RpcError;
use rutis::{BoxFuture, CordisError, Ctx, Effect, FiberView, Plugin, TypeKey, TypedPlugin};
use rutis_bridge::{
    Handler, Identity, LinkConfig, LinkPlugin, LinkState, Peer, StaticIdentity,
    channel::PeerId,
    peer_key,
    session::{Connection, Error as SessionError, Value as SessionValue, settle},
    transport::local::{Handover, LocalTransport, Spawn, Stdio},
    transport_key,
};
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use uuid::Uuid;

/// How to start one UI leaf. `args` are the launcher's own arguments; the
/// supervisor appends the leaf channel and leaf/daemon endpoint ids.
#[derive(Clone, Debug)]
pub struct UiLauncherConfig {
    pub program: PathBuf,
    pub args: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UiLeafState {
    Starting,
    Online,
    Replacing,
    Offline,
    Failed,
}
impl UiLeafState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Online => "online",
            Self::Replacing => "replacing",
            Self::Offline => "offline",
            Self::Failed => "failed",
        }
    }
}

pub(crate) struct UiLeafSnapshot {
    pub state: UiLeafState,
    pub pid: u32,
    pub generation: u64,
}

#[derive(Debug)]
pub(crate) struct Started {
    pub pid: u32,
}

struct Leaf {
    session_id: String,
    kind: String,
    token: [u8; 32],
    generation: u64,
    replaces: Option<String>,
    spawner: String,
    state: UiLeafState,
    pid: u32,
    ready: bool,
    attached: bool,
    last_heartbeat: Instant,
    link: Option<FiberView>,
    capture: Option<FiberView>,
    peer: Option<Arc<Peer>>,
    offered: Option<rutis_bridge::Offered>,
    load_key: String,
}

/// Process-wide owner of UI leaves. A leaf is inserted before its link
/// starts, so its business `ui.attach` can race the `ui.open` reply; it
/// becomes `Online` only after the leaf reported ready and its RPC attached.
pub(crate) struct UiLeafSupervisor {
    launcher: Option<UiLauncherConfig>,
    home: Option<PathBuf>,
    model_config: Option<PathBuf>,
    ctx: Ctx,
    transport: Arc<LocalTransport>,
    leaves: Mutex<HashMap<String, Leaf>>,
}

impl UiLeafSupervisor {
    pub fn new(
        launcher: Option<UiLauncherConfig>,
        home: Option<PathBuf>,
        model_config: Option<PathBuf>,
        ctx: Ctx,
        transport: Arc<LocalTransport>,
    ) -> Self {
        Self {
            launcher,
            home,
            model_config,
            ctx,
            transport,
            leaves: Mutex::new(HashMap::new()),
        }
    }

    /// The id a registration is inserted under before its link starts.
    /// Generations are assigned by `UiHost`, which also owns legacy ids.
    pub fn new_ui_id(&self) -> String {
        format!("ui-{}", Uuid::new_v4())
    }

    /// Starts the launcher and its local rutis link, then loads the `UiLeaf`
    /// plugin. A failure removes the leaf; an old registration is untouched.
    pub async fn start(
        self: &Arc<Self>,
        ui_id: &str,
        session_id: &str,
        kind: &str,
        tty: Option<&str>,
        generation: u64,
        replaces: Option<String>,
    ) -> Result<Started, RpcError> {
        let launcher = self.launcher.clone().ok_or_else(ui_launcher_unavailable)?;
        let token = new_token();
        let leaf_peer =
            PeerId::new(ui_id.to_owned()).map_err(|error| ui_leaf_error(error.to_string()))?;
        let spawner = format!("spawn-{ui_id}");
        let mut spawn = Spawn::new(launcher.program.clone(), leaf_peer.clone());
        spawn.args = launcher.args.into_iter().map(Into::into).collect();
        spawn.args.extend([
            "--daemon-peer".into(),
            "dim-daemon".into(),
            "--leaf-peer".into(),
            leaf_peer.to_string().into(),
        ]);
        spawn.env = vec![
            ("DIMCODE_UI_ID".into(), ui_id.into()),
            ("DIMCODE_UI_TOKEN".into(), token.clone().into()),
            ("DIMCODE_UI_SESSION".into(), session_id.into()),
            ("DIMCODE_UI_KIND".into(), kind.into()),
            (
                "DIMCODE_UI_GENERATION".into(),
                generation.to_string().into(),
            ),
            (
                "DIMCODE_UI_LEAF_CAP".into(),
                token_capability(&token).into(),
            ),
        ];
        if let Some(tty) = tty {
            spawn.env.push(("DIMCODE_UI_TTY".into(), tty.into()));
        }
        if let Some(home) = &self.home {
            spawn
                .env
                .push(("DIMCODE_HOME".into(), home.as_os_str().into()));
        }
        if let Some(model_config) = &self.model_config {
            spawn.env.push((
                "DIMCODE_UI_MODEL_CONFIG".into(),
                model_config.as_os_str().into(),
            ));
        }
        if let Ok(cwd) = std::env::current_dir() {
            spawn
                .env
                .push(("DIMCODE_UI_CWD".into(), cwd.as_os_str().into()));
            spawn.cwd = Some(cwd);
        }
        spawn.stdin = Stdio::Null;
        spawn.stdout = Stdio::Null;
        spawn.stderr = Stdio::Lines;
        #[cfg(unix)]
        {
            spawn.handover = Handover::DialBack;
        }

        self.leaves.lock().expect("ui leaves").insert(
            ui_id.to_owned(),
            Leaf {
                session_id: session_id.to_owned(),
                kind: kind.to_owned(),
                token: digest(&token),
                generation,
                replaces,
                spawner: spawner.clone(),
                state: UiLeafState::Starting,
                pid: 0,
                ready: false,
                attached: false,
                last_heartbeat: Instant::now(),
                link: None,
                capture: None,
                peer: None,
                offered: None,
                load_key: format!("leaf-{ui_id}"),
            },
        );
        self.transport.spawner(&spawner, spawn);

        let address = format!("spawn:{spawner}");
        let mut config = LinkConfig::dial(leaf_peer.clone(), "local", "ui-leaf", &address)
            .require(&token_capability(&token));
        config.reconnect = false;
        let link = LinkPlugin::new(config);
        let mut link_state = link.state();
        let mut view = Some(self.ctx.plugin(link));
        if let Err(error) = wait_ready(&mut link_state).await {
            dispose_view(&mut view).await;
            let _ = self.remove(ui_id).await;
            return Err(error);
        }
        // A child plugin declares the Peer dependency, so the read is
        // gated and declared exactly as the link's provider expects. The
        // capture is a child of this plugin and goes with the leaf.
        let (peer_tx, peer_rx) = tokio::sync::oneshot::channel();
        let capture = self.ctx.plugin(PeerCapture {
            injects: [peer_key(&leaf_peer)],
            tx: Mutex::new(Some(peer_tx)),
        });
        let peer = match tokio::time::timeout(Duration::from_secs(5), peer_rx).await {
            Ok(Ok(peer)) => peer,
            _ => {
                let _ = capture.dispose().await;
                dispose_view(&mut view).await;
                let _ = self.remove(ui_id).await;
                return Err(ui_leaf_error("the leaf link provided no peer"));
            }
        };
        let handler: Arc<dyn Handler> = Arc::new(ControlHandler {
            supervisor: self.clone(),
            ui_id: ui_id.to_owned(),
        });
        let offered = match peer.register("uiLeafControl", handler) {
            Ok(offered) => offered,
            Err(error) => {
                let _ = capture.dispose().await;
                dispose_view(&mut view).await;
                let _ = self.remove(ui_id).await;
                return Err(ui_leaf_error(format!(
                    "cannot register the leaf control handler: {error}"
                )));
            }
        };
        let missing = {
            let mut leaves = self.leaves.lock().expect("ui leaves");
            match leaves.get_mut(ui_id) {
                Some(leaf) => {
                    leaf.link = view.take();
                    leaf.capture = Some(capture);
                    leaf.peer = Some(peer.clone());
                    leaf.offered = Some(offered);
                    None
                }
                None => Some(capture),
            }
        };
        let load_key = match missing {
            Some(capture) => {
                dispose_view(&mut view).await;
                let _ = capture.dispose().await;
                return Err(ui_unknown());
            }
            None => self
                .leaves
                .lock()
                .expect("ui leaves")
                .get(ui_id)
                .map(|leaf| leaf.load_key.clone())
                .ok_or_else(ui_unknown)?,
        };
        let monitor = Arc::downgrade(self);
        let monitor_id = ui_id.to_owned();
        let mut watched = link_state;
        tokio::spawn(async move {
            while watched.changed().await.is_ok() {
                if matches!(
                    &*watched.borrow_and_update(),
                    LinkState::Stopped { .. } | LinkState::Idle
                ) {
                    if let Some(supervisor) = monitor.upgrade() {
                        supervisor.link_ended(&monitor_id);
                    }
                    return;
                }
            }
        });

        let load = async {
            let loaded = peer
                .connection()
                .invoke_async(
                    "",
                    "plugins.load",
                    json!([load_key, "UiLeaf", {}, [], []]).into(),
                )
                .await?;
            settle(loaded).await
        };
        let loaded = match tokio::time::timeout(Duration::from_secs(10), load).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(error.to_string()),
            Err(_) => Err("the leaf plugin load timed out".to_owned()),
        };
        if let Err(error) = loaded {
            let _ = self.remove(ui_id).await;
            return Err(ui_leaf_error(format!(
                "the leaf plugin did not start: {error}"
            )));
        }
        let pid = self
            .leaves
            .lock()
            .expect("ui leaves")
            .get(ui_id)
            .map_or(0, |leaf| leaf.pid);
        Ok(Started { pid })
    }

    /// Validates the launcher's attach token and marks its RPC attached.
    pub fn attach(
        &self,
        ui_id: &str,
        token: &str,
        generation: Option<u64>,
    ) -> Result<(String, String, u64), RpcError> {
        let mut leaves = self.leaves.lock().expect("ui leaves");
        let leaf = leaves.get_mut(ui_id).ok_or_else(ui_unknown)?;
        if matches!(leaf.state, UiLeafState::Failed) {
            return Err(ui_leaf_error("the UI leaf failed to start"));
        }
        if digest(token) != leaf.token {
            return Err(ui_leaf_error("The UI launcher token is invalid"));
        }
        if generation.is_some_and(|generation| generation != leaf.generation) {
            return Err(RpcError::app(
                -32000,
                "ui_generation",
                "The UI launcher generation is not current",
            ));
        }
        leaf.attached = true;
        leaf.last_heartbeat = Instant::now();
        leaf.state = if leaf.ready {
            if leaf.replaces.is_some() {
                UiLeafState::Replacing
            } else {
                UiLeafState::Online
            }
        } else {
            UiLeafState::Starting
        };
        Ok((leaf.session_id.clone(), leaf.kind.clone(), leaf.generation))
    }

    pub fn ready(&self, ui_id: &str, pid: u32) -> Result<(), RpcError> {
        let mut leaves = self.leaves.lock().expect("ui leaves");
        let leaf = leaves.get_mut(ui_id).ok_or_else(ui_unknown)?;
        leaf.ready = true;
        leaf.pid = pid;
        leaf.last_heartbeat = Instant::now();
        leaf.state = if leaf.attached {
            if leaf.replaces.is_some() {
                UiLeafState::Replacing
            } else {
                UiLeafState::Online
            }
        } else {
            UiLeafState::Starting
        };
        Ok(())
    }

    pub fn heartbeat(&self, ui_id: &str) -> Result<(), RpcError> {
        let mut leaves = self.leaves.lock().expect("ui leaves");
        let leaf = leaves.get_mut(ui_id).ok_or_else(ui_unknown)?;
        if matches!(leaf.state, UiLeafState::Online | UiLeafState::Replacing) {
            leaf.last_heartbeat = Instant::now();
        }
        Ok(())
    }

    pub fn detach(&self, ui_id: &str) {
        let mut leaves = self.leaves.lock().expect("ui leaves");
        if let Some(leaf) = leaves.get_mut(ui_id) {
            leaf.attached = false;
            if !matches!(leaf.state, UiLeafState::Failed) {
                leaf.state = UiLeafState::Offline;
            }
        }
    }

    pub fn offline(&self, ui_id: &str, failed: bool) {
        let mut leaves = self.leaves.lock().expect("ui leaves");
        if let Some(leaf) = leaves.get_mut(ui_id) {
            leaf.ready = false;
            leaf.state = if failed {
                UiLeafState::Failed
            } else {
                UiLeafState::Offline
            };
        }
    }

    pub fn replaced(&self, ui_id: &str) {
        let mut leaves = self.leaves.lock().expect("ui leaves");
        if let Some(leaf) = leaves.get_mut(ui_id) {
            leaf.replaces = None;
            if leaf.ready && leaf.attached {
                leaf.state = UiLeafState::Online;
            }
        }
    }

    fn link_ended(&self, ui_id: &str) {
        let mut leaves = self.leaves.lock().expect("ui leaves");
        if let Some(leaf) = leaves.get_mut(ui_id)
            && !matches!(leaf.state, UiLeafState::Offline | UiLeafState::Failed)
        {
            leaf.ready = false;
            leaf.state = UiLeafState::Offline;
        }
    }

    pub fn snapshot(&self, ui_id: &str) -> Option<UiLeafSnapshot> {
        let leaves = self.leaves.lock().expect("ui leaves");
        let leaf = leaves.get(ui_id)?;
        let stale = matches!(leaf.state, UiLeafState::Online | UiLeafState::Replacing)
            && leaf.last_heartbeat.elapsed() > Duration::from_secs(5);
        Some(UiLeafSnapshot {
            state: if stale {
                UiLeafState::Offline
            } else {
                leaf.state
            },
            pid: leaf.pid,
            generation: leaf.generation,
        })
    }

    /// Stops one leaf: unload its plugin, then dispose its link. The local
    /// transport ends the process after the channel closes, or kills it
    /// after its grace period.
    pub async fn stop(&self, ui_id: &str) -> bool {
        let leaf = self.leaves.lock().expect("ui leaves").remove(ui_id);
        let Some(leaf) = leaf else {
            return false;
        };
        if let Some(peer) = leaf.peer {
            let unload = async {
                let loaded = peer
                    .connection()
                    .invoke_async("", "plugins.unload", json!([leaf.load_key]).into())
                    .await?;
                settle(loaded).await
            };
            if tokio::time::timeout(Duration::from_secs(2), unload)
                .await
                .is_err()
            {
                // A stopped or wedged leaf cannot answer unload. Kill the
                // process directly; the link disposal below is best effort.
                force_kill(leaf.pid);
            }
        }
        // Drop the spawner's environment (it held the token) even though the
        // local transport has no remove operation.
        self.transport.spawner(
            &leaf.spawner,
            Spawn::new(
                "",
                PeerId::new("ui-retired").expect("valid retired leaf peer id"),
            ),
        );
        drop(leaf.offered);
        if let Some(view) = leaf.capture
            && tokio::time::timeout(Duration::from_secs(2), view.dispose())
                .await
                .is_err()
        {
            force_kill(leaf.pid);
        }
        if let Some(view) = leaf.link
            && tokio::time::timeout(Duration::from_secs(2), view.dispose())
                .await
                .is_err()
        {
            force_kill(leaf.pid);
        }
        true
    }

    async fn remove(&self, ui_id: &str) -> bool {
        self.stop(ui_id).await
    }

    /// Stops every leaf. Each `stop` closes that leaf's link, which ends its
    /// own process; the local transport's process-wide kill switch is not
    /// used because other runtimes in the same daemon may own children too.
    /// Used when the daemon shuts down: no launcher may outlive it.
    pub async fn close_all(&self) {
        let ids: Vec<String> = self
            .leaves
            .lock()
            .expect("ui leaves")
            .keys()
            .cloned()
            .collect();
        for id in ids {
            self.stop(&id).await;
        }
    }
}

/// Captures the link's `Peer` through a declared dependency, so the
/// supervisor does not read a child service from its own parent context.
struct PeerCapture {
    injects: [TypeKey; 1],
    tx: Mutex<Option<tokio::sync::oneshot::Sender<Arc<Peer>>>>,
}

impl Plugin for PeerCapture {
    fn name(&self) -> &str {
        "UiLeafPeerCapture"
    }

    fn injects(&self) -> &[TypeKey] {
        &self.injects
    }

    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let peer = ctx.require_as::<Peer>(self.injects[0].clone())?;
            if let Some(tx) = self.tx.lock().expect("peer capture").take() {
                let _ = tx.send(peer);
            }
            Ok(Effect::Done)
        })
    }
}

fn force_kill(pid: u32) {
    if pid == 0 {
        return;
    }
    #[cfg(unix)]
    {
        // SAFETY: kill(2) on a pid this supervisor started and has not
        // reaped through the transport ownership path.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_TERMINATE, TerminateProcess,
        };
        // SAFETY: handles are opened for this pid, used, and closed.
        unsafe {
            let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if !handle.is_null() {
                let _ = TerminateProcess(handle, 1);
                let _ = CloseHandle(handle);
            }
        }
    }
}

async fn dispose_view(view: &mut Option<FiberView>) {
    if let Some(view) = view.take() {
        let _ = view.dispose().await;
    }
}

async fn wait_ready(state: &mut tokio::sync::watch::Receiver<LinkState>) -> Result<(), RpcError> {
    let wait = async {
        loop {
            match &*state.borrow_and_update() {
                LinkState::Ready { .. } => return Ok(()),
                LinkState::Stopped { error, .. } => {
                    return Err(ui_leaf_error(format!("the leaf link stopped: {error}")));
                }
                LinkState::Waiting { error, .. } => {
                    return Err(ui_leaf_error(format!("the leaf link failed: {error}")));
                }
                LinkState::Idle | LinkState::Connecting => {}
            }
            if state.changed().await.is_err() {
                return Err(ui_leaf_error("the leaf link ended"));
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(10), wait)
        .await
        .map_err(|_| ui_leaf_error("the UI leaf did not connect"))?
}

struct ControlHandler {
    supervisor: Arc<UiLeafSupervisor>,
    ui_id: String,
}

impl Handler for ControlHandler {
    fn invoke(
        &self,
        _peer: &Connection,
        _target: &str,
        method: &str,
        args: SessionValue,
    ) -> Result<SessionValue, SessionError> {
        let method = method.rsplit('.').next().unwrap_or(method);
        let payload = args
            .list()?
            .into_iter()
            .next()
            .ok_or_else(|| SessionError::Value("leaf control needs its fields".into()))?
            .json()?;
        let requested = payload
            .get("uiId")
            .and_then(Json::as_str)
            .unwrap_or_default();
        if requested != self.ui_id {
            return Err(SessionError::Value(
                "the leaf identified another uiId".into(),
            ));
        }
        match method {
            "ready" => {
                let pid = payload
                    .get("pid")
                    .and_then(Json::as_u64)
                    .and_then(|pid| u32::try_from(pid).ok())
                    .ok_or_else(|| SessionError::Value("ready needs a process id".into()))?;
                self.supervisor
                    .ready(&self.ui_id, pid)
                    .map_err(|error| SessionError::Value(error.message))?;
            }
            "heartbeat" => {
                self.supervisor
                    .heartbeat(&self.ui_id)
                    .map_err(|error| SessionError::Value(error.message))?;
            }
            "offline" => {
                let failed = payload
                    .get("failed")
                    .and_then(Json::as_bool)
                    .unwrap_or(false);
                self.supervisor.offline(&self.ui_id, failed);
            }
            other => {
                return Err(SessionError::Value(format!(
                    "unknown leaf control method {other}"
                )));
            }
        }
        Ok(SessionValue::Undefined)
    }
}

/// The process plugin providing the registry and the local link plumbing.
pub(crate) struct UiHostPlugin {
    launcher: Option<UiLauncherConfig>,
    home: Option<PathBuf>,
    model_config: Option<PathBuf>,
}

impl UiHostPlugin {
    pub fn new(
        launcher: Option<UiLauncherConfig>,
        home: Option<PathBuf>,
        model_config: Option<PathBuf>,
    ) -> Self {
        Self {
            launcher,
            home,
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
            let transport = Arc::new(LocalTransport::default());
            ctx.provide_as::<dyn rutis_bridge::Transport>(
                transport_key("local"),
                transport.clone(),
            )?;
            let daemon_peer = PeerId::new("dim-daemon").expect("valid daemon peer id");
            let identity: Arc<dyn Identity> = Arc::new(StaticIdentity::new(daemon_peer));
            ctx.provide_as::<dyn Identity>(rutis_bridge::identity_key("ui-leaf"), identity)?;
            let supervisor = Arc::new(UiLeafSupervisor::new(
                self.launcher.clone(),
                self.home.clone(),
                self.model_config.clone(),
                ctx.clone(),
                transport,
            ));
            let host = Arc::new(crate::ui_host::UiHost::new(supervisor.clone()));
            ctx.provide_as::<crate::ui_host::UiHost>(
                rutis::TypeKey::of::<crate::ui_host::UiHost>(),
                host.clone(),
            )?;
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        supervisor.close_all().await;
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

fn token_capability(token: &str) -> String {
    format!("ui-leaf-token:{}", digest_secret(token))
}

fn new_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("operating system randomness");
    URL_SAFE_NO_PAD.encode(bytes)
}

fn digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

fn digest_secret(token: &str) -> String {
    URL_SAFE_NO_PAD.encode(digest(token))
}

fn ui_launcher_unavailable() -> RpcError {
    RpcError::app(
        -32000,
        "ui_launcher_unavailable",
        "This App Server has no UI launcher",
    )
}

fn ui_unknown() -> RpcError {
    RpcError::app(-32000, "ui_unknown", "No such UI leaf is registered")
}

fn ui_leaf_error(message: impl Into<String>) -> RpcError {
    RpcError::app(-32000, "ui_leaf_failed", message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_not_stored_in_clear() {
        let token = new_token();
        assert_ne!(token, digest_secret(&token));
        assert_eq!(token_capability(&token).len(), "ui-leaf-token:".len() + 43);
    }
}
