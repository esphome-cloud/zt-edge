#![allow(clippy::module_name_repetitions)]

pub mod device_actor;
pub mod device_manager;
pub mod domains;
pub mod entity_actor;
pub mod entity_id;
pub mod entity_state;
pub mod messages;
pub mod registry;
pub mod state_updater;

pub use device_actor::DeviceActor;
pub use device_manager::DeviceManagerActor;
pub use domains::spec::{DomainSpec, DomainSpecError, DomainSpecRegistry, ServiceSpec};
pub use domains::{DomainDef, DomainError, DomainRegistry};
pub use entity_actor::EntityActor;
pub use entity_id::{AreaId, DeviceId, EntityId};
pub use entity_state::{AlarmState, CoverState, EntityState, LockState, MediaPlayerState};
pub use messages::{
    DeviceDescriptor, DeviceManagerMsg, DeviceMsg, EntityCategory, EntityCommand, EntityDescriptor,
    EntityMsg, EntityStateChanged,
};
pub use registry::EntityRegistry;
pub use state_updater::{NullStateUpdater, StateUpdater};
