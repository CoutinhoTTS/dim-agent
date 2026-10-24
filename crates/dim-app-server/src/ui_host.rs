//! The UI registry (Tui design §3.6): which interfaces are alive on which
//! Session. A connection registration is a protocol v3 connection that
//! registered itself; a leaf is an interactive `dimcode` process linked to
//! the daemon, which hosts `Tui` in it (`crate::ui_leaf`). A leaf lives as
//! long as its link, and its state comes from facts that exist: whether its
//! `Tui` is loaded and whether its connection has attached.

use crate::{
    connection::{Connection, ConnectionHub, Entry},
    protocol::{
        Notification, Outbound, UiAttach, UiAttached, UiInstance, UiOpen, UiOpened, UiReplace,
        UiShutdownNotice, UiStatusResult,
    },
    ui_leaf::{LeafSpec, UiLeaves},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use dim_agent_capabilities::methods::RpcError;
use rutis::FiberView;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};
use tokio_util::task::TaskTracker;

const LEAF_KIND: &str = "tui";

struct Instance {
    session_id: String,
    kind: String,
    since: Instant,
    generation: u64,
    /// The protocol v3 connection registered as this interface.
    connection: Option<String>,
    leaf: Option<Leaf>,
    /// A replacement that claimed this registration before its own start,
    /// so two callers observing the same generation cannot both win.
    replace_claim: Option<String>,
}

struct Leaf {
    token: [u8; 32],
    /// `Tui` is loaded; until then the `UiLeaf` is linking or loading.
    loaded: bool,
    /// Its connection has attached: it is no longer `starting`.
    attached: bool,
    /// The new side of a replacement that has not closed the old one yet.
    replacing: bool,
    view: Option<FiberView>,
}

impl Instance {
    fn state(&self) -> &'static str {
        match &self.leaf {
            None => "online",
            // A detach while the leaf runs (`/new`, resume) is not leaving:
            // only its link stopping is, and that removes it.
            Some(leaf) if !(leaf.loaded && leaf.attached) => "starting",
            Some(leaf) if leaf.replacing => "replacing",
            Some(_) => "online",
        }
    }
}

/// Process-wide registry, provided by `UiHostPlugin`.
pub(crate) struct UiHost {
    leaves: UiLeaves,
    instances: Mutex<HashMap<String, Instance>>,
    generations: AtomicU64,
    /// Leaf disposals started outside a caller that awaits them (a link
    /// that stopped); `close_all` waits for them.
    closing: TaskTracker,
    this: Weak<UiHost>,
}

impl UiHost {
    pub fn new(leaves: UiLeaves) -> Arc<Self> {
        Arc::new_cyclic(|this| Self {
            leaves,
            instances: Mutex::new(HashMap::new()),
            generations: AtomicU64::new(0),
            closing: TaskTracker::new(),
            this: this.clone(),
        })
    }

