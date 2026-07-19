use rshome_entity::{EntityId, EntityState};
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct StateSnapshot {
    pub entity_id: EntityId,
    pub state: EntityState,
    pub last_updated: SystemTime,
}
