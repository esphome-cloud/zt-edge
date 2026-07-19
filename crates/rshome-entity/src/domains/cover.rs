use super::{DomainDef, DomainError};
use crate::entity_state::{CoverState, EntityState};
use crate::messages::EntityCommand;
use serde_json::Value;

pub struct Cover;

impl DomainDef for Cover {
    fn id(&self) -> &'static str {
        "cover"
    }
    fn required_features(&self) -> &'static [&'static str] {
        &["state"]
    }
    fn optional_features(&self) -> &'static [&'static str] {
        &[
            "position",
            "tilt",
            "open_cover",
            "close_cover",
            "set_cover_position",
        ]
    }
    fn device_classes(&self) -> &'static [&'static str] {
        &["blind", "curtain", "garage", "shutter"]
    }
    fn services(&self, _features: &[String]) -> Vec<&'static str> {
        vec!["open_cover", "close_cover", "set_cover_position"]
    }
    fn apply_command(
        &self,
        state: &EntityState,
        cmd: &EntityCommand,
    ) -> Result<EntityState, DomainError> {
        match (state, cmd) {
            (EntityState::Cover { tilt, .. }, EntityCommand::SetCoverPosition(p)) => {
                let cover_state = if *p == 100 {
                    CoverState::Open
                } else if *p == 0 {
                    CoverState::Closed
                } else {
                    CoverState::Stopped
                };
                Ok(EntityState::Cover {
                    state: cover_state,
                    position: Some(*p),
                    tilt: *tilt,
                })
            }
            _ => Err(DomainError::CommandNotApplicable {
                domain: "cover",
                command: format!("{cmd:?}"),
            }),
        }
    }
    fn encode_command(&self, service: &str, data: &Value) -> Option<EntityCommand> {
        match service {
            "open_cover" => Some(EntityCommand::SetCoverPosition(100)),
            "close_cover" => Some(EntityCommand::SetCoverPosition(0)),
            "set_cover_position" => {
                let pos = data.get("position")?.as_u64()? as u8;
                Some(EntityCommand::SetCoverPosition(pos))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cover_closed() -> EntityState {
        EntityState::Cover {
            state: CoverState::Closed,
            position: Some(0),
            tilt: None,
        }
    }

    #[test]
    fn open() {
        let result = Cover
            .apply_command(&cover_closed(), &EntityCommand::SetCoverPosition(100))
            .unwrap();
        match result {
            EntityState::Cover {
                state, position, ..
            } => {
                assert_eq!(state, CoverState::Open);
                assert_eq!(position, Some(100));
            }
            _ => panic!("expected Cover"),
        }
    }

    #[test]
    fn encode_open_cover() {
        let cmd = Cover.encode_command("open_cover", &Value::Null).unwrap();
        assert!(matches!(cmd, EntityCommand::SetCoverPosition(100)));
    }

    #[test]
    fn encode_close_cover() {
        let cmd = Cover.encode_command("close_cover", &Value::Null).unwrap();
        assert!(matches!(cmd, EntityCommand::SetCoverPosition(0)));
    }

    #[test]
    fn encode_set_position() {
        let data = serde_json::json!({"position": 50});
        let cmd = Cover.encode_command("set_cover_position", &data).unwrap();
        assert!(matches!(cmd, EntityCommand::SetCoverPosition(50)));
    }
}
