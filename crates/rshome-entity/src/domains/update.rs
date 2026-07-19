use super::{read_only_apply, DomainDef, DomainError};
use crate::entity_state::EntityState;
use crate::messages::EntityCommand;
use serde_json::Value;

pub struct Update;

impl DomainDef for Update {
    fn id(&self) -> &'static str {
        "update"
    }
    fn required_features(&self) -> &'static [&'static str] {
        &["state"]
    }
    fn optional_features(&self) -> &'static [&'static str] {
        &[]
    }
    fn device_classes(&self) -> &'static [&'static str] {
        &["firmware"]
    }
    fn services(&self, _features: &[String]) -> Vec<&'static str> {
        vec![]
    }
    fn apply_command(
        &self,
        state: &EntityState,
        cmd: &EntityCommand,
    ) -> Result<EntityState, DomainError> {
        read_only_apply("update", state, cmd)
    }
    fn encode_command(&self, _service: &str, _data: &Value) -> Option<EntityCommand> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only() {
        let state = EntityState::Update {
            installed_version: "1.0".to_string(),
            latest_version: None,
            in_progress: false,
        };
        assert!(Update
            .apply_command(&state, &EntityCommand::TurnOn)
            .is_err());
    }
}
