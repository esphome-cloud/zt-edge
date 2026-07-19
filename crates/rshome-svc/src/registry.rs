use crate::builtin::{builtin_services, command_for_service};
use crate::dispatch::resolve_target;
use crate::messages::{ServiceDescriptor, ServiceError, ServiceMsg};
use rshome_actor::{Actor, ActorContext, ActorRef};
use rshome_entity::{DeviceManagerMsg, EntityMsg, EntityRegistry};
use std::collections::HashMap;

#[allow(clippy::module_name_repetitions)]
pub struct ServiceRegistryActor {
    services: HashMap<(String, String), ServiceDescriptor>,
    registry: EntityRegistry,
    device_manager: Option<ActorRef<DeviceManagerMsg>>,
}

impl ServiceRegistryActor {
    pub fn new(
        registry: EntityRegistry,
        device_manager: Option<ActorRef<DeviceManagerMsg>>,
    ) -> Self {
        let mut actor = Self {
            services: HashMap::new(),
            registry,
            device_manager,
        };
        for (domain, service) in builtin_services() {
            actor.services.insert(
                (domain.clone(), service.clone()),
                ServiceDescriptor {
                    domain,
                    service,
                    description: None,
                },
            );
        }
        actor
    }
}

#[async_trait::async_trait]
impl Actor for ServiceRegistryActor {
    type Msg = ServiceMsg;

    async fn handle(&mut self, msg: ServiceMsg, _ctx: &mut ActorContext<ServiceMsg>) {
        match msg {
            ServiceMsg::Register(descriptor) => {
                self.services.insert(
                    (descriptor.domain.clone(), descriptor.service.clone()),
                    descriptor,
                );
            }
            ServiceMsg::List(reply) => {
                let list = self.services.values().cloned().collect();
                let _ = reply.send(list);
            }
            ServiceMsg::Call {
                domain,
                service,
                target,
                data,
                reply,
            } => {
                if !self
                    .services
                    .contains_key(&(domain.clone(), service.clone()))
                {
                    let _ = reply.send(Err(ServiceError::NotFound { domain, service }));
                    return;
                }

                let refs =
                    resolve_target(&target, &self.registry, self.device_manager.as_ref()).await;
                if refs.is_empty() {
                    let _ = reply.send(Err(ServiceError::NoTargets));
                    return;
                }

                let count = refs.len();
                if let Some(cmd) = command_for_service(&domain, &service, &data) {
                    for r in refs {
                        let _ = r.send(EntityMsg::Command(cmd.clone()));
                    }
                    let _ = reply.send(Ok(count));
                } else {
                    tracing::warn!(domain, service, "no command mapping for service");
                    let _ = reply.send(Ok(0));
                }
            }
        }
    }
}
