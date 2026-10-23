//! The daemon-started UI launcher: a rutis node that hosts the `UiLeaf`
//! plugin, owns the terminal and starts the ordinary TUI as its child. The
//! daemon loads the plugin over a local rutis link; the plugin reports
//! `ready`/`heartbeat`/`offline`, and the TUI keeps the protocol v3 business
//! connection.
//!
//! The launcher owns the terminal. The daemon passes its path in
//! `DIMCODE_UI_TTY`; the launcher adopts it and the TUI child inherits it.

use rutis::{BoxFuture, CordisError, Ctx, Effect, Plugin, PluginFactory, TypeKey};
use rutis_bridge::{
    Described, Dial, Features, IdentityPlugin, LinkConfig, LinkState, Peer, PeerPlugin,
    StaticCatalog, StaticIdentity, Transport,
    channel::PeerId,
    channel::{Channel, Closer, ConnectError},
    peer_key,
    session::Value as SessionValue,
    transport::local::framed,
    transport_key,
};
use serde_json::{Value as Json, json};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Notify, oneshot};

const PLUGIN: &str = "UiLeaf";
const DAEMON_PEER: &str = "dim-daemon";

/// Entry point for `dimcode ui-leaf ... <channel>`. The local transport
/// appends the channel address after the arguments configured here.
pub fn run(args: impl IntoIterator<Item = String>) -> i32 {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("Error: {error}");
            return 1;
        }
    };
    runtime.block_on(async {
        match run_inner(args).await {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("Error: {error}");
                1
            }
        }
    })
}

async fn run_inner(args: impl IntoIterator<Item = String>) -> Result<(), String> {
    let (daemon_peer, leaf_peer, channel) = parse_args(args)?;
    let token_capability =
        std::env::var("DIMCODE_UI_LEAF_CAP").map_err(|_| "DIMCODE_UI_LEAF_CAP is missing")?;
    let token = std::env::var("DIMCODE_UI_TOKEN").map_err(|_| "DIMCODE_UI_TOKEN is missing")?;
    // The launcher's single copy is consumed here; the child receives its own
    // copy below and removes it before spawning tools.
    unsafe { std::env::remove_var("DIMCODE_UI_TOKEN") };
    let ui_id = std::env::var("DIMCODE_UI_ID").map_err(|_| "DIMCODE_UI_ID is missing")?;
    let session_id =
        std::env::var("DIMCODE_UI_SESSION").map_err(|_| "DIMCODE_UI_SESSION is missing")?;
    if std::env::var("DIMCODE_UI_KIND").as_deref() == Ok("tui")
        && std::env::var_os("DIMCODE_UI_TTY").is_none()
        && !std::io::IsTerminal::is_terminal(&std::io::stdin())
    {
        return Err("the tui leaf has no terminal".into());
    }
    if let Some(tty) = std::env::var_os("DIMCODE_UI_TTY") {
        dim_cli::adopt_terminal(std::path::Path::new(&tty))
            .map_err(|error| format!("cannot open the terminal: {error}"))?;
        capture_terminal();
        install_terminal_signal_guard();
    }
    let home = std::env::var_os("DIMCODE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(|home| PathBuf::from(home).join(".dimcode"))
        })
        .ok_or("neither DIMCODE_HOME nor HOME is set")?;
    let cwd = std::env::var_os("DIMCODE_UI_CWD")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .ok_or("the current directory is unavailable")?;
    let endpoint = std::env::var_os("DIMCODE_DAEMON_ENDPOINT").map(PathBuf::from);
    let exit = Arc::new(Notify::new());
    let factory = UiLeafFactory {
        injects: [peer_key(&daemon_peer)],
        ui_id: ui_id.clone(),
        session_id,
        home,
        cwd,
        endpoint,
        token: std::sync::Mutex::new(Some(token)),
        exit: exit.clone(),
    };
    let catalog = StaticCatalog::new().with(
        PLUGIN,
        Described {
            schema: Some(json!({
                "type": "object",
                "properties": {
                    "uiId": {"type": "string"},
                    "sessionId": {"type": "string"},
                    "kind": {"type": "string"},
                    "generation": {"type": "integer"}
                }
            })),
            version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            integrity: None,
        },
        factory,
    );
    let features = Features {
        host: Some(Arc::new(catalog)),
        ..Features::default()
    };
    let link = LinkConfig::dial(daemon_peer.clone(), "local", "ui-leaf", &channel)
        .declare(&token_capability);
    let mut link = link;
    link.reconnect = false;
    let peer = PeerPlugin::new(link, features).map_err(|error| error.to_string())?;
    let handle = peer.handle();
    let root = Ctx::root().map_err(|error| error.to_string())?;
    root.plugin(LeafTransportPlugin);
    root.plugin(IdentityPlugin::new(
        "ui-leaf",
        StaticIdentity::new(leaf_peer),
    ));
    let peer_view = root.plugin(peer);
    (&peer_view)
        .await
        .map_err(|error| format!("the leaf node did not start: {error}"))?;
    let mut state = handle.link_state().ok_or("the leaf link did not start")?;
    let waiting = async {
        loop {
            if matches!(&*state.borrow_and_update(), LinkState::Stopped { .. }) {
                return;
            }
            if state.changed().await.is_err() {
                return;
            }
        }
    };
    tokio::select! {
        _ = exit.notified() => {}
        _ = waiting => {}
    }
    // The link may end before the plugin's bounded cleanup completes; the
    // launcher still owns the terminal and must hand it back on every path.
    restore_terminal();
    let _ = root.shutdown().await;
    Ok(())
}

