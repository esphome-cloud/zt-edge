//! Platform model — typed platform, target, capability, surface, and signal path types.
//!
//! This module is the foundation of the rshome-schema platform model. It defines
//! the type system for hardware platforms (ESP-IDF), chip targets, hardware capabilities,
//! I/O surfaces, signal paths, and component binding metadata.
//!
//! Key design decisions:
//! - All enums are `#[non_exhaustive]` to allow future extension without breaking changes.
//! - Surface enums (Input/Transform/Output/Feedback) represent the typed signal-flow model
//!   from the design dialogue Rounds 3-5.
//! - `SignalPath` is a first-class model object connecting source → transforms → sink → feedback.
//! - `PlatformCatalog` holds per-target capability profiles and supported surfaces.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ── Base type aliases ────────────────────────────────────────────────────────

pub type TargetId = String;
pub type PathId = String;

// ── Platform classification ──────────────────────────────────────────────────

/// Platform family. Currently only ESP-IDF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlatformKind {
    EspIdf,
}

/// Top-level tree classification for components.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlatformTree {
    /// ESPHome/IDF-style device components (sensors, switches, etc.)
    Device,
}

// ── Chip target ──────────────────────────────────────────────────────────────

/// Supported ESP32 chip targets. This is the authoritative definition;
/// `pin.rs` will be updated to import from here in Phase 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ChipTarget {
    Esp32,
    Esp32S2,
    Esp32S3,
    Esp32C2,
    Esp32C3,
    Esp32C5,
    Esp32C6,
    Esp32C61,
    Esp32H2,
    Esp32P4,
}

impl ChipTarget {
    /// Returns all known chip targets.
    pub fn all() -> &'static [ChipTarget] {
        &[
            ChipTarget::Esp32,
            ChipTarget::Esp32S2,
            ChipTarget::Esp32S3,
            ChipTarget::Esp32C2,
            ChipTarget::Esp32C3,
            ChipTarget::Esp32C5,
            ChipTarget::Esp32C6,
            ChipTarget::Esp32C61,
            ChipTarget::Esp32H2,
            ChipTarget::Esp32P4,
        ]
    }

    /// Returns the IDF target string (e.g. `"esp32s3"`) used in `sdkconfig.defaults`
    /// and `esp_board_manager` YAML.
    ///
    /// Note: this differs from serde serialization which uses snake_case (`"esp32_s3"`).
    pub fn to_idf_target(&self) -> &'static str {
        match self {
            ChipTarget::Esp32 => "esp32",
            ChipTarget::Esp32S2 => "esp32s2",
            ChipTarget::Esp32S3 => "esp32s3",
            ChipTarget::Esp32C2 => "esp32c2",
            ChipTarget::Esp32C3 => "esp32c3",
            ChipTarget::Esp32C5 => "esp32c5",
            ChipTarget::Esp32C6 => "esp32c6",
            ChipTarget::Esp32C61 => "esp32c61",
            ChipTarget::Esp32H2 => "esp32h2",
            ChipTarget::Esp32P4 => "esp32p4",
        }
    }
}

// ── Hardware capability ──────────────────────────────────────────────────────

/// Hardware capabilities available on a chip target or module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Capability {
    DualCoreCpu,
    SingleCoreCpu,
    Wifi,
    Ble,
    UsbOtg,
    UsbSerialJtag,
    Gptimer,
    Ledc,
    Mcpwm,
    Rmt,
    Pcnt,
    Gpio,
    Uart,
    I2c,
    Spi,
    I2s,
    Adc,
    Touch,
    TemperatureSensor,
    Lcd,
    Camera,
    AudioI2s,
    Thread,
    Zigbee,
    Psram,
    // ── Vehicle / gateway capabilities ──────────────────────────────────
    MotorControl,
    Imu,
    LongRange,
    Csi,
    FailsafeStop,
    ApSta,
    Bridge,
    EspNow,
    MeshLite,
    // ── Industrial bus capabilities ──────────────────────────────────
    CanBus,
    Rs485,
}

// ── Domain classification ───────────────────────────────────────────────────

/// Target domain for wizard scoping — determines which modules and solutions
/// are visible in the wizard's selection flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DomainKind {
    /// FPV cars, drones, boats, robotic vehicles.
    VehicleAircraftControl,
    /// Sensor hubs, monitoring, debugging tools.
    IotDeviceTooling,
    /// Edge compute, storage, home-lab infrastructure.
    HomeDataCenter,
    /// On-device inference, vision, voice processing.
    EdgeAi,
}

/// What role this MCU serves in the vehicle/aircraft system.
///
/// Each ESP32 in the system has a specific purpose. The wizard asks
/// "what is this MCU being compiled for?" and derives the applicable
/// solutions, communication chains, and pin assignments from the role.
///
/// `Ord`/`PartialOrd` derived so `BTreeMap<McuRole, _>` (used by
/// `TopologyMeta.role_compatibility`) iterates deterministically —
/// matches the precedent set by `ChipFamilyKind` for `chip_coverage`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum McuRole {
    /// Remote control TX: handheld joystick/buttons OR phone bridge (USB/BLE → 802.11 LR).
    RemoteControlTx,
    /// ESP32 gateway firmware bridging phone (BLE/USB) to vehicles (ESP-NOW / Wi-Fi Mesh / 802.11 LR / BLE-Mesh).
    /// Can relay for multiple vehicles; see doc §L3.
    SmartphoneGateway,
    /// Vehicle control board: motor/servo/RC input/failsafe. Control uplink only.
    ControlBoard,
    /// Vehicle control board + on-board MAVLink/WiFi telemetry back-channel.
    ControlTelemetryBoard,
    /// All-in-one: control + video + optional dual-mode relay.
    AllInOneCam,
    /// Dedicated video/streaming board, connects to control board via inter-board UART.
    VideoBoard,
    /// RC/WiFi LR/AP+STA receiver driving actuators directly; MCU assists telemetry/safety.
    ReceiverDirectDrive,
}

impl McuRole {
    /// Returns `true` if this role drives vehicle actuators (motors, servos,
    /// camera frame delivery) and is therefore subject to the actuator-focused
    /// V&A lints: failsafe presence, form-factor-families pinning, sensor-tier
    /// floor enforcement.
    ///
    /// Non-vehicle-bound roles (TX, gateway, video-only, passthrough) carry
    /// V&A metadata but participate in different invariants and are exempt
    /// from these lints. See `dag-current.md` §9 for the scoped-lint rationale.
    pub const fn is_vehicle_bound(self) -> bool {
        matches!(
            self,
            McuRole::ControlBoard | McuRole::ControlTelemetryBoard | McuRole::AllInOneCam
        )
    }
}

// Keep legacy type aliases for backward compatibility during migration.
/// Backward-compatible alias.
pub type ArchitectureTier = McuRole;
/// Backward-compatible alias.
pub type CommunicationChainKind = McuRole;

// ── Pin assignment ──────────────────────────────────────────────────────────

/// A GPIO pin assignment for a specific function within a solution.
///
/// Used to display interactive pin diagrams in the wizard and detect
/// pin conflicts at review time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PinAssignment {
    /// Human-readable function name, e.g. "Motor A PWM".
    pub function: String,
    /// Default GPIO number for this function.
    pub default_gpio: u8,
    /// Alternative GPIO numbers the user could reassign to.
    #[serde(default)]
    pub alternatives: Vec<u8>,
    /// The hardware capability this pin assignment relates to, e.g. "MotorControl".
    pub capability: String,
}

// ── Surface enums (typed signal-flow model) ──────────────────────────────────

/// Input surfaces — where data enters the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InputSurface {
    ButtonGpio,
    TouchPad,
    UartRx,
    UsbCdcCommand,
    RotaryEncoder,
    AdcVoltage,
    I2cSensor,
    SpiSensor,
    UartSensor,
    PulseCounter,
    CaptureSignal,
    TimerTick,
    InterruptEvent,
    WifiEvent,
    BleEvent,
    WakeupReason,
    CameraFrame,
    AudioInput,
    // ── Software-level input surfaces (Brookesia / HA integration) ──
    ServiceCall,
    ApiCommand,
    MqttMessage,
    EspNowData,
    // ── Industrial bus input surfaces ────────────────────────────────
    CanBusFrame,
    Rs485Data,
}

/// Transform/processing nodes in a signal path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TransformNode {
    UartProtocolParse,
    CommandDispatch,
    SensorFrameDecode,
    Debounce,
    Filter,
    Calibration,
    Normalization,
    Threshold,
    Mapping,
    StateMachine,
    PidLoop,
    SafetyInterlock,
    PeriodicTask,
    DelayedAction,
    PwmSchedule,
    CaptureCompare,
    OneToOne,
    OneToMany,
    ManyToOne,
    ClosedLoopFeedback,
    JpegEncode,
    AudioEncode,
    ProtobufEncode,
    // ── Industrial bus transforms ───────────────────────────────────
    CanFrameDecode,
    Rs485ProtocolDecode,
}

/// Output surfaces — where data leaves the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum OutputSurface {
    GpioLevel,
    LedcPwm,
    McpwmPwm,
    RmtWaveform,
    RelayDrive,
    MotorDrive,
    ServoDrive,
    BuzzerDrive,
    UartTx,
    UsbTx,
    I2cMasterWrite,
    SpiMasterWrite,
    WifiPacket,
    BlePacket,
    StatusLed,
    LcdFrame,
    AudioStream,
    NetworkApiState,
    HttpMjpegStream,
    RtspStream,
    EspNowData,
    // ── Industrial bus output surfaces ──────────────────────────────
    CanBusTx,
    Rs485Tx,
    SdCardWrite,
}

