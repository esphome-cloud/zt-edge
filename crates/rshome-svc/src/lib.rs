pub mod builtin;
pub mod dispatch;
pub mod messages;
pub mod registry;
pub mod target;

pub use messages::{ServiceDescriptor, ServiceError, ServiceMsg};
pub use registry::ServiceRegistryActor;
pub use target::ServiceTarget;
