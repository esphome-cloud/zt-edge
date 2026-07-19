use super::{DomainDef, DomainError};
use crate::entity_state::EntityState;
use crate::messages::EntityCommand;
use serde_json::Value;

pub struct Switch;

impl DomainDef for Switch {
    fn id(&self) -> &'static str {
        "switch"
    }
    fn required_features(&self) -> &'static [&'static str] {
        &["state", "toggle"]
    }
    fn optional_features(&self) -> &'static [&'static str] {
        &[]
    }
    fn device_classes(&self) -> &'static [&'static str] {
        &["outlet", "switch"]
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
            (EntityState::Switch { .. }, EntityCommand::TurnOn) => {
                Ok(EntityState::Switch { is_on: true })
            }
            (EntityState::Switch { .. }, EntityCommand::TurnOff) => {
                Ok(EntityState::Switch { is_on: false })
            }
            (EntityState::Switch { is_on }, EntityCommand::Toggle) => {
                Ok(EntityState::Switch { is_on: !is_on })
            }
            _ => Err(DomainError::CommandNotApplicable {
                domain: "switch",
                command: format!("{cmd:?}"),
            }),
        }
    }
    fn encode_command(&self, service: &str, _data: &Value) -> Option<EntityCommand> {
        match service {
            "turn_on" => Some(EntityCommand::TurnOn),
            "turn_off" => Some(EntityCommand::TurnOff),
            "toggle" => Some(EntityCommand::Toggle),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_on() {
        let state = EntityState::Switch { is_on: false };
        let result = Switch
            .apply_command(&state, &EntityCommand::TurnOn)
            .unwrap();
        assert_eq!(result, EntityState::Switch { is_on: true });
    }

    #[test]
    fn toggle() {
        let state = EntityState::Switch { is_on: true };
        let result = Switch
            .apply_command(&state, &EntityCommand::Toggle)
            .unwrap();
        assert_eq!(result, EntityState::Switch { is_on: false });
    }

    #[test]
    fn rejects_set_value() {
        let state = EntityState::Switch { is_on: false };
        assert!(Switch
            .apply_command(&state, &EntityCommand::SetValue(1.0))
            .is_err());
    }

    #[test]
    fn encode_turn_on() {
        assert!(matches!(
            Switch.encode_command("turn_on", &Value::Null),
            Some(EntityCommand::TurnOn)
        ));
    }

    #[test]
    fn encode_unknown() {
        assert!(Switch.encode_command("set_value", &Value::Null).is_none());
    }
}