/// Feedback/observability surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FeedbackSurface {
    SerialLog,
    UsbSerialJtag,
    DebugPinToggle,
    RuntimeMetrics,
    LedIndicator,
    DisplayText,
    SoundFeedback,
    PhysicalMotion,
    ApiState,
    MqttPublish,
    WebStatus,
    BusErrorAlert,
}

// ── Component domain ─────────────────────────────────────────────────────────

/// Domain classification for components in the taxonomy tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ComponentDomain {
    EntityPlatforms,
    System,
    Connectivity,
    IoBuses,
    Sensors,
    BinaryInputs,
    ActuationUi,
    StorageSecurity,
    Diagnostics,
    CoreRuntime,
    Providers,
    Channels,
    Tools,
    RoomDelegation,
    Voice,
    Guards,
    Profiles,
}

// ── Component binding types ──────────────────────────────────────────────────

/// Binds a component to a platform, tree, domain, and set of supported targets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ComponentPlatformBinding {
    pub platform: PlatformKind,
    pub tree: PlatformTree,
    pub domain: ComponentDomain,
    pub taxonomy_path: Vec<String>,
    pub supported_targets: Vec<ChipTarget>,
}

/// Describes which I/O surfaces a component participates in.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct ComponentInteraction {
    pub input_surfaces: Vec<InputSurface>,
    pub transform_roles: Vec<TransformNode>,
    pub output_surfaces: Vec<OutputSurface>,
    pub feedback_surfaces: Vec<FeedbackSurface>,
}

// ── Signal path ──────────────────────────────────────────────────────────────

/// A step in a signal path with ordering and optional labeling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SignalPathStep<T> {
    pub order: u32,
    pub node: T,
    pub label: Option<String>,
    pub description: Option<String>,
}

/// A concrete signal path: source → transforms → sink → feedback.
///
/// This is the most original contribution of the platform model design —
/// it captures the real data flow through an embedded system, not just
/// a component inventory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SignalPath {
    pub id: PathId,
    pub name: String,
    pub source: InputSurface,
    pub transforms: Vec<SignalPathStep<TransformNode>>,
    pub sink: OutputSurface,
    pub feedback: Vec<SignalPathStep<FeedbackSurface>>,
    pub expected_user_result: String,
}

/// A signal path template with component bindings.
///
/// Templates connect the abstract signal-flow model to concrete components
/// in the registry. Solutions reference templates to declare their fixed
/// I/O paths.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SignalPathTemplate {
    pub id: String,
    pub name: String,
    pub source: InputSurface,
    pub transforms: Vec<TransformNode>,
    pub sink: OutputSurface,
    pub feedback: Vec<FeedbackSurface>,
    pub required_components: Vec<String>,
    pub optional_components: Vec<String>,
    pub expected_user_result: String,
}

// ── Platform catalog ─────────────────────────────────────────────────────────

/// Set of hardware capabilities for a target chip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CapabilityProfile {
    pub capabilities: Vec<Capability>,
}

/// Full definition of a platform target including capabilities and supported surfaces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PlatformTargetDefinition {
    pub id: TargetId,
    pub platform: PlatformKind,
    pub target: ChipTarget,
    pub capability_profile: CapabilityProfile,
    pub supported_inputs: Vec<InputSurface>,
    pub supported_transforms: Vec<TransformNode>,
    pub supported_outputs: Vec<OutputSurface>,
    pub supported_feedbacks: Vec<FeedbackSurface>,
    pub feedback_paths: Vec<SignalPath>,
}

/// A platform definition holding all target definitions for that platform.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PlatformDefinition {
    pub kind: PlatformKind,
    pub targets: Vec<PlatformTargetDefinition>,
}

/// Top-level catalog of all platforms and their target definitions.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct PlatformCatalog {
    pub platforms: Vec<PlatformDefinition>,
}

impl PlatformCatalog {
    /// Returns a pre-populated catalog for ESP-IDF with profiles for
    /// ESP32, ESP32-S3, and ESP32-C6.
    pub fn esp_idf_default() -> Self {
        use FeedbackSurface::*;
        use InputSurface::*;
        use OutputSurface::*;
        use TransformNode::*;

        let esp32 = PlatformTargetDefinition {
            id: "esp32".into(),
            platform: PlatformKind::EspIdf,
            target: ChipTarget::Esp32,
            capability_profile: CapabilityProfile {
                capabilities: vec![
                    Capability::DualCoreCpu,
                    Capability::Wifi,
                    Capability::Ble,
                    Capability::Gptimer,
                    Capability::Ledc,
                    Capability::Mcpwm,
                    Capability::Rmt,
                    Capability::Pcnt,
                    Capability::Gpio,
                    Capability::Uart,
                    Capability::I2c,
                    Capability::Spi,
                    Capability::I2s,
                    Capability::Adc,
                    Capability::Touch,
                    Capability::TemperatureSensor,
                    Capability::EspNow,
                ],
            },
            supported_inputs: vec![
                ButtonGpio,
                TouchPad,
                UartRx,
                AdcVoltage,
                I2cSensor,
                SpiSensor,
                UartSensor,
                PulseCounter,
                CaptureSignal,
                TimerTick,
                InterruptEvent,
                WifiEvent,
                BleEvent,
                WakeupReason,
                ServiceCall,
                ApiCommand,
                MqttMessage,
                InputSurface::EspNowData,
            ],
            supported_transforms: vec![
                UartProtocolParse,
                CommandDispatch,
                SensorFrameDecode,
                Debounce,
                Filter,
                Calibration,
                Normalization,
                Threshold,
                Mapping,
                StateMachine,
                PidLoop,
                SafetyInterlock,
                PeriodicTask,
                DelayedAction,
                PwmSchedule,
                CaptureCompare,
                OneToOne,
                OneToMany,
                ManyToOne,
                ClosedLoopFeedback,
            ],
            supported_outputs: vec![
                GpioLevel,
                LedcPwm,
                McpwmPwm,
                RmtWaveform,
                RelayDrive,
                MotorDrive,
                ServoDrive,
                BuzzerDrive,
                UartTx,
                I2cMasterWrite,
                SpiMasterWrite,
                WifiPacket,
                BlePacket,
                StatusLed,
                NetworkApiState,
                OutputSurface::EspNowData,
            ],
            supported_feedbacks: vec![
                SerialLog,
                DebugPinToggle,
                RuntimeMetrics,
                LedIndicator,
                ApiState,
                MqttPublish,
                WebStatus,
            ],
            feedback_paths: vec![],
        };

        let esp32s3 = PlatformTargetDefinition {
            id: "esp32s3".into(),
            platform: PlatformKind::EspIdf,
            target: ChipTarget::Esp32S3,
            capability_profile: CapabilityProfile {
                capabilities: vec![
                    Capability::DualCoreCpu,
                    Capability::Wifi,
                    Capability::Ble,
                    Capability::UsbOtg,
                    Capability::UsbSerialJtag,
                    Capability::Gptimer,
                    Capability::Ledc,
                    Capability::Mcpwm,
                    Capability::Rmt,
                    Capability::Pcnt,
                    Capability::Gpio,
                    Capability::Uart,
                    Capability::I2c,
                    Capability::Spi,
                    Capability::I2s,
                    Capability::Adc,
                    Capability::Touch,
                    Capability::TemperatureSensor,
                    Capability::Lcd,
                    Capability::Camera,
                    Capability::AudioI2s,
                    Capability::Psram,
                    Capability::EspNow,
                ],
            },
            supported_inputs: vec![
                ButtonGpio,
                TouchPad,
                UartRx,
                UsbCdcCommand,
                RotaryEncoder,
                AdcVoltage,
                I2cSensor,
                SpiSensor,
                UartSensor,
                PulseCounter,
                CaptureSignal,
                TimerTick,
                InterruptEvent,
                WifiEvent,
                BleEvent,
                WakeupReason,
                CameraFrame,
                AudioInput,
                ServiceCall,
                ApiCommand,
                MqttMessage,
                InputSurface::EspNowData,
            ],
            supported_transforms: vec![
                UartProtocolParse,
                CommandDispatch,
                SensorFrameDecode,
                Debounce,
                Filter,
                Calibration,
                Normalization,
                Threshold,
                Mapping,
                StateMachine,
                PidLoop,
                SafetyInterlock,
                PeriodicTask,
                DelayedAction,
                PwmSchedule,
                CaptureCompare,
                OneToOne,
                OneToMany,
                ManyToOne,
                ClosedLoopFeedback,
                JpegEncode,
                AudioEncode,
                ProtobufEncode,
            ],
            supported_outputs: vec![
                GpioLevel,
                LedcPwm,
                McpwmPwm,
                RmtWaveform,
                RelayDrive,
                MotorDrive,
                ServoDrive,
                BuzzerDrive,
                UartTx,
                UsbTx,
                I2cMasterWrite,
                SpiMasterWrite,
                WifiPacket,
                BlePacket,
                StatusLed,
                LcdFrame,
                AudioStream,
                NetworkApiState,
                HttpMjpegStream,
                RtspStream,
                OutputSurface::EspNowData,
            ],
            supported_feedbacks: vec![
                SerialLog,
                UsbSerialJtag,
                DebugPinToggle,
                RuntimeMetrics,
                LedIndicator,
                DisplayText,
                SoundFeedback,
                PhysicalMotion,
                ApiState,
                MqttPublish,
                WebStatus,
            ],
            feedback_paths: vec![SignalPath {
                id: "uart_command_to_motor".into(),
                name: "UART command to motor".into(),
                source: UartRx,
                transforms: vec![
                    SignalPathStep {
                        order: 1,
                        node: UartProtocolParse,
                        label: None,
                        description: None,
                    },
                    SignalPathStep {
                        order: 2,
                        node: Mapping,
                        label: None,
                        description: None,
                    },
                    SignalPathStep {
                        order: 3,
                        node: StateMachine,
                        label: None,
                        description: None,
                    },
                ],
                sink: McpwmPwm,
                feedback: vec![
                    SignalPathStep {
                        order: 1,
                        node: SerialLog,
                        label: None,
                        description: None,
                    },
                    SignalPathStep {
                        order: 2,
                        node: PhysicalMotion,
                        label: None,
                        description: None,
                    },
                ],
                expected_user_result:
                    "Motor speed or direction changes in response to serial commands".into(),
            }],
        };

        let esp32c6 = PlatformTargetDefinition {
            id: "esp32c6".into(),
            platform: PlatformKind::EspIdf,
            target: ChipTarget::Esp32C6,
            capability_profile: CapabilityProfile {
                capabilities: vec![
                    Capability::SingleCoreCpu,
                    Capability::Wifi,
                    Capability::Ble,
                    Capability::UsbSerialJtag,
                    Capability::Gptimer,
                    Capability::Ledc,
                    Capability::Rmt,
                    Capability::Pcnt,
                    Capability::Gpio,
                    Capability::Uart,
                    Capability::I2c,
                    Capability::Spi,
                    Capability::Adc,
                    Capability::TemperatureSensor,
                    Capability::Thread,
                    Capability::Zigbee,
                    Capability::EspNow,
                    Capability::MeshLite,
                ],
            },
            supported_inputs: vec![
                ButtonGpio,
                UartRx,
                AdcVoltage,
                I2cSensor,
                SpiSensor,
                UartSensor,
                PulseCounter,
                TimerTick,
                InterruptEvent,
                WifiEvent,
                BleEvent,
                WakeupReason,
                ServiceCall,
                ApiCommand,
                MqttMessage,
                InputSurface::EspNowData,
            ],
            supported_transforms: vec![
                UartProtocolParse,
                CommandDispatch,
                SensorFrameDecode,
                Debounce,
                Filter,
                Calibration,
                Normalization,
                Threshold,
                Mapping,
                StateMachine,
                PidLoop,
                SafetyInterlock,
                PeriodicTask,
                DelayedAction,
                PwmSchedule,
                OneToOne,
                OneToMany,
                ManyToOne,
                ClosedLoopFeedback,
            ],
            supported_outputs: vec![
                GpioLevel,
                LedcPwm,
                RmtWaveform,
                RelayDrive,
                UartTx,
                I2cMasterWrite,
                SpiMasterWrite,
                WifiPacket,
                BlePacket,
                StatusLed,
                NetworkApiState,
                OutputSurface::EspNowData,
            ],
            supported_feedbacks: vec![
                SerialLog,
                UsbSerialJtag,
                RuntimeMetrics,
                LedIndicator,
                ApiState,
                MqttPublish,
                WebStatus,
            ],
            feedback_paths: vec![],
        };

        PlatformCatalog {
            platforms: vec![PlatformDefinition {
                kind: PlatformKind::EspIdf,
                targets: vec![esp32, esp32s3, esp32c6],
            }],
        }
    }
}

