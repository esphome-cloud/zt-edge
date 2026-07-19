use super::{read_only_apply, DomainDef, DomainError};
use crate::entity_state::EntityState;
use crate::messages::EntityCommand;
use serde_json::Value;

pub struct Event;

impl DomainDef for Event {
    fn id(&self) -> &'static str {
        "event"
    }
    fn required_features(&self) -> &'static [&'static str] {
        &["state"]
    }
    fn optional_features(&self) -> &'static [&'static str] {
        &[]
    }
    fn device_classes(&self) -> &'static [&'static str] {
        &["button", "doorbell", "motion"]
    }
    fn services(&self, _features: &[String]) -> Vec<&'static str> {
        vec![]
    }
    fn apply_command(
        &self,
        state: &EntityState,
        cmd: &EntityCommand,
    ) -> Result<EntityState, DomainError> {
        read_only_apply("event", state, cmd)
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
        let state = EntityState::Event {
            event_type: "click".to_string(),
            event_data: HashMap::new(),
        };
        assert!(Event.apply_command(&state, &EntityCommand::TurnOn).is_err());
    }
}
