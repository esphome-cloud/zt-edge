use crate::messages::EntityDescriptor;
use crate::{EntityId, EntityMsg};
use parking_lot::RwLock;
use rshome_actor::ActorRef;
use std::collections::HashMap;
use std::sync::Arc;

#[allow(clippy::module_name_repetitions)]
#[derive(Clone, Default)]
pub struct EntityRegistry {
    inner: Arc<RwLock<HashMap<EntityId, ActorRef<EntityMsg>>>>,
    descriptors: Arc<RwLock<HashMap<EntityId, EntityDescriptor>>>,
}

impl EntityRegistry {
    pub fn register(&self, id: EntityId, actor_ref: ActorRef<EntityMsg>) {
        self.inner.write().insert(id, actor_ref);
    }

    /// Store the entity descriptor alongside the actor ref for metadata queries.
    pub fn register_descriptor(&self, descriptor: EntityDescriptor) {
        self.descriptors
            .write()
            .insert(descriptor.entity_id.clone(), descriptor);
    }

    /// Look up the descriptor for an entity.
    #[must_use]
    pub fn get_descriptor(&self, id: &EntityId) -> Option<EntityDescriptor> {
        self.descriptors.read().get(id).cloned()
    }

    #[must_use]
    pub fn get(&self, id: &EntityId) -> Option<ActorRef<EntityMsg>> {
        self.inner.read().get(id).cloned()
    }

    pub fn remove(&self, id: &EntityId) {
        self.inner.write().remove(id);
        self.descriptors.write().remove(id);
    }

    pub fn list_all(&self) -> Vec<EntityId> {
        self.inner.read().keys().cloned().collect()
    }

    pub fn list_by_domain(&self, domain: &str) -> Vec<EntityId> {
        self.inner
            .read()
            .keys()
            .filter(|id| id.domain() == domain)
            .cloned()
            .collect()
    }

    pub fn count(&self) -> usize {
        self.inner.read().len()
    }
}
