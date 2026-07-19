use super::{DomainDef, DomainError};
use crate::entity_state::{EntityState, MediaPlayerState};
use crate::messages::EntityCommand;
use serde_json::Value;

pub struct MediaPlayer;

impl DomainDef for MediaPlayer {
    fn id(&self) -> &'static str {
        "media_player"
    }
    fn required_features(&self) -> &'static [&'static str] {
        &["state", "toggle"]
    }
    fn optional_features(&self) -> &'static [&'static str] {
        &["volume_set"]
    }
    fn device_classes(&self) -> &'static [&'static str] {
        &["speaker", "tv", "receiver"]
    }
    fn services(&self, _features: &[String]) -> Vec<&'static str> {
        vec!["turn_on", "turn_off", "toggle", "volume_set"]
    }
    fn apply_command(
        &self,
        state: &EntityState,
        cmd: &EntityCommand,
    ) -> Result<EntityState, DomainError> {
        match (state, cmd) {
            (
                EntityState::MediaPlayer {
                    volume,
                    muted,
                    media_title,
                    ..
                },
                EntityCommand::TurnOn,
            ) => Ok(EntityState::MediaPlayer {
                state: MediaPlayerState::Standby,
                volume: *volume,
                muted: *muted,
                media_title: media_title.clone(),
            }),
            (
                EntityState::MediaPlayer {
                    volume,
                    muted,
                    media_title,
                    ..
                },
                EntityCommand::TurnOff,
            ) => Ok(EntityState::MediaPlayer {
                state: MediaPlayerState::Off,
                volume: *volume,
                muted: *muted,
                media_title: media_title.clone(),
            }),
            _ => Err(DomainError::CommandNotApplicable {
                domain: "media_player",
                command: format!("{cmd:?}"),
            }),
        }
    }
    fn encode_command(&self, service: &str, data: &Value) -> Option<EntityCommand> {
        match service {
            "turn_on" => Some(EntityCommand::TurnOn),
            "turn_off" => Some(EntityCommand::TurnOff),
            "toggle" => Some(EntityCommand::Toggle),
            "volume_set" => {
                let vol = data.get("volume_level")?.as_f64()?;
                Some(EntityCommand::SetValue(vol))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mp_idle() -> EntityState {
        EntityState::MediaPlayer {
            state: MediaPlayerState::Idle,
            volume: None,
            muted: None,
            media_title: None,
        }
    }

    #[test]
    fn turn_on() {
        let result = MediaPlayer
            .apply_command(&mp_idle(), &EntityCommand::TurnOn)
            .unwrap();
        match result {
            EntityState::MediaPlayer { state, .. } => {
                assert_eq!(state, MediaPlayerState::Standby);
            }
            _ => panic!("expected MediaPlayer"),
        }
    }

    #[test]
    fn turn_off() {
        let result = MediaPlayer
            .apply_command(&mp_idle(), &EntityCommand::TurnOff)
            .unwrap();
        match result {
            EntityState::MediaPlayer { state, .. } => assert_eq!(state, MediaPlayerState::Off),
            _ => panic!("expected MediaPlayer"),
        }
    }

    #[test]
    fn encode_volume_set() {
        let data = serde_json::json!({"volume_level": 0.75});
        let cmd = MediaPlayer.encode_command("volume_set", &data).unwrap();
        assert!(matches!(cmd, EntityCommand::SetValue(v) if (v - 0.75).abs() < 0.01));
    }
}
