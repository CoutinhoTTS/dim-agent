//! The UI registry: which interfaces are alive on which Session, and the
//! way to close one. The daemon is the authority over interfaces without
//! being their parent process or touching a terminal: an interface registers
//! the connection it already has, and a shutdown is a notification on it.
//! A daemon-started launcher is additionally owned by `UiLeafSupervisor`,
//! which owns its process and attach token.
//! See `docs/design/plugins/Tui.md` §3.6.

use crate::{
    connection::{Connection, ConnectionHub, Entry},
    protocol::{
        Notification, Outbound, UiAttached, UiInstance, UiOpened, UiShutdownNotice, UiStatusResult,
    },
    ui_leaf::UiLeafSupervisor,
};
use dim_agent_capabilities::methods::RpcError;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};

/// One registered interface. Legacy registrations are created by
/// `ui.attach` on an attached connection; managed registrations are created
/// by `ui.open` and bound by `ui.attach` with the launcher token.
struct Instance {
    connection: Option<String>,
    session_id: String,
    kind: String,
    since: Instant,
    leaf: bool,
    pid: Option<u32>,
}

/// Process-wide registry; a registration ends with its connection.
pub(crate) struct UiHost {
    supervisor: Arc<UiLeafSupervisor>,
    instances: Mutex<HashMap<String, Instance>>,
}

impl UiHost {
    pub fn new(supervisor: Arc<UiLeafSupervisor>) -> Self {
        Self {
            supervisor,
            instances: Mutex::new(HashMap::new()),
        }
    }

    /// Starts a launcher for `session_id`. The caller must already be
    /// attached to the Session; the launcher itself attaches later with the
    /// token in its environment.
    pub fn open(
        &self,
        connection: &Connection,
        session_id: &str,
        kind: &str,
    ) -> Result<UiOpened, RpcError> {
        if !connection.is_attached(session_id) {
            return Err(RpcError::app(
                -32000,
                "session_not_attached",
                "Attach to the Session before starting an interface on it",
            ));
        }
        validate_kind(kind)?;
        let started = self.supervisor.start(session_id, kind)?;
        let mut instances = self.instances.lock().expect("ui registry");
        instances.insert(
            started.ui_id.clone(),
            Instance {
                connection: None,
                session_id: session_id.to_owned(),
                kind: kind.to_owned(),
                since: Instant::now(),
                leaf: true,
                pid: Some(started.pid),
            },
        );
        Ok(UiOpened {
            ui_id: started.ui_id,
            pid: started.pid,
        })
    }

    /// Registers `connection` as an interface of the Session it is attached
    /// to. Registering again returns the same id. A managed launcher passes
    /// its `ui_id` and token; a legacy interface passes neither.
    pub fn attach(
        &self,
        connection: &Connection,
        session_id: &str,
        kind: &str,
        ui_id: Option<&str>,
        token: Option<&str>,
    ) -> Result<UiAttached, RpcError> {
        if !connection.is_attached(session_id) {
            return Err(RpcError::app(
                -32000,
                "session_not_attached",
                "Attach to the Session before registering an interface on it",
            ));
        }
        validate_kind(kind)?;
        if let Some(ui_id) = ui_id {
            let token = token
                .ok_or_else(|| RpcError::invalid("uiId and token must be supplied together"))?;
            let (leaf_session, leaf_kind) = self.supervisor.attach(ui_id, token)?;
            if leaf_session != session_id || leaf_kind != kind {
                return Err(RpcError::invalid(
                    "The launcher is registered for another Session or kind",
                ));
            }
            let mut instances = self.instances.lock().expect("ui registry");
            if let Some(instance) = instances.get_mut(ui_id) {
                if instance
                    .connection
                    .as_deref()
                    .is_some_and(|owner| owner != connection.id)
                {
                    return Err(RpcError::app(
                        -32000,
                        "ui_in_use",
                        "Another connection already owns this launcher",
                    ));
                }
                instance.connection = Some(connection.id.clone());
                instance.kind = kind.to_owned();
                return Ok(UiAttached {
                    ui_id: ui_id.to_owned(),
                });
            }
            // A launcher may attach after a daemon restart lost its
            // placeholder; the supervisor still owns the process.
            let pid = self.supervisor.snapshot(ui_id).map(|leaf| leaf.pid);
            instances.insert(
                ui_id.to_owned(),
                Instance {
                    connection: Some(connection.id.clone()),
                    session_id: session_id.to_owned(),
                    kind: kind.to_owned(),
                    since: Instant::now(),
                    leaf: true,
                    pid,
                },
            );
            return Ok(UiAttached {
                ui_id: ui_id.to_owned(),
            });
        }
        if token.is_some() {
            return Err(RpcError::invalid(
                "uiId and token must be supplied together",
            ));
        }
        let mut instances = self.instances.lock().expect("ui registry");
        if let Some((id, instance)) = instances.iter_mut().find(|(_, i)| {
            i.connection.as_deref() == Some(connection.id.as_str()) && i.session_id == session_id
        }) {
            instance.kind = kind.to_owned();
            return Ok(UiAttached { ui_id: id.clone() });
        }
        let ui_id = uuid::Uuid::new_v4().to_string();
        instances.insert(
            ui_id.clone(),
            Instance {
                connection: Some(connection.id.clone()),
                session_id: session_id.to_owned(),
                kind: kind.to_owned(),
                since: Instant::now(),
                leaf: false,
                pid: None,
            },
        );
        Ok(UiAttached { ui_id })
    }

