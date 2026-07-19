//! Entity type definitions for all ESPHome-compatible component entities.
//!
//! Every entity has common base fields (`EntityCommon`) plus entity-specific
//! fields.  The top-level `EntitySchema` enum is the primary dispatch type and
//! is the source of truth for JSON Schema generation via `schemars`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ── Entity common base ────────────────────────────────────────────────────────

/// Fields present on every ESPHome entity configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EntityCommon {
    /// Optional stable object ID for Home Assistant entity registry.
    pub id: Option<String>,
    /// Human-readable name shown in the Home Assistant UI.
    pub name: String,
    /// MDI icon name, e.g. `"mdi:thermometer"`.
    pub icon: Option<String>,
    /// When `true`, the entity is not exposed to Home Assistant.
    #[serde(default)]
    pub internal: bool,
    /// When `true`, the entity is hidden in the HA dashboard by default.
    #[serde(default)]
    pub disabled_by_default: bool,
    /// HA entity category controlling where the entity appears.
    #[serde(default)]
    pub entity_category: EntityCategory,
}

/// Home Assistant entity category.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EntityCategory {
    /// Regular entity (default).
    #[default]
    None,
    /// Configuration entity (shown in device config page).
    Config,
    /// Diagnostic entity (shown in device diagnostic page).
    Diagnostic,
}

// ── Entity type discriminant ──────────────────────────────────────────────────

/// Discriminant used in component definitions to indicate which entity category
/// a platform component produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    Sensor,
    BinarySensor,
    Switch,
    Number,
    Select,
    Text,
    Button,
    Event,
    Light,
    Climate,
    Fan,
    Cover,
    Lock,
    MediaPlayer,
    AlarmControlPanel,
    TextSensor,
}

// ── Sensor ────────────────────────────────────────────────────────────────────

/// Schema for `sensor` platform entities (read-only numeric state).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SensorSchema {
    #[serde(flatten)]
    pub common: EntityCommon,
    pub device_class: Option<SensorDeviceClass>,
    pub unit_of_measurement: Option<String>,
    pub accuracy_decimals: Option<u8>,
    pub state_class: Option<StateClass>,
    #[serde(default)]
    pub filters: Vec<FilterConfig>,
    pub force_update: Option<bool>,
    pub expire_after: Option<u32>,
}

/// Subset of Home Assistant sensor device classes (most common values).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SensorDeviceClass {
    ApparentPower,
    Aqi,
    AtmosphericPressure,
    Battery,
    CarbonDioxide,
    CarbonMonoxide,
    Current,
    DataRate,
    DataSize,
    Date,
    Distance,
    Duration,
    Energy,
    EnergyStorage,
    Enum,
    Frequency,
    Gas,
    Humidity,
    Illuminance,
    Irradiance,
    Moisture,
    Monetary,
    NitrogenDioxide,
    NitrogenMonoxide,
    NitrousOxide,
    Ozone,
    Ph,
    Pm1,
    Pm25,
    Pm10,
    Power,
    PowerFactor,
    Precipitation,
    PrecipitationIntensity,
    Pressure,
    ReactivePower,
    SignalStrength,
    SoundPressure,
    Speed,
    SulphurDioxide,
    Temperature,
    Timestamp,
    VolatileOrganicCompounds,
    VolatileOrganicCompoundsPartsPerBillion,
    Voltage,
    Volume,
    VolumeFlowRate,
    VolumeStorage,
    Water,
    Weight,
    WindSpeed,
}

/// Home Assistant state class for sensor history tracking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StateClass {
    Measurement,
    Total,
    TotalIncreasing,
}

/// ESPHome sensor filter configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FilterConfig {
    /// Simple moving average over a window of readings.
    SlidingWindowMovingAverage { window_size: u32, send_every: u32 },
    /// Exponential moving average.
    ExponentialMovingAverage { alpha: f32 },
    /// Lambda filter (arbitrary Rust expression — stored as string).
    Lambda { code: String },
    /// Throttle: emit at most once per interval (ms).
    Throttle { window_length_ms: u32 },
    /// Clamp to a min/max range.
    Clamp {
        min_value: Option<f32>,
        max_value: Option<f32>,
    },
    /// Calibrate linear: apply `y = slope * x + offset`.
    CalibrateLinear { slope: f32, offset: f32 },
    /// Multiply the value by a constant factor.
    Multiply { factor: f32 },
    /// Add an offset to every reading.
    Offset { offset: f32 },
    /// Skip the first N readings.
    Skip { num_first_readings: u32 },
    /// Debounce: only emit after value stays stable for the given time (ms).
    Debounce { time_ms: u32 },
}

