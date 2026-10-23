mod args;
mod attachments;
mod browser;
mod client;
mod clipboard;
mod composer;
mod configure;
mod display;
mod export;
mod images;
mod input;
mod login;
mod mcp;
mod mcp_command;
mod model;
mod permission;
mod permission_command;
mod plugins;
mod projection;
mod question;
mod runtime;
mod sanitize;
mod session;
mod session_host;
mod ui;
mod update;
mod usage;
mod wire;

use args::{Action, Options};
use std::{
    io::{IsTerminal, Read},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

type Error = Box<dyn std::error::Error + Send + Sync>;
type Result<T> = std::result::Result<T, Error>;

/// Runs the CLI with the process arguments and returns its exit code. The
/// host binary must answer `app-server` with the App Server command line,
/// which the CLI starts as its daemon unless `--server` names another.
pub fn run_cli() -> i32 {
    let result = prepare().and_then(|options| {
        let Some(options) = options else { return Ok(0) };
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        runtime.block_on(run(options))
    });
    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("Error: {}", crate::sanitize::safe(&error.to_string()));
            1
        }
    }
}

/// Runs the internal TUI child of a daemon-owned launcher. The launcher has
/// already adopted the terminal; this process only builds the ordinary
/// protocol v3 client tree. The identity is read once here and removed from
/// the environment before any tool or child process can inherit it.
pub fn run_tui_child() -> i32 {
    let mut session = None;
    let mut home = None;
    let mut cwd = None;
    let mut endpoint = None;
    let mut args = std::env::args().skip(2);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} requires a value"));
        match arg.as_str() {
            "--session" => match value() {
                Ok(value) => session = Some(value),
                Err(error) => return child_error(error),
            },
            "--home" => match value() {
                Ok(value) => home = Some(PathBuf::from(value)),
                Err(error) => return child_error(error),
            },
            "--cwd" => match value() {
                Ok(value) => cwd = Some(PathBuf::from(value)),
                Err(error) => return child_error(error),
            },
            "--endpoint" => match value() {
                Ok(value) => endpoint = Some(PathBuf::from(value)),
                Err(error) => return child_error(error),
            },
            _ => return child_error(format!("unknown TUI child argument {arg}")),
        }
    }
    let Some(session) = session else {
        return child_error("the TUI child has no Session");
    };
    let Some(home) = home else {
        return child_error("the TUI child has no product home");
    };
    let Some(cwd) = cwd else {
        return child_error("the TUI child has no workspace");
    };
    let identity = match leaf_identity_from_env() {
        Some(identity) => identity,
        None => return child_error("the TUI child has no leaf identity"),
    };
    // The child is still single-threaded here. Do not let tools spawned by
    // the TUI inherit the control token or the link capability.
    for name in [
        "DIMCODE_UI_ID",
        "DIMCODE_UI_TOKEN",
        "DIMCODE_UI_GENERATION",
        "DIMCODE_UI_LEAF_CAP",
    ] {
        unsafe { std::env::remove_var(name) };
    }
    run_options(leaf_options(session, home, cwd, endpoint, Some(identity)))
}

fn leaf_identity_from_env() -> Option<args::UiIdentity> {
    let ui_id = std::env::var("DIMCODE_UI_ID").ok()?;
    let token = std::env::var("DIMCODE_UI_TOKEN").ok()?;
    let generation = std::env::var("DIMCODE_UI_GENERATION").ok()?.parse().ok()?;
    Some(args::UiIdentity {
        ui_id,
        token,
        generation,
    })
}

fn leaf_options(
    session_id: String,
    home: PathBuf,
    cwd: PathBuf,
    endpoint: Option<PathBuf>,
    ui_identity: Option<args::UiIdentity>,
) -> Options {
    Options {
        action: Action::Chat,
        prompt: None,
        images: Vec::new(),
        json: false,
        resume: Some(session_id),
        home,
        cwd,
        server: None,
        endpoint,
        model_config: std::env::var_os("DIMCODE_UI_MODEL_CONFIG").map(PathBuf::from),
        plugin_dir: None,
        node: None,
        js_runtime: None,
        provider: None,
        model: None,
        permission: None,
        directory: None,
        device: false,
        ui_identity,
        no_tui: false,
        fullscreen: None,
        refresh: false,
        redirect_uri: "http://localhost:54321/auth/callback".into(),
        no_browser: false,
        client_secret: false,
    }
}

