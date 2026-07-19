//! Component registry: models the ESPHome platform/component hierarchy.
//!
//! Each ESPHome component is a `ComponentDefinition` stored in a `ComponentRegistry`.
//! The registry resolves AUTO_LOAD chains, validates DEPENDENCIES, and checks
//! CONFLICTS_WITH — exactly mirroring ESPHome's Python validation logic.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::entity::EntityType;

// ── Core types ────────────────────────────────────────────────────────────────

/// Stable string ID for a component, e.g. `"dht"`, `"sensor"`, `"wifi"`.
pub type ComponentId = String;

/// Stable string ID for a platform (component instance within a parent platform).
pub type PlatformId = String;

/// Controls how many instances of a component can be selected.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstancePolicy {
    /// Only one instance allowed globally.
    Singleton,
    /// Only one component from the named exclusive group can be selected.
    ExclusiveGroup(String),
    /// Multiple instances are allowed (default).
    #[default]
    MultiInstance,
}

/// Controls whether a component's config can be changed at runtime.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigMode {
    /// Config is baked into firmware at compile time.
    CompileTimeOnly,
    /// Config can be updated at runtime (e.g. via NVS).
    #[default]
    RuntimeMutable,
}

/// A single component or platform definition.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComponentDefinition {
    /// Unique identifier, e.g. `"dht"` or `"sensor"`.
    pub id: ComponentId,
    /// Human-readable summary for component pickers and docs.
    #[serde(default)]
    pub description: String,
    /// `true` for abstract family parents like `"sensor"` or `"binary_sensor"`.
    pub is_family: bool,
    /// Child component IDs that implement this family parent (empty for non-families).
    pub child_components: Vec<PlatformId>,
    /// Components that are automatically loaded when this component is used.
    pub auto_load: Vec<ComponentId>,
    /// Components that must be present when this component is used.
    pub dependencies: Vec<ComponentId>,
    /// Components that cannot coexist with this component.
    pub conflicts_with: Vec<ComponentId>,
    /// Which entity type this component produces (if any).
    pub entity_type: Option<EntityType>,
    /// Mutual exclusion group name (e.g. `"llm_provider"`).
    #[serde(default)]
    pub exclusive_group: Option<String>,
    /// How many instances of this component can be selected.
    #[serde(default)]
    pub instance_policy: InstancePolicy,
    /// Whether config is compile-time-only or runtime-mutable.
    #[serde(default)]
    pub config_mode: ConfigMode,
    /// Credential field names that must be provisioned via NVS, never in firmware.
    #[serde(default)]
    pub secret_fields: Vec<String>,
    /// Platform binding: domain, tree, and supported targets (None = universal).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_binding: Option<crate::platform::ComponentPlatformBinding>,
    /// Signal-flow interaction: inputs, transforms, outputs, feedback (None = no I/O).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interaction: Option<crate::platform::ComponentInteraction>,
}

