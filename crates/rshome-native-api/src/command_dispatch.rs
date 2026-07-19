use crate::error::ApiError;
use crate::fnv::entity_key;
use crate::msg_types;
use crate::proto_gen::*;
use prost::Message;
use rshome_actor::ActorRef;
use rshome_entity::{EntityId, EntityRegistry};
use rshome_svc::{ServiceMsg, ServiceTarget};

#[allow(clippy::module_name_repetitions)]
pub struct CommandDispatcher {
    pub registry: EntityRegistry,
    pub service_registry: ActorRef<ServiceMsg>,
}

impl CommandDispatcher {
    /// O(n) scan to find an entity by its FNV-1a key.
    fn find_entity_id(&self, key: u32) -> Option<EntityId> {
        self.registry
            .list_all()
            .into_iter()
            .find(|id| entity_key(id) == key)
    }

    /// Dispatch an inbound command message.
    pub async fn dispatch(&self, msg_type: u32, payload: &[u8]) -> Result<(), ApiError> {
        match msg_type {
            msg_types::SWITCH_COMMAND => {
                let cmd = SwitchCommandRequest::decode(payload)?;
                let id = self
                    .find_entity_id(cmd.key)
                    .ok_or(ApiError::EntityNotFound { key: cmd.key })?;
                let service = if cmd.state { "turn_on" } else { "turn_off" };
                self.call_service("switch", service, id, serde_json::Value::Null)
                    .await
            }
            msg_types::LIGHT_COMMAND => {
                let cmd = LightCommandRequest::decode(payload)?;
                let id = self
                    .find_entity_id(cmd.key)
                    .ok_or(ApiError::EntityNotFound { key: cmd.key })?;
                let mut data = serde_json::json!({});
                if cmd.has_brightness {
                    data["brightness"] = serde_json::json!(cmd.brightness);
                }
                if cmd.has_rgb {
                    data["rgb_color"] = serde_json::json!([
                        (cmd.red * 255.0) as u8,
                        (cmd.green * 255.0) as u8,
                        (cmd.blue * 255.0) as u8
                    ]);
                }
                let service = if cmd.has_state {
                    if cmd.state {
                        "turn_on"
                    } else {
                        "turn_off"
                    }
                } else {
                    "turn_on"
                };
                self.call_service("light", service, id, data).await
            }
            msg_types::CLIMATE_COMMAND => {
                let cmd = ClimateCommandRequest::decode(payload)?;
                let id = self
                    .find_entity_id(cmd.key)
                    .ok_or(ApiError::EntityNotFound { key: cmd.key })?;
                let mut data = serde_json::json!({});
                if cmd.has_mode {
                    data["hvac_mode"] = serde_json::json!(climate_int_to_mode(cmd.mode));
                }
                if cmd.has_target_temperature {
                    data["temperature"] = serde_json::json!(cmd.target_temperature);
                }
                self.call_service("climate", "set_temperature", id, data)
                    .await
            }
            msg_types::FAN_COMMAND => {
                let cmd = FanCommandRequest::decode(payload)?;
                let id = self
                    .find_entity_id(cmd.key)
                    .ok_or(ApiError::EntityNotFound { key: cmd.key })?;
                let mut data = serde_json::json!({});
                if cmd.has_speed_level {
                    data["percentage"] = serde_json::json!(cmd.speed_level);
                }
                let service = if cmd.has_state {
                    if cmd.state {
                        "turn_on"
                    } else {
                        "turn_off"
                    }
                } else {
                    "turn_on"
                };
                self.call_service("fan", service, id, data).await
            }
            msg_types::COVER_COMMAND => {
                let cmd = CoverCommandRequest::decode(payload)?;
                let id = self
                    .find_entity_id(cmd.key)
                    .ok_or(ApiError::EntityNotFound { key: cmd.key })?;
                if cmd.stop {
                    return self
                        .call_service("cover", "stop_cover", id, serde_json::Value::Null)
                        .await;
                }
                let mut data = serde_json::json!({});
                if cmd.has_position {
                    data["position"] = serde_json::json!((cmd.position * 100.0) as u8);
                }
                self.call_service("cover", "set_cover_position", id, data)
                    .await
            }
            msg_types::NUMBER_COMMAND => {
                let cmd = NumberCommandRequest::decode(payload)?;
                let id = self
                    .find_entity_id(cmd.key)
                    .ok_or(ApiError::EntityNotFound { key: cmd.key })?;
                let data = serde_json::json!({ "value": cmd.state });
                self.call_service("number", "set_value", id, data).await
            }
            msg_types::SELECT_COMMAND => {
                let cmd = SelectCommandRequest::decode(payload)?;
                let id = self
                    .find_entity_id(cmd.key)
                    .ok_or(ApiError::EntityNotFound { key: cmd.key })?;
                let data = serde_json::json!({ "option": cmd.state });
                self.call_service("select", "select_option", id, data).await
            }
            msg_types::BUTTON_COMMAND => {
                let cmd = ButtonCommandRequest::decode(payload)?;
                let id = self
                    .find_entity_id(cmd.key)
                    .ok_or(ApiError::EntityNotFound { key: cmd.key })?;
                self.call_service("button", "press", id, serde_json::Value::Null)
                    .await
            }
            _ => Ok(()), // silently ignore unknown command types
        }
    }