// ── Binary Sensor ─────────────────────────────────────────────────────────────

/// Schema for `binary_sensor` platform entities (on/off state).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BinarySensorSchema {
    #[serde(flatten)]
    pub common: EntityCommon,
    pub device_class: Option<BinarySensorDeviceClass>,
    #[serde(default)]
    pub filters: Vec<BinaryFilterConfig>,
    pub publish_initial_state: Option<bool>,
}

/// Home Assistant binary sensor device classes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BinarySensorDeviceClass {
    Battery,
    BatteryCharging,
    Co,
    Cold,
    Connectivity,
    Door,
    GarageDoor,
    Gas,
    Heat,
    Light,
    Lock,
    Moisture,
    Motion,
    Moving,
    Occupancy,
    Opening,
    Plug,
    Power,
    Presence,
    Problem,
    Running,
    Safety,
    Smoke,
    Sound,
    Tamper,
    Update,
    Vibration,
    Window,
}

/// Binary sensor filter types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BinaryFilterConfig {
    Invert,
    Delayed { delay_ms: u32 },
    Settle { delay_ms: u32 },
    Lambda { code: String },
    Autorepeat { delay_ms: u32, repeat_delay_ms: u32 },
}

// ── Switch ────────────────────────────────────────────────────────────────────

/// Schema for `switch` platform entities (writable on/off).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SwitchSchema {
    #[serde(flatten)]
    pub common: EntityCommon,
    pub device_class: Option<SwitchDeviceClass>,
    #[serde(default)]
    pub restore_mode: RestoreMode,
}

/// Home Assistant switch device classes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SwitchDeviceClass {
    Outlet,
    Switch,
}

/// How a switch/light/fan restores its state after a reboot.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RestoreMode {
    /// Restore the last state (default).
    #[default]
    RestoreDefaultOff,
    RestoreDefaultOn,
    AlwaysOff,
    AlwaysOn,
    RestoreInvertedDefaultOff,
    RestoreInvertedDefaultOn,
    Disabled,
}

// ── Number ────────────────────────────────────────────────────────────────────

/// Schema for `number` platform entities (writable numeric value).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NumberSchema {
    #[serde(flatten)]
    pub common: EntityCommon,
    pub min_value: f32,
    pub max_value: f32,
    pub step: f32,
    #[serde(default)]
    pub mode: NumberMode,
    pub unit_of_measurement: Option<String>,
    pub device_class: Option<SensorDeviceClass>,
}

/// How the number entity is displayed in the HA UI.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NumberMode {
    #[default]
    Auto,
    Box,
    Slider,
}

// ── Select ────────────────────────────────────────────────────────────────────

/// Schema for `select` platform entities (writable enum from a fixed option list).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SelectSchema {
    #[serde(flatten)]
    pub common: EntityCommon,
    pub options: Vec<String>,
}

// ── Text ──────────────────────────────────────────────────────────────────────

/// Schema for `text` platform entities (writable string value).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TextSchema {
    #[serde(flatten)]
    pub common: EntityCommon,
    #[serde(default)]
    pub mode: TextMode,
    pub min_length: Option<u32>,
    pub max_length: Option<u32>,
    pub pattern: Option<String>,
}

/// Text input display mode.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextMode {
    #[default]
    Text,
    Password,
}

// ── Button ────────────────────────────────────────────────────────────────────

/// Schema for `button` platform entities (stateless momentary action).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ButtonSchema {
    #[serde(flatten)]
    pub common: EntityCommon,
    pub device_class: Option<ButtonDeviceClass>,
}

/// Home Assistant button device classes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ButtonDeviceClass {
    Identify,
    Restart,
    Update,
}

// ── Event ─────────────────────────────────────────────────────────────────────

/// Schema for `event` platform entities (fire-and-forget state changes).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EventSchema {
    #[serde(flatten)]
    pub common: EntityCommon,
    pub event_types: Vec<String>,
    pub device_class: Option<EventDeviceClass>,
}

/// Home Assistant event device classes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventDeviceClass {
    Doorbell,
    Button,
    Motion,
}

