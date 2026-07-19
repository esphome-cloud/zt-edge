use crate::context::ActorContext;
use uuid::Uuid;

#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ActorId(Uuid);

impl ActorId {
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for ActorId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ActorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[allow(clippy::module_name_repetitions)]
#[derive(thiserror::Error, Debug, Clone)]
pub enum ActorError {
    #[error("actor channel disconnected")]
    Disconnected,
    #[error("ask timed out after {millis}ms")]
    AskTimeout { millis: u64 },
    #[error("actor panicked: {0}")]
    Panicked(String),
    #[error("restart limit exceeded after {count} restarts")]
    RestartLimitExceeded { count: u32 },
    #[error("actor already stopped")]
    AlreadyStopped,
    #[error("mailbox full")]
    MailboxFull,
    #[error("internal error: {0}")]
    Internal(String),
}

#[async_trait::async_trait]
pub trait Actor: Send + 'static {
    type Msg: Send + 'static;

    async fn handle(&mut self, msg: Self::Msg, ctx: &mut ActorContext<Self::Msg>);

    async fn pre_start(&mut self, _ctx: &mut ActorContext<Self::Msg>) {}
    async fn post_stop(&mut self) {}
    async fn pre_restart(&mut self, _err: &ActorError) {}
}
