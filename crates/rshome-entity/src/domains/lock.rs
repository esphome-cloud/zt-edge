use super::{DomainDef, DomainError};
use crate::entity_state::{EntityState, LockState};
use crate::messages::EntityCommand;
use serde_json::Value;

pub struct Lock;

impl DomainDef for Lock {
    fn id(&self) -> &'static str {
        "lock"
    }
    fn required_features(&self) -> &'static [&'static str] {
        &["lock", "unlock"]
    }
    fn optional_features(&self) -> &'static [&'static str] {
        &[]
    }
    fn device_classes(&self) -> &'static [&'static str] {
        &[]
    }
    fn services(&self, _features: &[String]) -> Vec<&'static str> {
        vec!["lock", "unlock"]
    }
    fn apply_command(
        &self,
        state: &EntityState,
        cmd: &EntityCommand,
    ) -> Result<EntityState, DomainError> {
        match (state, cmd) {
            (EntityState::Lock { .. }, EntityCommand::TurnOn) => Ok(EntityState::Lock {
                state: LockState::Locked,
            }),
            (EntityState::Lock { .. }, EntityCommand::TurnOff) => Ok(EntityState::Lock {
                state: LockState::Unlocked,
            }),
            _ => Err(DomainError::CommandNotApplicable {
                domain: "lock",
                command: format!("{cmd:?}"),
            }),
        }
    }
    fn encode_command(&self, service: &str, _data: &Value) -> Option<EntityCommand> {
        match service {
            "lock" => Some(EntityCommand::TurnOn),
            "unlock" => Some(EntityCommand::TurnOff),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_command() {
        let state = EntityState::Lock {
            state: LockState::Unlocked,
        };
        let result = Lock.apply_command(&state, &EntityCommand::TurnOn).unwrap();
        assert_eq!(
            result,
            EntityState::Lock {
                state: LockState::Locked
            }
        );
    }

    #[test]
    fn unlock_command() {
        let state = EntityState::Lock {
            state: LockState::Locked,
        };
        let result = Lock.apply_command(&state, &EntityCommand::TurnOff).unwrap();
        assert_eq!(
            result,
            EntityState::Lock {
                state: LockState::Unlocked
            }
        );
    }

    #[test]
    fn encode_lock() {
        assert!(matches!(
            Lock.encode_command("lock", &Value::Null),
            Some(EntityCommand::TurnOn)
        ));
    }

    #[test]
    fn encode_unlock() {
        assert!(matches!(
            Lock.encode_command("unlock", &Value::Null),
            Some(EntityCommand::TurnOff)
        ));
    }
}
