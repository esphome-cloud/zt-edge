use crate::error::DeviceLinkError;
use prost::Message;
use rshome_entity::{CoverState, DomainRegistry, EntityState};
use rshome_native_api::{msg_types, proto_gen::*};
use std::collections::HashMap;

/// A state value parsed from a firmware-pushed state-response frame.
#[derive(Debug, Clone)]
pub struct IngestedState {
    /// The entity's FNV-1a key (used to look up local EntityId).
    pub key: u32,
    /// The parsed entity state.
    pub state: EntityState,
    /// Domain resolved from `DomainRegistry` (e.g. `"sensor"`, `"switch"`).
    pub domain_id: String,
    /// Features resolved from `DomainRegistry` for this domain.
    pub feature_set: Vec<String>,
}

/// Map a state-response `msg_type` to its ESPHome domain wire type string.
fn wire_type_for_msg(msg_type: u32) -> Option<&'static str> {
    match msg_type {
        t if t == msg_types::SENSOR_STATE => Some("sensor"),
        t if t == msg_types::BINARY_SENSOR_STATE => Some("binary_sensor"),
        t if t == msg_types::SWITCH_STATE => Some("switch"),
        t if t == msg_types::LIGHT_STATE => Some("light"),
        t if t == msg_types::FAN_STATE => Some("fan"),
        t if t == msg_types::COVER_STATE => Some("cover"),
        t if t == msg_types::NUMBER_STATE => Some("number"),
        t if t == msg_types::SELECT_STATE => Some("select"),
        t if t == msg_types::TEXT_SENSOR_STATE => Some("text_sensor"),
        t if t == msg_types::CLIMATE_STATE => Some("climate"),
        _ => None,
    }
}

