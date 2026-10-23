//! Launcher-managed UI leaves. The daemon starts one launcher process per
//! UI, records its token and lifecycle, and closes it when asked. The UI's
//! business connection remains the existing protocol v3 connection; this
//! module owns only process lifecycle and the attach token.
//!
//! A launcher holds the terminal. The daemon is the lifecycle authority but
//! is not the UI's OS parent: launching from a terminal remains the caller's
//! job (see `docs/design/plugins/Tui.md` §3.6).

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use dim_agent_capabilities::methods::RpcError;
use dim_agent_runtime::{Capability, KeysOf, scoped};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypeKey, TypedPlugin};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
};
use uuid::Uuid;

/// How to start one UI launcher. The launcher receives the leaf identity,
/// token, Session and kind in its environment and is expected to connect the
/// ordinary App Server socket and call `ui.attach` with them.
#[derive(Clone, Debug)]
pub struct UiLauncherConfig {
    pub program: PathBuf,
    pub args: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UiLeafState {
    Starting,
    Online,
    Closing,
    Offline,
    Failed,
}
impl UiLeafState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Online => "online",
            Self::Closing => "closing",
            Self::Offline => "offline",
            Self::Failed => "failed",
        }
    }
}

pub(crate) struct UiLeafSnapshot {
    pub state: UiLeafState,
    pub pid: u32,
}

#[derive(Debug)]
pub(crate) struct Started {
    pub ui_id: String,
    pub pid: u32,
    #[cfg(test)]
    pub token: String,
}

struct Leaf {
    session_id: String,
    kind: String,
    token: [u8; 32],
    pid: u32,
    state: UiLeafState,
    child: Option<Child>,
}

/// Process-wide owner of launcher children. A leaf is created by `start`,
/// becomes `Online` when its business connection attaches with the token,
/// and is removed by `stop` or daemon shutdown.
pub(crate) struct UiLeafSupervisor {
    launcher: Option<UiLauncherConfig>,
    leaves: Mutex<HashMap<String, Leaf>>,
}

impl UiLeafSupervisor {
    pub fn new(launcher: Option<UiLauncherConfig>) -> Self {
        Self {
            launcher,
            leaves: Mutex::new(HashMap::new()),
        }
    }

    pub fn start(&self, session_id: &str, kind: &str) -> Result<Started, RpcError> {
        let launcher = self.launcher.as_ref().ok_or_else(|| {
            RpcError::app(
                -32000,
                "ui_launcher_unavailable",
                "This App Server has no UI launcher",
            )
        })?;
        let ui_id = Uuid::new_v4().to_string();
        let token = new_token();
        let mut command = Command::new(&launcher.program);
        command
            .args(&launcher.args)
            .env("DIMCODE_UI_ID", &ui_id)
            .env("DIMCODE_UI_TOKEN", &token)
            .env("DIMCODE_UI_SESSION", session_id)
            .env("DIMCODE_UI_KIND", kind)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = command
            .spawn()
            .map_err(|error| RpcError::app(-32000, "ui_launcher_failed", error.to_string()))?;
        let pid = child.id();
        self.leaves.lock().expect("ui leaves").insert(
            ui_id.clone(),
            Leaf {
                session_id: session_id.to_owned(),
                kind: kind.to_owned(),
                token: digest(&token),
                pid,
                state: UiLeafState::Starting,
                child: Some(child),
            },
        );
        Ok(Started {
            ui_id,
            pid,
            #[cfg(test)]
            token,
        })
    }

    /// Validates the launcher's attach token and marks the leaf online.
    /// Returns the Session and kind the launcher was started for.
    pub fn attach(&self, ui_id: &str, token: &str) -> Result<(String, String), RpcError> {
        let mut leaves = self.leaves.lock().expect("ui leaves");
        let leaf = leaves.get_mut(ui_id).ok_or_else(ui_unknown)?;
        refresh(leaf);
        if !matches!(leaf.state, UiLeafState::Starting | UiLeafState::Online) {
            return Err(RpcError::app(
                -32000,
                "ui_leaf_not_running",
                "The UI launcher is no longer running",
            ));
        }
        if digest(token) != leaf.token {
            return Err(RpcError::app(
                -32000,
                "ui_leaf_token",
                "The UI launcher token is invalid",
            ));
        }
        leaf.state = UiLeafState::Online;
        Ok((leaf.session_id.clone(), leaf.kind.clone()))
    }

    pub fn snapshot(&self, ui_id: &str) -> Option<UiLeafSnapshot> {
        let mut leaves = self.leaves.lock().expect("ui leaves");
        let leaf = leaves.get_mut(ui_id)?;
        refresh(leaf);
        Some(snapshot(leaf))
    }

