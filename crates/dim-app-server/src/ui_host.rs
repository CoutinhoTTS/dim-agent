//! The UI registry: which interfaces are alive on which Session, and the
//! way to close one. A daemon-started interface is a `UiLeaf` owned by
//! `UiLeafSupervisor`: the leaf process hosts a rutis plugin, its TUI keeps
//! the protocol v3 business connection, and the registry combines
//! `leaf.ready && rpc.attached` into `online`.
//!
//! `ui.open` starts a leaf, `ui.close` stops it, and `ui.replace` advances
//! a generation. A legacy connection registration still works and has no
//! process. See `docs/design/plugins/Tui.md` §3.6.

use crate::{
    connection::{Connection, ConnectionHub, Entry},
    protocol::{
        Notification, Outbound, UiAttach, UiAttached, UiInstance, UiOpen, UiOpened, UiReplace,
        UiShutdownNotice, UiStatusResult,
    },
    ui_leaf::UiLeafSupervisor,
};
use dim_agent_capabilities::methods::RpcError;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

struct Instance {
    connection: Option<String>,
    session_id: String,
    kind: String,
    since: Instant,
    generation: u64,
    leaf: bool,
    pid: Option<u32>,
    /// A replacement launch that has claimed this registration. The claim
    /// is taken before the asynchronous start so two callers observing the
    /// same old generation cannot both win.
    replace_claim: Option<String>,
}

/// Process-wide registry; a registration ends with its connection's
/// attachment to the Session, or with the connection itself. A leaf
/// registration stays registered when its RPC detaches, but is no longer
/// online until it reattaches; `ui.close` stops its process.
pub(crate) struct UiHost {
    supervisor: Arc<UiLeafSupervisor>,
    instances: Mutex<HashMap<String, Instance>>,
    generations: AtomicU64,
}

impl UiHost {
    pub fn new(supervisor: Arc<UiLeafSupervisor>) -> Self {
        Self {
            supervisor,
            instances: Mutex::new(HashMap::new()),
            generations: AtomicU64::new(0),
        }
    }

