use crate::fnv::entity_key;
use crate::msg_types;
use crate::proto_gen::*;
use prost::Message;
use rshome_entity::{EntityId, EntityState};

/// Convert an entity state to an ESPHome state-response frame.
/// Returns `None` for entity types that have no corresponding state response
/// (Button, Event, Text, Lock, MediaPlayer, AlarmControlPanel, Update, Unavailable).
pub fn state_to_frame(id: &EntityId, state: &EntityState) -> Option<(u32, Vec<u8>)> {
    let key = entity_key(id);

    match state {
        EntityState::Sensor { value, .. } => {
            let msg = SensorStateResponse {
                key,
                state: *value as f32,
                missing_state: false,
            };
            Some((msg_types::SENSOR_STATE, msg.encode_to_vec()))
        }
        EntityState::BinarySensor { is_on, .. } => {
            let msg = BinarySensorStateResponse {
                key,
                state: *is_on,
                missing_state: false,
            };
            Some((msg_types::BINARY_SENSOR_STATE, msg.encode_to_vec()))
        }
        EntityState::Switch { is_on } => {
            let msg = SwitchStateResponse { key, state: *is_on };
            Some((msg_types::SWITCH_STATE, msg.encode_to_vec()))
        }
        EntityState::Light {
            is_on,
            brightness,
            rgb,
            ..
        } => {
            let mut msg = LightStateResponse {
                key,
                state: *is_on,
                ..Default::default()
            };
            if let Some(b) = brightness {
                msg.brightness = *b as f32;
            }
            if let Some([r, g, b]) = rgb {
                msg.red = f32::from(*r) / 255.0;
                msg.green = f32::from(*g) / 255.0;
                msg.blue = f32::from(*b) / 255.0;
            }
            Some((msg_types::LIGHT_STATE, msg.encode_to_vec()))
        }
        EntityState::Climate {
            mode,
            current_temp,
            target_temp,
            ..
        } => {
            let msg = ClimateStateResponse {
                key,
                mode: climate_mode_to_int(mode),
                current_temperature: current_temp.map(|t| t as f32).unwrap_or(0.0),
                target_temperature: target_temp.map(|t| t as f32).unwrap_or(0.0),
                ..Default::default()
            };
            Some((msg_types::CLIMATE_STATE, msg.encode_to_vec()))
        }
        EntityState::Fan { is_on, speed, .. } => {
            let msg = FanStateResponse {
                key,
                state: *is_on,
                speed_level: i32::from(speed.unwrap_or(0)),
                ..Default::default()
            };
            Some((msg_types::FAN_STATE, msg.encode_to_vec()))
        }
        EntityState::Cover {
            state: cover_state,
            position,
            ..
        } => {
            use rshome_entity::CoverState;
            let current_op = match cover_state {
                CoverState::Opening => 1,
                CoverState::Closing => 2,
                _ => 0,
            };
            let msg = CoverStateResponse {
                key,
                position: f32::from(position.unwrap_or(0)) / 100.0,
                current_operation: current_op,
                ..Default::default()
            };
            Some((msg_types::COVER_STATE, msg.encode_to_vec()))
        }
        EntityState::Number { value, .. } => {
            let msg = NumberStateResponse {
                key,
                state: *value as f32,
                missing_state: false,
            };
            Some((msg_types::NUMBER_STATE, msg.encode_to_vec()))
        }
        EntityState::Select { current, .. } => {
            let msg = SelectStateResponse {
                key,
                state: current.clone(),
                missing_state: false,
            };
            Some((msg_types::SELECT_STATE, msg.encode_to_vec()))
        }
        EntityState::TextSensor { value } => {
            let msg = TextSensorStateResponse {
                key,
                state: value.clone(),
                missing_state: false,
            };
            Some((msg_types::TEXT_SENSOR_STATE, msg.encode_to_vec()))
        }
        EntityState::Unavailable
        | EntityState::Button
        | EntityState::Event { .. }
        | EntityState::Text { .. }
        | EntityState::Lock { .. }
        | EntityState::MediaPlayer { .. }
        | EntityState::AlarmControlPanel { .. }
        | EntityState::Update { .. } => None,
    }
}

/// Climate mode string → ESPHome int (OFF=0, HEAT_COOL=1, COOL=2, HEAT=3, FAN_ONLY=4, DRY=5, AUTO=6)
pub fn climate_mode_to_int(mode: &str) -> i32 {
    match mode {
        "off" => 0,
        "heat_cool" => 1,
        "cool" => 2,
        "heat" => 3,
        "fan_only" => 4,
        "dry" => 5,
        "auto" => 6,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rshome_entity::{EntityId, EntityState};
    use std::collections::HashMap;

    fn sensor_id() -> EntityId {
        EntityId::new("sensor", "temp")
    }

    #[test]
    fn sensor_state_to_frame() {
        let id = sensor_id();
        let state = EntityState::Sensor {
            value: 23.5,
            unit: None,
            attributes: HashMap::new(),
        };
        let (msg_type, bytes) = state_to_frame(&id, &state).unwrap();
        assert_eq!(msg_type, msg_types::SENSOR_STATE);
        let msg = SensorStateResponse::decode(bytes.as_slice()).unwrap();
        assert_eq!(msg.key, entity_key(&id));
        assert!((msg.state - 23.5f32).abs() < 0.01);
        assert!(!msg.missing_state);
    }

    #[test]
    fn switch_state_to_frame() {
        let id = EntityId::new("switch", "relay");
        let state = EntityState::Switch { is_on: true };
        let (msg_type, bytes) = state_to_frame(&id, &state).unwrap();
        assert_eq!(msg_type, msg_types::SWITCH_STATE);
        let msg = SwitchStateResponse::decode(bytes.as_slice()).unwrap();
        assert!(msg.state);
    }

    #[test]
    fn light_brightness_and_rgb() {
        let id = EntityId::new("light", "led");
        let state = EntityState::Light {
            is_on: true,
            brightness: Some(0.75),
            color_temp: None,
            rgb: Some([255, 128, 0]),
            color_mode: None,
        };
        let (msg_type, bytes) = state_to_frame(&id, &state).unwrap();
        assert_eq!(msg_type, msg_types::LIGHT_STATE);
        let msg = LightStateResponse::decode(bytes.as_slice()).unwrap();
        assert!(msg.state);
        assert!((msg.brightness - 0.75f32).abs() < 0.01);
        assert!((msg.red - 1.0f32).abs() < 0.01);
        assert!((msg.green - 128.0 / 255.0f32).abs() < 0.01);
    }

    #[test]
    fn climate_mode_string_to_int() {
        assert_eq!(climate_mode_to_int("off"), 0);
        assert_eq!(climate_mode_to_int("heat"), 3);
        assert_eq!(climate_mode_to_int("cool"), 2);
        assert_eq!(climate_mode_to_int("auto"), 6);
        assert_eq!(climate_mode_to_int("unknown"), 0);
    }

    #[test]
    fn unavailable_returns_none() {
        let id = sensor_id();
        let result = state_to_frame(&id, &EntityState::Unavailable);
        assert!(result.is_none());
    }
}