// ── Light ─────────────────────────────────────────────────────────────────────

/// Schema for `light` platform entities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LightSchema {
    #[serde(flatten)]
    pub common: EntityCommon,
    pub light_type: LightType,
    pub default_transition_length: Option<u32>,
    pub gamma_correct: Option<f32>,
    pub restore_mode: Option<LightRestoreMode>,
}

/// Light hardware type, determining which colour channels are available.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LightType {
    Binary,
    Monochromatic,
    Rgb,
    RgbWw,
    RgbCct,
    ColdWarmWhite,
    Partition,
}

/// Light-specific restore mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LightRestoreMode {
    RestoreDefaultOff,
    RestoreDefaultOn,
    AlwaysOff,
    AlwaysOn,
    RestoreAndOff,
    RestoreAndOn,
    Disabled,
}

// ── Climate ───────────────────────────────────────────────────────────────────

/// Schema for `climate` platform entities (thermostat / HVAC control).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClimateSchema {
    #[serde(flatten)]
    pub common: EntityCommon,
    pub traits: ClimateTraitsConfig,
    pub visual: ClimateVisualConfig,
}

/// HVAC capability traits exposed to Home Assistant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClimateTraitsConfig {
    #[serde(default)]
    pub supported_modes: Vec<ClimateMode>,
    #[serde(default)]
    pub supported_fan_modes: Vec<ClimateFanMode>,
    #[serde(default)]
    pub supported_presets: Vec<ClimatePreset>,
    pub visual_min_temperature: f32,
    pub visual_max_temperature: f32,
    pub visual_temperature_step: f32,
    pub supports_two_point_target_temperature: bool,
}

/// Visual presentation options for the climate card.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClimateVisualConfig {
    pub min_temperature: Option<f32>,
    pub max_temperature: Option<f32>,
    pub temperature_step: Option<f32>,
}

/// Climate/HVAC operating modes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClimateMode {
    Off,
    HeatCool,
    Cool,
    Heat,
    FanOnly,
    Dry,
    Auto,
}

/// Climate fan speed modes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClimateFanMode {
    On,
    Off,
    Auto,
    Low,
    Medium,
    High,
    Middle,
    Focus,
    Diffuse,
    Quiet,
}

/// Climate preset scenes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClimatePreset {
    None,
    Home,
    Away,
    Boost,
    Comfort,
    Eco,
    Sleep,
    Activity,
}

// ── Fan ───────────────────────────────────────────────────────────────────────

/// Schema for `fan` platform entities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FanSchema {
    #[serde(flatten)]
    pub common: EntityCommon,
    pub speed_count: Option<u32>,
    pub has_oscillation: Option<bool>,
    pub has_direction: Option<bool>,
    pub restore_mode: Option<RestoreMode>,
}

// ── Cover ─────────────────────────────────────────────────────────────────────

/// Schema for `cover` platform entities (blinds, garage doors, etc.).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CoverSchema {
    #[serde(flatten)]
    pub common: EntityCommon,
    pub device_class: Option<CoverDeviceClass>,
    pub assumed_state: Option<bool>,
    pub has_position: Option<bool>,
    pub has_tilt: Option<bool>,
}

/// Home Assistant cover device classes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CoverDeviceClass {
    Awning,
    Blind,
    Curtain,
    Damper,
    Door,
    Garage,
    Gate,
    Shade,
    Shutter,
    Window,
}

// ── Lock ──────────────────────────────────────────────────────────────────────

/// Schema for `lock` platform entities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LockSchema {
    #[serde(flatten)]
    pub common: EntityCommon,
    pub assumed_state: Option<bool>,
    pub requires_code: Option<bool>,
}

// ── Media Player ──────────────────────────────────────────────────────────────

/// Schema for `media_player` platform entities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MediaPlayerSchema {
    #[serde(flatten)]
    pub common: EntityCommon,
    #[serde(default)]
    pub supported_features: Vec<MediaPlayerFeature>,
}

/// Media player capability flags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MediaPlayerFeature {
    Pause,
    Seek,
    VolumeSet,
    VolumeMute,
    PreviousTrack,
    NextTrack,
    TurnOn,
    TurnOff,
    PlayMedia,
    VolumeStep,
    SelectSource,
    Stop,
    ClearPlaylist,
    Play,
    ShuffleSet,
    SelectSoundMode,
    Browse,
    RepeatSet,
}

