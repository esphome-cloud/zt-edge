use crate::actor::{ActorError, ActorId};
use std::time::Duration;
use tokio::sync::oneshot;

#[allow(clippy::module_name_repetitions)]
pub struct ActorRef<M: Send + 'static> {
    pub(crate) id: ActorId,
    pub(crate) tx: flume::Sender<M>,
}

impl<M: Send + 'static> Clone for ActorRef<M> {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            tx: self.tx.clone(),
        }
    }
}

impl<M: Send + 'static> ActorRef<M> {
    pub(crate) fn new(id: ActorId, tx: flume::Sender<M>) -> Self {
        Self { id, tx }
    }

    pub fn actor_id(&self) -> &ActorId {
        &self.id
    }

    /// Non-blocking synchronous send.
    #[must_use = "send errors must be handled"]
    pub fn send(&self, msg: M) -> Result<(), ActorError> {
        self.tx.send(msg).map_err(|_| ActorError::Disconnected)
    }

    /// Non-blocking try-send. Returns `Err(ActorError::MailboxFull)` if a bounded mailbox is full,
    /// or `Err(ActorError::Disconnected)` if the actor has stopped.
    #[must_use = "send errors must be handled"]
    pub fn try_send(&self, msg: M) -> Result<(), ActorError> {
        self.tx.try_send(msg).map_err(|e| match e {
            flume::TrySendError::Full(_) => ActorError::MailboxFull,
            flume::TrySendError::Disconnected(_) => ActorError::Disconnected,
        })
    }

    /// Async send — waits if the mailbox is bounded and full.
    #[must_use = "send errors must be handled"]
    pub async fn send_async(&self, msg: M) -> Result<(), ActorError> {
        self.tx
            .send_async(msg)
            .await
            .map_err(|_| ActorError::Disconnected)
    }

    /// Request-response: build a message containing a oneshot reply sender, send it, await reply.
    pub async fn ask<R, F>(&self, f: F) -> Result<R, ActorError>
    where
        F: FnOnce(oneshot::Sender<R>) -> M,
        R: Send + 'static,
    {
        let (reply_tx, reply_rx) = oneshot::channel();
        let msg = f(reply_tx);
        self.tx
            .send_async(msg)
            .await
            .map_err(|_| ActorError::Disconnected)?;
        reply_rx.await.map_err(|_| ActorError::Disconnected)
    }

    /// Like `ask` but with a timeout.
    pub async fn ask_timeout<R, F>(&self, f: F, timeout: Duration) -> Result<R, ActorError>
    where
        F: FnOnce(oneshot::Sender<R>) -> M,
        R: Send + 'static,
    {
        let millis = timeout.as_millis() as u64;
        tokio::time::timeout(timeout, self.ask(f))
            .await
            .map_err(|_| ActorError::AskTimeout { millis })?
    }
}
