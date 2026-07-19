use super::{DomainDef, DomainError};
use crate::entity_state::EntityState;
use crate::messages::EntityCommand;
use serde_json::Value;

pub struct Number;

impl DomainDef for Number {
    fn id(&self) -> &'static str {
        "number"
    }
    fn required_features(&self) -> &'static [&'static str] {
        &["state", "set_value"]
    }
    fn optional_features(&self) -> &'static [&'static str] {
        &["unit"]
    }
    fn device_classes(&self) -> &'static [&'static str] {
        &[]
    }
    fn services(&self, _features: &[String]) -> Vec<&'static str> {
        vec!["set_value"]
    }
    fn apply_command(
        &self,
        state: &EntityState,
        cmd: &EntityCommand,
    ) -> Result<EntityState, DomainError> {
        match (state, cmd) {
            (
                EntityState::Number {
                    min,
                    max,
                    step,
                    unit,
                    ..
                },
                EntityCommand::SetValue(v),
            ) => Ok(EntityState::Number {
                value: *v,
                min: *min,
                max: *max,
                step: *step,
                unit: unit.clone(),
            }),
            _ => Err(DomainError::CommandNotApplicable {
                domain: "number",
                command: format!("{cmd:?}"),
            }),
        }
    }
    fn encode_command(&self, service: &str, data: &Value) -> Option<EntityCommand> {
        match service {
            "set_value" => {
                let v = data.get("value")?.as_f64()?;
                Some(EntityCommand::SetValue(v))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn number_state() -> EntityState {
        EntityState::Number {
            value: 0.0,
            min: 0.0,
            max: 100.0,
            step: 1.0,
            unit: None,
        }
    }

    #[test]
    fn set_value() {
        let result = Number
            .apply_command(&number_state(), &EntityCommand::SetValue(42.0))
            .unwrap();
        match result {
            EntityState::Number { value, .. } => assert!((value - 42.0).abs() < 0.01),
            _ => panic!("expected Number"),
        }
    }

    #[test]
    fn rejects_turn_on() {
        assert!(Number
            .apply_command(&number_state(), &EntityCommand::TurnOn)
            .is_err());
    }

    #[test]
    fn encode_set_value() {
        let data = serde_json::json!({"value": 55.0});
        let cmd = Number.encode_command("set_value", &data).unwrap();
        assert!(matches!(cmd, EntityCommand::SetValue(v) if (v - 55.0).abs() < 0.01));
    }
}