/// Parse an inbound state-response frame from a firmware device.
///
/// Returns `Ok(None)` if `msg_type` is not a recognised state-response type
/// (e.g. a ping or handshake frame).
pub fn parse_state_frame(
    msg_type: u32,
    payload: &[u8],
) -> Result<Option<IngestedState>, DeviceLinkError> {
    let Some(wire_type) = wire_type_for_msg(msg_type) else {
        return Ok(None);
    };

    let (domain_id, feature_set) = DomainRegistry::built_in()
        .resolve_wire_type(wire_type)
        .map(|(d, f)| (d.to_string(), f))
        .unwrap_or_else(|| (wire_type.to_string(), vec![]));

    let (key, state) = match msg_type {
        t if t == msg_types::SENSOR_STATE => {
            let m = SensorStateResponse::decode(payload)?;
            let value = if m.missing_state {
                0.0
            } else {
                f64::from(m.state)
            };
            (
                m.key,
                EntityState::Sensor {
                    value,
                    unit: None,
                    attributes: HashMap::new(),
                },
            )
        }
        t if t == msg_types::BINARY_SENSOR_STATE => {
            let m = BinarySensorStateResponse::decode(payload)?;
            (
                m.key,
                EntityState::BinarySensor {
                    is_on: m.state,
                    attributes: HashMap::new(),
                },
            )
        }
        t if t == msg_types::SWITCH_STATE => {
            let m = SwitchStateResponse::decode(payload)?;
            (m.key, EntityState::Switch { is_on: m.state })
        }
        t if t == msg_types::LIGHT_STATE => {
            let m = LightStateResponse::decode(payload)?;
            let brightness = if m.brightness > 0.0 {
                Some(f64::from(m.brightness))
            } else {
                None
            };
            let rgb = if m.red > 0.0 || m.green > 0.0 || m.blue > 0.0 {
                Some([
                    (m.red * 255.0) as u8,
                    (m.green * 255.0) as u8,
                    (m.blue * 255.0) as u8,
                ])
            } else {
                None
            };
            (
                m.key,
                EntityState::Light {
                    is_on: m.state,
                    brightness,
                    color_temp: None,
                    rgb,
                    color_mode: None,
                },
            )
        }
        t if t == msg_types::FAN_STATE => {
            let m = FanStateResponse::decode(payload)?;
            let speed = if m.speed_level > 0 {
                Some(m.speed_level as u8)
            } else {
                None
            };
            (
                m.key,
                EntityState::Fan {
                    is_on: m.state,
                    speed,
                    oscillating: None,
                    direction: None,
                },
            )
        }
        t if t == msg_types::COVER_STATE => {
            let m = CoverStateResponse::decode(payload)?;
            let cover_state = match m.current_operation {
                1 => CoverState::Opening,
                2 => CoverState::Closing,
                _ => {
                    if m.position >= 0.99 {
                        CoverState::Open
                    } else if m.position <= 0.01 {
                        CoverState::Closed
                    } else {
                        CoverState::Stopped
                    }
                }
            };
            let position = Some((m.position * 100.0) as u8);
            (
                m.key,
                EntityState::Cover {
                    state: cover_state,
                    position,
                    tilt: None,
                },
            )
        }
        t if t == msg_types::NUMBER_STATE => {
            let m = NumberStateResponse::decode(payload)?;
            let value = if m.missing_state {
                0.0
            } else {
                f64::from(m.state)
            };
            // min/max/step/unit are only known from the ListEntities frame; use defaults
            (
                m.key,
                EntityState::Number {
                    value,
                    min: 0.0,
                    max: 100.0,
                    step: 1.0,
                    unit: None,
                },
            )
        }
        t if t == msg_types::SELECT_STATE => {
            let m = SelectStateResponse::decode(payload)?;
            (
                m.key,
                EntityState::Select {
                    current: if m.missing_state {
                        String::new()
                    } else {
                        m.state
                    },
                    options: vec![],
                },
            )
        }
        t if t == msg_types::TEXT_SENSOR_STATE => {
            let m = TextSensorStateResponse::decode(payload)?;
            (
                m.key,
                EntityState::TextSensor {
                    value: if m.missing_state {
                        String::new()
                    } else {
                        m.state
                    },
                },
            )
        }
        t if t == msg_types::CLIMATE_STATE => {
            let m = ClimateStateResponse::decode(payload)?;
            (
                m.key,
                EntityState::Climate {
                    mode: climate_int_to_mode(m.mode).to_string(),
                    current_temp: if m.current_temperature != 0.0 {
                        Some(f64::from(m.current_temperature))
                    } else {
                        None
                    },
                    target_temp: if m.target_temperature != 0.0 {
                        Some(f64::from(m.target_temperature))
                    } else {
                        None
                    },
                    hvac_action: None,
                },
            )
        }
        _ => return Ok(None),
    };

    Ok(Some(IngestedState {
        key,
        state,
        domain_id,
        feature_set,
    }))
}