// ── Alarm Control Panel ───────────────────────────────────────────────────────

/// Schema for `alarm_control_panel` platform entities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AlarmControlPanelSchema {
    #[serde(flatten)]
    pub common: EntityCommon,
    #[serde(default)]
    pub supported_features: Vec<AlarmControlPanelFeature>,
    pub code_arm_required: Option<bool>,
    pub code_disarm_required: Option<bool>,
    pub code_format: Option<AlarmCodeFormat>,
}

/// Alarm control panel capabilities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AlarmControlPanelFeature {
    ArmHome,
    ArmAway,
    ArmNight,
    ArmVacation,
    ArmCustomBypass,
    TriggerAlarm,
}

/// Format of alarm PIN code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AlarmCodeFormat {
    Number,
    Text,
}

// ── Text Sensor ───────────────────────────────────────────────────────────────

/// Schema for `text_sensor` platform entities (read-only string state).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TextSensorSchema {
    #[serde(flatten)]
    pub common: EntityCommon,
    pub device_class: Option<TextSensorDeviceClass>,
    #[serde(default)]
    pub filters: Vec<TextFilterConfig>,
}

/// Home Assistant text sensor device classes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextSensorDeviceClass {
    Date,
    Enum,
    Timestamp,
}

/// Text sensor filter types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TextFilterConfig {
    ToLower,
    ToUpper,
    Append { str: String },
    Prepend { str: String },
    SubstituteMap { substitutions: Vec<[String; 2]> },
    Lambda { code: String },
}

// ── Top-level dispatch union ──────────────────────────────────────────────────

/// Top-level entity schema: a tagged union over all supported entity types.
///
/// The `"type"` field discriminates which variant is active.  This is the
/// primary source of truth for JSON Schema generation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EntitySchema {
    Sensor(SensorSchema),
    BinarySensor(BinarySensorSchema),
    Switch(SwitchSchema),
    Number(NumberSchema),
    Select(SelectSchema),
    Text(TextSchema),
    Button(ButtonSchema),
    Event(EventSchema),
    Light(LightSchema),
    Climate(ClimateSchema),
    Fan(FanSchema),
    Cover(CoverSchema),
    Lock(LockSchema),
    MediaPlayer(MediaPlayerSchema),
    AlarmControlPanel(AlarmControlPanelSchema),
    TextSensor(TextSensorSchema),
}

