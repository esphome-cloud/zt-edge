use crate::{AreaId, DeviceId, EntityId, EntityState};
use tokio::sync::oneshot;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EntityCategory {
    None,
    Config,
    Diagnostic,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EntityDescriptor {
    pub entity_id: EntityId,
    pub name: String,
    pub icon: Option<String>,
    pub device_id: Option<DeviceId>,
    pub area_id: Option<AreaId>,
    pub entity_category: EntityCategory,
    /// ESPHome domain (e.g. `"sensor"`, `"switch"`); empty string means unknown.
    pub domain_id: String,
    /// Feature capabilities resolved from `DomainRegistry` (e.g. `["state", "toggle"]`).
    pub feature_set: Vec<String>,
    /// Optional HA device class (e.g. `"temperature"`, `"motion"`).
    pub device_class: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EntityStateChanged {
    pub entity_id: EntityId,
    pub old_state: EntityState,
    pub new_state: EntityState,
    pub changed_at: std::time::SystemTime,
}

#[allow(clippy::module_name_repetitions)]
pub enum EntityMsg {
    GetState(oneshot::Sender<EntityState>),
    SetState(EntityState),
    Command(EntityCommand),
    Stop,
}

#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Clone)]
pub enum EntityCommand {
    TurnOn,
    TurnOff,
    Toggle,
    SetValue(f64),
    SetOption(String),
    SetText(String),
    SetLightBrightness(f64),
    SetLightColor {
        rgb: Option<[u8; 3]>,
        color_temp: Option<u16>,
    },
    SetClimateMode(String),
    SetClimateTemp(f64),
    SetFanSpeed(u8),
    SetCoverPosition(u8),
    PressButton,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeviceDescriptor {
    pub device_id: DeviceId,
    pub name: String,
    pub model: Option<String>,
    pub manufacturer: Option<String>,
    pub sw_version: Option<String>,
    pub area_id: Option<AreaId>,
}

#[allow(clippy::large_enum_variant)]
pub enum DeviceMsg {
    GetInfo(oneshot::Sender<DeviceDescriptor>),
    UpdateInfo(DeviceDescriptor),
    GetEntities(oneshot::Sender<Vec<EntityId>>),
    AddEntity {
        descriptor: EntityDescriptor,
        initial_state: EntityState,
        reply: oneshot::Sender<rshome_actor::ActorRef<EntityMsg>>,
    },
    AttachEntity {
        descriptor: EntityDescriptor,
        entity_ref: rshome_actor::ActorRef<EntityMsg>,
    },
    RemoveEntity(EntityId),
    Stop,
}

pub enum DeviceManagerMsg {
    AddDevice {
        descriptor: DeviceDescriptor,
        reply: oneshot::Sender<rshome_actor::ActorRef<DeviceMsg>>,
    },
    RemoveDevice(DeviceId),
    GetDevice {
        id: DeviceId,
        reply: oneshot::Sender<Option<rshome_actor::ActorRef<DeviceMsg>>>,
    },
    ListDevices(oneshot::Sender<Vec<DeviceDescriptor>>),
    GetEntitiesForDevice {
        device_id: DeviceId,
        reply: oneshot::Sender<Vec<EntityId>>,
    },
    Stop,
}
