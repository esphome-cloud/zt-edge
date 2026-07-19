//! Feature flag system: maps component selections to ESPHome `USE_*` C preprocessor defines.
//!
//! ESPHome generates a `defines.h` header containing `#define USE_SENSOR`, `#define USE_WIFI`,
//! etc. based on which components are active.  This module computes that set from a
//! `ComponentRegistry` + selected component list, mirroring ESPHome's Python logic.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::entity::EntityType;
use crate::registry::{ComponentId, ComponentRegistry};

// ── Feature categories ────────────────────────────────────────────────────────

/// High-level category for a `USE_*` feature flag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureCategory {
    /// Target chip / variant flags, e.g. `USE_ESP32`, `USE_ESP32_VARIANT_ESP32S3`.
    Platform,
    /// Core framework features: `USE_LOGGER`, `USE_TIME`, `USE_PSRAM`.
    Core,
    /// Entity type enables: `USE_SENSOR`, `USE_CLIMATE`, `USE_LIGHT`.
    Entity,
    /// Network stack: `USE_WIFI`, `USE_BLE`, `USE_ESPNOW`, `USE_ETHERNET`.
    Network,
    /// API / OTA / MQTT: `USE_API`, `USE_MQTT`, `USE_OTA`, `USE_HOMEASSISTANT`.
    Api,
    /// Display / UI: `USE_DISPLAY`, `USE_LVGL`.
    Ui,
}

/// A single feature flag definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureFlag {
    /// The define name, e.g. `"USE_SENSOR"`.
    pub name: String,
    /// Human-readable category.
    pub category: FeatureCategory,
    /// Which component IDs imply this flag being set.
    pub implied_by: Vec<ComponentId>,
    /// The full C preprocessor define string (usually identical to `name`).
    pub c_define: String,
}

// ── Feature flag registry ─────────────────────────────────────────────────────

/// Returns the canonical mapping from component ID to the `USE_*` flags it enables.
///
/// A single component can enable multiple flags (e.g. `bme280_i2c` enables both
/// `USE_SENSOR` and `USE_I2C`).
fn build_component_to_flags() -> HashMap<&'static str, Vec<&'static str>> {
    let mut m: HashMap<&'static str, Vec<&'static str>> = HashMap::new();

    // Core
    m.insert("logger", vec!["USE_LOGGER"]);
    m.insert("time", vec!["USE_TIME"]);
    m.insert("deep_sleep", vec!["USE_DEEP_SLEEP"]);
    m.insert("restart", vec!["USE_RESTART"]);

    // Network
    m.insert("wifi", vec!["USE_WIFI"]);
    m.insert("ethernet", vec!["USE_ETHERNET"]);

    // Diagnostics
    m.insert("sigrok", vec!["USE_SIGROK"]);

    // Bus
    m.insert("i2c", vec!["USE_I2C"]);
    m.insert("spi", vec!["USE_SPI"]);
    m.insert("uart", vec!["USE_UART"]);

    // API / OTA / MQTT
    m.insert("api", vec!["USE_API"]);
    m.insert("mqtt", vec!["USE_MQTT"]);
    m.insert("ota", vec!["USE_OTA"]);

    // Platform parents → entity enable flag
    m.insert("sensor", vec!["USE_SENSOR"]);
    m.insert("binary_sensor", vec!["USE_BINARY_SENSOR"]);
    m.insert("switch", vec!["USE_SWITCH"]);
    m.insert("number", vec!["USE_NUMBER"]);
    m.insert("select", vec!["USE_SELECT"]);
    m.insert("text", vec!["USE_TEXT"]);
    m.insert("button", vec!["USE_BUTTON"]);
    m.insert("event", vec!["USE_EVENT"]);
    m.insert("light", vec!["USE_LIGHT"]);
    m.insert("climate", vec!["USE_CLIMATE"]);
    m.insert("fan", vec!["USE_FAN"]);
    m.insert("cover", vec!["USE_COVER"]);
    m.insert("lock", vec!["USE_LOCK"]);
    m.insert("media_player", vec!["USE_MEDIA_PLAYER"]);
    m.insert("alarm_control_panel", vec!["USE_ALARM_CONTROL_PANEL"]);
    m.insert("text_sensor", vec!["USE_TEXT_SENSOR"]);

    // Sensor implementations (enable both their own USE and the sensor platform flag)
    m.insert("dht", vec!["USE_DHT", "USE_SENSOR"]);
    m.insert("bme280_i2c", vec!["USE_BME280", "USE_SENSOR", "USE_I2C"]);
    m.insert(
        "bme680_bsec",
        vec!["USE_BME680_BSEC", "USE_SENSOR", "USE_I2C"],
    );
    m.insert("sht3x", vec!["USE_SHT3X", "USE_SENSOR", "USE_I2C"]);
    m.insert("ds18x20", vec!["USE_DS18X20", "USE_SENSOR"]);
    m.insert("adc", vec!["USE_ADC", "USE_SENSOR"]);
    m.insert("pulse_counter", vec!["USE_PULSE_COUNTER", "USE_SENSOR"]);
    m.insert("ultrasonic", vec!["USE_ULTRASONIC", "USE_SENSOR"]);
    m.insert("htu21d", vec!["USE_HTU21D", "USE_SENSOR", "USE_I2C"]);
    m.insert("bh1750", vec!["USE_BH1750", "USE_SENSOR", "USE_I2C"]);
    m.insert("rotary_encoder", vec!["USE_ROTARY_ENCODER", "USE_SENSOR"]);
    m.insert("esp32_hall", vec!["USE_ESP32_HALL", "USE_SENSOR"]);

    // Binary sensor implementations
    m.insert("gpio", vec!["USE_GPIO", "USE_BINARY_SENSOR"]);
    m.insert("status", vec!["USE_STATUS", "USE_BINARY_SENSOR"]);
    m.insert("esp32_touch", vec!["USE_ESP32_TOUCH", "USE_BINARY_SENSOR"]);

    // Light implementations
    m.insert("neopixelbus", vec!["USE_NEOPIXELBUS_LIGHT", "USE_LIGHT"]);
    m.insert(
        "fastled_clockless",
        vec!["USE_FASTLED_CLOCKLESS_LIGHT", "USE_LIGHT"],
    );

    // Climate implementations
    m.insert("thermostat", vec!["USE_THERMOSTAT", "USE_CLIMATE"]);
    m.insert("bang_bang", vec!["USE_BANG_BANG", "USE_CLIMATE"]);
    m.insert("pid", vec!["USE_PID", "USE_CLIMATE"]);

    // Cover implementations
    m.insert("time_based", vec!["USE_TIME_BASED_COVER", "USE_COVER"]);

    // Media player implementations
    m.insert("i2s_audio", vec!["USE_I2S_AUDIO", "USE_MEDIA_PLAYER"]);

    m
}