/// The leaf's side of the local transport. The daemon starts this process
/// with a Unix socket path (Unix) or a loopback address plus the one-time
/// `RUTIS_CHANNEL_TOKEN` (Windows); this transport accepts both.
struct LeafTransportPlugin;

impl Plugin for LeafTransportPlugin {
    fn name(&self) -> &str {
        "ui-leaf/transport"
    }

    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            ctx.provide_as::<dyn Transport>(transport_key("local"), Arc::new(LeafTransport))?;
            Ok(Effect::Done)
        })
    }
}

struct LeafTransport;

impl Transport for LeafTransport {
    fn kind(&self) -> &str {
        "local"
    }

    fn dial<'a>(&'a self, dial: &'a Dial) -> BoxFuture<'a, Result<Channel, ConnectError>> {
        Box::pin(async move {
            let address = dial.address.as_str();
            if let Some(address) = address.strip_prefix("tcp:") {
                let stream = tokio::net::TcpStream::connect(address)
                    .await
                    .map_err(retryable)?;
                stream.set_nodelay(true).map_err(retryable)?;
                let stream = stream.into_std().map_err(retryable)?;
                stream.set_nonblocking(false).map_err(retryable)?;
                if let Ok(token) = std::env::var("RUTIS_CHANNEL_TOKEN") {
                    let mut writer = stream.try_clone().map_err(retryable)?;
                    use std::io::Write;
                    writeln!(writer, "{token}").map_err(retryable)?;
                    writer.flush().map_err(retryable)?;
                }
                let reader = stream.try_clone().map_err(retryable)?;
                let closer = Arc::new(TcpCloser(stream.try_clone().map_err(retryable)?));
                return Ok(framed(reader, stream, closer));
            }
            #[cfg(unix)]
            {
                let path = address.strip_prefix("unix:").unwrap_or(address);
                let stream = tokio::net::UnixStream::connect(path)
                    .await
                    .map_err(retryable)?;
                let stream = stream.into_std().map_err(retryable)?;
                stream.set_nonblocking(false).map_err(retryable)?;
                let reader = stream.try_clone().map_err(retryable)?;
                let closer = Arc::new(UnixCloser(stream.try_clone().map_err(retryable)?));
                Ok(framed(reader, stream, closer))
            }
            #[cfg(not(unix))]
            {
                let _ = address;
                Err(ConnectError::Incompatible {
                    reason: format!("the leaf transport cannot dial {address}"),
                })
            }
        })
    }
}

fn retryable(error: std::io::Error) -> ConnectError {
    ConnectError::Retryable {
        reason: error.to_string(),
    }
}

struct TcpCloser(std::net::TcpStream);

impl Closer for TcpCloser {
    fn close(&self, _reason: &str) {
        let _ = self.0.shutdown(std::net::Shutdown::Both);
    }
}

#[cfg(unix)]
struct UnixCloser(std::os::unix::net::UnixStream);