    /// Stops one leaf. The caller sends an advisory `ui.shutdown` first when
    /// a business connection exists; `grace` lets the terminal restore before
    /// the process is killed. This is the boundary that guarantees the
    /// process is gone.
    pub async fn stop(&self, ui_id: &str, grace: std::time::Duration) -> bool {
        let child = self
            .leaves
            .lock()
            .expect("ui leaves")
            .remove(ui_id)
            .and_then(|mut leaf| {
                leaf.state = UiLeafState::Closing;
                leaf.child.take()
            });
        let Some(mut child) = child else {
            return false;
        };
        if !grace.is_zero() {
            let deadline = std::time::Instant::now() + grace;
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => return true,
                    Ok(None) if std::time::Instant::now() < deadline => {
                        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                    }
                    _ => break,
                }
            }
        }
        let _ = child.kill();
        let _ = tokio::task::spawn_blocking(move || child.wait()).await;
        true
    }

    pub async fn close_all(&self) {
        let children: Vec<_> = self
            .leaves
            .lock()
            .expect("ui leaves")
            .drain()
            .filter_map(|(_, mut leaf)| {
                leaf.state = UiLeafState::Closing;
                leaf.child.take()
            })
            .collect();
        for child in children {
            stop_child(Some(child)).await;
        }
    }
}

fn snapshot(leaf: &Leaf) -> UiLeafSnapshot {
    UiLeafSnapshot {
        state: leaf.state,
        pid: leaf.pid,
    }
}

fn refresh(leaf: &mut Leaf) {
    if matches!(leaf.state, UiLeafState::Offline | UiLeafState::Failed) {
        return;
    }
    let Some(child) = leaf.child.as_mut() else {
        return;
    };
    match child.try_wait() {
        Ok(Some(status)) => {
            leaf.state = if status.success() {
                UiLeafState::Offline
            } else {
                UiLeafState::Failed
            };
            leaf.child = None;
        }
        Ok(None) => {}
        Err(_) => {
            leaf.state = UiLeafState::Failed;
            leaf.child = None;
        }
    }
}

async fn stop_child(child: Option<Child>) -> bool {
    let Some(mut child) = child else {
        return false;
    };
    let _ = child.kill();
    let _ = tokio::task::spawn_blocking(move || child.wait()).await;
    true
}

fn new_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("operating system randomness");
    URL_SAFE_NO_PAD.encode(bytes)
}

fn digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

fn ui_unknown() -> RpcError {
    RpcError::app(-32000, "ui_unknown", "No such interface is registered")
}

/// Daemon process plugin that owns the launcher children.
pub(crate) struct UiLeafSupervisorPlugin {
    launcher: Option<UiLauncherConfig>,
}
impl UiLeafSupervisorPlugin {
    pub fn new(launcher: Option<UiLauncherConfig>) -> Self {
        Self { launcher }
    }
}
impl TypedPlugin for UiLeafSupervisorPlugin {
    type Deps = ();
    fn name(&self) -> &str {
        "UiLeafSupervisor"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let supervisor = Arc::new(UiLeafSupervisor::new(self.launcher.clone()));
            let owner = supervisor.clone();
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        owner.close_all().await;
                        Ok(())
                    })
                }))
            })?;
            ctx.provide_as(TypeKey::of::<UiLeafSupervisor>(), supervisor)?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for UiLeafSupervisorPlugin {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launcher(script: &str) -> UiLauncherConfig {
        UiLauncherConfig {
            program: "sh".into(),
            args: vec!["-c".into(), script.into()],
        }
    }

    #[tokio::test]
    async fn starts_attaches_and_stops_a_leaf() {
        let supervisor = UiLeafSupervisor::new(Some(launcher("sleep 30")));
        let started = supervisor.start("session", "tui").unwrap();
        let snapshot = supervisor.snapshot(&started.ui_id).expect("leaf");
        assert_eq!(snapshot.state, UiLeafState::Starting);
        assert_eq!(snapshot.pid, started.pid);

        // A wrong token fails; the token from start binds the leaf.
        assert!(supervisor.attach(&started.ui_id, "wrong").is_err());
        assert_eq!(
            supervisor.attach(&started.ui_id, &started.token).unwrap(),
            ("session".into(), "tui".into())
        );
        assert!(
            supervisor
                .stop(&started.ui_id, std::time::Duration::ZERO)
                .await
        );
        assert!(supervisor.snapshot(&started.ui_id).is_none());
    }

    #[tokio::test]
    async fn a_finished_launcher_is_not_online() {
        let supervisor = UiLeafSupervisor::new(Some(launcher("exit 7")));
        let started = supervisor.start("session", "tui").unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let snapshot = supervisor.snapshot(&started.ui_id).expect("leaf");
        assert_eq!(snapshot.state, UiLeafState::Failed);
        assert!(!supervisor.attach(&started.ui_id, "wrong").is_ok());
        supervisor
            .stop(&started.ui_id, std::time::Duration::ZERO)
            .await;
    }

    #[tokio::test]
    async fn close_all_stops_every_child() {
        let supervisor = UiLeafSupervisor::new(Some(launcher("sleep 30")));
        let first = supervisor.start("session", "tui").unwrap();
        let second = supervisor.start("session", "electron").unwrap();
        supervisor.close_all().await;
        assert!(supervisor.snapshot(&first.ui_id).is_none());
        assert!(supervisor.snapshot(&second.ui_id).is_none());
    }

    #[test]
    fn an_unconfigured_supervisor_reports_unavailable() {
        let supervisor = UiLeafSupervisor::new(None);
        let error = supervisor.start("session", "tui").unwrap_err();
        assert_eq!(
            error.data.as_ref().map(|data| data.app_error.as_str()),
            Some("ui_launcher_unavailable")
        );
    }
}
