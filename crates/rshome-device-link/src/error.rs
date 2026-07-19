use rshome_actor::ActorError;

#[derive(Debug, thiserror::Error)]
pub enum DeviceLinkError {
    #[error("TCP connection failed: {0}")]
    Connect(#[from] std::io::Error),
    #[error("protobuf decode error: {0}")]
    Decode(#[from] prost::DecodeError),
    #[error("actor error: {0}")]
    Actor(#[from] ActorError),
    #[error("mDNS error: {0}")]
    Mdns(String),
    /// The device session is not active (disconnected, in backoff, or not found).
    #[error("device session not active")]
    SessionNotActive,
    /// The entity ID is not registered under this session.
    #[error("entity not found: {0}")]
    EntityNotFound(String),
    /// The entity does not support outbound commands (read-only entity).
    #[error("command not supported for entity: {0}")]
    CommandNotSupported(String),
    /// An inbound frame could not be decoded (malformed wire data).
    #[error("inbound frame error: {0}")]
    InboundFrameError(String),
    /// Device exports more entities than the hard cap allows.
    #[error("entity cap exceeded: device exports {0} entities (hard cap {1})")]
    HardEntityCapExceeded(usize, usize),
}

impl From<mdns_sd::Error> for DeviceLinkError {
    fn from(e: mdns_sd::Error) -> Self {
        Self::Mdns(e.to_string())
    }
}
