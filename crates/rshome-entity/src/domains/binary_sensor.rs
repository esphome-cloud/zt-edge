use super::{read_only_apply, DomainDef, DomainError};
use crate::entity_state::EntityState;
use crate::messages::EntityCommand;
use serde_json::Value;

pub struct BinarySensor;

impl DomainDef for BinarySensor {
    fn id(&self) -> &'static str {
        "binary_sensor"
    }
    fn required_features(&self) -> &'static [&'static str] {
        &["state"]
    }
    fn optional_features(&self) -> &'static [&'static str] {
        &[]
    }
    fn device_classes(&self) -> &'static [&'static str] {
        &[
            "motion",
            "door",
            "window",
            "smoke",
            "moisture",
            "occupancy",
            "vibration",
        ]
    }
    fn services(&self, _features: &[String]) -> Vec<&'static str> {
        vec![]
    }
    fn apply_command(
        &self,
        state: &EntityState,
        cmd: &EntityCommand,
    ) -> Result<EntityState, DomainError> {
        read_only_apply("binary_sensor", state, cmd)
    }
    fn encode_command(&self, _service: &str, _data: &Value) -> Option<EntityCommand> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn read_only() {
        let state = EntityState::BinarySensor {
            is_on: true,
            attributes: HashMap::new(),
        };
        assert!(BinarySensor
            .apply_command(&state, &EntityCommand::TurnOn)
            .is_err());
    }
}