// ── Vehicle & Aircraft Control — extended annotations ──────────────────────
//
/// Vehicle locomotion family — high-level grouping shown in the wizard's L1 picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FormFactorFamily {
    GroundWheeledReactive,
    GroundWheeledBalancing,
    GroundLegged,
    AquaticSurface,
    AquaticSubmerged,
    AerialMultirotor,
    AerialHelicopter,
    AerialFixedwing,
    AerialVtol,
    LighterThanAir,
    Articulated,
    Agricultural,
    Construction,
    Climbing,
    Amphibious,
    SoftContinuum,
    EducationalModular,
    JumpingHopping,
}

/// Specific vehicle form factor. Carried as a parameter into the selected
/// solution (see doc §L1 and the §"DAG edge summary"); **never** used as a
/// solution filter.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FormFactorKind {
    // Ground — wheeled reactive
    #[serde(rename = "wheeled_2wd_diff")]
    Wheeled2wdDiff,
    #[serde(rename = "wheeled_4wd_diff")]
    Wheeled4wdDiff,
    #[serde(rename = "wheeled_4wd_ackermann")]
    Wheeled4wdAckermann,
    #[serde(rename = "wheeled_6wd")]
    Wheeled6wd,
    #[serde(rename = "mecanum_4wheel")]
    Mecanum4wheel,
    #[serde(rename = "omniwheel_3wheel")]
    Omniwheel3wheel,
    #[serde(rename = "omniwheel_4wheel")]
    Omniwheel4wheel,
    BigfootMonsterTruck,
    BigfootRockCrawler,
    AtvOffroad,
    DriftRallyRacer,
    TrackedSkidsteer,
    // Ground — balancing
    #[serde(rename = "balance_2wheel")]
    Balance2wheel,
    BalanceUnicycle,
    Ballbot,
    // Ground — legged
    BipedHumanoid,
    Quadruped,
    Hexapod,
    Octopod,
    // Aquatic — surface
    BoatSingleRudder,
    BoatTwinPropDiff,
    Hovercraft,
    Hydrofoil,
    Sailboat,
    // Aquatic — submerged
    #[serde(rename = "rov_4thruster")]
    Rov4thruster,
    #[serde(rename = "rov_6thruster")]
    Rov6thruster,
    AuvTorpedo,
    // Aerial — multirotor
    QuadcopterX,
    QuadcopterPlus,
    Tricopter,
    Hexacopter,
    OctocopterX,
    OctocopterCoax,
    // Aerial — helicopter
    HeliSingleRotor,
    HeliCoaxial,
    HeliTandem,
    // Aerial — fixed-wing
    FixedwingStandard,
    FixedwingVtail,
    FlyingWing,
    Glider,
    // Aerial — VTOL / hybrid
    VtolTailsitter,
    VtolTiltrotor,
    VtolQuadplane,
    VtolBicopter,
    // Lighter-than-air
    LtaBlimp,
    LtaAirship,
    // Articulated
    SnakeSerpentine,
    WormModular,
    RollingBall,
    // ModularReconfigurable retired 2026-04-21 (va-residuals Phase 4 T4.2 / ADR-04):
    // orphan variant (no registry solution listed it) — removed to honor the
    // "enum = exhaustive set of supported cases" contract.
    // Agricultural
    AutonomousMower,
    SprayerSpot,
    TractorTowedImplement,
    // Construction
    ExcavatorArm,
    CraneBoom,
    SkidSteerLoader,
    // Climbing
    WallClimbingSuction,
    CableClimbing,
    MagneticClimber,
    // Amphibious
    AmphibiousWheelsPlusProp,
    // Soft / continuum
    SoftGripper,
    TentacleArm,
    // Educational
    ModularCubelets,
    // Jumping / hopping
    JumpingRobot,
    Grasshopper,
}

/// Minimum IMU / sensor tier — floored by form factor per doc §"Sensor-tier floor".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SensorTierKind {
    /// Accel + gyro — roll/pitch stable, yaw drifts.
    #[serde(rename = "basic_6ax")]
    Basic6ax,
    /// + magnetometer — heading stable once calibrated.
    #[serde(rename = "standard_9ax")]
    Standard9ax,
    /// + barometer — altitude / vertical velocity.
    #[serde(rename = "advanced_10ax")]
    Advanced10ax,
    /// + GPS / ToF / optical flow — autonomous nav.
    Research,
}

/// Non-IMU sensor requirement — orthogonal to `SensorTierKind` (which is IMU-centric).
/// Carried per-solution as `required_sensors: Vec<SensorRequirement>`. Added
/// 2026-04-21 by va-residuals Phase 3 T3.1 / ADR-06 to preserve semantic
/// distinctions that IMU tiers cannot express (GPS/RTK for agri, depth for
/// submerged, pressure/strain for soft robots, joint encoders for construction,
/// adhesion for climbing, water-contact for amphibious).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SensorRequirement {
    /// Global Navigation Satellite System — basic position fix. Outdoor agri + rovers.
    Gps,
    /// Real-Time Kinematic GPS — centimeter-accurate. Precision agri (sprayer), survey.
    GpsRtk,
    /// Depth / pressure sensor for aquatic-submerged (ROV / AUV).
    Depth,
    /// Pressure or strain gauges per chamber/segment (soft/continuum robots).
    PressureStrain,
    /// Per-joint encoders for multi-DOF articulated arms (excavator, crane, loader, legged).
    JointEncoder,
    /// Adhesion-state sensor for climbing (vacuum pressure, magnet current).
    Adhesion,
    /// Water-contact detection for amphibious mode-switching.
    WaterContact,
}

