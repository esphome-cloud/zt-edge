use crate::entity_actor::EntityActor;
use crate::messages::{DeviceDescriptor, DeviceMsg, EntityMsg};
use crate::registry::EntityRegistry;
use crate::state_updater::StateUpdater;
use crate::EntityId;
use rshome_actor::{Actor, ActorContext, ActorRef};
use std::collections::HashMap;
use std::sync::Arc;

#[allow(clippy::module_name_repetitions)]
pub struct DeviceActor {
    descriptor: DeviceDescriptor,
    entity_refs: HashMap<EntityId, ActorRef<EntityMsg>>,
    registry: EntityRegistry,
    state_store: Arc<dyn StateUpdater>,
}

impl DeviceActor {
    pub fn new(
        descriptor: DeviceDescriptor,
        registry: EntityRegistry,
        state_store: Arc<dyn StateUpdater>,
    ) -> Self {
        Self {
            descriptor,
            entity_refs: HashMap::new(),
            registry,
            state_store,
        }
    }
}

#[async_trait::async_trait]
impl Actor for DeviceActor {
    type Msg = DeviceMsg;

    async fn handle(&mut self, msg: DeviceMsg, ctx: &mut ActorContext<DeviceMsg>) {
        match msg {
            DeviceMsg::UpdateInfo(descriptor) => {
                self.descriptor = descriptor;
            }
            DeviceMsg::AddEntity {
                descriptor,
                initial_state,
                reply,
            } => {
                let entity_id = descriptor.entity_id.clone();
                self.state_store.update(&entity_id, initial_state.clone());
                self.registry.register_descriptor(descriptor.clone());
                let (actor, _change_tx) =
                    EntityActor::new(descriptor, initial_state, self.state_store.clone());
                let actor_ref = ctx.spawn_child_default(actor);
                self.registry.register(entity_id.clone(), actor_ref.clone());
                self.entity_refs.insert(entity_id, actor_ref.clone());
                let _ = reply.send(actor_ref);
            }
            DeviceMsg::AttachEntity {
                descriptor,
                entity_ref,
            } => {
                let entity_id = descriptor.entity_id.clone();
                self.registry.register_descriptor(descriptor);
                self.registry
                    .register(entity_id.clone(), entity_ref.clone());
                self.entity_refs.insert(entity_id, entity_ref);
            }
            DeviceMsg::RemoveEntity(id) => {
                if let Some(r) = self.entity_refs.remove(&id) {
                    let _ = r.send(EntityMsg::Stop);
                    self.registry.remove(&id);
                }
            }
            DeviceMsg::GetEntities(reply) => {
                let ids = self.entity_refs.keys().cloned().collect();
                let _ = reply.send(ids);
            }
            DeviceMsg::GetInfo(reply) => {
                let _ = reply.send(self.descriptor.clone());
            }
            DeviceMsg::Stop => {
                for (id, r) in &self.entity_refs {
                    let _ = r.send(EntityMsg::Stop);
                    self.registry.remove(id);
                }
                ctx.stop();
            }
        }
    }
}
