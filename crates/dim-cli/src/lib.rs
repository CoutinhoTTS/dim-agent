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
use std::io::{IsTerminal, Read};
use std::sync::Arc;

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
    handoff: Option<Box<ui::tui::WebHandoff>>,
) -> Result<i32> {
    daemon_link(root, options, true).await?;
    let (done, held) = tokio::sync::oneshot::channel();
    let hold = root.plugin(rutis::Typed::new(plugins::WebHold {
        port,
        open_browser,
        home: options.home.clone(),
        handoff,
        done: std::sync::Mutex::new(Some(done)),
    }));
    (&hold).await.map_err(plugin_error)?;
    held.await
        .map_err(|_| "the Web workspace hold ended without a result")?
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
    if matches!(options.action, Action::Chat)
        && options.prompt.is_none()
        && !options.no_tui
        && ui::tui::is_interactive()
    {
        let env = ui::Env {
            home: options.home.clone(),
            workspace: options.cwd.clone(),
            fullscreen: options
                .fullscreen
                .unwrap_or_else(|| ui::config::saved_fullscreen(&options.home)),
        };
        // The preset chosen in /approvals stays the default until changed
        // there; the flag overrides it for one run.
        if options.permission.is_none() {
            options.permission = ui::config::saved_permission(&options.home)
                .and_then(|preset| permission::parse_preset(&preset).ok());
        }
        let (link, _) = daemon_link(root, &options, true).await?;
        let (leave, left) = tokio::sync::oneshot::channel();
        let tui = root.plugin(rutis::Typed::new(plugins::Tui {
            env,
            options: options.clone(),
            leave: std::sync::Mutex::new(Some(leave)),
        }));
        (&tui).await.map_err(plugin_error)?;
        let handoff = match left
            .await
            .map_err(|_| "the terminal interface ended without a result")?
        {
            ui::tui::Leave::Exit(code) => return Ok(code),
            ui::tui::Leave::Web(handoff) => handoff,
            ui::tui::Leave::Restart(restart) => {
                tui.dispose().await?;
                link.dispose().await?;
                return relaunch(root, &options, restart).await;
            }
        };
        // `/web` hands the session over: the interface and its connection are
        // gone, and the browser opens on the session over a new connection,
        // as `dimcode web` would.
        tui.dispose().await?;
        link.dispose().await?;
        return hold_web(root, &options, None, true, Some(handoff)).await;
    }
    if let Action::Web { port, open_browser } = options.action {
        return hold_web(root, &options, port, open_browser, None).await;
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