    async fn call_service(
        &self,
        domain: &str,
        service: &str,
        id: EntityId,
        data: serde_json::Value,
    ) -> Result<(), ApiError> {
        self.service_registry
            .ask(|reply| ServiceMsg::Call {
                domain: domain.to_string(),
                service: service.to_string(),
                target: ServiceTarget::EntityIds(vec![id]),
                data,
                reply,
            })
            .await?
            .map(|_| ())
            .map_err(ApiError::Service)
    }
}

fn climate_int_to_mode(mode: i32) -> &'static str {
    match mode {
        0 => "off",
        1 => "heat_cool",
        2 => "cool",
        3 => "heat",
        4 => "fan_only",
        5 => "dry",
        6 => "auto",
        _ => "off",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rshome_actor::{Actor, ActorContext, ActorSystem};
    use rshome_entity::{
        EntityActor, EntityCategory, EntityDescriptor, EntityState, NullStateUpdater,
    };
    use std::sync::Arc;

    // Mock service registry: always responds Ok(1) to Call
    struct MockSvcRegistry;

    #[async_trait::async_trait]
    impl Actor for MockSvcRegistry {
        type Msg = ServiceMsg;
        async fn handle(&mut self, msg: Self::Msg, _ctx: &mut ActorContext<Self::Msg>) {
            if let ServiceMsg::Call { reply, .. } = msg {
                let _ = reply.send(Ok(1));
            }
        }
    }

    fn make_descriptor(id: &EntityId) -> EntityDescriptor {
        EntityDescriptor {
            entity_id: id.clone(),
            name: id.object_id().to_string(),
            icon: None,
            device_id: None,
            area_id: None,
            entity_category: EntityCategory::None,
            domain_id: id.domain().to_string(),
            feature_set: vec![],
            device_class: None,
        }
    }

    async fn make_dispatcher(
        entities: Vec<(EntityId, EntityState)>,
    ) -> (ActorSystem, CommandDispatcher) {
        let system = ActorSystem::new();
        let registry = EntityRegistry::default();
        let updater = Arc::new(NullStateUpdater);
        for (id, state) in entities {
            let desc = make_descriptor(&id);
            let (actor, _tx) = EntityActor::new(desc, state, updater.clone());
            let actor_ref = system.spawn(actor);
            registry.register(id, actor_ref);
        }
        let svc_ref = system.spawn(MockSvcRegistry);
        let dispatcher = CommandDispatcher {
            registry,
            service_registry: svc_ref,
        };
        (system, dispatcher)
    }

    #[tokio::test]
    async fn switch_command_turn_on() {
        let id = EntityId::new("switch", "relay");
        let key = entity_key(&id);
        let (_sys, dispatcher) =
            make_dispatcher(vec![(id, EntityState::Switch { is_on: false })]).await;

        let cmd = SwitchCommandRequest { key, state: true };
        let result = dispatcher
            .dispatch(msg_types::SWITCH_COMMAND, &cmd.encode_to_vec())
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn switch_command_turn_off() {
        let id = EntityId::new("switch", "relay");
        let key = entity_key(&id);
        let (_sys, dispatcher) =
            make_dispatcher(vec![(id, EntityState::Switch { is_on: true })]).await;

        let cmd = SwitchCommandRequest { key, state: false };
        let result = dispatcher
            .dispatch(msg_types::SWITCH_COMMAND, &cmd.encode_to_vec())
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn light_command_brightness() {
        let id = EntityId::new("light", "led");
        let key = entity_key(&id);
        let (_sys, dispatcher) = make_dispatcher(vec![(
            id,
            EntityState::Light {
                is_on: true,
                brightness: None,
                color_temp: None,
                rgb: None,
                color_mode: None,
            },
        )])
        .await;

        let cmd = LightCommandRequest {
            key,
            has_brightness: true,
            brightness: 0.5,
            ..Default::default()
        };
        let result = dispatcher
            .dispatch(msg_types::LIGHT_COMMAND, &cmd.encode_to_vec())
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn climate_command_mode() {
        let id = EntityId::new("climate", "therm");
        let key = entity_key(&id);
        let (_sys, dispatcher) = make_dispatcher(vec![(
            id,
            EntityState::Climate {
                mode: "off".into(),
                current_temp: None,
                target_temp: None,
                hvac_action: None,
            },
        )])
        .await;

        let cmd = ClimateCommandRequest {
            key,
            has_mode: true,
            mode: 3,
            ..Default::default()
        };
        let result = dispatcher
            .dispatch(msg_types::CLIMATE_COMMAND, &cmd.encode_to_vec())
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn climate_command_temperature() {
        let id = EntityId::new("climate", "therm");
        let key = entity_key(&id);
        let (_sys, dispatcher) = make_dispatcher(vec![(
            id,
            EntityState::Climate {
                mode: "heat".into(),
                current_temp: Some(20.0),
                target_temp: Some(22.0),
                hvac_action: None,
            },
        )])
        .await;

        let cmd = ClimateCommandRequest {
            key,
            has_target_temperature: true,
            target_temperature: 24.0,
            ..Default::default()
        };
        let result = dispatcher
            .dispatch(msg_types::CLIMATE_COMMAND, &cmd.encode_to_vec())
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn fan_command_speed() {
        let id = EntityId::new("fan", "ceiling");
        let key = entity_key(&id);
        let (_sys, dispatcher) = make_dispatcher(vec![(
            id,
            EntityState::Fan {
                is_on: true,
                speed: Some(50),
                oscillating: None,
                direction: None,
            },
        )])
        .await;

        let cmd = FanCommandRequest {
            key,
            has_speed_level: true,
            speed_level: 3,
            ..Default::default()
        };
        let result = dispatcher
            .dispatch(msg_types::FAN_COMMAND, &cmd.encode_to_vec())
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn cover_command_position() {
        let id = EntityId::new("cover", "blind");
        let key = entity_key(&id);
        let (_sys, dispatcher) = make_dispatcher(vec![(
            id,
            EntityState::Cover {
                state: rshome_entity::CoverState::Open,
                position: Some(100),
                tilt: None,
            },
        )])
        .await;

        let cmd = CoverCommandRequest {
            key,
            has_position: true,
            position: 0.5,
            ..Default::default()
        };
        let result = dispatcher
            .dispatch(msg_types::COVER_COMMAND, &cmd.encode_to_vec())
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn button_command() {
        let id = EntityId::new("button", "restart");
        let key = entity_key(&id);
        let (_sys, dispatcher) = make_dispatcher(vec![(id, EntityState::Button)]).await;

        let cmd = ButtonCommandRequest { key };
        let result = dispatcher
            .dispatch(msg_types::BUTTON_COMMAND, &cmd.encode_to_vec())
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn unknown_key_returns_entity_not_found() {
        let (_sys, dispatcher) = make_dispatcher(vec![]).await;

        let cmd = SwitchCommandRequest {
            key: 0xDEAD_BEEF,
            state: true,
        };
        let result = dispatcher
            .dispatch(msg_types::SWITCH_COMMAND, &cmd.encode_to_vec())
            .await;
        assert!(matches!(result, Err(ApiError::EntityNotFound { .. })));
    }
}