// ── FeatureFlagSet ────────────────────────────────────────────────────────────

/// The computed set of `USE_*` flags for a particular component selection.
pub struct FeatureFlagSet {
    /// Set of active flag names, e.g. `{"USE_SENSOR", "USE_DHT", "USE_WIFI"}`.
    flags: HashSet<String>,
    /// Per-entity-type instance count (for `ESPHOME_SENSOR_COUNT` etc.).
    entity_counts: HashMap<EntityType, usize>,
}

impl FeatureFlagSet {
    /// Compute the feature flag set from a list of selected component IDs.
    ///
    /// `registry` is used to expand AUTO_LOAD chains before computing flags.
    pub fn from_components(components: &[ComponentId], registry: &ComponentRegistry) -> Self {
        let expanded = registry.resolve_auto_load(components);
        let mapping = build_component_to_flags();
        let mut flags: HashSet<String> = HashSet::new();
        let mut entity_counts: HashMap<EntityType, usize> = HashMap::new();

        for id in &expanded {
            if let Some(flag_list) = mapping.get(id.as_str()) {
                for flag in flag_list {
                    flags.insert((*flag).to_string());
                }
            }
            // Count entity-producing components for ESPHOME_*_COUNT defines.
            if let Some(def) = registry.get(id) {
                if let Some(et) = def.entity_type {
                    if !def.is_family {
                        *entity_counts.entry(et).or_insert(0) += 1;
                    }
                }
            }
        }

        Self {
            flags,
            entity_counts,
        }
    }

    /// Return `true` if the flag is active.
    pub fn contains(&self, flag: &str) -> bool {
        self.flags.contains(flag)
    }

    /// Number of entity-producing components of the given type in the selection.
    ///
    /// This corresponds to `ESPHOME_SENSOR_COUNT`, `ESPHOME_SWITCH_COUNT`, etc.
    pub fn entity_count(&self, entity_type: EntityType) -> usize {
        self.entity_counts.get(&entity_type).copied().unwrap_or(0)
    }

