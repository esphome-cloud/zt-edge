use crate::fnv::entity_key;
use crate::msg_types;
use crate::proto_gen::*;
use prost::Message;
use rshome_entity::{EntityId, EntityMsg, EntityRegistry, EntityState};

/// Build list-entities frames for all available entities in the registry.
/// Returns `Vec<(msg_type, encoded_proto_bytes)>`.
pub async fn build_entity_list(registry: &EntityRegistry) -> Vec<(u32, Vec<u8>)> {
    let mut ids = registry.list_all();
    ids.sort_by(|a, b| a.0.cmp(&b.0));

    let mut frames = Vec::new();

    for id in &ids {
        let actor_ref = match registry.get(id) {
            Some(r) => r,
            None => continue,
        };

        let state = match actor_ref.ask(EntityMsg::GetState).await {
            Ok(s) => s,
            Err(_) => continue,
        };

        if matches!(state, EntityState::Unavailable) {
            continue;
        }

        if let Some(frame) = entity_to_list_frame(id, &state) {
            frames.push(frame);
        }
    }

    frames
}

fn entity_to_list_frame(id: &EntityId, state: &EntityState) -> Option<(u32, Vec<u8>)> {
    let key = entity_key(id);
    let name = id.object_id().to_string();
    let object_id = id.object_id().to_string();
    let unique_id = id.to_string();

    match state {
        EntityState::Sensor { unit, .. } => {
            let msg = ListEntitiesSensorResponse {
                object_id,
                key,
                name,
                unique_id,
                unit_of_measurement: unit.clone().unwrap_or_default(),
                ..Default::default()
            };
            Some((msg_types::LIST_ENTITIES_SENSOR, msg.encode_to_vec()))
        }
        EntityState::BinarySensor { .. } => {
            let msg = ListEntitiesBinarySensorResponse {
                object_id,
                key,
                name,
                unique_id,
                ..Default::default()
            };
            Some((msg_types::LIST_ENTITIES_BINARY_SENSOR, msg.encode_to_vec()))
        }
        EntityState::Switch { .. } => {
            let msg = ListEntitiesSwitchResponse {
                object_id,
                key,
                name,
                unique_id,
                ..Default::default()
            };
            Some((msg_types::LIST_ENTITIES_SWITCH, msg.encode_to_vec()))
        }
        EntityState::Light { brightness, .. } => {
            let msg = ListEntitiesLightResponse {
                object_id,
                key,
                name,
                unique_id,
                legacy_supports_brightness: brightness.is_some(),
                ..Default::default()
            };
            Some((msg_types::LIST_ENTITIES_LIGHT, msg.encode_to_vec()))
        }
        EntityState::Climate { .. } => {
            let msg = ListEntitiesClimateResponse {
                object_id,
                key,
                name,
                unique_id,
                supports_current_temperature: true,
                ..Default::default()
            };
            Some((msg_types::CLIMATE_LIST, msg.encode_to_vec()))
        }
        EntityState::Fan { .. } => {
            let msg = ListEntitiesFanResponse {
                object_id,
                key,
                name,
                unique_id,
                supports_speed: true,
                ..Default::default()
            };
            Some((msg_types::LIST_ENTITIES_FAN, msg.encode_to_vec()))
        }
        EntityState::Cover { .. } => {
            let msg = ListEntitiesCoverResponse {
                object_id,
                key,
                name,
                unique_id,
                supports_position: true,
                ..Default::default()
            };
            Some((msg_types::LIST_ENTITIES_COVER, msg.encode_to_vec()))
        }
        EntityState::Number {
            min,
            max,
            step,
            unit,
            ..
        } => {
            let msg = ListEntitiesNumberResponse {
                object_id,
                key,
                name,
                unique_id,
                min_value: *min as f32,
                max_value: *max as f32,
                step: *step as f32,
                unit_of_measurement: unit.clone().unwrap_or_default(),
                ..Default::default()
            };
            Some((msg_types::LIST_ENTITIES_NUMBER, msg.encode_to_vec()))
        }
        EntityState::Select { options, .. } => {
            let msg = ListEntitiesSelectResponse {
                object_id,
                key,
                name,
                unique_id,
                options: options.clone(),
                ..Default::default()
            };
            Some((msg_types::LIST_ENTITIES_SELECT, msg.encode_to_vec()))
        }
        EntityState::TextSensor { .. } => {
            let msg = ListEntitiesTextSensorResponse {
                object_id,
                key,
                name,
                unique_id,
                ..Default::default()
            };
            Some((msg_types::LIST_ENTITIES_TEXT_SENSOR, msg.encode_to_vec()))
        }
        // Button, Event, Text, Lock, MediaPlayer, AlarmControlPanel, Update, Unavailable
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::EspHomeCodec;
    use bytes::BytesMut;
    use rshome_actor::ActorSystem;
    use rshome_entity::{
        EntityActor, EntityCategory, EntityDescriptor, EntityState, NullStateUpdater,
    };
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio_util::codec::{Decoder, Encoder};

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

    async fn make_registry_with(entities: Vec<(EntityId, EntityState)>) -> EntityRegistry {
        let system = ActorSystem::new();
        let registry = EntityRegistry::default();
        let updater = Arc::new(NullStateUpdater);
        for (id, state) in entities {
            let desc = make_descriptor(&id);
            let (actor, _tx) = EntityActor::new(desc, state.clone(), updater.clone());
            let actor_ref = system.spawn(actor);
            registry.register(id, actor_ref);
        }
        registry
    }

    #[tokio::test]
    async fn empty_registry_produces_no_frames() {
        let registry = EntityRegistry::default();
        let frames = build_entity_list(&registry).await;
        assert!(frames.is_empty());
    }

    #[tokio::test]
    async fn sensor_entity_correct_fields() {
        let id = EntityId::new("sensor", "temperature");
        let state = EntityState::Sensor {
            value: 23.5,
            unit: Some("°C".into()),
            attributes: HashMap::new(),
        };
        let registry = make_registry_with(vec![(id.clone(), state)]).await;
        let frames = build_entity_list(&registry).await;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].0, msg_types::LIST_ENTITIES_SENSOR);
        let msg = ListEntitiesSensorResponse::decode(frames[0].1.as_slice()).unwrap();
        assert_eq!(msg.object_id, "temperature");
        assert_eq!(msg.key, entity_key(&id));
        assert_eq!(msg.unit_of_measurement, "°C");
    }

    #[tokio::test]
    async fn switch_entity_correct_fields() {
        let id = EntityId::new("switch", "relay1");
        let state = EntityState::Switch { is_on: false };
        let registry = make_registry_with(vec![(id.clone(), state)]).await;
        let frames = build_entity_list(&registry).await;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].0, msg_types::LIST_ENTITIES_SWITCH);
        let msg = ListEntitiesSwitchResponse::decode(frames[0].1.as_slice()).unwrap();
        assert_eq!(msg.object_id, "relay1");
        assert_eq!(msg.key, entity_key(&id));
    }

    #[tokio::test]
    async fn light_supports_brightness_flag() {
        let id = EntityId::new("light", "led");
        let state = EntityState::Light {
            is_on: true,
            brightness: Some(0.8),
            color_temp: None,
            rgb: None,
            color_mode: None,
        };
        let registry = make_registry_with(vec![(id, state)]).await;
        let frames = build_entity_list(&registry).await;
        assert_eq!(frames[0].0, msg_types::LIST_ENTITIES_LIGHT);
        let msg = ListEntitiesLightResponse::decode(frames[0].1.as_slice()).unwrap();
        assert!(msg.legacy_supports_brightness);
    }

    #[tokio::test]
    async fn climate_entity_correct_fields() {
        let id = EntityId::new("climate", "thermostat");
        let state = EntityState::Climate {
            mode: "heat".into(),
            current_temp: Some(20.0),
            target_temp: Some(22.0),
            hvac_action: None,
        };
        let registry = make_registry_with(vec![(id.clone(), state)]).await;
        let frames = build_entity_list(&registry).await;
        assert_eq!(frames[0].0, msg_types::CLIMATE_LIST);
        let msg = ListEntitiesClimateResponse::decode(frames[0].1.as_slice()).unwrap();
        assert_eq!(msg.object_id, "thermostat");
        assert_eq!(msg.key, entity_key(&id));
    }

    #[tokio::test]
    async fn multi_type_mix() {
        let entities = vec![
            (
                EntityId::new("sensor", "temp"),
                EntityState::Sensor {
                    value: 22.0,
                    unit: None,
                    attributes: HashMap::new(),
                },
            ),
            (
                EntityId::new("switch", "sw1"),
                EntityState::Switch { is_on: true },
            ),
        ];
        let registry = make_registry_with(entities).await;
        let frames = build_entity_list(&registry).await;
        assert_eq!(frames.len(), 2);
    }

    #[tokio::test]
    async fn unavailable_entity_excluded() {
        let id = EntityId::new("sensor", "broken");
        let state = EntityState::Unavailable;
        let registry = make_registry_with(vec![(id, state)]).await;
        let frames = build_entity_list(&registry).await;
        assert!(frames.is_empty());
    }

    /// Wire-compatibility trace fixture for `ListEntitiesSwitchResponse`.
    ///
    /// Encodes a specific switch entity to an ESPHome frame and verifies the
    /// exact wire layout. If this test fails after a refactor, the wire format
    /// has changed and HA compatibility is broken.
    ///
    /// Expected proto bytes (33):
    ///   object_id "relay"   → 0x0a 0x05 + 5 bytes  = 7
    ///   key       fixed32   → 0x15 + 4 bytes        = 5
    ///   name      "relay"   → 0x1a 0x05 + 5 bytes  = 7
    ///   unique_id "switch.relay" → 0x22 0x0c + 12  = 14  (total 33)
    /// Frame: 0x00 | 0x21 (len=33) | 0x11 (type=17) | 33 bytes = 36 bytes
    #[test]
    fn wire_fixture_switch_list_entities_frame() {
        let id = EntityId::new("switch", "relay");
        let state = EntityState::Switch { is_on: false };

        let (msg_type, proto_bytes) =
            entity_to_list_frame(&id, &state).expect("switch must produce a list frame");
        assert_eq!(msg_type, msg_types::LIST_ENTITIES_SWITCH);

        // proto3 wire: object_id(7) + key(5) + name(7) + unique_id(14) = 33 bytes
        assert_eq!(
            proto_bytes.len(),
            33,
            "proto payload must be exactly 33 bytes"
        );

        // Wrap in EspHomeCodec frame
        let mut buf = BytesMut::new();
        EspHomeCodec
            .encode((msg_type, proto_bytes.clone()), &mut buf)
            .unwrap();
        let frame = buf.to_vec();

        // ESPHome wire frame: 0x00 | varint(33)=0x21 | varint(17)=0x11 | 33 bytes = 36 total
        assert_eq!(frame[0], 0x00, "frame preamble must be 0x00");
        assert_eq!(frame[1], 0x21, "payload length varint must be 0x21 (33)");
        assert_eq!(
            frame[2], 0x11,
            "msg_type varint must be 0x11 (17 = LIST_ENTITIES_SWITCH)"
        );
        assert_eq!(frame.len(), 36, "total frame length must be 36 bytes");

        // Round-trip: codec must decode identically
        let mut decode_buf = BytesMut::from(frame.as_slice());
        let (rt_type, rt_payload) = EspHomeCodec.decode(&mut decode_buf).unwrap().unwrap();
        assert_eq!(rt_type, msg_types::LIST_ENTITIES_SWITCH);
        assert_eq!(rt_payload, proto_bytes);

        // Semantic verification
        let msg = ListEntitiesSwitchResponse::decode(rt_payload.as_slice() as &[u8]).unwrap();
        assert_eq!(msg.object_id, "relay");
        assert_eq!(msg.key, entity_key(&id));
        assert_eq!(msg.name, "relay");
        assert_eq!(msg.unique_id, "switch.relay");
    }

    /// Wire-compatibility trace fixture for `ClimateStateResponse`.
    ///
    /// Expected proto bytes (17):
    ///   key              fixed32    → 0x0d + 4 bytes   = 5
    ///   mode             int32=3    → 0x10 0x03        = 2
    ///   current_temp     float=20.0 → 0x1d + 4 bytes   = 5
    ///   target_temp      float=22.0 → 0x25 + 4 bytes   = 5  (total 17)
    /// Frame: 0x00 | 0x11 (len=17) | 0x2f (type=47) | 17 bytes = 20 bytes
    #[test]
    fn wire_fixture_climate_state_frame() {
        use crate::state_push::state_to_frame;

        let id = EntityId::new("climate", "thermostat");
        let state = EntityState::Climate {
            mode: "heat".into(),
            current_temp: Some(20.0),
            target_temp: Some(22.0),
            hvac_action: None,
        };

        let (msg_type, proto_bytes) =
            state_to_frame(&id, &state).expect("climate must produce a state frame");
        assert_eq!(msg_type, msg_types::CLIMATE_STATE);

        // proto3 wire: key(5) + mode(2) + current_temp(5) + target_temp(5) = 17 bytes
        assert_eq!(
            proto_bytes.len(),
            17,
            "proto payload must be exactly 17 bytes"
        );

        // Wrap in EspHomeCodec frame
        let mut buf = BytesMut::new();
        EspHomeCodec
            .encode((msg_type, proto_bytes.clone()), &mut buf)
            .unwrap();
        let frame = buf.to_vec();

        // ESPHome wire frame: 0x00 | varint(17)=0x11 | varint(47)=0x2f | 17 bytes = 20 total
        assert_eq!(frame[0], 0x00, "frame preamble must be 0x00");
        assert_eq!(frame[1], 0x11, "payload length varint must be 0x11 (17)");
        assert_eq!(
            frame[2], 0x2f,
            "msg_type varint must be 0x2f (47 = CLIMATE_STATE)"
        );
        assert_eq!(frame.len(), 20, "total frame length must be 20 bytes");

        // Round-trip: codec must decode identically
        let mut decode_buf = BytesMut::from(frame.as_slice());
        let (rt_type, rt_payload) = EspHomeCodec.decode(&mut decode_buf).unwrap().unwrap();
        assert_eq!(rt_type, msg_types::CLIMATE_STATE);
        assert_eq!(rt_payload, proto_bytes);

        // Semantic verification
        let msg = ClimateStateResponse::decode(rt_payload.as_slice() as &[u8]).unwrap();
        assert_eq!(msg.key, entity_key(&id));
        assert_eq!(msg.mode, 3); // "heat" → 3
        assert!((msg.current_temperature - 20.0f32).abs() < 0.001);
        assert!((msg.target_temperature - 22.0f32).abs() < 0.001);
    }
}
