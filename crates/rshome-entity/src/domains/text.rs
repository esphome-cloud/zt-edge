use super::{DomainDef, DomainError};
use crate::entity_state::EntityState;
use crate::messages::EntityCommand;
use serde_json::Value;

pub struct Text;

impl DomainDef for Text {
    fn id(&self) -> &'static str {
        "text"
    }
    fn required_features(&self) -> &'static [&'static str] {
        &["state", "set_text"]
    }
    fn optional_features(&self) -> &'static [&'static str] {
        &[]
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
            (EntityState::Text { .. }, EntityCommand::SetText(t)) => {
                Ok(EntityState::Text { value: t.clone() })
            }
            _ => Err(DomainError::CommandNotApplicable {
                domain: "text",
                command: format!("{cmd:?}"),
            }),
        }
    }
    fn encode_command(&self, service: &str, data: &Value) -> Option<EntityCommand> {
        match service {
            "set_value" => {
                let v = data.get("value")?.as_str()?.to_string();
                Some(EntityCommand::SetText(v))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_text() {
        let state = EntityState::Text {
            value: "old".to_string(),
        };
        let result = Text
            .apply_command(&state, &EntityCommand::SetText("new".to_string()))
            .unwrap();
        assert_eq!(
            result,
            EntityState::Text {
                value: "new".to_string()
            }
        );
    }

    #[test]
    fn encode_set_value() {
        let data = serde_json::json!({"value": "hello"});
        let cmd = Text.encode_command("set_value", &data).unwrap();
        assert!(matches!(cmd, EntityCommand::SetText(t) if t == "hello"));
    }
}
