use super::{read_only_apply, DomainDef, DomainError};
use crate::entity_state::EntityState;
use crate::messages::EntityCommand;
use serde_json::Value;

pub struct Sensor;

impl DomainDef for Sensor {
    fn id(&self) -> &'static str {
        "sensor"
    }
    fn required_features(&self) -> &'static [&'static str] {
        &["state"]
    }
    fn optional_features(&self) -> &'static [&'static str] {
        &["unit"]
    }
    fn device_classes(&self) -> &'static [&'static str] {
        &[
            "temperature",
            "humidity",
            "pressure",
            "illuminance",
            "power",
            "energy",
            "voltage",
            "current",
            "battery",
            "signal_strength",
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
        read_only_apply("sensor", state, cmd)
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
        let state = EntityState::Sensor {
            value: 22.0,
            unit: None,
            attributes: HashMap::new(),
        };
        assert!(Sensor
            .apply_command(&state, &EntityCommand::TurnOn)
            .is_err());
    }

    #[test]
    fn encode_returns_none() {
        assert!(Sensor.encode_command("anything", &Value::Null).is_none());
    }

    #[test]
    fn services_empty() {
        assert!(Sensor.services(&[]).is_empty());
    }
}
