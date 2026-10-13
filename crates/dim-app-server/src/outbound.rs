use crate::protocol::Outbound;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

pub(crate) struct Frame {
    pub bytes: Vec<u8>,
    pub written: Option<oneshot::Sender<()>>,
}
pub(crate) struct Sender {
    queue: mpsc::UnboundedSender<Frame>,
    closed: CancellationToken,
}
impl Sender {
    pub fn new(closed: CancellationToken) -> (Self, mpsc::UnboundedReceiver<Frame>) {
        let (queue, receiver) = mpsc::unbounded_channel();
        (Self { queue, closed }, receiver)
    }
    /// Encodes and queues one frame; a closed connection or an unencodable
    /// frame closes the connection.
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
        let bytes = serde_json::to_vec(&value).map_err(|_| ())?;
        let (written, receiver) = if acknowledge {
            let (tx, rx) = oneshot::channel();
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };
        self.queue.send(Frame { bytes, written }).map_err(|_| ())?;
        Ok(receiver)
    }
}
