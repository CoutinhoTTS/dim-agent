//! The hosted `Tui` of a UI leaf (Tui design §3.6). The daemon loads it in
//! the leaf node with `plugins.load("Tui", config)`; `config` is the leaf's
//! whole input and is parsed here, at the leaf boundary. The plugin adopts
//! the terminal and runs the ordinary client tree (`DaemonLink` + `Tui`)
//! as children, so unloading it restores the terminal through `Tui`'s own
//! effect.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rutis::{BoxFuture, CordisError, Ctx, Effect, Plugin};
use serde::Deserialize;

use crate::args::{Action, Options, UiIdentity};
use crate::plugins::{DaemonLink, Tui};
use crate::ui;

/// Receives the exit code once the user quit and the daemon was told.
type Left = Box<dyn FnOnce(i32) + Send>;

/// The hosted `Tui` config the daemon sends.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LeafConfig {
    ui_id: String,
    /// Reported in `ui.status`; the leaf itself does not use it.
    #[allow(dead_code)]
    generation: u64,
    token: String,
    session_id: String,
    tty: PathBuf,
    endpoint: PathBuf,
    home: PathBuf,
    cwd: PathBuf,
    model_config: Option<PathBuf>,
}

/// The JSON schema of [`LeafConfig`], for the leaf's host catalog.
pub fn schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["uiId", "generation", "token", "sessionId", "tty", "endpoint", "home", "cwd"],
        "properties": {
            "uiId": {"type": "string"},
            "generation": {"type": "integer"},
            "token": {"type": "string"},
            "sessionId": {"type": "string"},
            "tty": {"type": "string"},
            "endpoint": {"type": "string"},
            "home": {"type": "string"},
            "cwd": {"type": "string"},
            "modelConfig": {"type": ["string", "null"]}
        }
    })
}

/// Builds the hosted `Tui` from its config. `left` receives the exit code
/// once the user quit and the daemon was told (`ui.leave`).
pub fn hosted_tui(config: &serde_json::Value, left: Left) -> Result<Box<dyn Plugin>, CordisError> {
    let config = LeafConfig::deserialize(config)
        .map_err(|error| CordisError::PluginFailed(format!("Tui config: {error}").into()))?;
    Ok(Box::new(HostedTui {
        config,
        left: Mutex::new(Some(left)),
    }))
}

struct HostedTui {
    config: LeafConfig,
    left: Mutex<Option<Left>>,
}

impl HostedTui {
    fn options(&self) -> Options {
        let config = &self.config;
        let mut options = Options::new(config.home.clone(), config.cwd.clone());
        options.action = Action::Chat;
        options.resume = Some(config.session_id.clone());
        options.endpoint = Some(config.endpoint.clone());
        options.model_config = config.model_config.clone();
        options.ui_identity = Some(UiIdentity {
            ui_id: config.ui_id.clone(),
            token: config.token.clone(),
        });
        options
    }
}

impl Plugin for HostedTui {
    fn name(&self) -> &str {
        "Tui"
    }

    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            adopt_terminal(&self.config.tty).map_err(|error| {
                CordisError::PluginFailed(format!("cannot open the terminal: {error}").into())
            })?;
            let options = self.options();
            let link = ctx.plugin(rutis::Typed::new(DaemonLink {
                options: options.clone(),
                // A leaf never starts a daemon: it shows the one that hosts it.
                start: false,
            }));
            (&link).await.map_err(unwrap)?;
            let (leave, leaving) = tokio::sync::oneshot::channel();
            let env = ui::Env {
                home: options.home.clone(),
                workspace: options.cwd.clone(),
                fullscreen: ui::config::saved_fullscreen(&options.home),
                leaf: true,
            };
            let tui = ctx.plugin(rutis::Typed::new(Tui {
                env,
                options: options.clone(),
                leave: Mutex::new(Some(leave)),
            }));
            (&tui).await.map_err(unwrap)?;
            let left = self
                .left
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
                .ok_or_else(|| CordisError::PluginFailed("Tui was loaded twice".into()))?;
            let identity = options.ui_identity.clone().expect("a leaf identity");
            let waiting = tokio::spawn(async move {
                let Ok(leave) = leaving.await else {
                    return;
                };
                // Restart is refused in a leaf (§3.6), so it never ends one.
                let code = match leave {
                    ui::tui::Leave::Exit(code) => code,
                    ui::tui::Leave::Restart(_) => 0,
                };
                report_leave(&options, &identity, code).await;
                left(code);
            });
            Ok(Effect::Disposer(Box::new(move || {
                waiting.abort();
                Ok(())
            })))
        })
    }
}