fn default_component_description(id: &str) -> &'static str {
    match id {
        "sensor" => "Parent platform for measurement-oriented sensor entities.",
        "binary_sensor" => "Parent platform for on/off, occupancy, and contact-style entities.",
        "switch" => "Parent platform for controllable on/off outputs.",
        "number" => "Parent platform for numeric values that can be adjusted at runtime.",
        "select" => "Parent platform for selecting one option from a predefined list.",
        "text" => "Parent platform for writable text-based values.",
        "button" => "Parent platform for stateless action buttons.",
        "event" => "Parent platform for transient event emitters.",
        "light" => "Parent platform for dimmable and color-capable light entities.",
        "climate" => "Parent platform for HVAC and thermostat-style controllers.",
        "fan" => "Parent platform for fan speed and direction control.",
        "cover" => "Parent platform for blinds, shutters, and other covers.",
        "lock" => "Parent platform for smart locks and latching actuators.",
        "media_player" => "Parent platform for audio playback and media transport.",
        "alarm_control_panel" => "Parent platform for arming and alarm-state control.",
        "text_sensor" => "Parent platform for read-only text state and diagnostics.",
        "wifi" => "Wi-Fi networking stack for station and provisioning flows.",
        "ethernet" => "Wired networking stack for Ethernet-connected devices.",
        "api" => "Native API server for Home Assistant and other clients.",
        "mqtt" => "MQTT transport for broker-based automation and telemetry.",
        "ota" => "Over-the-air firmware updates delivered across the network.",
        "logger" => "Serial and runtime logging for diagnostics.",
        "time" => "System time services for schedules, clocks, and timestamps.",
        "i2c" => "Shared I2C bus support for digital peripherals.",
        "spi" => "Shared SPI bus support for high-speed peripherals.",
        "uart" => "Shared UART bus support for serial peripherals.",
        "deep_sleep" => "Low-power deep sleep controller for battery-powered devices.",
        "restart" => "Restart action and reboot control component.",
        "dht" => "Single-bus temperature and humidity sensor such as DHT11 or DHT22.",
        "bme280_i2c" => "I2C temperature, humidity, and pressure sensor.",
        "bme680_bsec" => {
            "I2C air-quality sensor with temperature, humidity, pressure, and gas readings."
        }
        "sht3x" => "I2C temperature and humidity sensor from Sensirion.",
        "ds18x20" => "1-Wire temperature sensor family such as DS18B20.",
        "adc" => "Analog-to-digital input for voltage and sensor measurements.",
        "pulse_counter" => "Pulse counter for meters, hall sensors, and reed switches.",
        "ultrasonic" => "Distance sensor driven by trigger and echo GPIO pins.",
        "htu21d" => "I2C temperature and humidity sensor from TE Connectivity.",
        "rotary_encoder" => "Rotary encoder input for knobs, dials, and position tracking.",
        "bh1750" => "I2C ambient light sensor that reports illuminance in lux.",
        "gpio" => "GPIO-backed digital input or output component.",
        "status" => "Binary sensor that reflects the device online and health state.",
        "esp32_touch" => "Capacitive touch binary sensor for ESP32 touch-capable pins.",
        "neopixelbus" => "Addressable LED strip or pixel light driven through NeoPixelBus.",
        "fastled_clockless" => "Clockless addressable LED strip light driven through FastLED.",
        "thermostat" => "Thermostat climate controller with setpoints and hysteresis.",
        "bang_bang" => "Simple on/off climate controller for heating or cooling.",
        "pid" => "PID-based climate controller for closed-loop temperature control.",
        "time_based" => "Cover controller that estimates position from travel time.",
        "i2s_audio" => "I2S audio output pipeline for speaker or DAC playback.",
        "esp32_hall" => "ESP32 internal hall-effect sensor.",
        "sigrok" => "sigrok-compatible logic analyzer (SUMP/OLS wire protocol, ESP32-S3 LCD_CAM).",
        "ld2410" => "Hi-Link LD2410/LD2420 mmWave presence and distance sensor over UART.",
        "ct_clamp" => "Current-transformer clamp input driven through an ADC + burden resistor.",
        "pms5003" => {
            "Plantower PMS5003/PMS7003 particulate-matter (PM1.0/2.5/10) sensor over UART."
        }
        "scd40" => {
            "Sensirion SCD40/SCD41 photoacoustic CO₂, temperature, and humidity sensor (I²C)."
        }
        "sx1302" => "Semtech SX1302/SX1303 LoRaWAN 8-channel concentrator over SPI.",
        _ => "",
    }
}

// ── Errors ────────────────────────────────────────────────────────────────────

/// A required dependency was not found in the selected component set.
#[derive(Debug, Clone, Error)]
#[error("component '{component}' requires '{dependency}' which is not selected")]
pub struct MissingDep {
    pub component: ComponentId,
    pub dependency: ComponentId,
}

/// Two selected components conflict with each other.
#[derive(Debug, Clone, Error)]
#[error("component '{a}' conflicts with '{b}'")]
pub struct Conflict {
    pub a: ComponentId,
    pub b: ComponentId,
}

// ── Registry ──────────────────────────────────────────────────────────────────

/// Registry of all known components and their dependency metadata.
pub struct ComponentRegistry {
    components: HashMap<ComponentId, ComponentDefinition>,
}