#[cfg(unix)]
impl Closer for UnixCloser {
    fn close(&self, _reason: &str) {
        let _ = self.0.shutdown(std::net::Shutdown::Both);
    }
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<(PeerId, PeerId, String), String> {
    let mut daemon = None;
    let mut leaf = None;
    let mut channel = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--daemon-peer" => daemon = Some(args.next().ok_or("--daemon-peer needs a value")?),
            "--leaf-peer" => leaf = Some(args.next().ok_or("--leaf-peer needs a value")?),
            value if value.starts_with('-') => {
                return Err(format!("unknown ui-leaf argument {value}"));
            }
            value => {
                if channel.replace(value.to_owned()).is_some() {
                    return Err("ui-leaf takes one channel address".into());
                }
            }
        }
    }
    let daemon = daemon.unwrap_or_else(|| DAEMON_PEER.to_owned());
    let leaf = leaf.ok_or("--leaf-peer is missing")?;
    let channel = channel.ok_or("the leaf channel address is missing")?;
    Ok((
        PeerId::new(daemon).map_err(|error| error.to_string())?,
        PeerId::new(leaf).map_err(|error| error.to_string())?,
        channel,
    ))
}

struct UiLeafFactory {
    injects: [TypeKey; 1],
    ui_id: String,
    session_id: String,
    home: PathBuf,
    cwd: PathBuf,
    endpoint: Option<PathBuf>,
    token: std::sync::Mutex<Option<String>>,
    exit: Arc<Notify>,
}

impl PluginFactory<Json> for UiLeafFactory {
    fn name(&self) -> &str {
        PLUGIN
    }

    fn injects(&self) -> &[TypeKey] {
        &self.injects
    }

    fn build(&self, _: &Json) -> Result<Box<dyn Plugin>, CordisError> {
        Ok(Box::new(UiLeaf {
            injects: self.injects.clone(),
            ui_id: self.ui_id.clone(),
            session_id: self.session_id.clone(),
            home: self.home.clone(),
            cwd: self.cwd.clone(),
            endpoint: self.endpoint.clone(),
            exit: self.exit.clone(),
            token: std::sync::Mutex::new(self.token.lock().expect("leaf token").take()),
        }))
    }
}

struct UiLeaf {
    injects: [TypeKey; 1],
    ui_id: String,
    session_id: String,
    home: PathBuf,
    cwd: PathBuf,
    endpoint: Option<PathBuf>,
    exit: Arc<Notify>,
    token: std::sync::Mutex<Option<String>>,
}

impl Plugin for UiLeaf {
    fn name(&self) -> &str {
        PLUGIN
    }

    fn injects(&self) -> &[TypeKey] {
        &self.injects
    }

    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let peer = ctx.require_as::<Peer>(self.injects[0].clone())?;
            let finished = Arc::new(AtomicBool::new(false));
            let child_pid = Arc::new(AtomicU32::new(0));
            let (tui_exit, exited) = oneshot::channel();
            let executable =
                std::env::current_exe().map_err(|error| CordisError::PluginFailed(error.into()))?;
            let mut command = Command::new(executable);
            command
                .arg("ui-tui")
                .arg("--session")
                .arg(&self.session_id)
                .arg("--home")
                .arg(&self.home)
                .arg("--cwd")
                .arg(&self.cwd)
                .env("DIMCODE_UI_ID", &self.ui_id)
                .arg("--endpoint");
            if let Some(endpoint) = &self.endpoint {
                command.arg(endpoint);
            } else {
                return Err(CordisError::PluginFailed(
                    "the launcher has no daemon endpoint".into(),
                ));
            }
            let token = self
                .token
                .lock()
                .expect("leaf token")
                .take()
                .ok_or_else(|| CordisError::PluginFailed("the launcher has no token".into()))?;
            command
                .env("DIMCODE_UI_TOKEN", token)
                .env(
                    "DIMCODE_UI_GENERATION",
                    std::env::var("DIMCODE_UI_GENERATION").unwrap_or_default(),
                )
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());
            let mut child = command
                .spawn()
                .map_err(|error| CordisError::PluginFailed(error.into()))?;
            child_pid.store(child.id(), Ordering::SeqCst);
            CHILD_PID.store(child.id(), Ordering::SeqCst);
            let finished_by_tui = finished.clone();
            std::thread::Builder::new()
                .name("dim-ui-leaf-tui".into())
                .spawn(move || {
                    let code = child
                        .wait()
                        .map(|status| status.code().unwrap_or(1))
                        .unwrap_or(1);
                    finished_by_tui.store(true, Ordering::SeqCst);
                    let _ = tui_exit.send(code);
                })
                .map_err(|error| CordisError::PluginFailed(error.into()))?;
            control(
                &peer,
                "ready",
                json!({
                    "uiId": self.ui_id,
                    "pid": std::process::id(),
                }),
            )
            .await
            .map_err(|error| CordisError::PluginFailed(error.into()))?;
            let ui_id = self.ui_id.clone();
            let peer_for_monitor = peer.clone();
            let exit = self.exit.clone();
            let monitor_exit = exit.clone();
            let mut exited = exited;
            let monitor = tokio::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_secs(1));
                loop {
                    tokio::select! {
                        code = &mut exited => {
                            let code = code.unwrap_or(1);
                            let failed = code != 0;
                            let _ = control(
                                &peer_for_monitor,
                                "offline",
                                json!({"uiId": ui_id, "failed": failed, "code": code}),
                            ).await;
                            monitor_exit.notify_waiters();
                            return;
                        }
                        _ = tick.tick() => {
                            if control(
                                &peer_for_monitor,
                                "heartbeat",
                                json!({"uiId": ui_id}),
                            ).await.is_err() {
                                monitor_exit.notify_waiters();
                                return;
                            }
                        }
                    }
                }
            });
            ctx.effect(move || {
                Effect::Disposer(Box::new(move || {
                    monitor.abort();
                    if !finished.load(Ordering::SeqCst) {
                        let pid = child_pid.load(Ordering::SeqCst);
                        signal_child(pid);
                        // Give the TUI signal guard a moment to hand the
                        // terminal back, then restore from the launcher's
                        // captured settings and reap the child.
                        std::thread::sleep(Duration::from_millis(200));
                        kill_child(pid);
                    }
                    restore_terminal();
                    exit.notify_waiters();
                    Ok(())
                }))
            })?;
            Ok(Effect::Done)
        })
    }
}

