use super::{DomainDef, DomainError};
use crate::entity_state::EntityState;
use crate::messages::EntityCommand;
use serde_json::Value;

pub struct Button;

impl DomainDef for Button {
    fn id(&self) -> &'static str {
        "button"
    }
    fn required_features(&self) -> &'static [&'static str] {
        &["press"]
    }
    fn optional_features(&self) -> &'static [&'static str] {
        &[]
    }
    fn device_classes(&self) -> &'static [&'static str] {
        &["restart", "update"]
    }
    fn services(&self, _features: &[String]) -> Vec<&'static str> {
        vec!["press"]
    }
    fn apply_command(
        &self,
        state: &EntityState,
        cmd: &EntityCommand,
    ) -> Result<EntityState, DomainError> {
        match (state, cmd) {
            (EntityState::Button, EntityCommand::PressButton) => Ok(EntityState::Button),
            _ => Err(DomainError::CommandNotApplicable {
                domain: "button",
                command: format!("{cmd:?}"),
            }),
        }
    }
    fn encode_command(&self, service: &str, _data: &Value) -> Option<EntityCommand> {
        match service {
            "press" => Some(EntityCommand::PressButton),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn press() {
        let result = Button
            .apply_command(&EntityState::Button, &EntityCommand::PressButton)
            .unwrap();
        assert_eq!(result, EntityState::Button);
    }

    #[test]
    fn rejects_turn_on() {
        assert!(Button
            .apply_command(&EntityState::Button, &EntityCommand::TurnOn)
            .is_err());
    }

    #[test]
    fn encode_press() {
        assert!(matches!(
            Button.encode_command("press", &Value::Null),
            Some(EntityCommand::PressButton)
        ));
    }
}