impl EntitySchema {
    /// Return the entity type discriminant for this schema.
    pub fn entity_type(&self) -> EntityType {
        match self {
            Self::Sensor(_) => EntityType::Sensor,
            Self::BinarySensor(_) => EntityType::BinarySensor,
            Self::Switch(_) => EntityType::Switch,
            Self::Number(_) => EntityType::Number,
            Self::Select(_) => EntityType::Select,
            Self::Text(_) => EntityType::Text,
            Self::Button(_) => EntityType::Button,
            Self::Event(_) => EntityType::Event,
            Self::Light(_) => EntityType::Light,
            Self::Climate(_) => EntityType::Climate,
            Self::Fan(_) => EntityType::Fan,
            Self::Cover(_) => EntityType::Cover,
            Self::Lock(_) => EntityType::Lock,
            Self::MediaPlayer(_) => EntityType::MediaPlayer,
            Self::AlarmControlPanel(_) => EntityType::AlarmControlPanel,
            Self::TextSensor(_) => EntityType::TextSensor,
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_common(name: &str) -> EntityCommon {
        EntityCommon {
            id: None,
            name: name.to_string(),
            icon: None,
            internal: false,
            disabled_by_default: false,
            entity_category: EntityCategory::None,
        }
    }

    #[test]
    fn entity_common_roundtrip() {
        let e = EntityCommon {
            id: Some("my_sensor".into()),
            name: "Living Room Temperature".into(),
            icon: Some("mdi:thermometer".into()),
            internal: false,
            disabled_by_default: true,
            entity_category: EntityCategory::Diagnostic,
        };
        let json = serde_json::to_string(&e).unwrap();
        let back: EntityCommon = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
    }

    #[test]
    fn sensor_schema_roundtrip() {
        let s = SensorSchema {
            common: make_common("Temperature"),
            device_class: Some(SensorDeviceClass::Temperature),
            unit_of_measurement: Some("°C".into()),
            accuracy_decimals: Some(1),
            state_class: Some(StateClass::Measurement),
            filters: vec![FilterConfig::SlidingWindowMovingAverage {
                window_size: 5,
                send_every: 1,
            }],
            force_update: None,
            expire_after: None,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: SensorSchema = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn binary_sensor_schema_roundtrip() {
        let b = BinarySensorSchema {
            common: make_common("Door"),
            device_class: Some(BinarySensorDeviceClass::Door),
            filters: vec![BinaryFilterConfig::Invert],
            publish_initial_state: Some(true),
        };
        let json = serde_json::to_string(&b).unwrap();
        let back: BinarySensorSchema = serde_json::from_str(&json).unwrap();
        assert_eq!(b, back);
    }

    #[test]
    fn switch_schema_roundtrip() {
        let s = SwitchSchema {
            common: make_common("Relay"),
            device_class: Some(SwitchDeviceClass::Outlet),
            restore_mode: RestoreMode::AlwaysOff,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: SwitchSchema = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn number_schema_roundtrip() {
        let n = NumberSchema {
            common: make_common("Brightness"),
            min_value: 0.0,
            max_value: 100.0,
            step: 1.0,
            mode: NumberMode::Slider,
            unit_of_measurement: Some("%".into()),
            device_class: None,
        };
        let json = serde_json::to_string(&n).unwrap();
        let back: NumberSchema = serde_json::from_str(&json).unwrap();
        assert_eq!(n, back);
    }

    #[test]
    fn select_schema_roundtrip() {
        let s = SelectSchema {
            common: make_common("Mode"),
            options: vec!["eco".into(), "comfort".into(), "boost".into()],
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: SelectSchema = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn button_schema_roundtrip() {
        let b = ButtonSchema {
            common: make_common("Restart"),
            device_class: Some(ButtonDeviceClass::Restart),
        };
        let json = serde_json::to_string(&b).unwrap();
        let back: ButtonSchema = serde_json::from_str(&json).unwrap();
        assert_eq!(b, back);
    }

    #[test]
    fn light_schema_roundtrip() {
        let l = LightSchema {
            common: make_common("Status LED"),
            light_type: LightType::Monochromatic,
            default_transition_length: Some(1000),
            gamma_correct: Some(2.8),
            restore_mode: Some(LightRestoreMode::RestoreDefaultOff),
        };
        let json = serde_json::to_string(&l).unwrap();
        let back: LightSchema = serde_json::from_str(&json).unwrap();
        assert_eq!(l, back);
    }

    #[test]
    fn climate_schema_roundtrip() {
        let c = ClimateSchema {
            common: make_common("Living Room"),
            traits: ClimateTraitsConfig {
                supported_modes: vec![ClimateMode::Off, ClimateMode::Heat, ClimateMode::Cool],
                supported_fan_modes: vec![
                    ClimateFanMode::Auto,
                    ClimateFanMode::Low,
                    ClimateFanMode::High,
                ],
                supported_presets: vec![ClimatePreset::Home, ClimatePreset::Away],
                visual_min_temperature: 10.0,
                visual_max_temperature: 35.0,
                visual_temperature_step: 0.5,
                supports_two_point_target_temperature: false,
            },
            visual: ClimateVisualConfig {
                min_temperature: None,
                max_temperature: None,
                temperature_step: None,
            },
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: ClimateSchema = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn fan_schema_roundtrip() {
        let f = FanSchema {
            common: make_common("Ceiling Fan"),
            speed_count: Some(3),
            has_oscillation: Some(true),
            has_direction: Some(true),
            restore_mode: None,
        };
        let json = serde_json::to_string(&f).unwrap();
        let back: FanSchema = serde_json::from_str(&json).unwrap();
        assert_eq!(f, back);
    }

    #[test]
    fn cover_schema_roundtrip() {
        let c = CoverSchema {
            common: make_common("Garage Door"),
            device_class: Some(CoverDeviceClass::Garage),
            assumed_state: Some(false),
            has_position: Some(true),
            has_tilt: None,
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: CoverSchema = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn lock_schema_roundtrip() {
        let l = LockSchema {
            common: make_common("Front Door"),
            assumed_state: Some(false),
            requires_code: Some(true),
        };
        let json = serde_json::to_string(&l).unwrap();
        let back: LockSchema = serde_json::from_str(&json).unwrap();
        assert_eq!(l, back);
    }

    #[test]
    fn text_sensor_schema_roundtrip() {
        let t = TextSensorSchema {
            common: make_common("WiFi Info"),
            device_class: None,
            filters: vec![TextFilterConfig::ToLower],
        };
        let json = serde_json::to_string(&t).unwrap();
        let back: TextSensorSchema = serde_json::from_str(&json).unwrap();
        assert_eq!(t, back);
    }

    #[test]
    fn entity_schema_dispatch_sensor() {
        let e = EntitySchema::Sensor(SensorSchema {
            common: make_common("Temp"),
            device_class: Some(SensorDeviceClass::Temperature),
            unit_of_measurement: Some("°C".into()),
            accuracy_decimals: Some(1),
            state_class: None,
            filters: vec![],
            force_update: None,
            expire_after: None,
        });
        assert_eq!(e.entity_type(), EntityType::Sensor);
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"type\":\"sensor\""));
    }

    #[test]
    fn entity_category_default_is_none() {
        let c: EntityCategory = serde_json::from_str("\"none\"").unwrap();
        assert_eq!(c, EntityCategory::None);
    }

    #[test]
    fn filter_config_lambda_roundtrip() {
        let f = FilterConfig::Lambda {
            code: "return x * 1.8 + 32.0;".into(),
        };
        let json = serde_json::to_string(&f).unwrap();
        let back: FilterConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(f, back);
    }

    #[test]
    fn filter_config_clamp_roundtrip() {
        let f = FilterConfig::Clamp {
            min_value: Some(0.0),
            max_value: Some(100.0),
        };
        let json = serde_json::to_string(&f).unwrap();
        let back: FilterConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(f, back);
    }

    #[test]
    fn text_filter_substitute_map_roundtrip() {
        let f = TextFilterConfig::SubstituteMap {
            substitutions: vec![["on".into(), "ON".into()], ["off".into(), "OFF".into()]],
        };
        let json = serde_json::to_string(&f).unwrap();
        let back: TextFilterConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(f, back);
    }

    #[test]
    fn event_schema_roundtrip() {
        let e = EventSchema {
            common: make_common("Doorbell"),
            event_types: vec!["press".into(), "long_press".into()],
            device_class: Some(EventDeviceClass::Doorbell),
        };
        let json = serde_json::to_string(&e).unwrap();
        let back: EventSchema = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
    }

    #[test]
    fn alarm_schema_roundtrip() {
        let a = AlarmControlPanelSchema {
            common: make_common("House Alarm"),
            supported_features: vec![
                AlarmControlPanelFeature::ArmHome,
                AlarmControlPanelFeature::ArmAway,
            ],
            code_arm_required: Some(true),
            code_disarm_required: Some(true),
            code_format: Some(AlarmCodeFormat::Number),
        };
        let json = serde_json::to_string(&a).unwrap();
        let back: AlarmControlPanelSchema = serde_json::from_str(&json).unwrap();
        assert_eq!(a, back);
    }

    #[test]
    fn media_player_schema_roundtrip() {
        let m = MediaPlayerSchema {
            common: make_common("Speaker"),
            supported_features: vec![
                MediaPlayerFeature::VolumeSet,
                MediaPlayerFeature::Play,
                MediaPlayerFeature::Pause,
            ],
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: MediaPlayerSchema = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn text_schema_roundtrip() {
        let t = TextSchema {
            common: make_common("SSID"),
            mode: TextMode::Text,
            min_length: Some(1),
            max_length: Some(32),
            pattern: None,
        };
        let json = serde_json::to_string(&t).unwrap();
        let back: TextSchema = serde_json::from_str(&json).unwrap();
        assert_eq!(t, back);
    }

    #[test]
    fn all_entity_types_have_correct_discriminant() {
        let cases: Vec<EntityType> = vec![
            EntityType::Sensor,
            EntityType::BinarySensor,
            EntityType::Switch,
            EntityType::Number,
            EntityType::Select,
            EntityType::Text,
            EntityType::Button,
            EntityType::Event,
            EntityType::Light,
            EntityType::Climate,
            EntityType::Fan,
            EntityType::Cover,
            EntityType::Lock,
            EntityType::MediaPlayer,
            EntityType::AlarmControlPanel,
            EntityType::TextSensor,
        ];
        assert_eq!(cases.len(), 16);
    }
}
