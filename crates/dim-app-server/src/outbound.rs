use crate::protocol::Outbound;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

pub(crate) const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
const MAX_QUEUED_FRAMES: usize = 1024;
const MAX_OUTPUT_BYTES: usize = 32 * 1024 * 1024;

pub(crate) struct Frame {
    pub bytes: Vec<u8>,
    pub written: Option<oneshot::Sender<()>>,
    // Held through the actual write, not merely until dequeue.
    _budget: tokio::sync::OwnedSemaphorePermit,
}
pub(crate) struct Sender {
    queue: mpsc::Sender<Frame>,
    budget: Arc<tokio::sync::Semaphore>,
    closed: CancellationToken,
}
impl Sender {
    pub fn new(closed: CancellationToken) -> (Self, mpsc::Receiver<Frame>) {
        let (queue, receiver) = mpsc::channel(MAX_QUEUED_FRAMES);
        (
            Self {
                queue,
                budget: Arc::new(tokio::sync::Semaphore::new(MAX_OUTPUT_BYTES)),
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
        let result = self.try_send(value, acknowledge);
        if result.is_err() {
            self.closed.cancel();
        }
        result
    }
    fn try_send(
        &self,
        value: Outbound,
        acknowledge: bool,
    ) -> Result<Option<oneshot::Receiver<()>>, ()> {
        if self.closed.is_cancelled() {
            return Err(());
        }
        let slot = self.queue.try_reserve().map_err(|_| ())?;
        let permit = self
            .budget
            .clone()
            .try_acquire_many_owned(0)
            .map_err(|_| ())?;
        let mut bytes = LimitedBytes {
            bytes: Vec::new(),
            budget: self.budget.clone(),
            permit,
        };
        serde_json::to_writer(&mut bytes, &value).map_err(|_| ())?;
        let (written, receiver) = if acknowledge {
            let (tx, rx) = oneshot::channel();
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };
        slot.send(Frame {
            bytes: bytes.bytes,
            written,
            _budget: bytes.permit,
        });
        Ok(receiver)
    }
}

struct LimitedBytes {
    bytes: Vec<u8>,
    budget: Arc<tokio::sync::Semaphore>,
    permit: tokio::sync::OwnedSemaphorePermit,
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
