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
mod leaf;
mod login;
mod mcp;
mod mcp_command;
mod model;
mod permission;
mod permission_command;
mod plugin_command;
mod plugins;
mod projection;
mod question;
mod sanitize;
mod session;
mod session_host;
mod ui;
mod update;
mod usage;
mod web_install;
mod wire;

use args::{Action, Options};
use rutis::host::CtxHost;
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
        let acp = matches!(
            options.action,
            Action::Acp {
                auth_setup: None,
                ..
            }
        );
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        let result = runtime.block_on(run(options));
        // Owned plugin work has drained; blocked OS stdio must not hold CLI exit.
        if acp {
            runtime.shutdown_background();
        }
        result
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
) -> Result<(rutis::host::FiberView, Arc<plugins::Connection>)> {
    let link = root.plugin(plugins::DaemonLink {
        options: options.clone(),
        start,
    });
    (&link).await.map_err(plugin_error)?;
    let connection = root
        .get::<plugins::Connection>()
        .ok_or("DaemonLink provided no connection")?;
    Ok((link, connection))
}

/// Runs the ordinary TUI plugin directly, for an older daemon without the
/// `ui.leaf` capability, initial model setup and a leaf's TUI child. The caller
/// owns the terminal; the daemon is only the protocol v3 business peer.
async fn run_tui_with_connection(
    root: &rutis::Ctx,
    options: &Options,
    link: rutis::host::FiberView,
    _connection: Arc<plugins::Connection>,
) -> Result<i32> {
    let env = ui::Env {
        home: options.home.clone(),
        workspace: options.cwd.clone(),
        fullscreen: options
            .fullscreen
            .unwrap_or_else(|| ui::config::saved_fullscreen(&options.home)),
    };
    let (leave, left) = tokio::sync::oneshot::channel();
    let tui = root.plugin(plugins::Tui {
        env,
        options: options.clone(),
        leave: std::sync::Mutex::new(Some(leave)),
    });
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

/// The interactive entry on a daemon that hosts UI leaves (Tui design
/// §3.6): this process creates or resumes the Session, mounts its leaf
/// node and asks the daemon to host `Tui` in it. It ends when the `Tui`
/// leaves or the daemon closes the link.
async fn open_tui_leaf(root: &rutis::Ctx, options: &Options) -> Result<i32> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Created {
        session_id: String,
    }

    let (link, connection) = daemon_link(root, options, true).await?;
    if !connection.offers("ui.leaf") {
        return run_tui_with_connection(root, options, link, connection).await;
    }
    let mut client = connection.take().ok_or("App Server did not start")?;
    link.dispose().await?;
    let session_id = if let Some(id) = &options.resume {
        if let Some(preset) = options.permission {
            permission::select(&mut client, id, preset)
                .await
                .map_err(|error| format!("Cannot set the permission preset: {error}"))?;
        }
        client
            .rpc("session.attach", serde_json::json!({ "sessionId": id }))
            .await?;
        id.clone()
    } else {
        let mut params = serde_json::json!({ "cwd": options.cwd });
        let mut preferences = None;
        if let (Some(provider), Some(model)) = (&options.provider, &options.model) {
            params["model"] = serde_json::json!({
                "provider": provider,
                "modelId": model,
            });
            preferences =
                session_host::saved_model_preferences(&mut client, provider, model).await?;
            if let Some(preferences) = &preferences {
                params["modelPreferences"] = preferences.clone();
            }
        }
        if let Some(preset) = options.permission {
            params["permissionPolicy"] = serde_json::json!({ "preset": preset });
        }
        let created: Created = match client.typed("session.create", params).await {
            Ok(created) => created,
            Err(error)
                if options.provider.is_none()
                    && options.model_config.is_none()
                    && client::app_error(&error) == Some("provider_failed") =>
            {
                drop(client);
                let (link, connection) = daemon_link(root, options, false).await?;
                return run_tui_with_connection(root, options, link, connection).await;
            }
            Err(error) => {
                let error = session_host::model_adoption_error(error, preferences.as_ref());
                return Err(format!(
                    "Cannot create session: {error}. Configure a provider or use --model-config PATH with an API-key environment reference"
                ).into());
            }
        };
        client
            .rpc(
                "session.attach",
                serde_json::json!({ "sessionId": created.session_id }),
            )
            .await?;
        created.session_id
    };

    let leaf = leaf::Leaf::bind(options)?;
    let (leave, left) = tokio::sync::oneshot::channel();
    let (node, state) = leaf.node(options.clone(), leave);
    let node = root.plugin(node);
    (&node).await.map_err(plugin_error)?;
    let opened = client
        .rpc(
            "ui.open",
            serde_json::json!({
                "sessionId": session_id,
                "kind": "tui",
                "peer": leaf.peer,
                "address": leaf.address,
            }),
        )
        .await;
    drop(client);
    if let Err(error) = opened {
        node.dispose().await?;
        return Err(error);
    }
    let leave = tokio::select! {
        leave = left => leave.ok(),
        () = leaf::stopped(&state) => None,
    };
    node.dispose().await?;
    match leave {
        Some(ui::tui::Leave::Exit(code)) => Ok(code),
        Some(ui::tui::Leave::Restart(restart)) => relaunch(root, options, restart).await,
        None => Ok(0),
    }
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
    let hold = root.plugin(plugins::WebHold {
        port,
        open_browser,
        home: options.home.clone(),
        done: std::sync::Mutex::new(Some(done)),
    });
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
fn plugin_error(error: Arc<rutis::host::ErrorKind>) -> Error {
    match &*error {
        rutis::host::ErrorKind::PluginFailed(source) => source.to_string().into(),
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

async fn run_acp(root: &rutis::Ctx, options: &Options, agent_id: Option<String>) -> Result<i32> {
    let mut signals = plugins::Signals::listen()?;
    let client = client::connect_inner(options, true)
        .await?
        .ok_or("App Server did not start")?;
    let (gateway, finished) = dim_acp::AcpGateway::new(
        client,
        dim_acp::Io {
            reader: Box::new(tokio::io::stdin()),
            writer: Box::new(tokio::io::stdout()),
        },
        agent_id,
    );
    let gateway = gateway.login(dim_acp::auth::Login {
        directory: options
            .directory
            .clone()
            .unwrap_or_else(|| login::account_directory().into()),
        redirect_uri: options.redirect_uri.clone(),
        open_browser: if std::env::var("DIMCODE_OAUTH_MANUAL").is_ok_and(|value| value == "1") {
            None
        } else {
            Some(browser::open)
        },
        terminal: true,
    });
    let gateway = root.plugin(gateway);
    (&gateway).await.map_err(plugin_error)?;
    let (code, result) = tokio::select! {
        result = finished => (0, result.map_err(|_| "ACP gateway ended without a result")?),
        () = signals.received() => (130, Ok(())),
    };
    gateway.dispose().await?;
    result?;
    Ok(code)
}

async fn act(root: &rutis::Ctx, mut options: Options) -> Result<i32> {
    if let Action::Acp {
        auth_setup: Some(method),
        ..
    } = &options.action
    {
        let mut client = client::connect_inner(&options, true)
            .await?
            .ok_or("App Server did not start")?;
        let (provider, device) = dim_acp::auth::terminal_method(&mut client, method).await?;
        options.device = device;
        return login::login(&mut client, &options, &provider).await;
    }
    if let Action::Acp { agent_id, .. } = &options.action {
        return run_acp(root, &options, agent_id.clone()).await;
    }
    if matches!(options.action, Action::Chat | Action::Tui)
        && options.prompt.is_none()
        && !options.no_tui
        && ui::tui::is_interactive()
    {
        if options.permission.is_none() {
            options.permission = ui::config::saved_permission(&options.home)
                .and_then(|preset| permission::parse_preset(&preset).ok());
        }
        return open_tui_leaf(root, &options).await;
    }
    if matches!(options.action, Action::WebInstall) {
        return web_install::install(&options.home).await;
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
            if let Some(model) = &options.clear_preferences {
                session_host::clear_saved_model_preferences(&mut client, provider, model).await?;
                println!("Saved model preferences cleared");
                return Ok(0);
            }
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
        Action::Plugin { method, params } => {
            plugin_command::run(&mut client, method, params.clone()).await
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
            let catalog: mcp::Catalog = match client
                .typed("mcp.catalog", serde_json::json!({"cwd":options.cwd}))
                .await
            {
                Ok(catalog) => catalog,
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
        Action::Acp { .. }
        | Action::Help
        | Action::Version
        | Action::DaemonStop
        | Action::Timeline { .. }
        | Action::Web { .. }
        | Action::WebInstall
        | Action::DefaultTerminalMode { .. } => unreachable!(),
    }
}
