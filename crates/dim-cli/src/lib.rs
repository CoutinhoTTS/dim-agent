
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
mod model;
mod permission;
mod permission_command;
mod projection;
mod session;
mod session_host;
mod ui;
mod usage;
mod wire;

use args::{Action, Options};
use std::io::{IsTerminal, Read};

type Error = Box<dyn std::error::Error + Send + Sync>;
type Result<T> = std::result::Result<T, Error>;

/// Runs the CLI with the process arguments and returns its exit code. The
/// host binary must answer `app-server` with the App Server command line,
/// which the CLI starts as its daemon unless `--server` names another.
pub fn run_cli() -> i32 {
    let result = prepare().and_then(|options| {
        let Some(options) = options else { return Ok(0) };
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
            let host = session_host::SessionHost::start(options.clone());
            let handoff = match ui::tui::run(Box::new(host), env) {
                ui::tui::Leave::Exit(code) => return Ok(code),
                ui::tui::Leave::Web(handoff) => handoff,
            };
            // `/web` hands the session over: the interface has closed and the
            // browser opens on it, as `dimcode web` would.
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            return runtime.block_on(async {
                let mut client = client::connect(&options).await?;
                open_web(&mut client, None, true, Some(&handoff)).await
            });
        }
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        runtime.block_on(run(options))
    });
    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("Error: {}", display::safe(&error.to_string()));
            1
        }
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

async fn run(options: Options) -> Result<i32> {
    if matches!(options.action, Action::DaemonStop) {
        return client::stop(&options).await;
    }
    let images = images::load(&options.images)?;
    let mut client = client::connect(&options).await?;
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
                    display::safe(&account.subject),
                    display::safe(&account.issuer)
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
                        display::safe(&model.id).replace(['\n', '\t'], " "),
                        display::safe(&model.name).replace(['\n', '\t'], " "),
                    );
                }
            } else {
                eprintln!("No cached catalog. Sign in and run models PROVIDER --refresh.");
            }
            Ok(0)
        }
        Action::McpList => {
            mcp::list(&mut client, &options).await?;
            Ok(0)
        }
        Action::McpLogin(id) => {
            let mut input = input::Input::new()?;
            mcp::sign_in(
                &mut client,
                &options,
                &mut input,
                id,
                options.no_browser,
                options.client_secret,
            )
            .await
        }
        Action::McpLogout(id) => mcp::sign_out(&mut client, &options, id).await,
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
                        display::safe(&error.to_string()),
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
                display::safe(&source)
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
        Action::Web { port, open_browser } => {
            open_web(&mut client, *port, *open_browser, None).await
        }
        Action::Chat | Action::Tui | Action::Exec => {
            session::chat(&mut client, &options, images).await
        }
        Action::Help
        | Action::Version
        | Action::DaemonStop
        | Action::DefaultTerminalMode { .. } => unreachable!(),
    }
}

/// Opens the daemon's web entry, at the handed-over session's chat for
/// `/web`, and shows where it is.
async fn open_web(
    client: &mut client::Client,
    port: Option<u16>,
    open_browser: bool,
    handoff: Option<&ui::tui::WebHandoff>,
) -> Result<i32> {
    #[derive(serde::Deserialize)]
    struct Opened {
        url: String,
        address: String,
    }
    let params = match port {
        Some(port) => serde_json::json!({ "port": port }),
        None => serde_json::json!({}),
    };
    let opened: Opened = client.typed("web.open", params).await?;
    let session_id = handoff.and_then(|handoff| handoff.session_id.as_deref());
    let fragment = opened.url.split_once('#').map_or("", |(_, f)| f);
    let path = session_id.map_or(String::new(), |id| format!("chat/{id}"));
    let url = format!("http://{}/{path}", opened.address);
    let sign_in = format!("{url}#{fragment}");
    // The sign-in link carries the token; it is shown only to this user
    // when no browser took it.
    let opened_browser = open_browser && browser::open(&sign_in);
    ui::tui::print_web_panel(
        &ui::tui::WebOpened {
            url: &url,
            sign_in: (!opened_browser).then_some(sign_in.as_str()),
            session_id,
        },
        handoff,
    );
    // The entry stays open while this connection does: wait in the
    // foreground until Ctrl+C or SIGTERM, then exit, which closes it.
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    #[cfg(unix)]
    let terminated = terminate.recv();
    #[cfg(not(unix))]
    let terminated = std::future::pending::<Option<()>>();
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminated => {}
        // Nothing is subscribed, so the stream ends only with the daemon.
        error = async { loop { if let Err(error) = client.next().await { break error } } } => {
            return Err(format!("The App Server stopped; the Web workspace is closed ({error})").into());
        }
    }
    println!();
    println!("Web workspace stopped.");
    Ok(0)
}