/// Tells the daemon the exit code. The `Tui`'s own connection has closed
/// with its session thread, so this is a short call on a new one.
async fn report_leave(options: &Options, identity: &UiIdentity, code: i32) {
    let call = async {
        let mut client = crate::client::connect_inner(options, false)
            .await?
            .ok_or("the App Server is gone")?;
        client
            .rpc(
                "ui.leave",
                serde_json::json!({
                    "uiId": identity.ui_id,
                    "token": identity.token,
                    "exitCode": code,
                }),
            )
            .await?;
        crate::Result::Ok(())
    };
    // Closed by the daemon meanwhile, or the daemon is gone: the exit code
    // then has no one to go to.
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), call).await;
}

fn unwrap(error: Arc<CordisError>) -> CordisError {
    CordisError::PluginFailed(error.to_string().into())
}

/// Makes the terminal at `path` this process's standard input and output.
/// The leaf's own stdio is detached; it has one terminal for its lifetime.
#[cfg(unix)]
fn adopt_terminal(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOCTTY)
        .open(path)?;
    let fd = file.as_raw_fd();
    // SAFETY: isatty on a descriptor owned by `file`.
    if unsafe { libc::isatty(fd) } != 1 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{} is not a terminal", path.display()),
        ));
    }
    for target in [libc::STDIN_FILENO, libc::STDOUT_FILENO] {
        // SAFETY: dup2 on an open descriptor owned by `file` onto a standard
        // descriptor; errors are reported to the caller.
        if unsafe { libc::dup2(fd, target) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(windows)]
fn adopt_terminal(path: &Path) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Console::{
        AllocConsole, FreeConsole, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
    };
    if path != Path::new("console") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "on Windows a leaf terminal is the new console",
        ));
    }
    // SAFETY: console calls on this process; errors are reported.
    unsafe {
        let _ = FreeConsole();
        if AllocConsole() == 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    let input = std::fs::OpenOptions::new().read(true).open("CONIN$")?;
    let output = std::fs::OpenOptions::new().write(true).open("CONOUT$")?;
    // SAFETY: the handles stay open for the process lifetime (forgotten below).
    unsafe {
        if SetStdHandle(STD_INPUT_HANDLE, input.as_raw_handle() as _) == 0
            || SetStdHandle(STD_OUTPUT_HANDLE, output.as_raw_handle() as _) == 0
        {
            return Err(std::io::Error::last_os_error());
        }
    }
    std::mem::forget(input);
    std::mem::forget(output);
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn adopt_terminal(_path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no terminal handoff is available on this platform",
    ))
}

/// The terminal behind standard input, for `ui.open`'s `tty`.
pub(crate) fn terminal_path() -> Option<String> {
    #[cfg(unix)]
    {
        // SAFETY: ttyname returns a NUL-terminated string owned by libc, or
        // null when standard input is not a terminal.
        let name = unsafe { libc::ttyname(libc::STDIN_FILENO) };
        if name.is_null() {
            return None;
        }
        // SAFETY: a non-null ttyname result is NUL-terminated.
        unsafe { std::ffi::CStr::from_ptr(name) }
            .to_str()
            .ok()
            .map(str::to_owned)
    }
    #[cfg(windows)]
    {
        Some("console".to_owned())
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}