    fn next_generation(&self) -> u64 {
        self.generations.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Reserves the new registration and, for a replacement, atomically
    /// claims the old generation. The claim is taken before the caller
    /// awaits the launch, so concurrent replacements cannot both pass the
    /// observed-generation check.
    fn reserve_open(
        &self,
        ui_id: &str,
        session_id: &str,
        kind: &str,
        replace: Option<&str>,
        expected_generation: Option<u64>,
    ) -> Result<u64, RpcError> {
        let mut instances = self.instances.lock().expect("ui registry");
        let old_generation = if let Some(old) = replace {
            let old_instance = instances.get_mut(old).ok_or_else(ui_unknown)?;
            if old_instance.session_id != session_id {
                return Err(RpcError::invalid(
                    "The replaced interface belongs to another Session",
                ));
            }
            if old_instance.replace_claim.is_some() {
                return Err(RpcError::app(
                    -32000,
                    "ui_generation",
                    "Another replacement already claimed this UI generation",
                ));
            }
            if expected_generation.is_some_and(|generation| generation != old_instance.generation) {
                return Err(RpcError::app(
                    -32000,
                    "ui_generation",
                    "The UI generation is not current",
                ));
            }
            let generation = old_instance.generation;
            old_instance.replace_claim = Some(ui_id.to_owned());
            Some(generation)
        } else {
            None
        };
        let generation = old_generation.map_or_else(
            || self.next_generation(),
            |old| old.saturating_add(1).max(self.next_generation()),
        );
        instances.insert(
            ui_id.to_owned(),
            Instance {
                connection: None,
                session_id: session_id.to_owned(),
                kind: kind.to_owned(),
                since: Instant::now(),
                generation,
                leaf: true,
                pid: None,
                replace_claim: None,
            },
        );
        Ok(generation)
    }

    /// Removes a provisional registration after its launch failed, and
    /// releases the old registration's claim so the old UI can be replaced
    /// again.
    fn abandon_open(&self, ui_id: &str, replace: Option<&str>) {
        let mut instances = self.instances.lock().expect("ui registry");
        instances.remove(ui_id);
        if let Some(old) = replace
            && let Some(old_instance) = instances.get_mut(old)
            && old_instance.replace_claim.as_deref() == Some(ui_id)
        {
            old_instance.replace_claim = None;
        }
    }

    /// Starts a launcher for `session_id`. `replace` names the previous
    /// leaf; the new one is started and loaded first, then the old one is
    /// stopped. A failed start leaves the old registration untouched.
    pub async fn open(
        &self,
        hub: &ConnectionHub,
        connection: &Connection,
        params: &UiOpen,
    ) -> Result<UiOpened, RpcError> {
        let UiOpen {
            session_id,
            kind,
            tty,
            cwd,
            home,
            replace,
            expected_generation,
        } = params;
        let (session_id, kind) = (session_id.as_str(), kind.as_str());
        if !connection.is_attached(session_id) {
            return Err(RpcError::app(
                -32000,
                "session_not_attached",
                "Attach to the Session before starting an interface on it",
            ));
        }
        validate_kind(kind)?;
        if kind != "tui" {
            return Err(RpcError::invalid("Managed UI leaves are tui"));
        }
        if kind == "tui"
            && tty.as_deref().is_none_or(|tty| {
                tty.is_empty() || (tty != "console" && !std::path::Path::new(tty).is_absolute())
            })
        {
            return Err(RpcError::app(
                -32000,
                "ui_tty_required",
                "ui.open of a tui needs an absolute terminal path in tty",
            ));
        }
        let ui_id = self.supervisor.new_ui_id();
        let generation = self.reserve_open(
            &ui_id,
            session_id,
            kind,
            replace.as_deref(),
            *expected_generation,
        )?;
        let started = match self
            .supervisor
            .start(
                &ui_id,
                session_id,
                kind,
                tty.as_deref(),
                cwd.as_deref(),
                home.as_deref(),
                generation,
                replace.clone(),
            )
            .await
        {
            Ok(started) => started,
            Err(error) => {
                self.abandon_open(&ui_id, replace.as_deref());
                return Err(error);
            }
        };
        if let Some(old) = replace {
            // The new leaf is ready. Closing the old one is best effort: the
            // new registration stands even if the old process is already
            // gone, and a failure here is never reported as a rollback.
            let _ = self.close(hub, old, Some("replaced".to_owned())).await;
            self.supervisor.replaced(&ui_id);
        }
        if let Some(instance) = self.instances.lock().expect("ui registry").get_mut(&ui_id) {
            instance.pid = Some(started.pid);
        }
        Ok(UiOpened {
            ui_id,
            pid: started.pid,
            generation,
        })
    }

    /// Registers `connection` as an interface of the Session it is attached
    /// to. Registering again returns the same id. A managed launcher passes
    /// its `ui_id`, token and generation; a legacy interface passes none.
    pub fn attach(
        &self,
        connection: &Connection,
        params: &UiAttach,
    ) -> Result<UiAttached, RpcError> {
        self.attach_inner(connection, params, false)
    }

    fn attach_inner(
        &self,
        connection: &Connection,
        params: &UiAttach,
        force_new: bool,
    ) -> Result<UiAttached, RpcError> {
        let UiAttach {
            session_id,
            kind,
            ui_id,
            token,
            generation,
        } = params;
        let (session_id, kind) = (session_id.as_str(), kind.as_str());
        let (ui_id, token, generation) = (ui_id.as_deref(), token.as_deref(), *generation);
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
            let mut instances = self.instances.lock().expect("ui registry");
            let instance = instances.get(ui_id).ok_or_else(ui_unknown)?;
            if instance.kind != kind {
                return Err(RpcError::invalid(
                    "The launcher is registered for another kind",
                ));
            }
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
            // Keep the registry lock across the leaf attach so two
            // connections cannot both claim the same launcher.
            let (leaf_session, leaf_kind, leaf_generation) =
                self.supervisor.attach(ui_id, token, generation)?;
            let instance = instances.get_mut(ui_id).ok_or_else(ui_unknown)?;
            if leaf_kind != kind {
                return Err(RpcError::invalid(
                    "The launcher is registered for another kind",
                ));
            }
            if leaf_session != session_id {
                // `/new` and resume keep the same leaf identity; the
                // registration follows the Session the TUI attached to.
                self.supervisor.reattach(ui_id, session_id, kind)?;
            }
            instance.session_id = session_id.to_owned();
            instance.connection = Some(connection.id.clone());
            instance.kind = kind.to_owned();
            instance.generation = leaf_generation;
            instance.pid = self.supervisor.snapshot(ui_id).map(|leaf| leaf.pid);
            return Ok(UiAttached {
                ui_id: ui_id.to_owned(),
                generation: leaf_generation,
            });
        }
        if token.is_some() {
            return Err(RpcError::invalid(
                "uiId and token must be supplied together",
            ));
        }
        if generation.is_some() {
            return Err(RpcError::invalid(
                "A generation belongs to a managed launcher",
            ));
        }
        let mut instances = self.instances.lock().expect("ui registry");
        if !force_new
            && let Some((id, instance)) = instances.iter_mut().find(|(_, i)| {
                i.connection.as_deref() == Some(connection.id.as_str())
                    && i.session_id == session_id
            })
        {
            instance.kind = kind.to_owned();
            return Ok(UiAttached {
                ui_id: id.clone(),
                generation: instance.generation,
            });
        }
        let ui_id = uuid::Uuid::new_v4().to_string();
        let generation = self.next_generation();
        instances.insert(
            ui_id.clone(),
            Instance {
                connection: Some(connection.id.clone()),
                session_id: session_id.to_owned(),
                kind: kind.to_owned(),
                since: Instant::now(),
                generation,
                leaf: false,
                pid: None,
                replace_claim: None,
            },
        );
        Ok(UiAttached { ui_id, generation })
    }