#[cfg(unix)]
static ORIGINAL_TERMINAL: OnceLock<libc::termios> = OnceLock::new();
static CHILD_PID: AtomicU32 = AtomicU32::new(0);

#[cfg(unix)]
extern "C" fn on_terminal_signal(signal: libc::c_int) {
    let pid = CHILD_PID.load(Ordering::SeqCst);
    if pid != 0 {
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    }
    restore_terminal();
    unsafe { libc::_exit(128 + signal) };
}

#[cfg(unix)]
fn install_terminal_signal_guard() {
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT] {
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = on_terminal_signal as *const () as usize;
            action.sa_flags = libc::SA_RESETHAND;
            libc::sigemptyset(&mut action.sa_mask);
            libc::sigaction(signal, &action, std::ptr::null_mut());
        }
    }
}

#[cfg(not(unix))]
fn install_terminal_signal_guard() {}

#[cfg(unix)]
fn capture_terminal() {
    let mut original = unsafe { std::mem::zeroed::<libc::termios>() };
    if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut original) } == 0 {
        let _ = ORIGINAL_TERMINAL.set(original);
    }
}

#[cfg(unix)]
fn restore_terminal() {
    let Some(original) = ORIGINAL_TERMINAL.get() else {
        return;
    };
    unsafe {
        libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, original);
        let restore = b"\x1b[?2004l\x1b[?25h";
        libc::write(libc::STDOUT_FILENO, restore.as_ptr().cast(), restore.len());
    }
}

#[cfg(not(unix))]
fn capture_terminal() {}

#[cfg(not(unix))]
fn restore_terminal() {}

fn signal_child(pid: u32) {
    if pid == 0 {
        return;
    }
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
    #[cfg(windows)]
    {
        // SIGTERM has no Windows equivalent; the bounded kill below handles
        // the child. Closing the console belongs to the TUI restore path.
    }
}

fn kill_child(pid: u32) {
    if pid == 0 {
        return;
    }
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_TERMINATE, TerminateProcess,
        };
        unsafe {
            let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if !handle.is_null() {
                let _ = TerminateProcess(handle, 1);
                let _ = CloseHandle(handle);
            }
        }
    }
}

async fn control(peer: &Peer, method: &str, payload: Json) -> Result<(), String> {
    let value = peer
        .connection()
        .invoke_async(
            "",
            &format!("uiLeafControl.{method}"),
            SessionValue::List(vec![SessionValue::Data(payload)]),
        )
        .await
        .map_err(|error| error.to_string())?;
    rutis_bridge::session::settle(value)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}
