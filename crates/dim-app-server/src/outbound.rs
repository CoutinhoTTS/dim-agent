use crate::protocol::Outbound;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

pub(crate) const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
const MAX_QUEUED_FRAMES: usize = 1024;
const MAX_OUTPUT_BYTES: usize = 32 * 1024 * 1024;
const MAX_PROGRESS_FRAMES: usize = 256;
const MAX_PROGRESS_BYTES: usize = 4 * 1024 * 1024;

pub(crate) struct Frame {
    pub bytes: Vec<u8>,
    pub written: Option<oneshot::Sender<()>>,
    // Held through the actual write, not merely until dequeue.
    _slot: OwnedSemaphorePermit,
    _budget: OwnedSemaphorePermit,
}
/// The frames and encoded bytes one class of frames may hold until written.
struct Allowance {
    frames: Arc<Semaphore>,
    bytes: Arc<Semaphore>,
}
impl Allowance {
    fn new(frames: usize, bytes: usize) -> Self {
        Self {
            frames: Arc::new(Semaphore::new(frames)),
            bytes: Arc::new(Semaphore::new(bytes)),
        }
    }
}
/// A connection's single ordered outbound queue. Replies and notices share
/// one allowance; call progress has its own, so it never takes their room.
///
/// ```text
/// send(reply | notice) --> reliable allowance --+--> FIFO queue --> writer
///                          full: close          |
/// offer(progress) -------> progress allowance --+
///                          full: drop the frame
/// ```
pub(crate) struct Sender {
    queue: mpsc::UnboundedSender<Frame>,
    reliable: Allowance,
    progress: Allowance,
    closed: CancellationToken,
}
impl Sender {
    pub fn new(closed: CancellationToken) -> (Self, mpsc::UnboundedReceiver<Frame>) {
        let (queue, receiver) = mpsc::unbounded_channel();
        (
            Self {
                queue,
                reliable: Allowance::new(MAX_QUEUED_FRAMES, MAX_OUTPUT_BYTES),
                progress: Allowance::new(MAX_PROGRESS_FRAMES, MAX_PROGRESS_BYTES),
                closed,
            },
            receiver,
        )
    }
    /// Admission is synchronous: slow consumers close rather than parking
    /// arbitrarily many producers behind a full queue. Durable notices are
    /// never silently dropped while leaving the connection healthy.
    pub fn send(
        &self,
        value: Outbound,
        acknowledge: bool,
    ) -> Result<Option<oneshot::Receiver<()>>, ()> {
        let result = self.enqueue(&self.reliable, value, acknowledge);
        if result.is_err() {
            self.closed.cancel();
        }
        result
    }
    /// Queues call progress (methods contract A.2), which is best effort: a
    /// frame its allowance cannot take, including one over the frame limit,
    /// is dropped and the connection stays open. A dropped frame is never
    /// queued, so it cannot reorder the frames around it.
    pub fn offer(&self, value: Outbound) {
        let _ = self.enqueue(&self.progress, value, false);
    }
    fn enqueue(
        &self,
        allowance: &Allowance,
        value: Outbound,
        acknowledge: bool,
    ) -> Result<Option<oneshot::Receiver<()>>, ()> {
        if self.closed.is_cancelled() {
            return Err(());
        }
        let slot = allowance
            .frames
            .clone()
            .try_acquire_owned()
            .map_err(|_| ())?;
        let permit = allowance
            .bytes
            .clone()
            .try_acquire_many_owned(0)
            .map_err(|_| ())?;
        let mut bytes = LimitedBytes {
            bytes: Vec::new(),
            budget: allowance.bytes.clone(),
            permit,
        };
        serde_json::to_writer(&mut bytes, &value).map_err(|_| ())?;
        let (written, receiver) = if acknowledge {
            let (tx, rx) = oneshot::channel();
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };
        self.queue
            .send(Frame {
                bytes: bytes.bytes,
                written,
                _slot: slot,
                _budget: bytes.permit,
            })
            .map_err(|_| ())?;
        Ok(receiver)
    }
}

struct LimitedBytes {
    bytes: Vec<u8>,
    budget: Arc<Semaphore>,
    permit: OwnedSemaphorePermit,
}
impl std::io::Write for LimitedBytes {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_FRAME_BYTES - self.bytes.len() {
            return Err(std::io::Error::other("outbound frame exceeds 16 MiB"));
        }
        // Charge serialization before allocating, including concurrent writers.
        let permit = self
            .budget
            .clone()
            .try_acquire_many_owned(bytes.len() as u32)
            .map_err(|_| std::io::Error::other("connection output budget exhausted"))?;
        self.permit.merge(permit);
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
