use super::{DomainDef, DomainError};
use crate::entity_state::EntityState;
use crate::messages::EntityCommand;
use serde_json::Value;

pub struct Select;

impl DomainDef for Select {
    fn id(&self) -> &'static str {
        "select"
    }
    fn required_features(&self) -> &'static [&'static str] {
        &["state", "set_option"]
    }
    fn optional_features(&self) -> &'static [&'static str] {
        &[]
    }
    fn device_classes(&self) -> &'static [&'static str] {
        &[]
    }
    fn services(&self, _features: &[String]) -> Vec<&'static str> {
        vec!["select_option"]
    }
    fn apply_command(
        &self,
        state: &EntityState,
        cmd: &EntityCommand,
    ) -> Result<EntityState, DomainError> {
        match (state, cmd) {
            (EntityState::Select { options, .. }, EntityCommand::SetOption(opt)) => {
                Ok(EntityState::Select {
                    current: opt.clone(),
                    options: options.clone(),
                })
            }
            _ => Err(DomainError::CommandNotApplicable {
                domain: "select",
                command: format!("{cmd:?}"),
            }),
        }
    }
    fn encode_command(&self, service: &str, data: &Value) -> Option<EntityCommand> {
        match service {
            "select_option" => {
                let opt = data.get("option")?.as_str()?.to_string();
                Some(EntityCommand::SetOption(opt))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn select_state() -> EntityState {
        EntityState::Select {
            current: "a".to_string(),
            options: vec!["a".to_string(), "b".to_string()],
        }
    }

    #[test]
    fn set_option() {
        let result = Select
            .apply_command(&select_state(), &EntityCommand::SetOption("b".to_string()))
            .unwrap();
        match result {
            EntityState::Select { current, .. } => assert_eq!(current, "b"),
            _ => panic!("expected Select"),
        }
    }

    #[test]
    fn encode_select_option() {
        let data = serde_json::json!({"option": "b"});
        let cmd = Select.encode_command("select_option", &data).unwrap();
        assert!(matches!(cmd, EntityCommand::SetOption(o) if o == "b"));
    }
}
