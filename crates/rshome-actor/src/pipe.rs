use crate::actor::ActorError;

pub struct CrossbeamPipe<T: Send + 'static> {
    tx: crossbeam_channel::Sender<T>,
    rx: crossbeam_channel::Receiver<T>,
}

impl<T: Send + 'static> CrossbeamPipe<T> {
    pub fn bounded(cap: usize) -> Self {
        let (tx, rx) = crossbeam_channel::bounded(cap);
        Self { tx, rx }
    }

    pub fn unbounded() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self { tx, rx }
    }

    /// Blocking send (panics if disconnected — use in sync hot-path).
    pub fn send(&self, val: T) -> Result<(), ActorError> {
        self.tx.send(val).map_err(|_| ActorError::Disconnected)
    }

    /// Non-blocking send — fails if bounded channel is full.
    pub fn try_send(&self, val: T) -> Result<(), crossbeam_channel::TrySendError<T>> {
        self.tx.try_send(val)
    }

    /// Blocking receive.
    pub fn recv(&self) -> Result<T, ActorError> {
        self.rx.recv().map_err(|_| ActorError::Disconnected)
    }

    /// Non-blocking receive.
    pub fn try_recv(&self) -> Result<T, crossbeam_channel::TryRecvError> {
        self.rx.try_recv()
    }

    /// Async receive — offloads blocking call to `spawn_blocking`.
    pub async fn recv_async(&self) -> Result<T, ActorError> {
        let rx = self.rx.clone();
        tokio::task::spawn_blocking(move || rx.recv().map_err(|_| ActorError::Disconnected))
            .await
            .map_err(|_| ActorError::Internal("spawn_blocking join error".into()))?
    }

    pub fn sender(&self) -> crossbeam_channel::Sender<T> {
        self.tx.clone()
    }

    pub fn receiver(&self) -> crossbeam_channel::Receiver<T> {
        self.rx.clone()
    }
}
