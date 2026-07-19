use tokio::sync::oneshot;

use crate::target::ServiceTarget;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServiceDescriptor {
    pub domain: String,
    pub service: String,
    pub description: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("unknown service {domain}.{service}")]
    NotFound { domain: String, service: String },
    #[error("no entities matched target")]
    NoTargets,
    #[error("entity error: {0}")]
    EntityError(String),
}

pub enum ServiceMsg {
    Register(ServiceDescriptor),
    List(oneshot::Sender<Vec<ServiceDescriptor>>),
    Call {
        domain: String,
        service: String,
        target: ServiceTarget,
        data: serde_json::Value,
        reply: oneshot::Sender<Result<usize, ServiceError>>,
    },
}