/// Wired inter-board link to a companion SBC (Pi / Jetson) — orthogonal to
/// the wireless `ControlUplinkKind`. Carried per-solution as
/// `companion_link: Option<CompanionLinkKind>`. Added 2026-04-21 by
/// va-residuals Phase 3 T3.2 / ADR-07. `None` = no SBC companion.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CompanionLinkKind {
    /// Point-to-point UART (typical for MAVLink bridge).
    Uart,
    /// CAN bus — preferred for multi-joint articulated arms.
    Can,
    /// I²C — for low-rate co-processor offloads.
    I2c,
}

/// Actuator family — fixed by the form factor, used to pick the mixing algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ActuatorFamily {
    BrushedHbridge,
    BrushlessEscPwm,
    BrushlessEscDshot,
    SteeringServo,
    MixedDiffDrive,
    MixedAckermann,
    QuadMix,
    HydraulicJoint,
    PneumaticChamber,
    TendonCable,
    ThrusterVector,
    HeliSwashplate,
    FixedwingSurfaces,
    VtolTransition,
}

/// Power rail scheme — drives the BOM and safety interlocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PowerRailKind {
    SingleLogicOnly,
    DualLogicMotor,
    TripleMotorServoLogic,
}

/// System topology — preset of compatibility filters over Role + Solution.
/// See doc §L2.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TopologyKind {
    DiyLowcost,
    StandardFpv,
    ResearchHybrid,
}

/// Firmware lineage for a solution. Used to cross-reference community docs
/// and tuning defaults. See doc §L4 family table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ImplementationFamily {
    EspDrone,
    Ardupilot,
    Px4,
    Betaflight,
    Custom,
    // Non-V&A lineages — added 2026-04-21 to retire the legacy free-text
    // `SolutionDefinition.implementation_family: Option<String>` field
    // (va-residuals Q11 full resolution).
    /// Espressif's managed-service framework (Brookesia). Used by
    /// `esp_now_sensor`, `mesh_lite_network`, home/IoT tooling.
    BrookesiaService,
    /// Espressif Audio Development Framework. Used by `phone_rtsp_av_solution`.
    EspAdf,
    /// Espressif IoT Solution reference library. Used by
    /// `phone_browser_video_solution`.
    EspIotSolution,
}

/// Medium + protocol used on the control uplink chain (TX → vehicle).
/// See doc §L5 control_uplink enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ControlUplinkKind {
    /// Receiver-only or passthrough solution — no uplink at this MCU.
    None,
    EspNow,
    WifiMesh,
    #[serde(rename = "wifi_80211lr")]
    Wifi80211lr,
    BleMesh,
    Crsf,
    Sbus,
    WifiMavlink,
    WifiCrtp,
    BleGatt,
    UsbCdc,
}

/// Video downlink medium. See doc §L5 video_downlink enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum VideoDownlinkKind {
    None,
    MjpegHttp,
    MjpegUart,
    AnalogVtx,
    DjiO4,
    Hdzero,
    Walksnail,
    WebrtcSbc,
}

/// Telemetry (back-channel) protocol. See doc §L5 telemetry enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TelemetryKind {
    None,
    MavlinkWifi,
    MavlinkUart,
    CrsfTelemetry,
    DshotTelemetry,
    CustomUart,
}

/// A single trigger that can cut motor output. ORed across a solution's
/// `killswitch_source` list. See doc §L5.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum KillswitchSource {
    RcSwitch,
    RxLoss,
    TimeoutNoPacket,
    EmergencyButton,
    SbcHeartbeatLoss,
    LowVoltage,
}

/// What the actuator does when no command arrives within the watchdog window.
/// See doc §L5.5 rx_loss_behavior enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RxLossBehavior {
    MotorCutoff,
    HoverHold,
    Rth,
    GlideTrim,
    Unpowered,
    PassthroughLast,
}

/// Hardware kill path — separate from the firmware loop. See doc §L5.5
/// emergency_stop_wiring enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EmergencyStopWiring {
    None,
    GpioPulldown,
    RelayCutoff,
    EscDshotCmd,
}

/// Per-solution failsafe policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FailsafeInfo {
    /// Triggers that may cut the motors. ORed; any trigger fires.
    #[serde(default)]
    pub killswitch_source: Vec<KillswitchSource>,
    /// Behaviour on control-link loss; `None` for solutions without a
    /// firmware watchdog (receiver_direct_drive, sbus_passthrough).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rx_loss_behavior: Option<RxLossBehavior>,
    /// No-command grace period before `rx_loss_behavior` fires; `None` for
    /// passthrough solutions that defer failsafe to the RX itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watchdog_ms: Option<u32>,
    /// Physical kill path separate from the firmware loop.
    pub emergency_stop_wiring: EmergencyStopWiring,
}

/// Chip family shorthand for the L3.5 coverage matrix. Matches
/// `ChipFamilyKind` in `types.ts`. `Ord` is required so a
/// `BTreeMap<ChipFamilyKind, ChipCoverageStatus>` gives deterministic
/// key order in the exported registry JSON.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ChipFamilyKind {
    Esp32D0wd,
    Esp32C6,
    Esp32S3,
}

/// How well a chip satisfies a given role, per doc §L3.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ChipCoverageStatus {
    Preferred,
    Caveat,
    Insufficient,
}

// ── L1 / L2 metadata mirror (PRD Task 1.2, closes master design §10.6) ──────
//
// `topologies.ts` + `form-factors.ts` ship the labels, descriptions, and
// role-compatibility tables that drive the wizard's L1 (form factor) and
// L2 (topology) cards. Pre-Phase-1 those tables lived ONLY in TS; this
// section is the Rust mirror so a Rust-only consumer (CLI / codegen
// preview / non-browser exporter) can render the same cards.
//
// Round-trip semantics: `default_topology_meta()` and
// `default_form_factor_meta()` serialize to JSON byte-equal to what the
// TS source emits (after `jq -S` normalization). The
// `va_metadata_parity.rs` lint enforces this.

/// Role-compatibility status under a given topology. Mirrors the TS
/// `TopologyRoleStatus` union. `Supported` is the default for any
/// (topology, role) cell not explicitly listed in
/// `TopologyMeta.role_compatibility`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TopologyRoleStatus {
    Supported,
    Warning,
    Hidden,
}

/// Catalog entry for one of the 3 V&A topologies — drives the wizard
/// L2 picker. Mirrors the TS `TopologyInfo` interface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TopologyMeta {
    pub id: TopologyKind,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_zh: Option<String>,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_zh: Option<String>,
    pub best_for: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best_for_zh: Option<String>,
    /// Per-role status under this topology. Roles not listed default to
    /// `Supported` at lookup time — but the constructor populates the
    /// full 7-entry table so the JSON shape matches TS byte-equal.
    pub role_compatibility: std::collections::BTreeMap<McuRole, TopologyRoleStatus>,
}

/// Catalog entry for one of the 64 V&A form factors — drives the
/// wizard L1 picker. Mirrors the TS `FormFactorInfo` interface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FormFactorMeta {
    pub id: FormFactorKind,
    pub family: FormFactorFamily,
    pub label: String,
    pub label_zh: String,
    pub description: String,
    pub description_zh: String,
    pub min_sensor_tier: SensorTierKind,
    /// `None` for legged / soft / educational / jumping form factors
    /// whose actuator topology lives at the joint level rather than a
    /// single family.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actuator_family: Option<ActuatorFamily>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<bool>,
}

fn topology_role_compat(
    overrides: &[(McuRole, TopologyRoleStatus)],
) -> std::collections::BTreeMap<McuRole, TopologyRoleStatus> {
    // Match the TS `mkCompat` helper: every role defaults to Supported,
    // overrides win.
    let mut out = std::collections::BTreeMap::new();
    for role in [
        McuRole::RemoteControlTx,
        McuRole::SmartphoneGateway,
        McuRole::ControlBoard,
        McuRole::ControlTelemetryBoard,
        McuRole::AllInOneCam,
        McuRole::VideoBoard,
        McuRole::ReceiverDirectDrive,
    ] {
        out.insert(role, TopologyRoleStatus::Supported);
    }
    for (role, status) in overrides {
        out.insert(*role, *status);
    }
    out
}