fn child_error(error: impl std::fmt::Display) -> i32 {
    eprintln!("Error: {error}");
    1
}

fn run_options(options: Options) -> i32 {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => return child_error(error),
    };
    match runtime.block_on(run(options)) {
        Ok(code) => code,
        Err(error) => child_error(crate::sanitize::safe(&error.to_string())),
    }
}

/// Makes the terminal at `path` this process's standard input, output and
/// error. Used by a daemon-started leaf, whose own stdio is detached.
#[cfg(unix)]
pub fn adopt_terminal(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::io::AsRawFd;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?;
    let fd = file.as_raw_fd();
    for target in [libc::STDIN_FILENO, libc::STDOUT_FILENO] {
        // SAFETY: dup2 on an open descriptor owned by `file`; the target is
        // a standard descriptor and errors are reported to the caller.
        if unsafe { libc::dup2(fd, target) < 0 } {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(windows)]
pub fn adopt_terminal(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Console::{
        AllocConsole, FreeConsole, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
    };
    if path != std::path::Path::new("console") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "on Windows a leaf terminal is the new console",
        ));
    }
    // A daemon started from a terminal may already own one; the leaf gets
    // its own console instead of sharing the daemon's.
    unsafe {
        let _ = FreeConsole();
        if AllocConsole() == 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    let input = std::fs::OpenOptions::new().read(true).open("CONIN$")?;
    let output = std::fs::OpenOptions::new().write(true).open("CONOUT$")?;
    unsafe {
        if SetStdHandle(STD_INPUT_HANDLE, input.as_raw_handle() as _) == 0
            || SetStdHandle(STD_OUTPUT_HANDLE, output.as_raw_handle() as _) == 0
        {
            return Err(std::io::Error::last_os_error());
        }
    }
    // The standard handles reference these files for the process lifetime.
    std::mem::forget(input);
    std::mem::forget(output);
    Ok(())
}

#[cfg(not(any(unix, windows)))]
pub fn adopt_terminal(_path: &std::path::Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no terminal handoff is available on this platform",
    ))
}