fn climate_int_to_mode(mode: i32) -> &'static str {
    match mode {
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
    use prost::Message;

    #[test]
    fn parse_sensor_state_value() {
        let frame = SensorStateResponse {
            key: 42,
            state: 23.5,
            missing_state: false,
        }
        .encode_to_vec();
        let result = parse_state_frame(msg_types::SENSOR_STATE, &frame)
            .unwrap()
            .unwrap();
        assert_eq!(result.key, 42);
        assert!(
            matches!(result.state, EntityState::Sensor { value, .. } if (value - 23.5).abs() < 0.01)
        );
    }

    #[test]
    fn parse_sensor_missing_state_yields_zero() {
        let frame = SensorStateResponse {
            key: 1,
            state: 0.0,
            missing_state: true,
        }
        .encode_to_vec();
        let result = parse_state_frame(msg_types::SENSOR_STATE, &frame)
            .unwrap()
            .unwrap();
        assert!(matches!(result.state, EntityState::Sensor { value, .. } if value == 0.0));
    }

    #[test]
    fn parse_binary_sensor_on() {
        let frame = BinarySensorStateResponse {
            key: 99,
            state: true,
            missing_state: false,
        }
        .encode_to_vec();
        let result = parse_state_frame(msg_types::BINARY_SENSOR_STATE, &frame)
            .unwrap()
            .unwrap();
        assert_eq!(result.key, 99);
        assert!(matches!(
            result.state,
            EntityState::BinarySensor { is_on: true, .. }
        ));
    }

    #[test]
    fn parse_switch_state_off() {
        let frame = SwitchStateResponse {
            key: 7,
            state: false,
        }
        .encode_to_vec();
        let result = parse_state_frame(msg_types::SWITCH_STATE, &frame)
            .unwrap()
            .unwrap();
        assert!(matches!(result.state, EntityState::Switch { is_on: false }));
    }

    #[test]
    fn parse_light_brightness() {
        let frame = LightStateResponse {
            key: 5,
            state: true,
            brightness: 0.75,
            ..Default::default()
        }
        .encode_to_vec();
        let result = parse_state_frame(msg_types::LIGHT_STATE, &frame)
            .unwrap()
            .unwrap();
        assert!(
            matches!(result.state, EntityState::Light { brightness: Some(b), .. } if (b - 0.75).abs() < 0.01)
        );
    }

    #[test]
    fn parse_light_rgb() {
        let frame = LightStateResponse {
            key: 5,
            state: true,
            red: 1.0,
            green: 0.5,
            blue: 0.0,
            ..Default::default()
        }
        .encode_to_vec();
        let result = parse_state_frame(msg_types::LIGHT_STATE, &frame)
            .unwrap()
            .unwrap();
        assert!(matches!(
            result.state,
            EntityState::Light {
                rgb: Some([255, 127, 0]),
                ..
            }
        ));
    }

    #[test]
    fn parse_fan_with_speed() {
        let frame = FanStateResponse {
            key: 3,
            state: true,
            speed_level: 2,
            ..Default::default()
        }
        .encode_to_vec();
        let result = parse_state_frame(msg_types::FAN_STATE, &frame)
            .unwrap()
            .unwrap();
        assert!(matches!(
            result.state,
            EntityState::Fan {
                is_on: true,
                speed: Some(2),
                ..
            }
        ));
    }

    #[test]
    fn parse_cover_open() {
        let frame = CoverStateResponse {
            key: 10,
            position: 1.0,
            current_operation: 0,
            ..Default::default()
        }
        .encode_to_vec();
        let result = parse_state_frame(msg_types::COVER_STATE, &frame)
            .unwrap()
            .unwrap();
        assert!(matches!(
            result.state,
            EntityState::Cover {
                state: CoverState::Open,
                position: Some(100),
                ..
            }
        ));
    }

    #[test]
    fn parse_climate_heat_mode() {
        let frame = ClimateStateResponse {
            key: 8,
            mode: 3, // heat
            current_temperature: 20.5,
            target_temperature: 22.0,
            ..Default::default()
        }
        .encode_to_vec();
        let result = parse_state_frame(msg_types::CLIMATE_STATE, &frame)
            .unwrap()
            .unwrap();
        assert!(
            matches!(&result.state, EntityState::Climate { mode, target_temp: Some(t), .. }
                if mode == "heat" && (*t - 22.0).abs() < 0.01)
        );
    }

    #[test]
    fn parse_number_value() {
        let frame = NumberStateResponse {
            key: 11,
            state: 42.5,
            missing_state: false,
        }
        .encode_to_vec();
        let result = parse_state_frame(msg_types::NUMBER_STATE, &frame)
            .unwrap()
            .unwrap();
        assert!(
            matches!(result.state, EntityState::Number { value, .. } if (value - 42.5).abs() < 0.01)
        );
    }

    #[test]
    fn unknown_msg_type_returns_none() {
        let result = parse_state_frame(9999, &[]).unwrap();
        assert!(result.is_none());
    }
}
