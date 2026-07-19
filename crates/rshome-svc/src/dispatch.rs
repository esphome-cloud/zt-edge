use rshome_actor::ActorRef;
use rshome_entity::{DeviceManagerMsg, EntityMsg, EntityRegistry};

use crate::target::ServiceTarget;

/// Resolve a ServiceTarget to a list of ActorRefs for entities.
pub async fn resolve_target(
    target: &ServiceTarget,
    registry: &EntityRegistry,
    device_manager: Option<&ActorRef<DeviceManagerMsg>>,
) -> Vec<ActorRef<EntityMsg>> {
    match target {
        ServiceTarget::All => registry
            .list_all()
            .iter()
            .filter_map(|id| registry.get(id))
            .collect(),
        ServiceTarget::EntityIds(ids) => ids.iter().filter_map(|id| registry.get(id)).collect(),
        ServiceTarget::Domain(domain) => registry
            .list_by_domain(domain)
            .iter()
            .filter_map(|id| registry.get(id))
            .collect(),
        ServiceTarget::DeviceId(device_id) => {
            if let Some(dm) = device_manager {
                match dm
                    .ask(|tx| DeviceManagerMsg::GetEntitiesForDevice {
                        device_id: device_id.clone(),
                        reply: tx,
                    })
                    .await
                {
                    Ok(ids) => ids.iter().filter_map(|id| registry.get(id)).collect(),
                    Err(_) => vec![],
                }
            } else {
                vec![]
            }
        }
    }
}
