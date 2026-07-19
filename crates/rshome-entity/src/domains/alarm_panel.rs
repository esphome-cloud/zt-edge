use super::{read_only_apply, DomainDef, DomainError};
use crate::entity_state::EntityState;
use crate::messages::EntityCommand;
use serde_json::Value;

pub struct AlarmPanel;

impl DomainDef for AlarmPanel {
    fn id(&self) -> &'static str {
        "alarm_control_panel"
    }
    fn required_features(&self) -> &'static [&'static str] {
        &["state"]
    }
    fn optional_features(&self) -> &'static [&'static str] {
        &[]
    }
    fn device_classes(&self) -> &'static [&'static str] {
        &[]
    }
    fn services(&self, _features: &[String]) -> Vec<&'static str> {
        vec![]
    }
    fn apply_command(
        &self,
        state: &EntityState,
        cmd: &EntityCommand,
    ) -> Result<EntityState, DomainError> {
        read_only_apply("alarm_control_panel", state, cmd)
    }
    fn encode_command(&self, _service: &str, _data: &Value) -> Option<EntityCommand> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity_state::AlarmState;

    #[test]
    fn read_only() {
        let state = EntityState::AlarmControlPanel {
            state: AlarmState::Disarmed,
            code_format: None,
        };
        assert!(AlarmPanel
            .apply_command(&state, &EntityCommand::TurnOn)
            .is_err());
    }
}