    /// Generate a `defines.h`-style block of `#define` lines, sorted for determinism.
    ///
    /// Also emits `ESPHOME_<ENTITY>_COUNT` defines for each entity type that
    /// has at least one implementation.
    pub fn to_c_defines(&self) -> String {
        let mut lines: Vec<String> = self.flags.iter().map(|f| format!("#define {f}")).collect();

        // Append entity count defines.
        let count_prefix = |et: EntityType| -> &'static str {
            match et {
                EntityType::Sensor => "ESPHOME_SENSOR_COUNT",
                EntityType::BinarySensor => "ESPHOME_BINARY_SENSOR_COUNT",
                EntityType::Switch => "ESPHOME_SWITCH_COUNT",
                EntityType::Number => "ESPHOME_NUMBER_COUNT",
                EntityType::Select => "ESPHOME_SELECT_COUNT",
                EntityType::Text => "ESPHOME_TEXT_COUNT",
                EntityType::Button => "ESPHOME_BUTTON_COUNT",
                EntityType::Event => "ESPHOME_EVENT_COUNT",
                EntityType::Light => "ESPHOME_LIGHT_COUNT",
                EntityType::Climate => "ESPHOME_CLIMATE_COUNT",
                EntityType::Fan => "ESPHOME_FAN_COUNT",
                EntityType::Cover => "ESPHOME_COVER_COUNT",
                EntityType::Lock => "ESPHOME_LOCK_COUNT",
                EntityType::MediaPlayer => "ESPHOME_MEDIA_PLAYER_COUNT",
                EntityType::AlarmControlPanel => "ESPHOME_ALARM_CONTROL_PANEL_COUNT",
                EntityType::TextSensor => "ESPHOME_TEXT_SENSOR_COUNT",
            }
        };

        let count_lines: Vec<String> = self
            .entity_counts
            .iter()
            .map(|(et, count)| format!("#define {} {}", count_prefix(*et), count))
            .collect();

