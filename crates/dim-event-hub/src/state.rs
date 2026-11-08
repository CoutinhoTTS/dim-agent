//! The install's state (design §2.1, §2.4, §2.5): what outlives one
//! generation of the `eventhub` row. `install_with` creates one instance and
//! the row builder captures it, so a factory rebuild — a reapply after a
//! dependency reload, or an off and on of the row — keeps every declaration,
//! and an in-flight change of one generation still serializes with the next
//! generation's. The state ends with the process.
use crate::catalog::Catalog;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::{OwnedMutexGuard, OwnedSemaphorePermit, Semaphore};

/// How many deliveries run at a time across every publish of the install
/// (design §2.5).
pub(crate) const MAX_DELIVERIES_IN_FLIGHT: usize = 8;

/// The catalog, the Session locks, the event locks and the delivery budget,
/// shared by every generation of the process part (design §2.1).
pub struct InstanceState {
    catalog: Arc<Catalog>,
    /// One lock per target Session (design §3.3).
    sessions: KeyedLocks,
    /// One lock per submission id (design §2.4).
    events: KeyedLocks,
    /// The install's delivery budget (design §2.5), not one per publish.
    budget: Arc<Semaphore>,
}

impl Default for InstanceState {
    fn default() -> Self {
        Self {
            catalog: Arc::new(Catalog::default()),
            sessions: KeyedLocks::default(),
            events: KeyedLocks::default(),
            budget: Arc::new(Semaphore::new(MAX_DELIVERIES_IN_FLIGHT)),
        }
    }
}

impl InstanceState {
    pub(crate) fn catalog(&self) -> &Arc<Catalog> {
        &self.catalog
    }

    /// Waits for the lock of `session_id` and holds it until the guard drops
    /// (design §3.3).
    pub(crate) async fn session_lock(&self, session_id: &str) -> KeyedLock {
        self.sessions.lock(session_id).await
    }

    /// Waits for the lock of one event's submission id (design §2.4).
    pub(crate) async fn event_lock(&self, submission_id: &str) -> KeyedLock {
        self.events.lock(submission_id).await
    }

    /// Takes one of the install's delivery permits (design §2.5). A permit is
    /// held for one target branch's delivery, guide refresh included.
    pub(crate) async fn delivery_permit(&self) -> OwnedSemaphorePermit {
        self.budget
            .clone()
            .acquire_owned()
            .await
            .expect("the install's delivery budget is never closed")
    }
}

/// Locks named by a key, each created on first use and dropped when its last
/// holder releases it (design §2.5), so a long-lived install keeps no map
/// entry per Session or event it ever saw.
#[derive(Default)]
pub(crate) struct KeyedLocks {
    inner: Arc<KeyedLocksInner>,
}

#[derive(Default)]
struct KeyedLocksInner {
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl KeyedLocks {
    async fn lock(&self, key: &str) -> KeyedLock {
        let lock = {
            let mut locks = self.inner.locks.lock().expect("the lock map");
            locks.entry(key.to_owned()).or_default().clone()
        };
        let guard = lock.clone().lock_owned().await;
        KeyedLock {
            inner: self.inner.clone(),
            key: key.to_owned(),
            lock: Some(lock),
            guard: Some(guard),
        }
    }
}

/// One keyed lock held; dropping it drops the map's entry when this was the
/// last holder.
pub(crate) struct KeyedLock {
    inner: Arc<KeyedLocksInner>,
    key: String,
    lock: Option<Arc<tokio::sync::Mutex<()>>>,
    guard: Option<OwnedMutexGuard<()>>,
}

impl Drop for KeyedLock {
    fn drop(&mut self) {
        self.guard.take();
        let lock = self
            .lock
            .take()
            .expect("a keyed lock holds its entry until it drops");
        let mut locks = self.inner.locks.lock().expect("the lock map");
        // This guard and the map hold the entry; a waiter, which cloned it
        // before awaiting, would hold a third reference.
        if Arc::strong_count(&lock) == 2 {
            locks.remove(&self.key);
        }
    }
}
