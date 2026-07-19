use rshome_entity::{DeviceId, EntityId};

#[derive(Debug, Clone)]
pub enum ServiceTarget {
    All,
    EntityIds(Vec<EntityId>),
    DeviceId(DeviceId),
    Domain(String),
}