    fn next_generation(&self) -> u64 {
        self.generations.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Links to the leaf node at `address` and hosts `Tui` there. With
    /// `replace`, the old interface is claimed first and closed once the
    /// new leaf is loaded; a failed start leaves it running.
    pub async fn open(
        &self,
        hub: &ConnectionHub,
        connection: &Connection,
        params: UiOpen,
    ) -> Result<UiOpened, RpcError> {
        let UiOpen {
            session_id,
            kind,
            peer,
            address,
            replace,
            expected_generation,
        } = params;
        if !connection.is_attached(&session_id) {
            return Err(not_attached());
        }
        if kind != LEAF_KIND {
            return Err(RpcError::invalid("A UI leaf is a tui"));
        }
        let valid = peer.strip_prefix("ui-").is_some_and(|rest| {
            !rest.is_empty()
                && rest
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        });
        if !valid {
            return Err(RpcError::invalid(
                "peer must be ui- followed by lowercase letters, digits and -",
            ));
        }
        if address.is_empty() {
            return Err(RpcError::invalid("address is required"));
        }
        if expected_generation.is_some() && replace.is_none() {
            return Err(RpcError::invalid("expectedGeneration needs replace"));
        }
        let token = new_token();
        let generation = self.reserve(
            &peer,
            &session_id,
            digest(&token),
            replace.as_deref(),
            expected_generation,
        )?;
        let stopped = {
            let (host, ui_id) = (self.this.clone(), peer.clone());
            Box::new(move || {
                if let Some(host) = host.upgrade() {
                    host.remove(&ui_id);
                }
            })
        };
        let spec = LeafSpec {
            ui_id: peer.clone(),
            address,
            token,
            session_id,
        };
        let view = match self.leaves.start(spec, stopped).await {
            Ok(view) => view,
            Err(error) => {
                self.abandon(&peer, replace.as_deref());
                return Err(error);
            }
        };
        let unclaimed = {
            let mut instances = self.instances.lock().expect("ui registry");
            match instances.get_mut(&peer).and_then(|i| i.leaf.as_mut()) {
                Some(leaf) => {
                    leaf.loaded = true;
                    leaf.view = Some(view);
                    None
                }
                // Closed while it started, or its link already stopped.
                None => Some(view),
            }
        };
        if let Some(view) = unclaimed {
            let _ = view.dispose().await;
            return Err(ui_unknown());
        }
        if let Some(old) = replace {
            // Best effort: the new leaf stands even if the old one is gone.
            let _ = self.close(hub, &old, Some("replaced".to_owned())).await;
            if let Some(leaf) = self
                .instances
                .lock()
                .expect("ui registry")
                .get_mut(&peer)
                .and_then(|i| i.leaf.as_mut())
            {
                leaf.replacing = false;
            }
        }
        Ok(UiOpened {
            ui_id: peer,
            generation,
        })
    }

    /// Inserts the new leaf and, for a replacement, claims the old
    /// registration under the same lock.
    fn reserve(
        &self,
        ui_id: &str,
        session_id: &str,
        token: [u8; 32],
        replace: Option<&str>,
        expected_generation: Option<u64>,
    ) -> Result<u64, RpcError> {
        let mut instances = self.instances.lock().expect("ui registry");
        if instances.contains_key(ui_id) {
            return Err(RpcError::invalid("This UI leaf id is already registered"));
        }
        if let Some(old) = replace {
            let old = instances.get_mut(old).ok_or_else(ui_unknown)?;
            claim(old, ui_id, session_id, expected_generation)?;
        }
        let generation = self.next_generation();
        instances.insert(
            ui_id.to_owned(),
            Instance {
                session_id: session_id.to_owned(),
                kind: LEAF_KIND.to_owned(),
                since: Instant::now(),
                generation,
                connection: None,
                leaf: Some(Leaf {
                    token,
                    loaded: false,
                    attached: false,
                    replacing: replace.is_some(),
                    view: None,
                }),
                replace_claim: None,
            },
        );
        Ok(generation)
    }

    /// Removes a leaf whose start failed and releases its claim.
    fn abandon(&self, ui_id: &str, replace: Option<&str>) {
        let mut instances = self.instances.lock().expect("ui registry");
        instances.remove(ui_id);
        if let Some(old) = replace.and_then(|old| instances.get_mut(old))
            && old.replace_claim.as_deref() == Some(ui_id)
        {
            old.replace_claim = None;
        }
    }

    /// The leaf's link stopped: it is gone, and its `UiLeaf` with it.
    fn remove(&self, ui_id: &str) {
        let view = self
            .instances
            .lock()
            .expect("ui registry")
            .remove(ui_id)
            .and_then(|instance| instance.leaf)
            .and_then(|leaf| leaf.view);
        if let Some(view) = view {
            self.closing.spawn(async move {
                let _ = view.dispose().await;
            });
        }
    }

    /// Registers `connection` as an interface of the Session it is attached
    /// to. A leaf's `Tui` passes its `uiId` and token; an ordinary interface
    /// neither, and registering again returns its id.
    pub fn attach(
        &self,
        connection: &Connection,
        params: &UiAttach,
    ) -> Result<UiAttached, RpcError> {
        let UiAttach {
            session_id,
            kind,
            ui_id,
            token,
        } = params;
        if !connection.is_attached(session_id) {
            return Err(not_attached());
        }
        validate_kind(kind)?;
        let mut instances = self.instances.lock().expect("ui registry");
        match (ui_id, token) {
            (Some(ui_id), Some(token)) => {
                let instance = instances.get_mut(ui_id).ok_or_else(ui_unknown)?;
                if &instance.kind != kind {
                    return Err(RpcError::invalid("This UI leaf is of another kind"));
                }
                let leaf = instance.leaf.as_mut().ok_or_else(ui_unknown)?;
                if digest(token) != leaf.token {
                    return Err(RpcError::app(
                        -32000,
                        "ui_token",
                        "The UI leaf token is not valid",
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
                        "Another connection already is this UI leaf",
                    ));
                }
                leaf.attached = true;
                // `/new` and resume keep the leaf; the registration follows.
                instance.session_id = session_id.clone();
                instance.connection = Some(connection.id.clone());
                Ok(UiAttached {
                    ui_id: ui_id.clone(),
                    generation: instance.generation,
                })
            }
            (None, None) => {
                if let Some((id, instance)) = instances.iter_mut().find(|(_, i)| {
                    i.leaf.is_none()
                        && i.connection.as_deref() == Some(connection.id.as_str())
                        && &i.session_id == session_id
                }) {
                    instance.kind = kind.clone();
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
                        session_id: session_id.clone(),
                        kind: kind.clone(),
                        since: Instant::now(),
                        generation,
                        connection: Some(connection.id.clone()),
                        leaf: None,
                        replace_claim: None,
                    },
                );
                Ok(UiAttached { ui_id, generation })
            }
            _ => Err(RpcError::invalid(
                "uiId and token must be supplied together",
            )),
        }
    }