    /// Registers the caller as a new generation and then closes the old
    /// interface. A failed registration leaves the old interface untouched.
    pub async fn replace(
        &self,
        hub: &ConnectionHub,
        connection: &Connection,
        params: &UiReplace,
    ) -> Result<UiAttached, RpcError> {
        let (old_generation, old_session) = {
            let mut instances = self.instances.lock().expect("ui registry");
            let instance = instances.get_mut(&params.ui_id).ok_or_else(ui_unknown)?;
            if instance.replace_claim.is_some() {
                return Err(RpcError::app(
                    -32000,
                    "ui_generation",
                    "Another replacement already claimed this UI generation",
                ));
            }
            let generation = instance.generation;
            let session = instance.session_id.clone();
            instance.replace_claim = Some("ui.replace".to_owned());
            (generation, session)
        };
        if old_session != params.session_id {
            self.clear_replace_claim(&params.ui_id);
            return Err(RpcError::invalid(
                "The replaced interface belongs to another Session",
            ));
        }
        if params
            .generation
            .is_some_and(|generation| generation != old_generation)
        {
            self.clear_replace_claim(&params.ui_id);
            return Err(RpcError::app(
                -32000,
                "ui_generation",
                "The UI generation is not current",
            ));
        }
        self.generations
            .fetch_max(old_generation.saturating_add(1), Ordering::SeqCst);
        let attached = self.attach_inner(
            connection,
            &UiAttach {
                session_id: params.session_id.clone(),
                kind: params.kind.clone(),
                ui_id: None,
                token: None,
                generation: None,
            },
            true,
        );
        let attached = match attached {
            Ok(attached) => attached,
            Err(error) => {
                self.clear_replace_claim(&params.ui_id);
                return Err(error);
            }
        };
        let _ = self
            .close(hub, &params.ui_id, Some("replaced".to_owned()))
            .await;
        self.clear_replace_claim(&params.ui_id);
        Ok(attached)
    }