/// The terminal device behind standard input, for the daemon launcher.
pub fn terminal_path() -> Option<String> {
    #[cfg(unix)]
    {
        use std::ffi::CStr;
        // SAFETY: ttyname returns a NUL-terminated string owned by libc, or
        // null when standard input is not a terminal.
        let name = unsafe { libc::ttyname(libc::STDIN_FILENO) };
        if name.is_null() {
            return None;
        }
        // SAFETY: a non-null ttyname result is NUL-terminated.
        unsafe { CStr::from_ptr(name) }
            .to_str()
            .ok()
            .map(str::to_owned)
    }
    #[cfg(windows)]
    {
        // The leaf allocates its own console; the caller only chooses that
        // mode.
        Some("console".to_owned())
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

/// The composition root: builds the client tree, loads what the action needs
/// and disposes the tree before the exit code goes back.
async fn run(options: Options) -> Result<i32> {
    let root = rutis::Ctx::root()?;
    let outcome = act(&root, options).await;
    // The action's own error is the one to report.
    let closed = root.shutdown().await;
    let code = outcome?;
    closed?;
    Ok(code)
}

/// Loads `DaemonLink` into `root` and returns it with the connection it
/// provides.
async fn daemon_link(
    root: &rutis::Ctx,
    options: &Options,
    start: bool,
) -> Result<(rutis::FiberView, Arc<plugins::Connection>)> {
    let link = root.plugin(rutis::Typed::new(plugins::DaemonLink {
        options: options.clone(),
        start,
    }));
    (&link).await.map_err(plugin_error)?;
    let connection = root
        .get::<plugins::Connection>()
        .ok_or("DaemonLink provided no connection")?;
    Ok((link, connection))
}

/// Runs the ordinary TUI plugin directly, for an older daemon without the
/// `ui.leaf` capability and for a leaf's TUI child. The caller owns the
/// terminal; the daemon is only the protocol v3 business peer.
async fn run_tui_with_connection(
    root: &rutis::Ctx,
    options: &Options,
    link: rutis::FiberView,
    _connection: Arc<plugins::Connection>,
) -> Result<i32> {
    let env = ui::Env {
        home: options.home.clone(),
        workspace: options.cwd.clone(),
        fullscreen: options
            .fullscreen
            .unwrap_or_else(|| ui::config::saved_fullscreen(&options.home)),
        leaf: options.ui_identity.is_some(),
    };
    let (leave, left) = tokio::sync::oneshot::channel();
    let tui = root.plugin(rutis::Typed::new(plugins::Tui {
        env,
        options: options.clone(),
        leave: std::sync::Mutex::new(Some(leave)),
    }));
    (&tui).await.map_err(plugin_error)?;
    let leave = left
        .await
        .map_err(|_| "the terminal interface ended without a result")?;
    tui.dispose().await?;
    link.dispose().await?;
    match leave {
        ui::tui::Leave::Exit(code) => Ok(code),
        ui::tui::Leave::Restart(restart) => relaunch(root, options, restart).await,
    }
}

/// Starts the daemon-owned leaf for the normal interactive entry point and
/// stays in the foreground until that leaf leaves. The parent creates or
/// resumes the Session so the leaf can attach, but it never touches the TUI
/// or the terminal after `ui.open`.
async fn open_tui_leaf(root: &rutis::Ctx, options: &Options) -> Result<i32> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Summary {
        session_id: String,
        cwd: String,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Created {
        session_id: String,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Opened {
        ui_id: String,
    }

    let (link, connection) = daemon_link(root, options, true).await?;
    if !connection.offers("ui.leaf") {
        return run_tui_with_connection(root, options, link, connection).await;
    }
    let mut client = connection.take().ok_or("App Server did not start")?;
    let (session_id, cwd) = if let Some(id) = &options.resume {
        let sessions: Vec<Summary> = client
            .typed("session.summaries", serde_json::json!({}))
            .await?;
        let summary = sessions
            .into_iter()
            .find(|session| &session.session_id == id)
            .ok_or_else(|| format!("Session {id} was not found"))?;
        if let Some(preset) = options.permission {
            permission::select(&mut client, id, preset)
                .await
                .map_err(|error| format!("Cannot set the permission preset: {error}"))?;
        }
        client
            .rpc("session.attach", serde_json::json!({ "sessionId": id }))
            .await?;
        (id.clone(), PathBuf::from(summary.cwd))
    } else {
        let mut params = serde_json::json!({ "cwd": options.cwd });
        if let (Some(provider), Some(model)) = (&options.provider, &options.model) {
            params["model"] = serde_json::json!({
                "provider": provider,
                "modelId": model,
            });
        }
        if let Some(preset) = options.permission {
            params["permissionPolicy"] = serde_json::json!({ "preset": preset });
        }
        let created: Created = client
            .typed("session.create", params)
            .await
            .map_err(|error| {
                format!(
                    "Cannot create session: {error}. Configure a provider or use --model-config PATH with an API-key environment reference"
                )
            })?;
        client
            .rpc(
                "session.attach",
                serde_json::json!({ "sessionId": created.session_id }),
            )
            .await?;
        (created.session_id, options.cwd.clone())
    };

    let tty = terminal_path().ok_or("the terminal UI needs an interactive terminal")?;
    // Install the parent signal listener before ui.open so a signal during
    // launch can still close the leaf after the daemon returns its id.
    let interrupted = Arc::new(AtomicBool::new(false));
    let signal = interrupted.clone();
    let signal_task = tokio::spawn(async move {
        wait_for_parent_signal().await;
        signal.store(true, Ordering::SeqCst);
    });
    if interrupted.load(Ordering::SeqCst) {
        signal_task.abort();
        let _ = link.dispose().await;
        return Ok(130);
    }
    let opened: Opened = client
        .typed(
            "ui.open",
            serde_json::json!({
                "sessionId": session_id,
                "kind": "tui",
                "tty": tty,
                "cwd": cwd,
                "home": options.home,
            }),
        )
        .await?;
    signal_task.abort();
    if interrupted.load(Ordering::SeqCst) {
        let _ = client
            .rpc("ui.close", serde_json::json!({ "uiId": opened.ui_id }))
            .await;
        let _ = link.dispose().await;
        return Ok(130);
    }

    // `ui.open` returns once the launcher is loaded; wait for the TUI to
    // attach, then for it to leave. The parent never reads the tty.
    let wait_online = async {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let status: serde_json::Value = client.rpc("ui.status", serde_json::json!({})).await?;
            let instance = status["instances"]
                .as_array()
                .and_then(|instances| instances.iter().find(|item| item["uiId"] == opened.ui_id));
            let Some(instance) = instance else {
                return Err::<(), Error>("the UI leaf disappeared before it became online".into());
            };
            match instance["state"].as_str() {
                Some("online") => return Ok::<(), Error>(()),
                Some("failed") => return Err::<(), Error>("the UI leaf failed to start".into()),
                _ if tokio::time::Instant::now() >= deadline => {
                    return Err::<(), Error>(
                        "the UI leaf did not attach its terminal in time".into(),
                    );
                }
                _ => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
            }
        }
    };
    let online = tokio::select! {
        online = wait_online => online,
        _ = wait_for_parent_signal() => {
            let _ = client
                .rpc("ui.close", serde_json::json!({ "uiId": opened.ui_id }))
                .await;
            let _ = link.dispose().await;
            return Ok(130);
        }
    };
    if let Err(error) = online {
        let _ = client
            .rpc("ui.close", serde_json::json!({ "uiId": opened.ui_id }))
            .await;
        let _ = link.dispose().await;
        return Err(error);
    }

    let wait = async {
        loop {
            let status: serde_json::Value = client.rpc("ui.status", serde_json::json!({})).await?;
            let instance = status["instances"]
                .as_array()
                .and_then(|instances| instances.iter().find(|item| item["uiId"] == opened.ui_id));
            match instance {
                Some(instance) => match instance["state"].as_str() {
                    Some("online" | "replacing" | "starting") => {
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                    Some("failed") => {
                        break Ok(instance["exitCode"]
                            .as_i64()
                            .and_then(|code| i32::try_from(code).ok())
                            .unwrap_or(1));
                    }
                    _ => {
                        break Ok(instance["exitCode"]
                            .as_i64()
                            .and_then(|code| i32::try_from(code).ok())
                            .unwrap_or(0));
                    }
                },
                None => break Ok(0),
            }
        }
    };
    let result = tokio::select! {
        result = wait => result,
        _ = wait_for_parent_signal() => {
            let _ = client
                .rpc("ui.close", serde_json::json!({ "uiId": opened.ui_id }))
                .await;
            let _ = link.dispose().await;
            return Ok(130);
        }
    };
    let _ = client
        .rpc("ui.close", serde_json::json!({ "uiId": opened.ui_id }))
        .await;
    let _ = link.dispose().await;
    result
}

/// The parent does not read the TTY, so an external SIGINT/SIGTERM/SIGHUP
/// must still ask the daemon to close the leaf before returning to the shell.
async fn wait_for_parent_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut terminate) = signal(SignalKind::terminate())
            && let Ok(mut hangup) = signal(SignalKind::hangup())
        {
            tokio::select! {
                _ = terminate.recv() => {}
                _ = hangup.recv() => {}
                _ = tokio::signal::ctrl_c() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

/// Starts the updated CLI on the session the interface left. The daemon
/// still runs the old executable, so it stops first and the new CLI starts
/// its own. The original arguments carry over except the interface mode,
/// which follows the one in use, and the session, which is the one left;
/// a resumed session keeps its model, so the model flags drop with it.
async fn relaunch(root: &rutis::Ctx, options: &Options, restart: ui::tui::Restart) -> Result<i32> {
    let (link, connection) = daemon_link(root, options, false).await?;
    if let Some(client) = connection.take() {
        client::shutdown(client).await?;
    }
    link.dispose().await?;
    let mut args = Vec::new();
    let mut original = std::env::args_os().skip(1);
    while let Some(arg) = original.next() {
        let resumed = restart.session_id.is_some();
        match arg.to_str() {
            Some("--fullscreen" | "--scrollback") => {}
            Some("--resume") => {
                original.next();
            }
            Some("--provider" | "--model") if resumed => {
                original.next();
            }
            _ => args.push(arg),
        }
    }
    args.push(
        (if restart.fullscreen {
            "--fullscreen"
        } else {
            "--scrollback"
        })
        .into(),
    );
    if let Some(session_id) = restart.session_id {
        args.extend(["--resume".into(), session_id.into()]);
    }
    let mut command = std::process::Command::new(&restart.relaunch);
    command.args(args);
    #[cfg(unix)]
    {
        // Replacing this process keeps the terminal and its signals with the
        // updated CLI, as the npm launcher does.
        use std::os::unix::process::CommandExt;
        let error = command.exec();
        Err(format!("Cannot start {}: {error}", restart.relaunch.display()).into())
    }
    #[cfg(not(unix))]
    {
        let status = command
            .status()
            .map_err(|error| format!("Cannot start {}: {error}", restart.relaunch.display()))?;
        Ok(status.code().unwrap_or(1))
    }
}

/// Links the daemon and loads `WebHold`; returns when the hold ends.
async fn hold_web(
    root: &rutis::Ctx,
    options: &Options,
    port: Option<u16>,
    open_browser: bool,
) -> Result<i32> {
    daemon_link(root, options, true).await?;
    let (done, held) = tokio::sync::oneshot::channel();
    let hold = root.plugin(rutis::Typed::new(plugins::WebHold {
        port,
        open_browser,
        home: options.home.clone(),
        done: std::sync::Mutex::new(Some(done)),
    }));
    (&hold).await.map_err(plugin_error)?;
    held.await
        .map_err(|_| "the Web workspace hold ended without a result")?
}

/// `dimcode web --off`: clear the daemon-owned switch without disconnecting a
/// foreground holder.
async fn disable_web(root: &rutis::Ctx, options: &Options) -> Result<i32> {
    let (link, connection) = daemon_link(root, options, false).await?;
    let Some(mut client) = connection.take() else {
        println!("Daemon is not running.");
        let _ = link.dispose().await;
        return Ok(0);
    };
    #[derive(serde::Deserialize)]
    struct Status {
        open: bool,
    }
    let status: Status = client.typed("web.disable", serde_json::json!({})).await?;
    println!(
        "{}",
        if status.open {
            "Web switch off; the entry is still held by another client."
        } else {
            "Web switch off."
        }
    );
    let _ = link.dispose().await;
    Ok(0)
}

/// A failed load as the plugin reported it, without the framework's framing.
fn plugin_error(error: Arc<rutis::CordisError>) -> Error {
    match &*error {
        rutis::CordisError::PluginFailed(source) => source.to_string().into(),
        other => other.to_string().into(),
    }
}

fn prepare() -> Result<Option<Options>> {
    let mut options = Options::parse(std::env::args().skip(1))?;
    if matches!(options.action, Action::Help) {
        print!("{}", args::HELP);
        return Ok(None);
    }
    if matches!(options.action, Action::Version) {
        println!("dimcode {}", env!("CARGO_PKG_VERSION"));
        return Ok(None);
    }
    if matches!(options.action, Action::McpLogin(_)) && !std::io::stdin().is_terminal() {
        return Err(
            "MCP sign-in requires a terminal; run dimcode mcp login ID interactively".into(),
        );
    }
    if matches!(options.action, Action::Tui) && !std::io::stdin().is_terminal() {
        return Err("tui requires an interactive terminal".into());
    }
    if matches!(options.action, Action::Exec)
        && options.prompt.is_none()
        && options.images.is_empty()
        && std::io::stdin().is_terminal()
    {
        return Err("exec requires --prompt TEXT, --image PATH or redirected stdin".into());
    }
    if let Action::DefaultTerminalMode { fullscreen } = options.action {
        if let Some(error) = ui::config::save_fullscreen(&options.home, fullscreen) {
            return Err(error.into());
        }
        let mode = if fullscreen {
            "fullscreen"
        } else {
            "scrollback"
        };
        println!("Default terminal mode: {mode}");
        return Ok(None);
    }
    if matches!(options.action, Action::Chat | Action::Exec)
        && options.prompt.is_none()
        && !std::io::stdin().is_terminal()
    {
        let mut prompt = String::new();
        std::io::stdin()
            .take(16 * 1024 * 1024 + 1)
            .read_to_string(&mut prompt)?;
        if prompt.len() > 16 * 1024 * 1024 {
            return Err("stdin prompt exceeds 16 MiB".into());
        }
        if prompt.trim().is_empty() && options.images.is_empty() {
            return Err("Provide --prompt TEXT, piped stdin, or an interactive terminal".into());
        }
        if !prompt.is_empty() {
            options.prompt = Some(prompt);
        }
    }
    Ok(Some(options))
}

async fn act(root: &rutis::Ctx, mut options: Options) -> Result<i32> {
    if matches!(options.action, Action::Chat | Action::Tui)
        && options.prompt.is_none()
        && !options.no_tui
        && ui::tui::is_interactive()
    {
        if options.permission.is_none() {
            options.permission = ui::config::saved_permission(&options.home)
                .and_then(|preset| permission::parse_preset(&preset).ok());
        }
        if options.ui_identity.is_none() {
            return open_tui_leaf(root, &options).await;
        }
        let (link, connection) = daemon_link(root, &options, false).await?;
        return run_tui_with_connection(root, &options, link, connection).await;
    }
    if let Action::Web {
        port,
        open_browser,
        off,
    } = options.action
    {
        if off {
            return disable_web(root, &options).await;
        }
        return hold_web(root, &options, port, open_browser).await;
    }

    // The Timeline export reads the database without a daemon.
    if let Action::Timeline {
        session_id,
        output,
        database,
    } = &options.action
    {
        export::timeline(&options, session_id, output.as_deref(), database.as_deref()).await?;
        return Ok(0);
    }
    let images = images::load(&options.images)?;
    let stop = matches!(options.action, Action::DaemonStop);
    let (_, connection) = daemon_link(root, &options, !stop).await?;
    if stop {
        return client::stop(connection.take()).await;
    }
    let mut client = connection.take().ok_or("App Server did not start")?;
    match &options.action {
        Action::Export {
            session_id,
            format,
            output,
        } => {
            export::write(&mut client, session_id, *format, output.as_deref()).await?;
            Ok(0)
        }
        Action::Login(provider) => login::login(&mut client, &options, provider).await,
        Action::Account(provider) => {
            #[derive(serde::Deserialize)]
            struct Snapshot {
                account: Option<Identity>,
            }
            #[derive(serde::Deserialize)]
            struct Identity {
                issuer: String,
                subject: String,
            }
            let snapshot: Snapshot = client
                .typed("account.status", serde_json::json!({"providerId":provider}))
                .await?;
            match snapshot.account {
                Some(account) => println!(
                    "Signed in: {} ({})",
                    crate::sanitize::safe(&account.subject),
                    crate::sanitize::safe(&account.issuer)
                ),
                None => println!("Signed out."),
            }
            Ok(0)
        }
        Action::Logout(provider) => {
            client
                .rpc("account.logout", serde_json::json!({"providerId":provider}))
                .await?;
            println!("Signed out.");
            Ok(0)
        }
        Action::Models(provider) => {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Catalog {
                models: Vec<Model>,
                disabled_model_ids: Vec<String>,
            }
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Model {
                id: String,
                name: String,
                unavailable_reason: Option<String>,
            }
            let params = serde_json::json!({"providerId":provider});
            let catalog = if options.refresh {
                Some(client.typed::<Catalog>("provider.refresh", params).await?)
            } else {
                #[derive(serde::Deserialize)]
                struct Snapshot {
                    catalog: Option<Catalog>,
                }
                client
                    .typed::<Snapshot>("provider.catalog", params)
                    .await?
                    .catalog
            };
            if let Some(catalog) = catalog {
                for model in catalog.models {
                    let availability = if catalog.disabled_model_ids.contains(&model.id) {
                        "disabled"
                    } else if model.unavailable_reason.is_some() {
                        "unavailable"
                    } else {
                        "available"
                    };
                    println!(
                        "{}\t{}\t{availability}",
                        crate::sanitize::safe(&model.id).replace(['\n', '\t'], " "),
                        crate::sanitize::safe(&model.name).replace(['\n', '\t'], " "),
                    );
                }
            } else {
                eprintln!("No cached catalog. Sign in and run models PROVIDER --refresh.");
            }
            Ok(0)
        }
        Action::McpList => {
            mcp_command::list(&mut client, &options).await?;
            Ok(0)
        }
        Action::McpLogin(id) => {
            let mut input = input::Input::new()?;
            mcp_command::sign_in(
                &mut client,
                &options,
                &mut input,
                id,
                options.no_browser,
                options.client_secret,
            )
            .await
        }
        Action::McpLogout(id) => mcp_command::sign_out(&mut client, &options, id).await,
        Action::McpEnable { id, enabled } => {
            // Target the entry `mcp list` shows: a project entry shadows the
            // user's, and enabling it is recorded for this user only.
            let catalog: mcp::Catalog = match client
                .typed("mcp.catalog", serde_json::json!({"cwd":options.cwd}))
                .await
            {
                Ok(catalog) => catalog,
                // Disabling never depends on reading the sources: the user's
                // record for this workspace is withdrawn without them.
                Err(error) if !*enabled && error.downcast_ref::<client::RpcError>().is_some() => {
                    client
                        .rpc(
                            "mcp.set_enabled",
                            serde_json::json!({"cwd":options.cwd,"id":id,"source":"project","expectedVersion":"","enabled":false}),
                        )
                        .await?;
                    println!("Disabled {:?} (project).", id);
                    eprintln!(
                        "The MCP catalog could not be read ({}); withdrew this workspace's enablement of {:?}. A user entry with this ID is unchanged.",
                        crate::sanitize::safe(&error.to_string()),
                        id
                    );
                    return Ok(0);
                }
                Err(error) => return Err(error),
            };
            // Disabling an id the workspace no longer lists still withdraws the
            // user's project record, so the entry stays off if it returns.
            let (source, listed) = match catalog.servers.into_iter().find(|server| server.id == *id)
            {
                Some(server) => (server.source, true),
                None if !*enabled && catalog.project_revision.is_some() => {
                    ("project".into(), false)
                }
                None => return Err(format!("No MCP server {:?} in mcp list", id).into()),
            };
            let expected = if source == "project" {
                catalog
                    .project_revision
                    .ok_or("Invalid mcp.catalog response")?
            } else {
                catalog.user_revision
            };
            client
                .rpc(
                    "mcp.set_enabled",
                    serde_json::json!({"cwd":options.cwd,"id":id,"source":source,"expectedVersion":expected,"enabled":enabled}),
                )
                .await?;
            println!(
                "{} {:?} ({}).",
                if *enabled { "Enabled" } else { "Disabled" },
                id,
                crate::sanitize::safe(&source)
            );
            if *enabled && source == "project" {
                eprintln!(
                    "It runs this workspace's .mcp.json entry, including later edits to it, in active and new sessions."
                );
            }
            if !listed {
                eprintln!(
                    "{:?} is not in mcp list; withdrew any stored enablement of it in this workspace.",
                    id
                );
            }
            Ok(0)
        }
        Action::Chat | Action::Tui | Action::Exec => {
            session::chat(&mut client, &options, images).await
        }
        Action::Help
        | Action::Version
        | Action::DaemonStop
        | Action::Timeline { .. }
        | Action::Web { .. }
        | Action::DefaultTerminalMode { .. } => unreachable!(),
    }
}
