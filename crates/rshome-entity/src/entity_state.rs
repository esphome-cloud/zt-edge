use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EntityState {
    Sensor {
        value: f64,
        unit: Option<String>,
        attributes: HashMap<String, Value>,
    },
    BinarySensor {
        is_on: bool,
        attributes: HashMap<String, Value>,
    },
    Switch {
        is_on: bool,
    },
    Number {
        value: f64,
        min: f64,
        max: f64,
        step: f64,
        unit: Option<String>,
    },
    Select {
        current: String,
        options: Vec<String>,
    },
    Text {
        value: String,
    },
    Button,
    Event {
        event_type: String,
        event_data: HashMap<String, Value>,
    },
    Light {
        is_on: bool,
        brightness: Option<f64>,
        color_temp: Option<u16>,
        rgb: Option<[u8; 3]>,
        color_mode: Option<String>,
    },
    Climate {
        mode: String,
        current_temp: Option<f64>,
        target_temp: Option<f64>,
        hvac_action: Option<String>,
    },
    Fan {
        is_on: bool,
        speed: Option<u8>,
        oscillating: Option<bool>,
        direction: Option<String>,
    },
    Cover {
        state: CoverState,
        position: Option<u8>,
        tilt: Option<u8>,
    },
    Lock {
        state: LockState,
    },
    MediaPlayer {
        state: MediaPlayerState,
        volume: Option<f64>,
        muted: Option<bool>,
        media_title: Option<String>,
    },
    AlarmControlPanel {
        state: AlarmState,
        code_format: Option<String>,
    },
    TextSensor {
        value: String,
    },
    Update {
        installed_version: String,
        latest_version: Option<String>,
        in_progress: bool,
    },
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CoverState {
    Open,
    Closed,
    Opening,
    Closing,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LockState {
    Locked,
    Unlocked,
    Locking,
    Unlocking,
    Jammed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MediaPlayerState {
    Idle,
    Playing,
    Paused,
    Buffering,
    Off,
    Standby,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AlarmState {
    Disarmed,
    ArmedHome,
    ArmedAway,
    Pending,
    Triggered,
}
