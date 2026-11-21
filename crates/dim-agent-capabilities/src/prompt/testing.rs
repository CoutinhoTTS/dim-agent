//! Hooks that hold a line's live configuration read open and report when it
//! has to wait for the Dispatcher, so a test commits an adoption in a window
//! it chose. Feature `testing` only: a production build has no hook.
use std::sync::Mutex;
use tokio::sync::{mpsc, watch};

/// One line's held read: where it arrived, whether it is waiting for the
/// Dispatcher, and the release that lets it continue.
type Hold = (
    String,
    String,
    mpsc::UnboundedSender<()>,
    mpsc::UnboundedSender<()>,
    watch::Receiver<bool>,
);
static HOLDS: Mutex<Vec<Hold>> = Mutex::new(Vec::new());

pub struct HeldRead {
    session: String,
    branch: String,
    arrived: mpsc::UnboundedReceiver<()>,
    waiting: mpsc::UnboundedReceiver<()>,
    release: watch::Sender<bool>,
}
impl HeldRead {
    /// Resolves when the read has captured its cutoff and stopped.
    pub async fn arrived(&mut self) {
        let _ = self.arrived.recv().await;
    }
    /// Resolves when the read has to wait for the Dispatcher to reach its
    /// cutoff; a read that never waits never resolves.
    pub async fn waiting(&mut self) {
        let _ = self.waiting.recv().await;
    }
    /// Lets the stopped read continue. Later reads of the line pass through.
    pub fn release(&self) {
        self.release.send_replace(true);
    }
}
impl Drop for HeldRead {
    fn drop(&mut self) {
        self.release.send_replace(true);
        let session = std::mem::take(&mut self.session);
        let branch = std::mem::take(&mut self.branch);
        HOLDS
            .lock()
            .unwrap()
            .retain(|(held_session, held_branch, ..)| {
                *held_session != session || *held_branch != branch
            });
    }
}

/// Holds the next live configuration read of `branch_id` in `session_id`.
/// The tests of one process run in parallel, so the key is the line.
pub fn hold_read(session_id: &str, branch_id: &str) -> HeldRead {
    let (arrived, arrivals) = mpsc::unbounded_channel();
    let (waiting, waits) = mpsc::unbounded_channel();
    let (release, released) = watch::channel(false);
    let mut holds = HOLDS.lock().unwrap();
    holds.retain(|(session, branch, ..)| session != session_id || branch != branch_id);
    holds.push((
        session_id.to_owned(),
        branch_id.to_owned(),
        arrived,
        waiting,
        released,
    ));
    HeldRead {
        session: session_id.to_owned(),
        branch: branch_id.to_owned(),
        arrived: arrivals,
        waiting: waits,
        release,
    }
}

fn armed(
    session_id: &str,
    branch_id: &str,
) -> Option<(
    mpsc::UnboundedSender<()>,
    mpsc::UnboundedSender<()>,
    watch::Receiver<bool>,
)> {
    let holds = HOLDS.lock().unwrap();
    let (_, _, arrived, waiting, release) = holds
        .iter()
        .find(|(session, branch, ..)| session == session_id && branch == branch_id)?;
    Some((arrived.clone(), waiting.clone(), release.clone()))
}

/// Stops the read until the test releases it; the cutoff is already captured.
pub(super) async fn read_captured(session_id: &str, branch_id: &str) {
    let Some((arrived, _, mut release)) = armed(session_id, branch_id) else {
        return;
    };
    let _ = arrived.send(());
    while !*release.borrow_and_update() {
        if release.changed().await.is_err() {
            return;
        }
    }
}

/// Reports that the read cannot answer at its cutoff before the Dispatcher
/// reaches it.
pub(super) fn read_waiting(session_id: &str, branch_id: &str) {
    if let Some((_, waiting, _)) = armed(session_id, branch_id) {
        let _ = waiting.send(());
    }
}
