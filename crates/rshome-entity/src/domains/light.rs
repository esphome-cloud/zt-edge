use super::{DomainDef, DomainError};
use crate::entity_state::EntityState;
use crate::messages::EntityCommand;
use serde_json::Value;

pub struct Light;

impl DomainDef for Light {
    fn id(&self) -> &'static str {
        "light"
    }
    fn required_features(&self) -> &'static [&'static str] {
        &["state", "toggle"]
    }
    fn optional_features(&self) -> &'static [&'static str] {
        &["brightness", "color_rgb", "color_temp"]
    }
    fn device_classes(&self) -> &'static [&'static str] {
        &[]
    }
    fn services(&self, _features: &[String]) -> Vec<&'static str> {
        vec!["turn_on", "turn_off", "toggle"]
    }
    fn apply_command(
        &self,
        state: &EntityState,
        cmd: &EntityCommand,
    ) -> Result<EntityState, DomainError> {
        match (state, cmd) {
            (
                EntityState::Light {
                    brightness,
                    color_temp,
                    rgb,
                    color_mode,
                    ..
                },
                EntityCommand::TurnOn,
            ) => Ok(EntityState::Light {
                is_on: true,
                brightness: *brightness,
                color_temp: *color_temp,
                rgb: *rgb,
                color_mode: color_mode.clone(),
            }),
            (
                EntityState::Light {
                    brightness,
                    color_temp,
                    rgb,
                    color_mode,
                    ..
                },
                EntityCommand::TurnOff,
            ) => Ok(EntityState::Light {
                is_on: false,
                brightness: *brightness,
                color_temp: *color_temp,
                rgb: *rgb,
                color_mode: color_mode.clone(),
            }),
            (
                EntityState::Light {
                    is_on,
                    brightness,
                    color_temp,
                    rgb,
                    color_mode,
                },
                EntityCommand::Toggle,
            ) => Ok(EntityState::Light {
                is_on: !is_on,
                brightness: *brightness,
                color_temp: *color_temp,
                rgb: *rgb,
                color_mode: color_mode.clone(),
            }),
            (
                EntityState::Light {
                    is_on,
                    color_temp,
                    rgb,
                    color_mode,
                    ..
                },
                EntityCommand::SetLightBrightness(b),
            ) => Ok(EntityState::Light {
                is_on: *is_on,
                brightness: Some(*b),
                color_temp: *color_temp,
                rgb: *rgb,
                color_mode: color_mode.clone(),
            }),
            (
                EntityState::Light {
                    is_on, brightness, ..
                },
                EntityCommand::SetLightColor { rgb, color_temp },
            ) => Ok(EntityState::Light {
                is_on: *is_on,
                brightness: *brightness,
                color_temp: *color_temp,
                rgb: *rgb,
                color_mode: None,
            }),
            _ => Err(DomainError::CommandNotApplicable {
                domain: "light",
                command: format!("{cmd:?}"),
            }),
        }
    }
    fn encode_command(&self, service: &str, data: &Value) -> Option<EntityCommand> {
        match service {
            "turn_on" => {
                if let Some(b) = data.get("brightness").and_then(|v| v.as_f64()) {
                    return Some(EntityCommand::SetLightBrightness(b / 255.0));
                }
                Some(EntityCommand::TurnOn)
            }
            "turn_off" => Some(EntityCommand::TurnOff),
            "toggle" => Some(EntityCommand::Toggle),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn light_off() -> EntityState {
        EntityState::Light {
            is_on: false,
            brightness: None,
            color_temp: None,
            rgb: None,
            color_mode: None,
        }
    }

    #[test]
    fn turn_on() {
        let result = Light
            .apply_command(&light_off(), &EntityCommand::TurnOn)
            .unwrap();
        match result {
            EntityState::Light { is_on, .. } => assert!(is_on),
            _ => panic!("expected Light"),
        }
    }

    #[test]
    fn set_brightness() {
        let result = Light
            .apply_command(&light_off(), &EntityCommand::SetLightBrightness(0.5))
            .unwrap();
        match result {
            EntityState::Light { brightness, .. } => assert_eq!(brightness, Some(0.5)),
            _ => panic!("expected Light"),
        }
    }

    #[test]
    fn encode_turn_on_with_brightness() {
        let data = serde_json::json!({"brightness": 128});
        let cmd = Light.encode_command("turn_on", &data).unwrap();
        match cmd {
            EntityCommand::SetLightBrightness(b) => {
                assert!((b - 128.0 / 255.0).abs() < 0.01);
            }
            _ => panic!("expected SetLightBrightness"),
        }
    }

    #[test]
    fn encode_turn_on_no_brightness() {
        assert!(matches!(
            Light.encode_command("turn_on", &Value::Null),
            Some(EntityCommand::TurnOn)
        ));
    }

    #[test]
    fn rejects_set_value() {
        assert!(Light
            .apply_command(&light_off(), &EntityCommand::SetValue(1.0))
            .is_err());
    }
}