    fn clear_replace_claim(&self, ui_id: &str) {
        if let Some(instance) = self.instances.lock().expect("ui registry").get_mut(ui_id) {
            instance.replace_claim = None;
        }
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
                let (state, pid, exit_code) = match (&leaf, instance.leaf) {
                    (Some(leaf), _) => (leaf.state.as_str(), leaf.pid, leaf.exit_code),
                    (None, true) => ("offline", instance.pid.unwrap_or(0), None),
                    (None, false) => ("online", 0, None),
                };
                UiInstance {
                    ui_id: id.clone(),
                    session_id: instance.session_id.clone(),
                    kind: instance.kind.clone(),
                    since_ms: instance.since.elapsed().as_millis() as u64,
                    generation: leaf
                        .as_ref()
                        .map_or(instance.generation, |leaf| leaf.generation),
                    state: state.to_owned(),
                    pid: (pid != 0).then_some(pid).or(instance.pid),
                    exit_code,
                }
            })
            .collect();
        listed.sort_by(|a, b| b.since_ms.cmp(&a.since_ms).then(a.ui_id.cmp(&b.ui_id)));
        UiStatusResult {
            instances: listed,
            web_connections: hub.entry_count(Entry::Web),
        }
    }

    /// Asks the interface `ui_id` to leave: a notification on its
    /// connection. A legacy registration stays until that connection ends;
    /// a managed leaf is stopped by `close`.
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
        let Some(connection_id) = connection else {
            return Err(ui_unknown());
        };
        let Some(connection) = hub.connection(&connection_id) else {
            self.release(&connection_id);
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
        if let Some(id) = instance.connection.as_deref()
            && let Some(connection) = hub.connection(id)
        {
            let _ = notify_shutdown(&connection, ui_id, reason);
        }
        if instance.leaf {
            let _ = self.supervisor.stop(ui_id).await;
        }
        Ok(())
    }

    /// Asks every registered TUI to leave before daemon cleanup removes its
    /// launcher. A disconnected leaf is already offline and has no one to
    /// address; its launcher is stopped by the supervisor.
    pub fn shutdown_all(&self, hub: &ConnectionHub) {
        let entries: Vec<(String, Option<String>)> = self
            .instances
            .lock()
            .expect("ui registry")
            .iter()
            .map(|(ui_id, instance)| (ui_id.clone(), instance.connection.clone()))
            .collect();
        for (ui_id, connection) in entries {
            if let Some(connection) = connection
                && let Some(connection) = hub.connection(&connection)
            {
                let _ = notify_shutdown(&connection, &ui_id, Some("daemon shutdown".to_owned()));
            }
        }
    }

    /// Called as `connection` stops viewing `session_id` (detached, or the
    /// Session closed): its legacy registration leaves with the attachment.
    /// A managed leaf stays registered but detaches from the TUI so it can
    /// reattach without losing the daemon-owned process.
    pub fn detach(&self, connection: &str, session_id: &str) {
        self.detach_where(connection, Some(session_id));
    }

    /// Called as a connection ends: its legacy registrations leave with it,
    /// while managed leaves stay registered and go offline until reattached.
    pub fn release(&self, connection: &str) {
        self.detach_where(connection, None);
    }

    fn detach_where(&self, connection: &str, session_id: Option<&str>) {
        let mut leaves = Vec::new();
        self.instances
            .lock()
            .expect("ui registry")
            .retain(|ui_id, instance| {
                if instance.connection.as_deref() != Some(connection) {
                    return true;
                }
                if session_id.is_some_and(|session_id| instance.session_id != session_id) {
                    return true;
                }
                if instance.leaf {
                    instance.connection = None;
                    instance.pid = None;
                    leaves.push(ui_id.clone());
                    true
                } else {
                    false
                }
            });
        for ui_id in leaves {
            self.supervisor.detach(&ui_id);
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
