use super::{DomainDef, DomainError};
use crate::entity_state::EntityState;
use crate::messages::EntityCommand;
use serde_json::Value;

pub struct Fan;

impl DomainDef for Fan {
    fn id(&self) -> &'static str {
        "fan"
    }
    fn required_features(&self) -> &'static [&'static str] {
        &["state", "toggle"]
    }
    fn optional_features(&self) -> &'static [&'static str] {
        &["speed", "oscillate", "direction", "set_percentage"]
    }
    fn device_classes(&self) -> &'static [&'static str] {
        &[]
    }
    fn services(&self, _features: &[String]) -> Vec<&'static str> {
        vec!["turn_on", "turn_off", "toggle", "set_percentage"]
    }
    fn apply_command(
        &self,
        state: &EntityState,
        cmd: &EntityCommand,
    ) -> Result<EntityState, DomainError> {
        match (state, cmd) {
            (
                EntityState::Fan {
                    is_on,
                    oscillating,
                    direction,
                    ..
                },
                EntityCommand::SetFanSpeed(s),
            ) => Ok(EntityState::Fan {
                is_on: *is_on,
                speed: Some(*s),
                oscillating: *oscillating,
                direction: direction.clone(),
            }),
            (
                EntityState::Fan {
                    speed,
                    oscillating,
                    direction,
                    ..
                },
                EntityCommand::TurnOn,
            ) => Ok(EntityState::Fan {
                is_on: true,
                speed: *speed,
                oscillating: *oscillating,
                direction: direction.clone(),
            }),
            (
                EntityState::Fan {
                    speed,
                    oscillating,
                    direction,
                    ..
                },
                EntityCommand::TurnOff,
            ) => Ok(EntityState::Fan {
                is_on: false,
                speed: *speed,
                oscillating: *oscillating,
                direction: direction.clone(),
            }),
            _ => Err(DomainError::CommandNotApplicable {
                domain: "fan",
                command: format!("{cmd:?}"),
            }),
        }
    }
    fn encode_command(&self, service: &str, data: &Value) -> Option<EntityCommand> {
        match service {
            "turn_on" => Some(EntityCommand::TurnOn),
            "turn_off" => Some(EntityCommand::TurnOff),
            "toggle" => Some(EntityCommand::Toggle),
            "set_percentage" => {
                let pct = data.get("percentage")?.as_f64()?;
                let speed = (pct / 100.0 * 255.0).round() as u8;
                Some(EntityCommand::SetFanSpeed(speed))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fan_off() -> EntityState {
        EntityState::Fan {
            is_on: false,
            speed: None,
            oscillating: None,
            direction: None,
        }
    }

    #[test]
    fn turn_on() {
        let result = Fan
            .apply_command(&fan_off(), &EntityCommand::TurnOn)
            .unwrap();
        match result {
            EntityState::Fan { is_on, .. } => assert!(is_on),
            _ => panic!("expected Fan"),
        }
    }

    #[test]
    fn set_speed() {
        let result = Fan
            .apply_command(&fan_off(), &EntityCommand::SetFanSpeed(128))
            .unwrap();
        match result {
            EntityState::Fan { speed, .. } => assert_eq!(speed, Some(128)),
            _ => panic!("expected Fan"),
        }
    }

    #[test]
    fn encode_set_percentage() {
        let data = serde_json::json!({"percentage": 50});
        let cmd = Fan.encode_command("set_percentage", &data).unwrap();
        assert!(matches!(cmd, EntityCommand::SetFanSpeed(s) if s == 128));
    }
}
