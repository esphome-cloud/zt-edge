use super::{DomainDef, DomainError};
use crate::entity_state::EntityState;
use crate::messages::EntityCommand;
use serde_json::Value;

pub struct Climate;

impl DomainDef for Climate {
    fn id(&self) -> &'static str {
        "climate"
    }
    fn required_features(&self) -> &'static [&'static str] {
        &["state", "set_mode"]
    }
    fn optional_features(&self) -> &'static [&'static str] {
        &["target_temp", "current_temp"]
    }
    fn device_classes(&self) -> &'static [&'static str] {
        &[]
    }
    fn services(&self, _features: &[String]) -> Vec<&'static str> {
        vec!["set_hvac_mode", "set_temperature"]
    }
    fn apply_command(
        &self,
        state: &EntityState,
        cmd: &EntityCommand,
    ) -> Result<EntityState, DomainError> {
        match (state, cmd) {
            (
                EntityState::Climate {
                    current_temp,
                    target_temp,
                    hvac_action,
                    ..
                },
                EntityCommand::SetClimateMode(mode),
            ) => Ok(EntityState::Climate {
                mode: mode.clone(),
                current_temp: *current_temp,
                target_temp: *target_temp,
                hvac_action: hvac_action.clone(),
            }),
            (
                EntityState::Climate {
                    mode,
                    current_temp,
                    hvac_action,
                    ..
                },
                EntityCommand::SetClimateTemp(t),
            ) => Ok(EntityState::Climate {
                mode: mode.clone(),
                current_temp: *current_temp,
                target_temp: Some(*t),
                hvac_action: hvac_action.clone(),
            }),
            _ => Err(DomainError::CommandNotApplicable {
                domain: "climate",
                command: format!("{cmd:?}"),
            }),
        }
    }
    fn encode_command(&self, service: &str, data: &Value) -> Option<EntityCommand> {
        match service {
            "set_temperature" => {
                let t = data.get("temperature")?.as_f64()?;
                Some(EntityCommand::SetClimateTemp(t))
            }
            "set_hvac_mode" => {
                let mode = data.get("hvac_mode")?.as_str()?.to_string();
                Some(EntityCommand::SetClimateMode(mode))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn climate_idle() -> EntityState {
        EntityState::Climate {
            mode: "off".to_string(),
            current_temp: Some(20.0),
            target_temp: None,
            hvac_action: None,
        }
    }

    #[test]
    fn set_mode() {
        let result = Climate
            .apply_command(
                &climate_idle(),
                &EntityCommand::SetClimateMode("heat".to_string()),
            )
            .unwrap();
        match result {
            EntityState::Climate { mode, .. } => assert_eq!(mode, "heat"),
            _ => panic!("expected Climate"),
        }
    }

    #[test]
    fn set_temp() {
        let result = Climate
            .apply_command(&climate_idle(), &EntityCommand::SetClimateTemp(25.0))
            .unwrap();
        match result {
            EntityState::Climate { target_temp, .. } => assert_eq!(target_temp, Some(25.0)),
            _ => panic!("expected Climate"),
        }
    }

    #[test]
    fn rejects_turn_on() {
        assert!(Climate
            .apply_command(&climate_idle(), &EntityCommand::TurnOn)
            .is_err());
    }

    #[test]
    fn encode_set_temperature() {
        let data = serde_json::json!({"temperature": 22.5});
        let cmd = Climate.encode_command("set_temperature", &data).unwrap();
        assert!(matches!(cmd, EntityCommand::SetClimateTemp(t) if (t - 22.5).abs() < 0.01));
    }

    #[test]
    fn encode_set_hvac_mode() {
        let data = serde_json::json!({"hvac_mode": "cool"});
        let cmd = Climate.encode_command("set_hvac_mode", &data).unwrap();
        assert!(matches!(cmd, EntityCommand::SetClimateMode(m) if m == "cool"));
    }
}