    pub fn status(&self, hub: &ConnectionHub) -> UiStatusResult {
        let instances = self.instances.lock().expect("ui registry");
        let mut listed: Vec<UiInstance> = instances
            .iter()
            .map(|(id, instance)| {
                let leaf = instance
                    .leaf
                    .then(|| self.supervisor.snapshot(id))
                    .flatten();
                let state = if instance.leaf {
                    leaf.as_ref().map_or("offline", |leaf| leaf.state.as_str())
                } else {
                    "online"
                };
                UiInstance {
                    ui_id: id.clone(),
                    session_id: instance.session_id.clone(),
                    kind: instance.kind.clone(),
                    since_ms: instance.since.elapsed().as_millis() as u64,
                    state: state.to_owned(),
                    pid: leaf.as_ref().map(|leaf| leaf.pid).or(instance.pid),
                }
            })
            .collect();
        listed.sort_by(|a, b| b.since_ms.cmp(&a.since_ms).then(a.ui_id.cmp(&b.ui_id)));
        UiStatusResult {
            instances: listed,
            web_connections: hub.entry_count(Entry::Web),
        }
    }

    /// Whether this registration belongs to a daemon-started launcher.
    pub fn is_leaf(&self, ui_id: &str) -> bool {
        self.instances
            .lock()
            .expect("ui registry")
            .get(ui_id)
            .is_some_and(|instance| instance.leaf)
    }

    /// Asks the interface `ui_id` to leave: a notification on its
    /// connection. The registration stays until that connection ends, so an
    /// interface that ignores the request is still listed. Use `close` to
    /// stop a launcher process.
    pub fn shutdown(
        &self,
        hub: &ConnectionHub,
        ui_id: &str,
        reason: Option<String>,
    ) -> Result<(), RpcError> {
        let connection = {
            let instances = self.instances.lock().expect("ui registry");
            instances.get(ui_id).and_then(|i| i.connection.clone())
        };
        let Some(connection) = connection else {
            return Err(ui_unknown());
        };
        let Some(connection) = hub.connection(&connection) else {
            return Err(ui_unknown());
        };
        notify_shutdown(&connection, ui_id, reason)
    }

    /// Stops a launcher and removes its registration. A legacy interface has
    /// no process: it is only unregistered, and its connection gets the same
    /// advisory notification as `shutdown`.
    pub async fn close(
        &self,
        hub: &ConnectionHub,
        ui_id: &str,
        reason: Option<String>,
    ) -> Result<(), RpcError> {
        let instance = self
            .instances
            .lock()
            .expect("ui registry")
            .remove(ui_id)
            .ok_or_else(ui_unknown)?;
        let notified = if let Some(id) = instance.connection.as_deref()
            && let Some(connection) = hub.connection(id)
        {
            notify_shutdown(&connection, ui_id, reason).is_ok()
        } else {
            false
        };
        if instance.leaf {
            // Give an attached terminal a chance to restore before killing
            // a launcher that ignored the advisory notification.
            let grace = if notified {
                std::time::Duration::from_secs(2)
            } else {
                std::time::Duration::ZERO
            };
            let _ = self.supervisor.stop(ui_id, grace).await;
        }
        Ok(())
    }

    /// Called as a connection ends: its registrations leave with it. A
    /// managed launcher is stopped with the connection that owned it.
    pub async fn release(&self, connection: &str) {
        let leaves: Vec<String> = {
            let mut instances = self.instances.lock().expect("ui registry");
            let leaves = instances
                .iter()
                .filter(|(_, instance)| {
                    instance.leaf && instance.connection.as_deref() == Some(connection)
                })
                .map(|(id, _)| id.clone())
                .collect();
            instances.retain(|_, instance| instance.connection.as_deref() != Some(connection));
            leaves
        };
        for ui_id in leaves {
            let _ = self
                .supervisor
                .stop(&ui_id, std::time::Duration::ZERO)
                .await;
        }
    }
}

fn notify_shutdown(
    connection: &Connection,
    ui_id: &str,
    reason: Option<String>,
) -> Result<(), RpcError> {
    let notice = Notification::UiShutdown(UiShutdownNotice {
        ui_id: ui_id.to_owned(),
        reason: reason.unwrap_or_else(|| "closed by the App Server".to_owned()),
    });
    connection
        .sender
        .send(Outbound::notice(notice), false)
        .map(|_| ())
        .map_err(|()| ui_unknown())
}

fn validate_kind(kind: &str) -> Result<(), RpcError> {
    if kind.trim().is_empty() || kind.chars().any(char::is_control) {
        return Err(RpcError::invalid("kind must be a printable name"));
    }
    Ok(())
}

fn ui_unknown() -> RpcError {
    RpcError::app(-32000, "ui_unknown", "No such interface is registered")
}
