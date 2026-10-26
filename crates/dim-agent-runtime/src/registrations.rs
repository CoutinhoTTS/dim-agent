//! Contributions registered with their owner
//! ([ADR 006 D4](../../../docs/adr/006-plugin-contract.md#d4-a-service-is-a-name-in-one-group-and-the-catalog-is-the-contract)).
//!
//! An owner such as ToolRegistry keeps one table of the contributions parts
//! register with it. A registration is an effect of its caller: it is
//! withdrawn when the caller unloads.
use rutis::host::ErrorKind;
use rutis::{Ctx, Error};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tokio::sync::watch;

/// One registration: what was registered, under which name.
pub struct Registration<T> {
    pub name: String,
    pub value: T,
    id: u64,
}

struct State<T> {
    next: u64,
    entries: BTreeMap<String, Arc<Registration<T>>>,
}

/// An owner's table of registered contributions, by name.
pub struct Registrations<T> {
    state: Mutex<State<T>>,
    version: watch::Sender<u64>,
}

impl<T> Default for Registrations<T> {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                next: 0,
                entries: BTreeMap::new(),
            }),
            version: watch::channel(0).0,
        }
    }
}

impl<T: Send + Sync + 'static> Registrations<T> {
    /// Registers `value` under `name` on behalf of the part that owns `ctx`,
    /// until that part unloads. A name registered already fails, as a second
    /// provider of a name does.
    pub fn register(
        self: &Arc<Self>,
        ctx: &Ctx,
        name: impl Into<String>,
        value: T,
    ) -> Result<Arc<Registration<T>>, Error> {
        let name = name.into();
        let registration = {
            let mut state = self.state.lock().unwrap();
            if state.entries.contains_key(&name) {
                return Err(rutis::Error::from(ErrorKind::PluginFailed(
                    format!("{name} is registered already").into(),
                )));
            }
            state.next += 1;
            let registration = Arc::new(Registration {
                name: name.clone(),
                value,
                id: state.next,
            });
            state.entries.insert(name, registration.clone());
            registration
        };
        let table = Arc::downgrade(self);
        let (name, id) = (registration.name.clone(), registration.id);
        let registered = ctx.effect(move || async move {
            if let Some(table) = table.upgrade() {
                table.withdraw(&name, id);
            }
            Ok(())
        });
        match registered {
            Ok(_) => {
                self.version.send_modify(|version| *version += 1);
                Ok(registration)
            }
            Err(error) => {
                // The caller is already unloading: nothing of it may stay.
                self.withdraw(&registration.name, registration.id);
                Err(error)
            }
        }
    }

    fn withdraw(&self, name: &str, id: u64) {
        let removed = {
            let mut state = self.state.lock().unwrap();
            match state.entries.get(name) {
                Some(entry) if entry.id == id => state.entries.remove(name),
                _ => None,
            }
        };
        if removed.is_some() {
            self.version.send_modify(|version| *version += 1);
        }
    }

    /// The registration of `name`, if one is registered now.
    pub fn get(&self, name: &str) -> Option<Arc<Registration<T>>> {
        self.state.lock().unwrap().entries.get(name).cloned()
    }

    /// Every registration now, by name.
    pub fn all(&self) -> Vec<Arc<Registration<T>>> {
        self.state
            .lock()
            .unwrap()
            .entries
            .values()
            .cloned()
            .collect()
    }

    /// Changes with every registration and withdrawal.
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.version.subscribe()
    }
}