    /// For connection registrations: the caller registers anew and the old
    /// interface is closed afterwards, under the same claim as `ui.open`.
    pub async fn replace(
        &self,
        hub: &ConnectionHub,
        connection: &Connection,
        params: &UiReplace,
    ) -> Result<UiAttached, RpcError> {
        if !connection.is_attached(&params.session_id) {
            return Err(not_attached());
        }
        validate_kind(&params.kind)?;
        let ui_id = uuid::Uuid::new_v4().to_string();
        let attached = {
            let mut instances = self.instances.lock().expect("ui registry");
            let old = instances.get_mut(&params.ui_id).ok_or_else(ui_unknown)?;
            claim(old, &ui_id, &params.session_id, params.generation)?;
            let generation = self.next_generation();
            instances.insert(
                ui_id.clone(),
                Instance {
                    session_id: params.session_id.clone(),
                    kind: params.kind.clone(),
                    since: Instant::now(),
                    generation,
                    connection: Some(connection.id.clone()),
                    leaf: None,
                    replace_claim: None,
                },
            );
            UiAttached { ui_id, generation }
        };
        let _ = self
            .close(hub, &params.ui_id, Some("replaced".to_owned()))
            .await;
        Ok(attached)
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
                generation: instance.generation,
                state: instance.state(),
            })
            .collect();
        listed.sort_by(|a, b| b.since_ms.cmp(&a.since_ms).then(a.ui_id.cmp(&b.ui_id)));
        UiStatusResult {
            instances: listed,
            web_connections: hub.entry_count(Entry::Web),
        }
    }

    /// The advisory notification on the interface's connection.
    pub fn shutdown(
        &self,
        hub: &ConnectionHub,
        ui_id: &str,
        reason: Option<String>,
    ) -> Result<(), RpcError> {
        let connection = {
            let instances = self.instances.lock().expect("ui registry");
            let instance = instances.get(ui_id).ok_or_else(ui_unknown)?;
            instance.connection.clone().ok_or_else(|| {
                RpcError::app(
                    -32000,
                    "ui_detached",
                    "This UI leaf is between a detach and its reattach",
                )
            })?
        };
        let connection = hub.connection(&connection).ok_or_else(ui_unknown)?;
        notify_shutdown(&connection, ui_id, reason)
    }

    /// Removes a registration. Its connection gets the advisory notice; a
    /// leaf's `UiLeaf` is disposed (unload, then link close).
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
        if let Some(connection) = instance
            .connection
            .as_deref()
            .and_then(|id| hub.connection(id))
        {
            let _ = notify_shutdown(&connection, ui_id, reason);
        }
        if let Some(view) = instance.leaf.and_then(|leaf| leaf.view) {
            let _ = view.dispose().await;
        }
        Ok(())
    }

    /// Closes every leaf and waits for disposals already under way; used as
    /// the daemon disposes `UiHostPlugin`.
    pub async fn close_all(&self) {
        let views: Vec<FiberView> = self
            .instances
            .lock()
            .expect("ui registry")
            .drain()
            .filter_map(|(_, instance)| instance.leaf.and_then(|leaf| leaf.view))
            .collect();
        for view in views {
            self.closing.spawn(async move {
                let _ = view.dispose().await;
            });
        }
        self.closing.close();
        self.closing.wait().await;
    }

    /// `connection` stops viewing `session_id`: its connection registration
    /// leaves; a leaf it is stays registered and reattaches.
    pub fn detach(&self, connection: &str, session_id: &str) {
        self.instances
            .lock()
            .expect("ui registry")
            .retain(|_, instance| {
                if instance.connection.as_deref() != Some(connection)
                    || instance.session_id != session_id
                {
                    return true;
                }
                instance.connection = None;
                instance.leaf.is_some()
            });
    }

    /// `connection` ended: as `detach` for every Session. A leaf lives as
    /// long as its link, not as any connection.
    pub fn release(&self, connection: &str) {
        self.instances
            .lock()
            .expect("ui registry")
            .retain(|_, instance| {
                if instance.connection.as_deref() != Some(connection) {
                    return true;
                }
                instance.connection = None;
                instance.leaf.is_some()
            });
    }
}

/// Claims `old` for the replacement `by`, checking its Session and the
/// generation the caller observed.
fn claim(
    old: &mut Instance,
    by: &str,
    session_id: &str,
    expected_generation: Option<u64>,
) -> Result<(), RpcError> {
    if old.session_id != session_id {
        return Err(RpcError::invalid(
            "The replaced interface belongs to another Session",
        ));
    }
    if old.replace_claim.is_some()
        || expected_generation.is_some_and(|generation| generation != old.generation)
    {
        return Err(RpcError::app(
            -32000,
            "ui_generation",
            "The UI generation is not current",
        ));
    }
    old.replace_claim = Some(by.to_owned());
    Ok(())
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

fn new_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("operating system randomness");
    URL_SAFE_NO_PAD.encode(bytes)
}

fn digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

fn not_attached() -> RpcError {
    RpcError::app(
        -32000,
        "session_not_attached",
        "Attach to the Session before registering an interface on it",
    )
}

fn ui_unknown() -> RpcError {
    RpcError::app(-32000, "ui_unknown", "No such interface is registered")
}
