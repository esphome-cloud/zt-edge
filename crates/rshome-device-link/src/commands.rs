use prost::Message;
use rshome_entity::EntityState;
use rshome_native_api::{msg_types, proto_gen::*, state_push::climate_mode_to_int};

/// Translate an entity's desired state into a Native API command frame to send
/// to firmware.
///
/// Returns `(msg_type, encoded_bytes)` or `None` for read-only entities
/// (Sensor, BinarySensor, TextSensor) or states with no command equivalent.
pub fn state_to_command(key: u32, state: &EntityState) -> Option<(u32, Vec<u8>)> {
    match state {
        EntityState::Switch { is_on } => {
            let cmd = SwitchCommandRequest { key, state: *is_on };
            Some((msg_types::SWITCH_COMMAND, cmd.encode_to_vec()))
        }
        EntityState::Light {
            is_on,
            brightness,
            rgb,
            ..
        } => {
            let mut cmd = LightCommandRequest {
                key,
                has_state: true,
                state: *is_on,
                ..Default::default()
            };
            if let Some(b) = brightness {
                cmd.has_brightness = true;
                cmd.brightness = *b as f32;
            }
            if let Some([r, g, b]) = rgb {
                cmd.has_rgb = true;
                cmd.red = f32::from(*r) / 255.0;
                cmd.green = f32::from(*g) / 255.0;
                cmd.blue = f32::from(*b) / 255.0;
            }
            Some((msg_types::LIGHT_COMMAND, cmd.encode_to_vec()))
        }
        EntityState::Climate {
            mode, target_temp, ..
        } => {
            let mut cmd = ClimateCommandRequest {
                key,
                has_mode: true,
                mode: climate_mode_to_int(mode),
                ..Default::default()
            };
            if let Some(t) = target_temp {
                cmd.has_target_temperature = true;
                cmd.target_temperature = *t as f32;
            }
            Some((msg_types::CLIMATE_COMMAND, cmd.encode_to_vec()))
        }
        EntityState::Fan { is_on, speed, .. } => {
            let mut cmd = FanCommandRequest {
                key,
                has_state: true,
                state: *is_on,
                ..Default::default()
            };
            if let Some(s) = speed {
                cmd.has_speed_level = true;
                cmd.speed_level = i32::from(*s);
            }
            Some((msg_types::FAN_COMMAND, cmd.encode_to_vec()))
        }
        EntityState::Cover { position, .. } => {
            let mut cmd = CoverCommandRequest {
                key,
                ..Default::default()
            };
            if let Some(p) = position {
                cmd.has_position = true;
                cmd.position = f32::from(*p) / 100.0;
            }
            Some((msg_types::COVER_COMMAND, cmd.encode_to_vec()))
        }
        EntityState::Number { value, .. } => {
            let cmd = NumberCommandRequest {
                key,
                state: *value as f32,
            };
            Some((msg_types::NUMBER_COMMAND, cmd.encode_to_vec()))
        }
        EntityState::Select { current, .. } => {
            let cmd = SelectCommandRequest {
                key,
                state: current.clone(),
            };
            Some((msg_types::SELECT_COMMAND, cmd.encode_to_vec()))
        }
        EntityState::Button => {
            let cmd = ButtonCommandRequest { key };
            Some((msg_types::BUTTON_COMMAND, cmd.encode_to_vec()))
        }
        // Read-only or no Native API command equivalent
        EntityState::Sensor { .. }
        | EntityState::BinarySensor { .. }
        | EntityState::TextSensor { .. }
        | EntityState::Text { .. }
        | EntityState::Lock { .. }
        | EntityState::MediaPlayer { .. }
        | EntityState::AlarmControlPanel { .. }
        | EntityState::Update { .. }
        | EntityState::Event { .. }
        | EntityState::Unavailable => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_turn_on() {
        let (msg_type, bytes) = state_to_command(42, &EntityState::Switch { is_on: true }).unwrap();
        assert_eq!(msg_type, msg_types::SWITCH_COMMAND);
        let cmd = SwitchCommandRequest::decode(bytes.as_slice()).unwrap();
        assert_eq!(cmd.key, 42);
        assert!(cmd.state);
    }

    #[test]
    fn switch_turn_off() {
        let (_, bytes) = state_to_command(1, &EntityState::Switch { is_on: false }).unwrap();
        let cmd = SwitchCommandRequest::decode(bytes.as_slice()).unwrap();
        assert!(!cmd.state);
    }

    #[test]
    fn light_brightness_command() {
        let state = EntityState::Light {
            is_on: true,
            brightness: Some(0.8),
            color_temp: None,
            rgb: None,
            color_mode: None,
        };
        let (msg_type, bytes) = state_to_command(7, &state).unwrap();
        assert_eq!(msg_type, msg_types::LIGHT_COMMAND);
        let cmd = LightCommandRequest::decode(bytes.as_slice()).unwrap();
        assert!(cmd.has_state && cmd.state);
        assert!(cmd.has_brightness);
        assert!((cmd.brightness - 0.8).abs() < 0.01);
    }

    #[test]
    fn light_rgb_command() {
        let state = EntityState::Light {
            is_on: true,
            brightness: None,
            color_temp: None,
            rgb: Some([255, 128, 0]),
            color_mode: None,
        };
        let (_, bytes) = state_to_command(7, &state).unwrap();
        let cmd = LightCommandRequest::decode(bytes.as_slice()).unwrap();
        assert!(cmd.has_rgb);
        assert!((cmd.red - 1.0).abs() < 0.01);
        assert!((cmd.green - 128.0 / 255.0).abs() < 0.01);
        assert!(cmd.blue < 0.01);
    }

    #[test]
    fn climate_mode_and_temp() {
        let state = EntityState::Climate {
            mode: "heat".into(),
            current_temp: Some(20.0),
            target_temp: Some(22.5),
            hvac_action: None,
        };
        let (msg_type, bytes) = state_to_command(5, &state).unwrap();
        assert_eq!(msg_type, msg_types::CLIMATE_COMMAND);
        let cmd = ClimateCommandRequest::decode(bytes.as_slice()).unwrap();
        assert_eq!(cmd.mode, 3); // heat
        assert!(cmd.has_target_temperature);
        assert!((cmd.target_temperature - 22.5).abs() < 0.01);
    }

    #[test]
    fn fan_state_with_speed() {
        let state = EntityState::Fan {
            is_on: true,
            speed: Some(3),
            oscillating: None,
            direction: None,
        };
        let (msg_type, bytes) = state_to_command(3, &state).unwrap();
        assert_eq!(msg_type, msg_types::FAN_COMMAND);
        let cmd = FanCommandRequest::decode(bytes.as_slice()).unwrap();
        assert!(cmd.state);
        assert_eq!(cmd.speed_level, 3);
    }

    #[test]
    fn cover_position() {
        let state = EntityState::Cover {
            state: rshome_entity::CoverState::Stopped,
            position: Some(50),
            tilt: None,
        };
        let (msg_type, bytes) = state_to_command(10, &state).unwrap();
        assert_eq!(msg_type, msg_types::COVER_COMMAND);
        let cmd = CoverCommandRequest::decode(bytes.as_slice()).unwrap();
        assert!(cmd.has_position);
        assert!((cmd.position - 0.5).abs() < 0.01);
    }

    #[test]
    fn number_value() {
        let state = EntityState::Number {
            value: 42.0,
            min: 0.0,
            max: 100.0,
            step: 1.0,
            unit: None,
        };
        let (msg_type, bytes) = state_to_command(8, &state).unwrap();
        assert_eq!(msg_type, msg_types::NUMBER_COMMAND);
        let cmd = NumberCommandRequest::decode(bytes.as_slice()).unwrap();
        assert!((cmd.state - 42.0).abs() < 0.01);
    }

    #[test]
    fn select_option() {
        let state = EntityState::Select {
            current: "option_b".into(),
            options: vec!["option_a".into(), "option_b".into()],
        };
        let (msg_type, bytes) = state_to_command(11, &state).unwrap();
        assert_eq!(msg_type, msg_types::SELECT_COMMAND);
        let cmd = SelectCommandRequest::decode(bytes.as_slice()).unwrap();
        assert_eq!(cmd.state, "option_b");
    }

    #[test]
    fn button_press() {
        let (msg_type, bytes) = state_to_command(12, &EntityState::Button).unwrap();
        assert_eq!(msg_type, msg_types::BUTTON_COMMAND);
        let cmd = ButtonCommandRequest::decode(bytes.as_slice()).unwrap();
        assert_eq!(cmd.key, 12);
    }

    #[test]
    fn sensor_returns_none() {
        let state = EntityState::Sensor {
            value: 23.5,
            unit: None,
            attributes: Default::default(),
        };
        assert!(state_to_command(1, &state).is_none());
    }
}
