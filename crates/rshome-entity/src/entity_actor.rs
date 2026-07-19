use crate::domains::DomainRegistry;
use crate::messages::{EntityCommand, EntityDescriptor, EntityMsg, EntityStateChanged};
use crate::state_updater::StateUpdater;
use crate::EntityState;
use rshome_actor::{Actor, ActorContext};
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::broadcast;

#[allow(clippy::module_name_repetitions)]
pub struct EntityActor {
    pub(crate) descriptor: EntityDescriptor,
    pub(crate) state: EntityState,
    state_store: Arc<dyn StateUpdater>,
    change_tx: broadcast::Sender<EntityStateChanged>,
}

impl EntityActor {
    pub fn new(
        descriptor: EntityDescriptor,
        initial_state: EntityState,
        state_store: Arc<dyn StateUpdater>,
    ) -> (Self, broadcast::Sender<EntityStateChanged>) {
        let (change_tx, _) = broadcast::channel(64);
        let actor = Self {
            descriptor,
            state: initial_state,
            state_store,
            change_tx: change_tx.clone(),
        };
        (actor, change_tx)
    }

    pub fn change_sender(&self) -> broadcast::Sender<EntityStateChanged> {
        self.change_tx.clone()
    }

    fn apply_command(&mut self, cmd: EntityCommand) {
        let registry = DomainRegistry::built_in();
        if let Some(domain) = registry.get(&self.descriptor.domain_id) {
            match domain.apply_command(&self.state, &cmd) {
                Ok(new_state) => self.set_state_internal(new_state),
                Err(e) => tracing::warn!(
                    entity_id = %self.descriptor.entity_id, error = %e,
                    "command rejected by domain"
                ),
            }
        } else {
            tracing::warn!(
                entity_id = %self.descriptor.entity_id,
                "unknown domain"
            );
        }
    }

    fn set_state_internal(&mut self, new_state: EntityState) {
        let old = std::mem::replace(&mut self.state, new_state.clone());
        self.state_store
            .update(&self.descriptor.entity_id, new_state.clone());
        let _ = self.change_tx.send(EntityStateChanged {
            entity_id: self.descriptor.entity_id.clone(),
            old_state: old,
            new_state,
            changed_at: SystemTime::now(),
        });
    }
}

#[async_trait::async_trait]
impl Actor for EntityActor {
    type Msg = EntityMsg;

    async fn handle(&mut self, msg: EntityMsg, ctx: &mut ActorContext<EntityMsg>) {
        match msg {
            EntityMsg::GetState(reply) => {
                let _ = reply.send(self.state.clone());
            }
            EntityMsg::SetState(new_state) => {
                self.set_state_internal(new_state);
            }
            EntityMsg::Command(cmd) => {
                self.apply_command(cmd);
            }
            EntityMsg::Stop => {
                ctx.stop();
            }
        }
    }

    async fn post_stop(&mut self) {
        self.state_store
            .update(&self.descriptor.entity_id, EntityState::Unavailable);
    }
}
