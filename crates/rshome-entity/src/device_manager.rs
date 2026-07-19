use crate::device_actor::DeviceActor;
use crate::messages::{DeviceDescriptor, DeviceManagerMsg, DeviceMsg};
use crate::registry::EntityRegistry;
use crate::state_updater::StateUpdater;
use crate::DeviceId;
use rshome_actor::{Actor, ActorContext, ActorRef};
use std::collections::HashMap;
use std::sync::Arc;

#[allow(clippy::module_name_repetitions)]
pub struct DeviceManagerActor {
    devices: HashMap<DeviceId, (DeviceDescriptor, ActorRef<DeviceMsg>)>,
    registry: EntityRegistry,
    state_store: Arc<dyn StateUpdater>,
}

impl DeviceManagerActor {
    pub fn new(registry: EntityRegistry, state_store: Arc<dyn StateUpdater>) -> Self {
        Self {
            devices: HashMap::new(),
            registry,
            state_store,
        }
    }
}

#[async_trait::async_trait]
impl Actor for DeviceManagerActor {
    type Msg = DeviceManagerMsg;

    async fn handle(&mut self, msg: DeviceManagerMsg, ctx: &mut ActorContext<DeviceManagerMsg>) {
        match msg {
            DeviceManagerMsg::AddDevice { descriptor, reply } => {
                let device_id = descriptor.device_id.clone();
                if let Some((stored_descriptor, actor_ref)) = self.devices.get_mut(&device_id) {
                    *stored_descriptor = descriptor.clone();
                    if actor_ref
                        .send(DeviceMsg::UpdateInfo(descriptor.clone()))
                        .is_ok()
                    {
                        let _ = reply.send(actor_ref.clone());
                        return;
                    }
                }

                let actor = DeviceActor::new(
                    descriptor.clone(),
                    self.registry.clone(),
                    self.state_store.clone(),
                );
                let actor_ref = ctx.spawn_child_default(actor);
                self.devices
                    .insert(device_id, (descriptor, actor_ref.clone()));
                let _ = reply.send(actor_ref);
            }
            DeviceManagerMsg::RemoveDevice(id) => {
                if let Some((_, r)) = self.devices.remove(&id) {
                    let _ = r.send(DeviceMsg::Stop);
                }
            }
            DeviceManagerMsg::GetDevice { id, reply } => {
                let actor_ref = self.devices.get(&id).map(|(_, r)| r.clone());
                let _ = reply.send(actor_ref);
            }
            DeviceManagerMsg::ListDevices(reply) => {
                let descs = self.devices.values().map(|(d, _)| d.clone()).collect();
                let _ = reply.send(descs);
            }
            DeviceManagerMsg::GetEntitiesForDevice { device_id, reply } => {
                if let Some((_, device_ref)) = self.devices.get(&device_id) {
                    let device_ref = device_ref.clone();
                    match device_ref.ask(DeviceMsg::GetEntities).await {
                        Ok(ids) => {
                            let _ = reply.send(ids);
                        }
                        Err(_) => {
                            let _ = reply.send(vec![]);
                        }
                    }
                } else {
                    let _ = reply.send(vec![]);
                }
            }
            DeviceManagerMsg::Stop => {
                for (_, r) in self.devices.values() {
                    let _ = r.send(DeviceMsg::Stop);
                }
                ctx.stop();
            }
        }
    }
}
