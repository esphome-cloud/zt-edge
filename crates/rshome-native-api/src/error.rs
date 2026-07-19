use thiserror::Error;

#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Error)]
pub enum ApiError {
    #[error("entity not found for key {key:#010x}")]
    EntityNotFound { key: u32 },

    #[error("actor error: {0}")]
    Actor(#[from] rshome_actor::ActorError),

    #[error("service error: {0}")]
    Service(#[from] rshome_svc::ServiceError),

    #[error("protobuf decode error: {0}")]
    Decode(#[from] prost::DecodeError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
