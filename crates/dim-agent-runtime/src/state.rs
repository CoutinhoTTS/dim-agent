use tokio::sync::watch;

/// A capability owns one state cell; readers get snapshots or a watch receiver.
#[derive(Clone)]
pub struct StateCell<T: Clone> {
    sender: watch::Sender<T>,
}
impl<T: Clone> StateCell<T> {
    pub fn new(value: T) -> Self {
        Self {
            sender: watch::channel(value).0,
        }
    }
    pub fn get(&self) -> T {
        self.sender.borrow().clone()
    }
    pub fn set(&self, value: T) {
        self.sender.send_replace(value);
    }
    pub fn update(&self, change: impl FnOnce(&mut T)) {
        self.sender.send_modify(change);
    }
    pub fn subscribe(&self) -> watch::Receiver<T> {
        self.sender.subscribe()
    }
}