/// Authoritative metadata for the 3 V&A topologies. Closes master
/// design §10.6 — previously TS-only.
pub fn default_topology_meta() -> Vec<TopologyMeta> {
    vec![
        TopologyMeta {
            id: TopologyKind::DiyLowcost,
            label: "DIY / Low-cost".into(),
            label_zh: Some("DIY 低成本".into()),
            description: "ESP32 family + ESP-NOW or WiFi + H-bridge + MJPEG-over-HTTP.".into(),
            description_zh: Some("ESP32 系列 + ESP-NOW/WiFi + H 桥驱动 + MJPEG HTTP 视频。".into()),
            best_for: "Teaching, prototyping, indoor short-range, custom protocols.".into(),
            best_for_zh: Some("教学、原型验证、室内短距、自定义协议。".into()),
            role_compatibility: topology_role_compat(&[
                (McuRole::ReceiverDirectDrive, TopologyRoleStatus::Warning),
                (McuRole::VideoBoard, TopologyRoleStatus::Hidden),
            ]),
        },
        TopologyMeta {
            id: TopologyKind::StandardFpv,
            label: "Standard FPV".into(),
            label_zh: Some("标准 FPV".into()),
            description: "ELRS CRSF + brushless ESC (PWM/DShot) + analog 5.8GHz or DJI O4.".into(),
            description_zh: Some(
                "ELRS CRSF + 无刷电调(PWM/DShot) + 模拟 5.8GHz 或 DJI O4 数字图传。".into(),
            ),
            best_for: "Racing, long-range, professional feel, control/video decoupled.".into(),
            best_for_zh: Some("竞速、长距离、专业手感，控制与图传解耦。".into()),
            role_compatibility: topology_role_compat(&[
                (McuRole::ControlTelemetryBoard, TopologyRoleStatus::Warning),
                (McuRole::AllInOneCam, TopologyRoleStatus::Hidden),
                (McuRole::ReceiverDirectDrive, TopologyRoleStatus::Warning),
            ]),
        },
        TopologyMeta {
            id: TopologyKind::ResearchHybrid,
            label: "Research / Hybrid".into(),
            label_zh: Some("研究/混合".into()),
            description: "ELRS → MCU + SBC (Pi/Jetson) + UART/CAN inter-board + WebRTC/ROS.".into(),
            description_zh: Some("ELRS→MCU + SBC(Pi/Jetson) + 板间 UART/CAN + WebRTC/ROS。".into()),
            best_for: "AI vision, research, web remote, kept MCU safety layer.".into(),
            best_for_zh: Some("AI 视觉、研究、Web 远程控制，保留 MCU 安全层。".into()),
            role_compatibility: topology_role_compat(&[
                (McuRole::AllInOneCam, TopologyRoleStatus::Hidden),
                (McuRole::ReceiverDirectDrive, TopologyRoleStatus::Hidden),
            ]),
        },
    ]
}