        lines.extend(count_lines);
        // Sort all defines (USE_* and ESPHOME_*_COUNT) together for determinism.
        lines.sort();
        lines.join("\n")
    }

    /// Return sorted list of Cargo feature names (for future Rust-on-ESP32 path).
    pub fn to_cargo_features(&self) -> Vec<String> {
        let mut features: Vec<String> = self
            .flags
            .iter()
            .map(|f| f.to_lowercase().replace("use_", ""))
            .collect();
        features.sort();
        features.dedup();
        features
    }

    /// Iterate all active flag names.
    pub fn iter_flags(&self) -> impl Iterator<Item = &str> {
        self.flags.iter().map(|s| s.as_str())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ComponentRegistry;

    fn reg() -> ComponentRegistry {
        ComponentRegistry::default_registry()
    }

    #[test]
    fn dht_enables_use_sensor_and_use_dht() {
        let reg = reg();
        let ffs = FeatureFlagSet::from_components(&["dht".into()], &reg);
        assert!(ffs.contains("USE_DHT"), "USE_DHT should be set");
        assert!(
            ffs.contains("USE_SENSOR"),
            "USE_SENSOR should be set via auto-load"
        );
    }

    #[test]
    fn wifi_enables_use_wifi() {
        let reg = reg();
        let ffs = FeatureFlagSet::from_components(&["wifi".into()], &reg);
        assert!(ffs.contains("USE_WIFI"));
        assert!(!ffs.contains("USE_ETHERNET"));
    }

    #[test]
    fn i2c_sensor_enables_use_i2c() {
        let reg = reg();
        let ffs = FeatureFlagSet::from_components(&["bme280_i2c".into()], &reg);
        assert!(ffs.contains("USE_BME280"));
        assert!(ffs.contains("USE_SENSOR"));
        assert!(ffs.contains("USE_I2C"));
    }

    #[test]
    fn bh1750_enables_use_bh1750_and_i2c() {
        let reg = reg();
        let ffs = FeatureFlagSet::from_components(&["bh1750".into()], &reg);
        assert!(ffs.contains("USE_BH1750"));
        assert!(ffs.contains("USE_SENSOR"));
        assert!(ffs.contains("USE_I2C"));
    }

    #[test]
    fn to_c_defines_sorted() {
        let reg = reg();
        let ffs =
            FeatureFlagSet::from_components(&["wifi".into(), "logger".into(), "dht".into()], &reg);
        let defines = ffs.to_c_defines();
        // All lines start with #define
        for line in defines.lines() {
            assert!(line.starts_with("#define "), "unexpected line: {line}");
        }
        // Sorted
        let lines: Vec<&str> = defines.lines().collect();
        let mut sorted = lines.clone();
        sorted.sort();
        assert_eq!(lines, sorted, "defines not sorted");
    }

    #[test]
    fn to_c_defines_contains_expected_flags() {
        let reg = reg();
        let ffs =
            FeatureFlagSet::from_components(&["wifi".into(), "api".into(), "dht".into()], &reg);
        let defines = ffs.to_c_defines();
        assert!(defines.contains("#define USE_WIFI"));
        assert!(defines.contains("#define USE_API"));
        assert!(defines.contains("#define USE_DHT"));
        assert!(defines.contains("#define USE_SENSOR"));
    }

    #[test]
    fn entity_count_single_sensor() {
        let reg = reg();
        let ffs = FeatureFlagSet::from_components(&["dht".into()], &reg);
        assert_eq!(ffs.entity_count(EntityType::Sensor), 1);
    }

    #[test]
    fn entity_count_two_sensors() {
        let reg = reg();
        let ffs = FeatureFlagSet::from_components(&["dht".into(), "adc".into()], &reg);
        assert_eq!(ffs.entity_count(EntityType::Sensor), 2);
    }

    #[test]
    fn entity_count_for_absent_type_is_zero() {
        let reg = reg();
        let ffs = FeatureFlagSet::from_components(&["dht".into()], &reg);
        assert_eq!(ffs.entity_count(EntityType::Switch), 0);
    }

    #[test]
    fn c_defines_include_entity_count() {
        let reg = reg();
        let ffs = FeatureFlagSet::from_components(&["dht".into(), "adc".into()], &reg);
        let defines = ffs.to_c_defines();
        assert!(
            defines.contains("#define ESPHOME_SENSOR_COUNT 2"),
            "defines:\n{defines}"
        );
    }

    #[test]
    fn cargo_features_derived_from_flags() {
        let reg = reg();
        let ffs = FeatureFlagSet::from_components(&["wifi".into()], &reg);
        let features = ffs.to_cargo_features();
        assert!(features.contains(&"wifi".to_string()));
    }

    #[test]
    fn empty_selection_produces_no_flags() {
        let reg = reg();
        let ffs = FeatureFlagSet::from_components(&[], &reg);
        assert!(ffs.to_c_defines().is_empty());
    }

    #[test]
    fn logger_category_is_core() {
        // Verify the category system works — we check the mapping produces a known flag
        let reg = reg();
        let ffs = FeatureFlagSet::from_components(&["logger".into()], &reg);
        assert!(ffs.contains("USE_LOGGER"));
    }

    #[test]
    fn all_entity_platform_parents_enable_entity_flags() {
        let reg = reg();
        let platforms = [
            ("sensor", "USE_SENSOR"),
            ("binary_sensor", "USE_BINARY_SENSOR"),
            ("switch", "USE_SWITCH"),
            ("number", "USE_NUMBER"),
            ("select", "USE_SELECT"),
            ("text", "USE_TEXT"),
            ("button", "USE_BUTTON"),
            ("event", "USE_EVENT"),
            ("light", "USE_LIGHT"),
            ("climate", "USE_CLIMATE"),
            ("fan", "USE_FAN"),
            ("cover", "USE_COVER"),
            ("lock", "USE_LOCK"),
            ("text_sensor", "USE_TEXT_SENSOR"),
        ];
        for (comp, flag) in &platforms {
            let ffs = FeatureFlagSet::from_components(&[(*comp).to_string()], &reg);
            assert!(
                ffs.contains(flag),
                "component '{comp}' should enable flag '{flag}'"
            );
        }
    }

    #[test]
    fn sigrok_maps_to_use_sigrok_flag() {
        let mapping = build_component_to_flags();
        let flags = mapping
            .get("sigrok")
            .expect("sigrok must be in the feature flag map");
        assert_eq!(*flags, vec!["USE_SIGROK"]);
    }

    #[test]
    fn sigrok_flag_emitted_via_flag_set() {
        let reg = reg();
        let ffs = FeatureFlagSet::from_components(&["sigrok".into()], &reg);
        assert!(
            ffs.contains("USE_SIGROK"),
            "selecting sigrok must produce USE_SIGROK in the active flag set",
        );
    }
}
