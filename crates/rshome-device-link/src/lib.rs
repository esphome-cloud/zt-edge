#![allow(clippy::module_name_repetitions)]

pub mod bindings;
pub mod commands;
pub mod discovery;
pub mod error;
pub mod ingest;
pub mod manager;
pub mod noise_transport;
pub mod security;
pub mod session;

pub use bindings::{ImportedCommandCapabilities, ImportedEntityBinding};
pub use commands::state_to_command;
pub use discovery::{
    device_id_from_hostname, device_id_from_mac, device_slug, DiscoveredDevice, MdnsBrowser,
};
pub use error::DeviceLinkError;
pub use ingest::{parse_state_frame, IngestedState};
pub use manager::{
    ConnectedDevice, DeviceLinkLimits, DeviceLinkManagerActor, DeviceLinkManagerMsg,
    DeviceLinkStatus, ResourceUsage, SessionStatus,
};
pub use security::{DeviceSecurityConfig, DiscoveryRecordState, SessionError};
pub use session::{DeviceSessionActor, DeviceSessionMsg, MAX_ENTITIES_PER_DEVICE};
