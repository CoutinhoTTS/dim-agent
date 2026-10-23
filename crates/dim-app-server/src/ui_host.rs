//! The UI registry: which interfaces are alive on which Session, and the
//! way to close one. The daemon is the authority over interfaces without
//! being their parent process or touching a terminal: an interface registers
//! the connection it already has, and a shutdown is a notification on it.
//! See `docs/design/plugins/Tui.md` §3.6.

use crate::{
    connection::{Connection, ConnectionHub, Entry},
    protocol::{Notification, Outbound, UiAttached, UiInstance, UiShutdownNotice, UiStatusResult},
};
use dim_agent_capabilities::methods::RpcError;
use std::{collections::HashMap, sync::Mutex, time::Instant};

/// One registered interface: the connection it lives on and the Session it
/// views. A Session has as many of these as connections register; none is
/// exclusive.
struct Instance {
    connection: String,
    session_id: String,
    kind: String,
    since: Instant,
}

/// Process-wide registry; a registration ends with its connection's
/// attachment to the Session, or with the connection itself.
#[derive(Default)]
pub(crate) struct UiHost {
    instances: Mutex<HashMap<String, Instance>>,
}

impl UiHost {
    /// Registers `connection` as an interface of the Session it is attached
    /// to. Registering again returns the same id.
    pub fn attach(
        &self,
        connection: &Connection,
        session_id: &str,
        kind: &str,
    ) -> Result<UiAttached, RpcError> {
        if !connection.is_attached(session_id) {
            return Err(RpcError::app(
                -32000,
                "session_not_attached",
                "Attach to the Session before registering an interface on it",
            ));
        }
        if kind.trim().is_empty() || kind.chars().any(char::is_control) {
            return Err(RpcError::invalid("kind must be a printable name"));
        }
        let mut instances = self.instances.lock().expect("ui registry");
        if let Some((id, instance)) = instances
            .iter_mut()
            .find(|(_, i)| i.connection == connection.id && i.session_id == session_id)
        {
            instance.kind = kind.to_owned();
            return Ok(UiAttached { ui_id: id.clone() });
        }
        let ui_id = uuid::Uuid::new_v4().to_string();
        instances.insert(
            ui_id.clone(),
            Instance {
                connection: connection.id.clone(),
                session_id: session_id.to_owned(),
                kind: kind.to_owned(),
                since: Instant::now(),
            },
        );
        Ok(UiAttached { ui_id })
    }

    pub fn status(&self, hub: &ConnectionHub) -> UiStatusResult {
        let instances = self.instances.lock().expect("ui registry");
        let mut listed: Vec<UiInstance> = instances
            .iter()
            .map(|(id, instance)| UiInstance {
                ui_id: id.clone(),
                session_id: instance.session_id.clone(),
                kind: instance.kind.clone(),
                since_ms: instance.since.elapsed().as_millis() as u64,
            })
            .collect();
        listed.sort_by(|a, b| b.since_ms.cmp(&a.since_ms).then(a.ui_id.cmp(&b.ui_id)));
        UiStatusResult {
            instances: listed,
            web_connections: hub.entry_count(Entry::Web),
        }
    }

    /// Asks the interface `ui_id` to leave: a notification on its
    /// connection. The registration stays until that connection ends, so an
    /// interface that ignores the request is still listed.
    pub fn shutdown(
        &self,
        hub: &ConnectionHub,
        ui_id: &str,
        reason: Option<String>,
    ) -> Result<(), RpcError> {
        let unknown = || RpcError::app(-32000, "ui_unknown", "No such interface is registered");
        let connection = {
            let instances = self.instances.lock().expect("ui registry");
            instances.get(ui_id).map(|i| i.connection.clone())
        }
        .ok_or_else(unknown)?;
        let Some(connection) = hub.connection(&connection) else {
            self.release(&connection);
            return Err(unknown());
        };
        let notice = Notification::UiShutdown(UiShutdownNotice {
            ui_id: ui_id.to_owned(),
            reason: reason.unwrap_or_else(|| "closed by the App Server".to_owned()),
        });
        connection
            .sender
            .send(Outbound::notice(notice), false)
            .map_err(|()| unknown())?;
        Ok(())
    }

    /// Called as `connection` stops viewing `session_id` (detached, or the
    /// Session closed): its registration there leaves with the attachment.
    pub fn detach(&self, connection: &str, session_id: &str) {
        self.instances
            .lock()
            .expect("ui registry")
            .retain(|_, i| i.connection != connection || i.session_id != session_id);
    }

    /// Called as a connection ends: its registrations leave with it.
    pub fn release(&self, connection: &str) {
        self.instances
            .lock()
            .expect("ui registry")
            .retain(|_, instance| instance.connection != connection);
    }
}
