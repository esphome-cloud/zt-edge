use crate::{EntityId, EntityState};

/// Trait implemented by StateStore (in rshome-state) to decouple rshome-entity from rshome-state.
pub trait StateUpdater: Send + Sync {
    fn update(&self, id: &EntityId, state: EntityState);
}

/// No-op implementation for tests or when state tracking is not needed.
pub struct NullStateUpdater;

impl StateUpdater for NullStateUpdater {
    fn update(&self, _id: &EntityId, _state: EntityState) {}
}