/// Authoritative metadata for the 64 V&A form factors. Closes
/// master design §10.6 — previously TS-only in form-factors.ts.
pub fn default_form_factor_meta() -> Vec<FormFactorMeta> {
    vec![
        FormFactorMeta {
            id: FormFactorKind::Wheeled2wdDiff,
            family: FormFactorFamily::GroundWheeledReactive,
            label: "2WD differential".into(),
            label_zh: "两轮差速".into(),
            description: "2 driven wheels, skid-steer. Entry DIY car.".into(),
            description_zh: "两轮差速驱动，最简单的 DIY 小车入门方案。".into(),
            min_sensor_tier: SensorTierKind::Basic6ax,
            actuator_family: Some(ActuatorFamily::MixedDiffDrive),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::Wheeled4wdDiff,
            family: FormFactorFamily::GroundWheeledReactive,
            label: "4WD differential".into(),
            label_zh: "四轮差速".into(),
            description: "4 driven wheels, skid-steer. Rock-crawler base.".into(),
            description_zh: "四轮差速驱动，适合越野/攀爬车基础方案。".into(),
            min_sensor_tier: SensorTierKind::Basic6ax,
            actuator_family: Some(ActuatorFamily::MixedDiffDrive),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::Wheeled4wdAckermann,
            family: FormFactorFamily::GroundWheeledReactive,
            label: "4WD Ackermann".into(),
            label_zh: "阿克曼转向".into(),
            description: "ESC throttle + servo steering. Standard RC car.".into(),
            description_zh: "电调+舵机，典型 RC 小车结构。".into(),
            min_sensor_tier: SensorTierKind::Basic6ax,
            actuator_family: Some(ActuatorFamily::MixedAckermann),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::Wheeled6wd,
            family: FormFactorFamily::GroundWheeledReactive,
            label: "6WD rover".into(),
            label_zh: "六轮车".into(),
            description: "6-wheel skid-steer with optional rocker bogie.".into(),
            description_zh: "六轮差速，可选摇臂悬挂，适合星球车。".into(),
            min_sensor_tier: SensorTierKind::Basic6ax,
            actuator_family: Some(ActuatorFamily::MixedDiffDrive),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::Mecanum4wheel,
            family: FormFactorFamily::GroundWheeledReactive,
            label: "Mecanum (4 wheels)".into(),
            label_zh: "麦克纳姆轮(4轮)".into(),
            description: "Holonomic omnidirectional, strafes sideways.".into(),
            description_zh: "全向轮，可侧向平移，需要 9 轴 IMU 锁定航向。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::MixedDiffDrive),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::Omniwheel3wheel,
            family: FormFactorFamily::GroundWheeledReactive,
            label: "Omni-wheel (3 wheels)".into(),
            label_zh: "全向轮(3轮)".into(),
            description: "Holonomic triangle platform at 120°.".into(),
            description_zh: "120° 三角全向轮底盘。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::MixedDiffDrive),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::Omniwheel4wheel,
            family: FormFactorFamily::GroundWheeledReactive,
            label: "Omni-wheel (4 wheels)".into(),
            label_zh: "全向轮(4轮)".into(),
            description: "Holonomic at 90°. Less common than mecanum.".into(),
            description_zh: "90° 四轮全向，较 mecanum 少见。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::MixedDiffDrive),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::BigfootMonsterTruck,
            family: FormFactorFamily::GroundWheeledReactive,
            label: "Monster truck".into(),
            label_zh: "大脚车".into(),
            description: "Scaled 4WD with large tires and long suspension.".into(),
            description_zh: "放大版 4WD + 大胎 + 长行程悬挂。".into(),
            min_sensor_tier: SensorTierKind::Basic6ax,
            actuator_family: Some(ActuatorFamily::MixedAckermann),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::BigfootRockCrawler,
            family: FormFactorFamily::GroundWheeledReactive,
            label: "Rock crawler (4-wheel-steer)".into(),
            label_zh: "攀岩车(4轮转向)".into(),
            description: "4WD + 4-wheel-steer + articulated suspension.".into(),
            description_zh: "四驱 + 四轮转向 + 铰接悬挂，可横行。".into(),
            min_sensor_tier: SensorTierKind::Basic6ax,
            actuator_family: Some(ActuatorFamily::MixedAckermann),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::AtvOffroad,
            family: FormFactorFamily::GroundWheeledReactive,
            label: "ATV / offroad".into(),
            label_zh: "ATV 越野车".into(),
            description: "High-clearance Ackermann, large tires.".into(),
            description_zh: "高底盘阿克曼，大尺寸轮胎。".into(),
            min_sensor_tier: SensorTierKind::Basic6ax,
            actuator_family: Some(ActuatorFamily::MixedAckermann),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::DriftRallyRacer,
            family: FormFactorFamily::GroundWheeledReactive,
            label: "Drift / rally racer".into(),
            label_zh: "漂移/拉力赛车".into(),
            description: "Brushless Ackermann, low CG, often DShot.".into(),
            description_zh: "无刷阿克曼，低重心，常用 DShot 电调。".into(),
            min_sensor_tier: SensorTierKind::Basic6ax,
            actuator_family: Some(ActuatorFamily::BrushlessEscDshot),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::TrackedSkidsteer,
            family: FormFactorFamily::GroundWheeledReactive,
            label: "Tracked skid-steer (tank)".into(),
            label_zh: "履带坦克".into(),
            description: "2 tracks, tank-steer. Torque-dense.".into(),
            description_zh: "双履带，坦克转向，扭矩大。".into(),
            min_sensor_tier: SensorTierKind::Basic6ax,
            actuator_family: Some(ActuatorFamily::MixedDiffDrive),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::Balance2wheel,
            family: FormFactorFamily::GroundWheeledBalancing,
            label: "Self-balancing 2-wheel".into(),
            label_zh: "两轮自平衡车".into(),
            description: "Segway-style. Needs fast IMU loop (≥500 Hz).".into(),
            description_zh: "赛格威式平衡车，需要 500 Hz+ 快速 IMU 回环。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::MixedDiffDrive),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::BalanceUnicycle,
            family: FormFactorFamily::GroundWheeledBalancing,
            label: "Unicycle (self-balancing)".into(),
            label_zh: "自平衡独轮".into(),
            description: "1 wheel + reaction wheel or gyro. Control-heavy.".into(),
            description_zh: "单轮 + 反作用轮/陀螺，控制难度高。".into(),
            min_sensor_tier: SensorTierKind::Advanced10ax,
            actuator_family: Some(ActuatorFamily::MixedDiffDrive),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::Ballbot,
            family: FormFactorFamily::GroundWheeledBalancing,
            label: "Ballbot".into(),
            label_zh: "球形平衡机器人".into(),
            description: "Rolling sphere. Omnidirectional balance.".into(),
            description_zh: "球面滚动平衡，研究型。".into(),
            min_sensor_tier: SensorTierKind::Advanced10ax,
            actuator_family: Some(ActuatorFamily::MixedDiffDrive),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::BipedHumanoid,
            family: FormFactorFamily::GroundLegged,
            label: "Biped humanoid".into(),
            label_zh: "双足人形".into(),
            description: "Walking robot; high DOF; SBC runs IK.".into(),
            description_zh: "双足行走，自由度高，IK 由 SBC 承担。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: None,
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::Quadruped,
            family: FormFactorFamily::GroundLegged,
            label: "Quadruped".into(),
            label_zh: "四足".into(),
            description: "Spot-style; SBC + MCU split.".into(),
            description_zh: "四足（Spot 风格），SBC + MCU 分工。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: None,
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::Hexapod,
            family: FormFactorFamily::GroundLegged,
            label: "Hexapod".into(),
            label_zh: "六足".into(),
            description: "Stable gait; lookup-table gaits feasible on MCU.".into(),
            description_zh: "六足稳定步态，可用 MCU 查表步态。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: None,
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::Octopod,
            family: FormFactorFamily::GroundLegged,
            label: "Octopod".into(),
            label_zh: "八足".into(),
            description: "Specialty; SBC required.".into(),
            description_zh: "八足专用型，必须配 SBC。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: None,
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::BoatSingleRudder,
            family: FormFactorFamily::AquaticSurface,
            label: "Boat (1 prop + rudder)".into(),
            label_zh: "单桨舵面船".into(),
            description: "Traditional boat with servo rudder.".into(),
            description_zh: "单桨配舵机舵面的传统船型。".into(),
            min_sensor_tier: SensorTierKind::Basic6ax,
            actuator_family: Some(ActuatorFamily::MixedAckermann),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::BoatTwinPropDiff,
            family: FormFactorFamily::AquaticSurface,
            label: "Twin-prop boat (differential)".into(),
            label_zh: "双桨差速船".into(),
            description: "Two props, differential steering.".into(),
            description_zh: "双桨差速驱动的小艇。".into(),
            min_sensor_tier: SensorTierKind::Basic6ax,
            actuator_family: Some(ActuatorFamily::MixedDiffDrive),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::Hovercraft,
            family: FormFactorFamily::AquaticSurface,
            label: "Hovercraft".into(),
            label_zh: "气垫船".into(),
            description: "Lift fan + thrust fan + rudder. Amphibious.".into(),
            description_zh: "升力风扇 + 推进风扇 + 舵面，两栖能力。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::MixedAckermann),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::Hydrofoil,
            family: FormFactorFamily::AquaticSurface,
            label: "Hydrofoil".into(),
            label_zh: "水翼船".into(),
            description: "Prop + foil angle servos. Lifts at speed.".into(),
            description_zh: "推进桨 + 水翼角度舵机，高速起升。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::FixedwingSurfaces),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::Sailboat,
            family: FormFactorFamily::AquaticSurface,
            label: "Autonomous sailboat".into(),
            label_zh: "自主帆船".into(),
            description: "Sail servo + rudder servo. Wind-dependent.".into(),
            description_zh: "风帆舵机 + 舵面舵机，依赖风向。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::SteeringServo),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::Rov4thruster,
            family: FormFactorFamily::AquaticSubmerged,
            label: "ROV (4 thrusters)".into(),
            label_zh: "4 推进 ROV".into(),
            description: "Tethered ROV, 3-DOF (forward + vertical).".into(),
            description_zh: "有缆 ROV，前进+垂直共 3 自由度。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::ThrusterVector),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::Rov6thruster,
            family: FormFactorFamily::AquaticSubmerged,
            label: "ROV (6 thrusters, vectored)".into(),
            label_zh: "6 推进 ROV(矢量)".into(),
            description: "Holonomic underwater, 6-DOF control.".into(),
            description_zh: "水下全自由度 6 推进矢量 ROV。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::ThrusterVector),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::AuvTorpedo,
            family: FormFactorFamily::AquaticSubmerged,
            label: "AUV torpedo".into(),
            label_zh: "鱼雷式 AUV".into(),
            description: "1 prop + control fins. Autonomous sub.".into(),
            description_zh: "单桨 + 控制翼面的鱼雷式自主潜航器。".into(),
            min_sensor_tier: SensorTierKind::Research,
            actuator_family: Some(ActuatorFamily::ThrusterVector),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::QuadcopterX,
            family: FormFactorFamily::AerialMultirotor,
            label: "Quadcopter X".into(),
            label_zh: "四旋翼 X".into(),
            description: "4 rotors in X config. Default drone (flix reference).".into(),
            description_zh: "X 型四旋翼，flix 参考实现。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::QuadMix),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::QuadcopterPlus,
            family: FormFactorFamily::AerialMultirotor,
            label: "Quadcopter +".into(),
            label_zh: "四旋翼十字".into(),
            description: "4 rotors in + config. Classic layout.".into(),
            description_zh: "十字型四旋翼，经典布局。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::QuadMix),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::Tricopter,
            family: FormFactorFamily::AerialMultirotor,
            label: "Tricopter".into(),
            label_zh: "三旋翼".into(),
            description: "3 rotors + tail servo for yaw.".into(),
            description_zh: "三旋翼 + 尾舵舵机做偏航控制。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::QuadMix),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::Hexacopter,
            family: FormFactorFamily::AerialMultirotor,
            label: "Hexacopter".into(),
            label_zh: "六旋翼".into(),
            description: "6 rotors, redundancy against 1-motor failure.".into(),
            description_zh: "六旋翼，单桨失效冗余。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::QuadMix),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::OctocopterX,
            family: FormFactorFamily::AerialMultirotor,
            label: "Octocopter X".into(),
            label_zh: "八旋翼 X".into(),
            description: "8 rotors, X flat. Payload-carrying.".into(),
            description_zh: "扁平 X 八旋翼，载重型。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::QuadMix),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::OctocopterCoax,
            family: FormFactorFamily::AerialMultirotor,
            label: "Octocopter coaxial".into(),
            label_zh: "八旋翼同轴".into(),
            description: "4 up + 4 down coaxial. Compact footprint.".into(),
            description_zh: "4 上 4 下同轴八旋翼，占地小。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::QuadMix),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::HeliSingleRotor,
            family: FormFactorFamily::AerialHelicopter,
            label: "Helicopter (single rotor)".into(),
            label_zh: "单旋翼直升机".into(),
            description: "Main rotor + tail rotor. Collective pitch + swashplate.".into(),
            description_zh: "单主旋翼 + 尾桨，共轴变距 + 斜盘。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::HeliSwashplate),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::HeliCoaxial,
            family: FormFactorFamily::AerialHelicopter,
            label: "Coaxial helicopter".into(),
            label_zh: "共轴双桨".into(),
            description: "2 counter-rotating main rotors; no tail rotor.".into(),
            description_zh: "双主旋翼反向对转，无尾桨。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::HeliSwashplate),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::HeliTandem,
            family: FormFactorFamily::AerialHelicopter,
            label: "Tandem helicopter".into(),
            label_zh: "纵列双旋".into(),
            description: "2 rotors fore/aft. Chinook-style.".into(),
            description_zh: "前后纵列双旋翼（奇努克风格）。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::HeliSwashplate),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::FixedwingStandard,
            family: FormFactorFamily::AerialFixedwing,
            label: "Fixed-wing (standard)".into(),
            label_zh: "固定翼(标准)".into(),
            description: "Aileron + elevator + rudder + throttle.".into(),
            description_zh: "副翼 + 升降舵 + 方向舵 + 油门。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::FixedwingSurfaces),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::FixedwingVtail,
            family: FormFactorFamily::AerialFixedwing,
            label: "Fixed-wing V-tail".into(),
            label_zh: "V 尾固定翼".into(),
            description: "Ruddervator + aileron + throttle.".into(),
            description_zh: "V 尾舵 + 副翼 + 油门。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::FixedwingSurfaces),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::FlyingWing,
            family: FormFactorFamily::AerialFixedwing,
            label: "Flying wing".into(),
            label_zh: "飞翼".into(),
            description: "Elevons + throttle. No tail surface.".into(),
            description_zh: "升降副翼 + 油门，无尾翼。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::FixedwingSurfaces),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::Glider,
            family: FormFactorFamily::AerialFixedwing,
            label: "Glider".into(),
            label_zh: "滑翔机".into(),
            description: "Aileron + elevator (+ rudder). No throttle.".into(),
            description_zh: "副翼 + 升降（可选方向舵），无动力。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::FixedwingSurfaces),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::VtolTailsitter,
            family: FormFactorFamily::AerialVtol,
            label: "Tail-sitter VTOL".into(),
            label_zh: "尾坐式 VTOL".into(),
            description: "Pitches from hover to forward flight.".into(),
            description_zh: "从悬停姿态转为前飞的尾坐布局。".into(),
            min_sensor_tier: SensorTierKind::Advanced10ax,
            actuator_family: Some(ActuatorFamily::VtolTransition),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::VtolTiltrotor,
            family: FormFactorFamily::AerialVtol,
            label: "Tilt-rotor VTOL".into(),
            label_zh: "倾转旋翼 VTOL".into(),
            description: "Rotors tilt from vertical to horizontal.".into(),
            description_zh: "旋翼从垂直到水平倾转。".into(),
            min_sensor_tier: SensorTierKind::Advanced10ax,
            actuator_family: Some(ActuatorFamily::VtolTransition),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::VtolQuadplane,
            family: FormFactorFamily::AerialVtol,
            label: "Quadplane VTOL".into(),
            label_zh: "四轴带推进".into(),
            description: "Quadcopter + pusher/tractor prop.".into(),
            description_zh: "四旋翼 + 前推/拉桨。".into(),
            min_sensor_tier: SensorTierKind::Advanced10ax,
            actuator_family: Some(ActuatorFamily::VtolTransition),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::VtolBicopter,
            family: FormFactorFamily::AerialVtol,
            label: "Bicopter VTOL".into(),
            label_zh: "双轴倾转".into(),
            description: "2 tilting rotors.".into(),
            description_zh: "双旋翼倾转布局。".into(),
            min_sensor_tier: SensorTierKind::Advanced10ax,
            actuator_family: Some(ActuatorFamily::VtolTransition),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::LtaBlimp,
            family: FormFactorFamily::LighterThanAir,
            label: "Blimp".into(),
            label_zh: "小型飞艇".into(),
            description: "Envelope + thrusters + rudder.".into(),
            description_zh: "软式飞艇 + 推进器 + 舵面。".into(),
            min_sensor_tier: SensorTierKind::Basic6ax,
            actuator_family: Some(ActuatorFamily::ThrusterVector),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::LtaAirship,
            family: FormFactorFamily::LighterThanAir,
            label: "Airship (rigid/semi-rigid)".into(),
            label_zh: "硬式/半硬式飞艇".into(),
            description: "LTA with control surfaces.".into(),
            description_zh: "带控制舵面的大型飞艇。".into(),
            min_sensor_tier: SensorTierKind::Basic6ax,
            actuator_family: Some(ActuatorFamily::FixedwingSurfaces),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::SnakeSerpentine,
            family: FormFactorFamily::Articulated,
            label: "Snake / serpentine".into(),
            label_zh: "蛇形机器人".into(),
            description: "Modular segment servos; slither gait.".into(),
            description_zh: "模块段舵机，蜿蜒步态。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::TendonCable),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::WormModular,
            family: FormFactorFamily::Articulated,
            label: "Modular worm".into(),
            label_zh: "蠕动蠕虫".into(),
            description: "Peristaltic locomotion.".into(),
            description_zh: "蠕动式推进。".into(),
            min_sensor_tier: SensorTierKind::Basic6ax,
            actuator_family: Some(ActuatorFamily::PneumaticChamber),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::RollingBall,
            family: FormFactorFamily::Articulated,
            label: "Rolling ball".into(),
            label_zh: "球形机器人".into(),
            description: "Internal mass-shift or pendulum.".into(),
            description_zh: "内部质心偏移/摆锤驱动。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::TendonCable),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::AutonomousMower,
            family: FormFactorFamily::Agricultural,
            label: "Autonomous mower".into(),
            label_zh: "自动割草机".into(),
            description: "Diff-drive + blade motor + bumper. Boundary-guided.".into(),
            description_zh: "差速底盘 + 刀盘电机 + 碰撞检测，依赖边界定位。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::MixedDiffDrive),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::SprayerSpot,
            family: FormFactorFamily::Agricultural,
            label: "Spot sprayer".into(),
            label_zh: "精准喷洒车".into(),
            description: "Ackermann + pump + boom servos. GPS-guided.".into(),
            description_zh: "阿克曼 + 药泵 + 喷臂舵机，GPS 引导。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::MixedAckermann),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::TractorTowedImplement,
            family: FormFactorFamily::Agricultural,
            label: "Tractor + implement (full-size)".into(),
            label_zh: "带挂具全尺寸拖拉机".into(),
            description: "Heavy 4WD + PTO + 3-point hitch. Research-grade.".into(),
            description_zh: "重型 4WD + PTO + 三点悬挂，研究级。".into(),
            min_sensor_tier: SensorTierKind::Research,
            actuator_family: Some(ActuatorFamily::HydraulicJoint),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::ExcavatorArm,
            family: FormFactorFamily::Construction,
            label: "Excavator arm".into(),
            label_zh: "挖掘机臂".into(),
            description: "4+ hydraulic joints + swing base.".into(),
            description_zh: "4+ 液压关节 + 回转底座。".into(),
            min_sensor_tier: SensorTierKind::Advanced10ax,
            actuator_family: Some(ActuatorFamily::HydraulicJoint),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::CraneBoom,
            family: FormFactorFamily::Construction,
            label: "Crane boom".into(),
            label_zh: "吊臂".into(),
            description: "3 joints + hoist winch.".into(),
            description_zh: "3 关节 + 提升绞盘。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::HydraulicJoint),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::SkidSteerLoader,
            family: FormFactorFamily::Construction,
            label: "Skid-steer loader".into(),
            label_zh: "滑移装载机".into(),
            description: "Tracked skidsteer + bucket tilt/lift.".into(),
            description_zh: "履带底盘 + 铲斗升降/倾角。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::HydraulicJoint),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::WallClimbingSuction,
            family: FormFactorFamily::Climbing,
            label: "Wall climber (suction)".into(),
            label_zh: "吸盘爬壁".into(),
            description: "Vacuum-pad feet or continuous fan.".into(),
            description_zh: "真空吸盘脚或持续风扇吸附。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::PneumaticChamber),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::CableClimbing,
            family: FormFactorFamily::Climbing,
            label: "Cable climber".into(),
            label_zh: "缆绳攀爬器".into(),
            description: "Drive rollers clamping a cable.".into(),
            description_zh: "滚轮夹持缆绳攀爬。".into(),
            min_sensor_tier: SensorTierKind::Basic6ax,
            actuator_family: Some(ActuatorFamily::MixedDiffDrive),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::MagneticClimber,
            family: FormFactorFamily::Climbing,
            label: "Magnetic climber".into(),
            label_zh: "磁吸爬壁".into(),
            description: "Switchable electromagnets on ferrous structures.".into(),
            description_zh: "可切换电磁铁，用于铁磁结构。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::TendonCable),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::AmphibiousWheelsPlusProp,
            family: FormFactorFamily::Amphibious,
            label: "Amphibious (wheels + prop)".into(),
            label_zh: "两栖(轮+桨)".into(),
            description: "4WD land drive + dual aft props. Auto mode switch.".into(),
            description_zh: "四驱陆地 + 双后桨水上，自动切换模式。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: None,
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::SoftGripper,
            family: FormFactorFamily::SoftContinuum,
            label: "Soft gripper".into(),
            label_zh: "软体夹爪".into(),
            description: "Pneumatic chambers or tendon cables.".into(),
            description_zh: "气动腔体或腱绳驱动。".into(),
            min_sensor_tier: SensorTierKind::Basic6ax,
            actuator_family: Some(ActuatorFamily::PneumaticChamber),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::TentacleArm,
            family: FormFactorFamily::SoftContinuum,
            label: "Tentacle arm (continuum)".into(),
            label_zh: "触手臂(连续体)".into(),
            description: "Multi-segment cable-driven continuum. IK on SBC.".into(),
            description_zh: "多段绳驱连续体，IK 运行在 SBC。".into(),
            min_sensor_tier: SensorTierKind::Standard9ax,
            actuator_family: Some(ActuatorFamily::TendonCable),
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::ModularCubelets,
            family: FormFactorFamily::EducationalModular,
            label: "Modular cubelets".into(),
            label_zh: "模块化方块".into(),
            description: "Snap-connect blocks with I²C chain.".into(),
            description_zh: "可拼接方块 + I²C 总线菊花链。".into(),
            min_sensor_tier: SensorTierKind::Basic6ax,
            actuator_family: None,
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::JumpingRobot,
            family: FormFactorFamily::JumpingHopping,
            label: "Jumping robot".into(),
            label_zh: "弹跳机器人".into(),
            description: "Compressed spring + latch servo. Ballistic phase.".into(),
            description_zh: "压缩弹簧 + 锁舵机，弹道飞行阶段。".into(),
            min_sensor_tier: SensorTierKind::Advanced10ax,
            actuator_family: None,
            deprecated: None,
        },
        FormFactorMeta {
            id: FormFactorKind::Grasshopper,
            family: FormFactorFamily::JumpingHopping,
            label: "Grasshopper".into(),
            label_zh: "蚱蜢跳跃".into(),
            description: "Motor-wound torsion spring. Repeated hops.".into(),
            description_zh: "电机卷起扭簧反复弹跳。".into(),
            min_sensor_tier: SensorTierKind::Advanced10ax,
            actuator_family: None,
            deprecated: None,
        },
    ]
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chip_target_all_returns_10() {
        assert_eq!(ChipTarget::all().len(), 10);
    }

    #[test]
    fn chip_target_serde_round_trip() {
        for target in ChipTarget::all() {
            let json = serde_json::to_string(target).unwrap();
            let back: ChipTarget = serde_json::from_str(&json).unwrap();
            assert_eq!(*target, back);
        }
    }

    #[test]
    fn chip_target_snake_case_serialization() {
        assert_eq!(
            serde_json::to_string(&ChipTarget::Esp32S3).unwrap(),
            "\"esp32_s3\""
        );
        assert_eq!(
            serde_json::to_string(&ChipTarget::Esp32C6).unwrap(),
            "\"esp32_c6\""
        );
        assert_eq!(
            serde_json::to_string(&ChipTarget::Esp32P4).unwrap(),
            "\"esp32_p4\""
        );
    }

    #[test]
    fn platform_kind_serde() {
        let json = serde_json::to_string(&PlatformKind::EspIdf).unwrap();
        assert_eq!(json, "\"esp_idf\"");
        let back: PlatformKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, PlatformKind::EspIdf);
    }

    #[test]
    fn capability_serde_round_trip() {
        let cap = Capability::UsbOtg;
        let json = serde_json::to_string(&cap).unwrap();
        assert_eq!(json, "\"usb_otg\"");
        let back: Capability = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cap);
    }

    #[test]
    fn input_surface_serde() {
        let s = InputSurface::I2cSensor;
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, "\"i2c_sensor\"");
        let back: InputSurface = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn transform_node_serde() {
        let t = TransformNode::PidLoop;
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(json, "\"pid_loop\"");
        let back: TransformNode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn output_surface_serde() {
        let o = OutputSurface::HttpMjpegStream;
        let json = serde_json::to_string(&o).unwrap();
        assert_eq!(json, "\"http_mjpeg_stream\"");
        let back: OutputSurface = serde_json::from_str(&json).unwrap();
        assert_eq!(back, o);
    }

    #[test]
    fn feedback_surface_serde() {
        let f = FeedbackSurface::PhysicalMotion;
        let json = serde_json::to_string(&f).unwrap();
        assert_eq!(json, "\"physical_motion\"");
        let back: FeedbackSurface = serde_json::from_str(&json).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn component_domain_serde() {
        let d = ComponentDomain::RoomDelegation;
        let json = serde_json::to_string(&d).unwrap();
        assert_eq!(json, "\"room_delegation\"");
        let back: ComponentDomain = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn component_interaction_default_is_empty() {
        let ci = ComponentInteraction::default();
        assert!(ci.input_surfaces.is_empty());
        assert!(ci.transform_roles.is_empty());
        assert!(ci.output_surfaces.is_empty());
        assert!(ci.feedback_surfaces.is_empty());
    }

    #[test]
    fn component_platform_binding_serde() {
        let binding = ComponentPlatformBinding {
            platform: PlatformKind::EspIdf,
            tree: PlatformTree::Device,
            domain: ComponentDomain::Connectivity,
            taxonomy_path: vec!["device".into(), "connectivity".into(), "wifi".into()],
            supported_targets: vec![ChipTarget::Esp32, ChipTarget::Esp32S3, ChipTarget::Esp32C6],
        };
        let json = serde_json::to_string(&binding).unwrap();
        let back: ComponentPlatformBinding = serde_json::from_str(&json).unwrap();
        assert_eq!(back, binding);
    }

    #[test]
    fn signal_path_construction() {
        let path = SignalPath {
            id: "button_to_led".into(),
            name: "Button to LED".into(),
            source: InputSurface::ButtonGpio,
            transforms: vec![
                SignalPathStep {
                    order: 1,
                    node: TransformNode::Debounce,
                    label: Some("debounce 50ms".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: TransformNode::Mapping,
                    label: None,
                    description: Some("toggle on/off".into()),
                },
            ],
            sink: OutputSurface::LedcPwm,
            feedback: vec![SignalPathStep {
                order: 1,
                node: FeedbackSurface::LedIndicator,
                label: None,
                description: None,
            }],
            expected_user_result: "LED brightness toggles on button press".into(),
        };
        let json = serde_json::to_string(&path).unwrap();
        let back: SignalPath = serde_json::from_str(&json).unwrap();
        assert_eq!(back, path);
        assert_eq!(back.transforms.len(), 2);
        assert_eq!(back.feedback.len(), 1);
    }

    #[test]
    fn signal_path_template_serde() {
        let tmpl = SignalPathTemplate {
            id: "uart_command_to_motor".into(),
            name: "UART command to motor".into(),
            source: InputSurface::UartRx,
            transforms: vec![
                TransformNode::UartProtocolParse,
                TransformNode::Mapping,
                TransformNode::StateMachine,
            ],
            sink: OutputSurface::McpwmPwm,
            feedback: vec![FeedbackSurface::SerialLog, FeedbackSurface::PhysicalMotion],
            required_components: vec!["uart".into(), "mcpwm".into(), "logger".into()],
            optional_components: vec!["pulse_counter".into()],
            expected_user_result: "motor speed or direction changes".into(),
        };
        let json = serde_json::to_string(&tmpl).unwrap();
        let back: SignalPathTemplate = serde_json::from_str(&json).unwrap();
        assert_eq!(back, tmpl);
    }

    #[test]
    fn platform_catalog_default_is_empty() {
        let cat = PlatformCatalog::default();
        assert!(cat.platforms.is_empty());
    }

    #[test]
    fn platform_catalog_esp_idf_default_has_targets() {
        let cat = PlatformCatalog::esp_idf_default();
        assert_eq!(cat.platforms.len(), 1);
        assert_eq!(cat.platforms[0].kind, PlatformKind::EspIdf);
        let targets = &cat.platforms[0].targets;
        assert_eq!(targets.len(), 3);
        assert_eq!(targets[0].target, ChipTarget::Esp32);
        assert_eq!(targets[1].target, ChipTarget::Esp32S3);
        assert_eq!(targets[2].target, ChipTarget::Esp32C6);
    }

    #[test]
    fn esp32s3_has_camera_and_usb() {
        let cat = PlatformCatalog::esp_idf_default();
        let s3 = &cat.platforms[0].targets[1];
        assert!(s3
            .capability_profile
            .capabilities
            .contains(&Capability::Camera));
        assert!(s3
            .capability_profile
            .capabilities
            .contains(&Capability::UsbOtg));
        assert!(s3.supported_inputs.contains(&InputSurface::CameraFrame));
        assert!(s3
            .supported_outputs
            .contains(&OutputSurface::HttpMjpegStream));
    }

    #[test]
    fn esp32c6_has_thread_and_zigbee() {
        let cat = PlatformCatalog::esp_idf_default();
        let c6 = &cat.platforms[0].targets[2];
        assert!(c6
            .capability_profile
            .capabilities
            .contains(&Capability::Thread));
        assert!(c6
            .capability_profile
            .capabilities
            .contains(&Capability::Zigbee));
    }

    #[test]
    fn esp32s3_has_feedback_path() {
        let cat = PlatformCatalog::esp_idf_default();
        let s3 = &cat.platforms[0].targets[1];
        assert_eq!(s3.feedback_paths.len(), 1);
        assert_eq!(s3.feedback_paths[0].id, "uart_command_to_motor");
    }

    #[test]
    fn vehicle_capability_serde_roundtrip() {
        let caps = vec![
            Capability::MotorControl,
            Capability::Imu,
            Capability::LongRange,
            Capability::Csi,
            Capability::FailsafeStop,
            Capability::ApSta,
            Capability::Bridge,
        ];
        for cap in caps {
            let json = serde_json::to_string(&cap).unwrap();
            let back: Capability = serde_json::from_str(&json).unwrap();
            assert_eq!(cap, back);
        }
        // Verify snake_case serialization
        assert_eq!(
            serde_json::to_string(&Capability::MotorControl).unwrap(),
            "\"motor_control\""
        );
        assert_eq!(
            serde_json::to_string(&Capability::LongRange).unwrap(),
            "\"long_range\""
        );
        assert_eq!(
            serde_json::to_string(&Capability::FailsafeStop).unwrap(),
            "\"failsafe_stop\""
        );
        assert_eq!(
            serde_json::to_string(&Capability::ApSta).unwrap(),
            "\"ap_sta\""
        );
    }

    #[test]
    fn new_input_surface_serde() {
        let surfaces = vec![
            (InputSurface::ServiceCall, "\"service_call\""),
            (InputSurface::ApiCommand, "\"api_command\""),
            (InputSurface::MqttMessage, "\"mqtt_message\""),
            (InputSurface::EspNowData, "\"esp_now_data\""),
        ];
        for (s, expected) in surfaces {
            let json = serde_json::to_string(&s).unwrap();
            assert_eq!(json, expected);
            let back: InputSurface = serde_json::from_str(&json).unwrap();
            assert_eq!(back, s);
        }
    }

    #[test]
    fn new_capability_serde() {
        let caps = vec![
            (Capability::EspNow, "\"esp_now\""),
            (Capability::MeshLite, "\"mesh_lite\""),
        ];
        for (c, expected) in caps {
            let json = serde_json::to_string(&c).unwrap();
            assert_eq!(json, expected);
            let back: Capability = serde_json::from_str(&json).unwrap();
            assert_eq!(back, c);
        }
    }

    #[test]
    fn all_targets_have_software_inputs() {
        let cat = PlatformCatalog::esp_idf_default();
        for target in &cat.platforms[0].targets {
            assert!(
                target.supported_inputs.contains(&InputSurface::ServiceCall),
                "{} missing ServiceCall",
                target.id
            );
            assert!(
                target.supported_inputs.contains(&InputSurface::ApiCommand),
                "{} missing ApiCommand",
                target.id
            );
            assert!(
                target.supported_inputs.contains(&InputSurface::MqttMessage),
                "{} missing MqttMessage",
                target.id
            );
            assert!(
                target.supported_inputs.contains(&InputSurface::EspNowData),
                "{} missing EspNowData",
                target.id
            );
        }
    }

    #[test]
    fn all_wifi_targets_have_esp_now() {
        let cat = PlatformCatalog::esp_idf_default();
        for target in &cat.platforms[0].targets {
            if target
                .capability_profile
                .capabilities
                .contains(&Capability::Wifi)
            {
                assert!(
                    target
                        .capability_profile
                        .capabilities
                        .contains(&Capability::EspNow),
                    "{} has Wifi but missing EspNow",
                    target.id
                );
            }
        }
    }

    #[test]
    fn esp32c6_has_mesh_lite() {
        let cat = PlatformCatalog::esp_idf_default();
        let c6 = &cat.platforms[0].targets[2];
        assert!(c6
            .capability_profile
            .capabilities
            .contains(&Capability::MeshLite));
    }

    #[test]
    fn platform_catalog_serde_round_trip() {
        let cat = PlatformCatalog::esp_idf_default();
        let json = serde_json::to_string(&cat).unwrap();
        let back: PlatformCatalog = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cat);
    }
}