impl ComponentRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            components: HashMap::new(),
        }
    }

    /// Register a component definition, replacing any existing entry with the same ID.
    pub fn register(&mut self, mut def: ComponentDefinition) {
        if def.description.trim().is_empty() {
            def.description = default_component_description(&def.id).to_string();
        }
        self.components.insert(def.id.clone(), def);
    }

    /// Look up a component by ID.
    pub fn get(&self, id: &str) -> Option<&ComponentDefinition> {
        self.components.get(id)
    }

    /// Return all registered component IDs.
    pub fn all_ids(&self) -> impl Iterator<Item = &str> {
        self.components.keys().map(|s| s.as_str())
    }

    /// Return all component definitions.
    pub fn all_definitions(&self) -> impl Iterator<Item = &ComponentDefinition> {
        self.components.values()
    }

    /// Transitively expand `selected` with all AUTO_LOAD components.
    ///
    /// Iterates until no new components are added (fixed-point).  Returns the
    /// full set (original + auto-loaded) sorted for determinism.
    pub fn resolve_auto_load(&self, selected: &[ComponentId]) -> Vec<ComponentId> {
        let mut result: HashSet<ComponentId> = selected.iter().cloned().collect();
        let mut worklist: Vec<ComponentId> = selected.to_vec();

        while let Some(id) = worklist.pop() {
            if let Some(def) = self.components.get(&id) {
                for dep in &def.auto_load {
                    if result.insert(dep.clone()) {
                        worklist.push(dep.clone());
                    }
                }
            }
        }

        let mut out: Vec<ComponentId> = result.into_iter().collect();
        out.sort();
        out
    }

    /// Check that all declared DEPENDENCIES are satisfied in `selected`.
    ///
    /// Auto-loads are expanded first, so callers can pass the raw selection.
    pub fn check_dependencies(&self, selected: &[ComponentId]) -> Result<(), Vec<MissingDep>> {
        let expanded: HashSet<ComponentId> = self.resolve_auto_load(selected).into_iter().collect();
        let mut errors = Vec::new();

        for id in &expanded {
            if let Some(def) = self.components.get(id) {
                for dep in &def.dependencies {
                    if !expanded.contains(dep) {
                        errors.push(MissingDep {
                            component: id.clone(),
                            dependency: dep.clone(),
                        });
                    }
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Check that no two selected components are in conflict.
    ///
    /// Auto-loads are expanded first.
    pub fn check_conflicts(&self, selected: &[ComponentId]) -> Result<(), Vec<Conflict>> {
        let expanded: Vec<ComponentId> = self.resolve_auto_load(selected);
        let set: HashSet<&str> = expanded.iter().map(|s| s.as_str()).collect();
        let mut errors = Vec::new();
        let mut seen: HashSet<(ComponentId, ComponentId)> = HashSet::new();

        for id in &expanded {
            if let Some(def) = self.components.get(id) {
                for other in &def.conflicts_with {
                    if set.contains(other.as_str()) {
                        // Deduplicate (a,b) and (b,a) pairs.
                        let key = if id < other {
                            (id.clone(), other.clone())
                        } else {
                            (other.clone(), id.clone())
                        };
                        if seen.insert(key) {
                            errors.push(Conflict {
                                a: id.clone(),
                                b: other.clone(),
                            });
                        }
                    }
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Build a petgraph-backed component dependency DAG for the given selection.
    ///
    /// Expands `selected` via auto_load chains, validates acyclicity, and returns
    /// a [`ComponentDag`](crate::graph::ComponentDag) with topological ordering.
    #[cfg(feature = "dag")]
    pub fn build_dag(
        &self,
        selected: &[ComponentId],
    ) -> Result<crate::graph::ComponentDag, crate::graph::CycleError> {
        crate::graph::ComponentDag::from_registry(self, selected)
    }

    /// Build a pre-populated registry with known components.
    pub fn default_registry() -> Self {
        let mut r = Self::new();

        // ── Platform parents ──────────────────────────────────────────────────
        r.register(ComponentDefinition {
            id: "sensor".into(),
            is_family: true,
            child_components: vec![
                "dht".into(),
                "bme280_i2c".into(),
                "bme680_bsec".into(),
                "sht3x".into(),
                "ds18x20".into(),
                "adc".into(),
                "pulse_counter".into(),
                "ultrasonic".into(),
                "htu21d".into(),
                "bh1750".into(),
                "ade7953_i2c".into(),
                "homeassistant".into(),
                "template".into(),
                "esp32_hall".into(),
                "rotary_encoder".into(),
                "ld2410".into(),
                "ct_clamp".into(),
                "pms5003".into(),
                "scd40".into(),
            ],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Sensor),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "binary_sensor".into(),
            is_family: true,
            child_components: vec![
                "gpio".into(),
                "status".into(),
                "homeassistant".into(),
                "template".into(),
                "esp32_touch".into(),
            ],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::BinarySensor),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "switch".into(),
            is_family: true,
            child_components: vec![
                "gpio".into(),
                "restart".into(),
                "template".into(),
                "uart".into(),
            ],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Switch),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "number".into(),
            is_family: true,
            child_components: vec!["template".into()],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Number),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "select".into(),
            is_family: true,
            child_components: vec!["template".into()],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Select),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "text".into(),
            is_family: true,
            child_components: vec!["template".into()],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Text),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "button".into(),
            is_family: true,
            child_components: vec!["template".into(), "restart".into()],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Button),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "event".into(),
            is_family: true,
            child_components: vec!["template".into()],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Event),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "light".into(),
            is_family: true,
            child_components: vec![
                "binary".into(),
                "monochromatic".into(),
                "rgb".into(),
                "rgbw".into(),
                "rgbww".into(),
                "neopixelbus".into(),
                "fastled_clockless".into(),
                "partition".into(),
            ],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Light),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "climate".into(),
            is_family: true,
            child_components: vec!["thermostat".into(), "bang_bang".into(), "pid".into()],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Climate),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "fan".into(),
            is_family: true,
            child_components: vec!["speed".into(), "template".into()],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Fan),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "cover".into(),
            is_family: true,
            child_components: vec!["time_based".into(), "template".into(), "endstop".into()],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Cover),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "lock".into(),
            is_family: true,
            child_components: vec!["template".into(), "gpio".into()],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Lock),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "media_player".into(),
            is_family: true,
            child_components: vec!["i2s_audio".into()],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::MediaPlayer),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "alarm_control_panel".into(),
            is_family: true,
            child_components: vec!["template".into()],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::AlarmControlPanel),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "text_sensor".into(),
            is_family: true,
            child_components: vec![
                "template".into(),
                "homeassistant".into(),
                "wifi_info".into(),
                "version".into(),
            ],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::TextSensor),
            ..Default::default()
        });

        // ── Core infrastructure ───────────────────────────────────────────────

        r.register(ComponentDefinition {
            id: "wifi".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec!["ethernet".into()],
            entity_type: None,
            platform_binding: Some(crate::platform::ComponentPlatformBinding {
                platform: crate::platform::PlatformKind::EspIdf,
                tree: crate::platform::PlatformTree::Device,
                domain: crate::platform::ComponentDomain::Connectivity,
                taxonomy_path: vec!["core".into(), "connectivity".into(), "wifi".into()],
                supported_targets: vec![
                    crate::platform::ChipTarget::Esp32,
                    crate::platform::ChipTarget::Esp32S2,
                    crate::platform::ChipTarget::Esp32S3,
                    crate::platform::ChipTarget::Esp32C3,
                    crate::platform::ChipTarget::Esp32C5,
                    crate::platform::ChipTarget::Esp32C6,
                ],
            }),
            interaction: Some(crate::platform::ComponentInteraction {
                input_surfaces: vec![crate::platform::InputSurface::WifiEvent],
                transform_roles: vec![],
                output_surfaces: vec![crate::platform::OutputSurface::WifiPacket],
                feedback_surfaces: vec![crate::platform::FeedbackSurface::ApiState],
            }),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "ethernet".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec!["wifi".into()],
            entity_type: None,
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "api".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: None,
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "mqtt".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: None,
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "ota".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec![],
            dependencies: vec!["wifi".into()],
            conflicts_with: vec![],
            entity_type: None,
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "logger".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: None,
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "time".into(),
            is_family: false,
            child_components: vec!["homeassistant".into(), "sntp".into(), "gps".into()],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: None,
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "i2c".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: None,
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "spi".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: None,
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "uart".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: None,
            platform_binding: Some(crate::platform::ComponentPlatformBinding {
                platform: crate::platform::PlatformKind::EspIdf,
                tree: crate::platform::PlatformTree::Device,
                domain: crate::platform::ComponentDomain::IoBuses,
                taxonomy_path: vec!["core".into(), "io_buses".into(), "uart".into()],
                supported_targets: crate::platform::ChipTarget::all().to_vec(),
            }),
            interaction: Some(crate::platform::ComponentInteraction {
                input_surfaces: vec![crate::platform::InputSurface::UartRx],
                transform_roles: vec![],
                output_surfaces: vec![crate::platform::OutputSurface::UartTx],
                feedback_surfaces: vec![crate::platform::FeedbackSurface::SerialLog],
            }),
            ..Default::default()
        });

        // ── Industrial bus components ─────────────────────────────────────────

        r.register(ComponentDefinition {
            id: "can_bus".into(),
            description: "Parent platform for CAN bus communication via ESP32 TWAI controller."
                .into(),
            is_family: true,
            child_components: vec!["can_esp32_twai".into()],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: None,
            platform_binding: Some(crate::platform::ComponentPlatformBinding {
                platform: crate::platform::PlatformKind::EspIdf,
                tree: crate::platform::PlatformTree::Device,
                domain: crate::platform::ComponentDomain::IoBuses,
                taxonomy_path: vec!["core".into(), "io_buses".into(), "can_bus".into()],
                supported_targets: vec![crate::platform::ChipTarget::Esp32],
            }),
            interaction: Some(crate::platform::ComponentInteraction {
                input_surfaces: vec![crate::platform::InputSurface::CanBusFrame],
                transform_roles: vec![crate::platform::TransformNode::CanFrameDecode],
                output_surfaces: vec![crate::platform::OutputSurface::CanBusTx],
                feedback_surfaces: vec![crate::platform::FeedbackSurface::BusErrorAlert],
            }),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "can_esp32_twai".into(),
            description: "ESP32 TWAI (Two-Wire Automotive Interface) CAN controller driver.".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["can_bus".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: None,
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "rs485_bus".into(),
            description: "Parent platform for RS485 half-duplex serial communication.".into(),
            is_family: true,
            child_components: vec!["rs485_uart_half_duplex".into()],
            auto_load: vec![],
            dependencies: vec!["uart".into()],
            conflicts_with: vec![],
            entity_type: None,
            platform_binding: Some(crate::platform::ComponentPlatformBinding {
                platform: crate::platform::PlatformKind::EspIdf,
                tree: crate::platform::PlatformTree::Device,
                domain: crate::platform::ComponentDomain::IoBuses,
                taxonomy_path: vec!["core".into(), "io_buses".into(), "rs485_bus".into()],
                supported_targets: vec![crate::platform::ChipTarget::Esp32],
            }),
            interaction: Some(crate::platform::ComponentInteraction {
                input_surfaces: vec![crate::platform::InputSurface::Rs485Data],
                transform_roles: vec![crate::platform::TransformNode::Rs485ProtocolDecode],
                output_surfaces: vec![crate::platform::OutputSurface::Rs485Tx],
                feedback_surfaces: vec![crate::platform::FeedbackSurface::BusErrorAlert],
            }),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "rs485_uart_half_duplex".into(),
            description: "UART-based RS485 half-duplex transceiver with automatic DE pin control."
                .into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["rs485_bus".into(), "uart".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: None,
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "sd_card_spi".into(),
            description: "SPI-based SD card for data logging with FAT filesystem.".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["spi".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: None,
            platform_binding: Some(crate::platform::ComponentPlatformBinding {
                platform: crate::platform::PlatformKind::EspIdf,
                tree: crate::platform::PlatformTree::Device,
                domain: crate::platform::ComponentDomain::StorageSecurity,
                taxonomy_path: vec!["core".into(), "storage".into(), "sd_card_spi".into()],
                supported_targets: crate::platform::ChipTarget::all().to_vec(),
            }),
            interaction: Some(crate::platform::ComponentInteraction {
                input_surfaces: vec![],
                transform_roles: vec![],
                output_surfaces: vec![crate::platform::OutputSurface::SdCardWrite],
                feedback_surfaces: vec![],
            }),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "deep_sleep".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: None,
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "restart".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: None,
            ..Default::default()
        });

        // ── Sensor platform implementations ───────────────────────────────────

        r.register(ComponentDefinition {
            id: "dht".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["sensor".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Sensor),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "bme280_i2c".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["sensor".into(), "i2c".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Sensor),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "bme680_bsec".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["sensor".into(), "i2c".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Sensor),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "sht3x".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["sensor".into(), "i2c".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Sensor),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "ds18x20".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["sensor".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Sensor),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "adc".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["sensor".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Sensor),
            ..Default::default()
        });

        // ── IoT Phase 2 additions (2026-04-16) ────────────────────────────
        r.register(ComponentDefinition {
            id: "ld2410".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["sensor".into(), "uart".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Sensor),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "ct_clamp".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["sensor".into(), "adc".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Sensor),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "pms5003".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["sensor".into(), "uart".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Sensor),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "scd40".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["sensor".into(), "i2c".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Sensor),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "sx1302".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["spi".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: None,
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "pulse_counter".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["sensor".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Sensor),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "ultrasonic".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["sensor".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Sensor),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "htu21d".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["sensor".into(), "i2c".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Sensor),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "bh1750".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["sensor".into(), "i2c".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Sensor),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "rotary_encoder".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["sensor".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Sensor),
            ..Default::default()
        });

        // ── Binary sensor implementations ─────────────────────────────────────

        r.register(ComponentDefinition {
            id: "gpio".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::BinarySensor),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "status".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["binary_sensor".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::BinarySensor),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "esp32_touch".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["binary_sensor".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::BinarySensor),
            ..Default::default()
        });

        // ── Light implementations ─────────────────────────────────────────────

        r.register(ComponentDefinition {
            id: "neopixelbus".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["light".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Light),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "fastled_clockless".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["light".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Light),
            ..Default::default()
        });

        // ── Climate implementations ───────────────────────────────────────────

        r.register(ComponentDefinition {
            id: "thermostat".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["climate".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Climate),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "bang_bang".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["climate".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Climate),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "pid".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["climate".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Climate),
            ..Default::default()
        });

        // ── Cover implementations ─────────────────────────────────────────────

        r.register(ComponentDefinition {
            id: "time_based".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["cover".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Cover),
            ..Default::default()
        });

        // ── Media player implementations ──────────────────────────────────────

        r.register(ComponentDefinition {
            id: "i2s_audio".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["media_player".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::MediaPlayer),
            ..Default::default()
        });

        r.register(ComponentDefinition {
            id: "esp32_hall".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["sensor".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Sensor),
            ..Default::default()
        });

        // ── Diagnostics ────────────────────────────────────────────────────────

        r.register(ComponentDefinition {
            id: "sigrok".into(),
            instance_policy: InstancePolicy::Singleton,
            config_mode: ConfigMode::CompileTimeOnly,
            platform_binding: Some(crate::platform::ComponentPlatformBinding {
                platform: crate::platform::PlatformKind::EspIdf,
                tree: crate::platform::PlatformTree::Device,
                domain: crate::platform::ComponentDomain::Diagnostics,
                taxonomy_path: vec!["device".into(), "diagnostics".into(), "sigrok".into()],
                supported_targets: vec![crate::platform::ChipTarget::Esp32S3],
            }),
            ..Default::default()
        });

        r
    }

    // ── Target-aware queries ─────────────────────────────────────────────────

    /// Return all components supporting the given chip target.
    ///
    /// Components without a `platform_binding` are assumed universal.
    pub fn all_for_target(&self, target: crate::platform::ChipTarget) -> Vec<&ComponentDefinition> {
        self.components
            .values()
            .filter(|c| match &c.platform_binding {
                Some(pb) => pb.supported_targets.contains(&target),
                None => true,
            })
            .collect()
    }

    /// Look up a component only if it supports the given chip target.
    pub fn get_for_target(
        &self,
        id: &str,
        target: crate::platform::ChipTarget,
    ) -> Option<&ComponentDefinition> {
        self.get(id).filter(|c| match &c.platform_binding {
            Some(pb) => pb.supported_targets.contains(&target),
            None => true,
        })
    }

    /// Find all components that produce a given output surface on the given target.
    pub fn components_for_output(
        &self,
        target: crate::platform::ChipTarget,
        output: crate::platform::OutputSurface,
    ) -> Vec<&ComponentDefinition> {
        self.all_for_target(target)
            .into_iter()
            .filter(|c| match &c.interaction {
                Some(i) => i.output_surfaces.contains(&output),
                None => false,
            })
            .collect()
    }
}

impl Default for ComponentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn default_reg() -> ComponentRegistry {
        ComponentRegistry::default_registry()
    }

    #[test]
    fn default_registry_has_required_components() {
        let reg = default_reg();
        for id in &[
            "sensor",
            "binary_sensor",
            "switch",
            "wifi",
            "logger",
            "i2c",
            "dht",
        ] {
            assert!(reg.get(id).is_some(), "missing component: {id}");
        }
    }

    #[test]
    fn default_registry_has_30_or_more_components() {
        let reg = default_reg();
        let count = reg.all_ids().count();
        assert!(count >= 30, "expected ≥30 components, got {count}");
    }

    #[test]
    fn resolve_auto_load_dht_adds_sensor() {
        let reg = default_reg();
        let expanded = reg.resolve_auto_load(&["dht".into()]);
        assert!(
            expanded.contains(&"sensor".into()),
            "sensor should be auto-loaded by dht"
        );
        assert!(expanded.contains(&"dht".into()));
    }

    #[test]
    fn resolve_auto_load_bme280_chain() {
        let reg = default_reg();
        // bme280_i2c auto-loads sensor + i2c
        let expanded = reg.resolve_auto_load(&["bme280_i2c".into()]);
        assert!(expanded.contains(&"sensor".into()));
        assert!(expanded.contains(&"i2c".into()));
        assert!(expanded.contains(&"bme280_i2c".into()));
    }

    #[test]
    fn resolve_auto_load_bh1750_chain() {
        let reg = default_reg();
        let expanded = reg.resolve_auto_load(&["bh1750".into()]);
        assert!(expanded.contains(&"sensor".into()));
        assert!(expanded.contains(&"i2c".into()));
        assert!(expanded.contains(&"bh1750".into()));
    }

    #[test]
    fn resolve_auto_load_is_idempotent() {
        let reg = default_reg();
        let sel: Vec<ComponentId> = vec!["dht".into(), "sensor".into()];
        let expanded = reg.resolve_auto_load(&sel);
        // sensor was already in the selection, no duplication
        let sensor_count = expanded.iter().filter(|id| id.as_str() == "sensor").count();
        assert_eq!(sensor_count, 1);
    }

    #[test]
    fn resolve_auto_load_empty_input() {
        let reg = default_reg();
        let expanded = reg.resolve_auto_load(&[]);
        assert!(expanded.is_empty());
    }

    #[test]
    fn check_dependencies_ota_needs_wifi() {
        let reg = default_reg();
        // ota depends on wifi — should fail without wifi
        let result = reg.check_dependencies(&["ota".into()]);
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(errs.iter().any(|e| e.dependency == "wifi"));
    }

    #[test]
    fn check_dependencies_ota_with_wifi_passes() {
        let reg = default_reg();
        let result = reg.check_dependencies(&["ota".into(), "wifi".into()]);
        assert!(result.is_ok());
    }

    #[test]
    fn check_dependencies_no_unknown_errors() {
        let reg = default_reg();
        // logger, wifi, api, sensor, dht: no unresolved deps
        let result =
            reg.check_dependencies(&["logger".into(), "wifi".into(), "api".into(), "dht".into()]);
        assert!(result.is_ok(), "unexpected errors: {:?}", result.err());
    }

    #[test]
    fn check_conflicts_wifi_ethernet_conflict() {
        let reg = default_reg();
        let result = reg.check_conflicts(&["wifi".into(), "ethernet".into()]);
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(!errs.is_empty());
    }

    #[test]
    fn check_conflicts_wifi_alone_is_ok() {
        let reg = default_reg();
        let result = reg.check_conflicts(&["wifi".into()]);
        assert!(result.is_ok());
    }

    #[test]
    fn check_conflicts_deduplicates_pairs() {
        let reg = default_reg();
        let result = reg.check_conflicts(&["wifi".into(), "ethernet".into()]);
        // Should only report the conflict once, not twice (a,b) and (b,a)
        let errs = result.unwrap_err();
        assert_eq!(errs.len(), 1);
    }

    #[test]
    fn lookup_nonexistent_returns_none() {
        let reg = default_reg();
        assert!(reg.get("nonexistent_xyz").is_none());
    }

    #[test]
    fn sensor_platform_has_entity_type() {
        let reg = default_reg();
        let sensor = reg.get("sensor").unwrap();
        assert_eq!(sensor.entity_type, Some(EntityType::Sensor));
        assert!(sensor.is_family);
    }

    #[test]
    fn dht_is_not_platform() {
        let reg = default_reg();
        let dht = reg.get("dht").unwrap();
        assert!(!dht.is_family);
    }

    #[test]
    fn default_registry_components_have_descriptions() {
        let reg = default_reg();
        for def in reg.all_definitions() {
            assert!(
                !def.description.trim().is_empty(),
                "component '{}' is missing a description",
                def.id
            );
        }
    }

    #[test]
    fn wifi_conflicts_with_ethernet() {
        let reg = default_reg();
        let wifi = reg.get("wifi").unwrap();
        assert!(wifi.conflicts_with.contains(&"ethernet".into()));
    }

    #[test]
    fn missing_dep_error_message() {
        let err = MissingDep {
            component: "ota".into(),
            dependency: "wifi".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("ota"));
        assert!(msg.contains("wifi"));
    }

    #[test]
    fn conflict_error_message() {
        let err = Conflict {
            a: "wifi".into(),
            b: "ethernet".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("wifi"));
        assert!(msg.contains("ethernet"));
    }

    #[test]
    fn register_custom_component() {
        let mut reg = ComponentRegistry::new();
        reg.register(ComponentDefinition {
            id: "my_sensor".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["sensor".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(EntityType::Sensor),
            ..Default::default()
        });
        assert!(reg.get("my_sensor").is_some());
    }

    // ── Task 0.1 tests ─────────────────────────────────────────────────────

    #[test]
    fn instance_policy_default_is_multi_instance() {
        assert_eq!(InstancePolicy::default(), InstancePolicy::MultiInstance);
    }

    #[test]
    fn serde_roundtrip_with_exclusive_group() {
        let def = ComponentDefinition {
            id: "test".into(),
            exclusive_group: Some("llm_provider".into()),
            instance_policy: InstancePolicy::ExclusiveGroup("llm_provider".into()),
            ..Default::default()
        };
        let json = serde_json::to_string(&def).unwrap();
        let back: ComponentDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(back.exclusive_group, Some("llm_provider".into()));
        assert_eq!(
            back.instance_policy,
            InstancePolicy::ExclusiveGroup("llm_provider".into())
        );
    }

    // ── Task 0.2 tests ─────────────────────────────────────────────────────

    #[test]
    fn config_mode_default_is_runtime_mutable() {
        assert_eq!(ConfigMode::default(), ConfigMode::RuntimeMutable);
    }

    #[test]
    fn serde_roundtrip_with_secret_fields() {
        let def = ComponentDefinition {
            id: "test".into(),
            secret_fields: vec!["api_key".into(), "token".into()],
            config_mode: ConfigMode::CompileTimeOnly,
            ..Default::default()
        };
        let json = serde_json::to_string(&def).unwrap();
        let back: ComponentDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(back.secret_fields, vec!["api_key", "token"]);
        assert_eq!(back.config_mode, ConfigMode::CompileTimeOnly);
    }

    #[test]
    fn compile_time_only_persists() {
        let def = ComponentDefinition {
            id: "test".into(),
            config_mode: ConfigMode::CompileTimeOnly,
            ..Default::default()
        };
        let json = serde_json::to_string(&def).unwrap();
        let back: ComponentDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(back.config_mode, ConfigMode::CompileTimeOnly);
    }

    #[test]
    fn empty_secret_fields_serializes_as_empty_array() {
        let def = ComponentDefinition {
            id: "test".into(),
            ..Default::default()
        };
        let json = serde_json::to_string(&def).unwrap();
        assert!(json.contains("\"secret_fields\":[]"));
    }

    // ── Task 0.4 tests ─────────────────────────────────────────────────────

    #[test]
    fn default_registry_has_50_or_more_components() {
        let reg = default_reg();
        let count = reg.all_ids().count();
        assert!(count >= 50, "expected ≥50 components, got {count}");
    }

    // ── Target-aware query tests ─────────────────────────────────────────────

    #[test]
    fn all_for_target_includes_unbound_components() {
        let reg = default_reg();
        let results = reg.all_for_target(crate::platform::ChipTarget::Esp32S3);
        // "logger" has no platform_binding → assumed universal → included
        assert!(results.iter().any(|c| c.id == "logger"));
    }

    #[test]
    fn all_for_target_includes_bound_matching_component() {
        let reg = default_reg();
        let results = reg.all_for_target(crate::platform::ChipTarget::Esp32S3);
        assert!(results.iter().any(|c| c.id == "wifi"));
        assert!(results.iter().any(|c| c.id == "uart"));
    }

    #[test]
    fn components_for_output_finds_uart_by_tx() {
        let reg = default_reg();
        let results = reg.components_for_output(
            crate::platform::ChipTarget::Esp32S3,
            crate::platform::OutputSurface::UartTx,
        );
        assert!(results.iter().any(|c| c.id == "uart"));
    }

    #[test]
    fn components_for_output_finds_wifi_by_packet() {
        let reg = default_reg();
        let results = reg.components_for_output(
            crate::platform::ChipTarget::Esp32,
            crate::platform::OutputSurface::WifiPacket,
        );
        assert!(results.iter().any(|c| c.id == "wifi"));
    }

    // ── sigrok component (Phase 0 — sigrok-la foundation) ──────────────────

    #[test]
    fn sigrok_component_registered() {
        let reg = default_reg();
        let def = reg
            .get("sigrok")
            .expect("sigrok component must be registered");
        assert_eq!(def.id, "sigrok");
        assert!(!def.is_family);
        assert_eq!(def.instance_policy, InstancePolicy::Singleton);
        assert_eq!(def.config_mode, ConfigMode::CompileTimeOnly);
    }

    #[test]
    fn sigrok_supports_only_esp32s3() {
        let reg = default_reg();
        let def = reg.get("sigrok").unwrap();
        let pb = def
            .platform_binding
            .as_ref()
            .expect("sigrok must have platform_binding");
        assert_eq!(
            pb.supported_targets,
            vec![crate::platform::ChipTarget::Esp32S3],
        );
        assert_eq!(pb.domain, crate::platform::ComponentDomain::Diagnostics);
        assert_eq!(pb.tree, crate::platform::PlatformTree::Device);
    }

    #[test]
    fn sigrok_rejected_on_non_s3_targets() {
        let reg = default_reg();
        // get_for_target filters by supported_targets; only S3 should match.
        assert!(reg
            .get_for_target("sigrok", crate::platform::ChipTarget::Esp32S3)
            .is_some());
        for target in [
            crate::platform::ChipTarget::Esp32,
            crate::platform::ChipTarget::Esp32S2,
            crate::platform::ChipTarget::Esp32C3,
            crate::platform::ChipTarget::Esp32C6,
        ] {
            assert!(
                reg.get_for_target("sigrok", target).is_none(),
                "sigrok must not be available on {target:?}",
            );
        }
    }

    #[test]
    fn sigrok_description_is_populated() {
        let reg = default_reg();
        let def = reg.get("sigrok").unwrap();
        assert!(
            def.description.contains("sigrok"),
            "description should mention sigrok, got: {:?}",
            def.description
        );
    }
}
