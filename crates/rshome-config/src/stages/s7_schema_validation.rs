//! Stage 7 — Schema validation (type checking).
//!
//! Validates every component's config values against the expected schema:
//! - Required fields must be present.
//! - Field types must match (string, number, boolean, object, array).
//! - Enum fields must use a known variant.
//! - Numeric values must be within valid ranges.
//!
//! Errors use precise JSON-path-like locations:
//! `"sensor[0].platform.dht.pin"` rather than just `"sensor"`.

use rshome_schema::ComponentRegistry;

use crate::error::{ValidationError, ValidationStage};
use crate::raw::{ComponentConfig, RawConfig};

// ── Field descriptor ──────────────────────────────────────────────────────────

/// Expected type for a schema field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    String,
    Integer,
    Float,
    Boolean,
    Object,
    Array,
    /// Any JSON value is acceptable.
    Any,
}

/// Descriptor for a single schema field.
#[derive(Debug, Clone)]
pub struct FieldDef {
    pub name: &'static str,
    pub field_type: FieldType,
    pub required: bool,
    pub enum_values: Option<&'static [&'static str]>,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

impl FieldDef {
    const fn req(name: &'static str, t: FieldType) -> Self {
        Self {
            name,
            field_type: t,
            required: true,
            enum_values: None,
            min: None,
            max: None,
        }
    }

    const fn opt(name: &'static str, t: FieldType) -> Self {
        Self {
            name,
            field_type: t,
            required: false,
            enum_values: None,
            min: None,
            max: None,
        }
    }

    const fn with_enum(mut self, variants: &'static [&'static str]) -> Self {
        self.enum_values = Some(variants);
        self
    }

    const fn with_range(mut self, min: f64, max: f64) -> Self {
        self.min = Some(min);
        self.max = Some(max);
        self
    }
}

// ── Built-in schemas ──────────────────────────────────────────────────────────

/// Common entity fields present on every entity component.
const ENTITY_COMMON: &[FieldDef] = &[
    FieldDef::req("name", FieldType::String),
    FieldDef::opt("id", FieldType::String),
    FieldDef::opt("icon", FieldType::String),
    FieldDef::opt("internal", FieldType::Boolean),
    FieldDef::opt("disabled_by_default", FieldType::Boolean),
    FieldDef::opt("entity_category", FieldType::String).with_enum(&[
        "none",
        "config",
        "diagnostic",
    ]),
];

const SENSOR_FIELDS: &[FieldDef] = &[
    FieldDef::opt("device_class", FieldType::String),
    FieldDef::opt("unit_of_measurement", FieldType::String),
    FieldDef::opt("accuracy_decimals", FieldType::Integer).with_range(0.0, 10.0),
    FieldDef::opt("state_class", FieldType::String).with_enum(&[
        "measurement",
        "total",
        "total_increasing",
    ]),
    FieldDef::opt("filters", FieldType::Array),
    FieldDef::opt("force_update", FieldType::Boolean),
    FieldDef::opt("expire_after", FieldType::Integer).with_range(0.0, u32::MAX as f64),
];

const DHT_FIELDS: &[FieldDef] = &[
    FieldDef::req("pin", FieldType::Integer).with_range(0.0, 48.0),
    FieldDef::opt("model", FieldType::String).with_enum(&[
        "AUTO_DETECT",
        "DHT11",
        "DHT22",
        "AM2302",
        "RHT03",
    ]),
    FieldDef::opt("update_interval", FieldType::String),
    FieldDef::opt("temperature", FieldType::Object),
    FieldDef::opt("humidity", FieldType::Object),
];

const WIFI_FIELDS: &[FieldDef] = &[
    FieldDef::opt("provisioning_mode", FieldType::String).with_enum(&[
        "nvs",
        "build_input",
    ]),
    FieldDef::opt("ssid", FieldType::String),
    FieldDef::opt("password", FieldType::String),
    FieldDef::opt("service_name", FieldType::String),
    FieldDef::opt("pop", FieldType::String),
    FieldDef::opt("networks", FieldType::Array),
    FieldDef::opt("ap", FieldType::Object),
    FieldDef::opt("reboot_timeout", FieldType::String),
    FieldDef::opt("power_save_mode", FieldType::String).with_enum(&["none", "light", "high"]),
    FieldDef::opt("fast_connect", FieldType::Boolean),
    FieldDef::opt("domain", FieldType::String),
];

const LOGGER_FIELDS: &[FieldDef] = &[
    FieldDef::opt("level", FieldType::String).with_enum(&[
        "NONE",
        "ERROR",
        "WARN",
        "INFO",
        "DEBUG",
        "VERBOSE",
        "VERY_VERBOSE",
    ]),
    FieldDef::opt("baud_rate", FieldType::Integer).with_range(0.0, 921600.0),
    FieldDef::opt("tx_buffer_size", FieldType::Integer).with_range(0.0, 65536.0),
    FieldDef::opt("hardware_uart", FieldType::String),
    FieldDef::opt("logs", FieldType::Object),
];

const API_FIELDS: &[FieldDef] = &[
    FieldDef::opt("password", FieldType::String),
    FieldDef::opt("port", FieldType::Integer).with_range(1.0, 65535.0),
    FieldDef::opt("reboot_timeout", FieldType::String),
    FieldDef::opt("encryption", FieldType::Object),
    FieldDef::opt("services", FieldType::Array),
];

const OTA_FIELDS: &[FieldDef] = &[
    FieldDef::opt("password", FieldType::String),
    FieldDef::opt("safe_mode", FieldType::Boolean),
    FieldDef::opt("reboot_timeout", FieldType::Integer),
    FieldDef::opt("num_attempts", FieldType::Integer).with_range(1.0, 100.0),
    FieldDef::opt("platform", FieldType::String),
];

const I2C_FIELDS: &[FieldDef] = &[
    FieldDef::opt("sda", FieldType::Integer).with_range(0.0, 48.0),
    FieldDef::opt("scl", FieldType::Integer).with_range(0.0, 48.0),
    FieldDef::opt("frequency", FieldType::Integer),
    FieldDef::opt("scan", FieldType::Boolean),
];

const SWITCH_FIELDS: &[FieldDef] = &[
    FieldDef::opt("restore_mode", FieldType::String).with_enum(&[
        "RESTORE_DEFAULT_OFF",
        "RESTORE_DEFAULT_ON",
        "ALWAYS_OFF",
        "ALWAYS_ON",
        "RESTORE_INVERTED_DEFAULT_OFF",
        "RESTORE_INVERTED_DEFAULT_ON",
        "DISABLED",
    ]),
    FieldDef::opt("inverted", FieldType::Boolean),
    FieldDef::opt("on_turn_on", FieldType::Array),
    FieldDef::opt("on_turn_off", FieldType::Array),
];

const GPIO_SWITCH_FIELDS: &[FieldDef] =
    &[FieldDef::req("pin", FieldType::Integer).with_range(0.0, 48.0)];

const BINARY_SENSOR_FIELDS: &[FieldDef] = &[
    FieldDef::opt("device_class", FieldType::String),
    FieldDef::opt("filters", FieldType::Array),
    FieldDef::opt("on_press", FieldType::Array),
    FieldDef::opt("on_release", FieldType::Array),
];

#[allow(dead_code)]
const GPIO_BINARY_SENSOR_FIELDS: &[FieldDef] =
    &[FieldDef::req("pin", FieldType::Integer).with_range(0.0, 48.0)];

// ── Schema lookup ─────────────────────────────────────────────────────────────

/// Return the field schema for a given component ID.
fn fields_for(component_id: &str) -> Option<(&'static [FieldDef], bool)> {
    // Returns (fields, has_entity_common)
    match component_id {
        "dht" => Some((DHT_FIELDS, false)),
        "wifi" => Some((WIFI_FIELDS, false)),
        "logger" => Some((LOGGER_FIELDS, false)),
        "api" => Some((API_FIELDS, false)),
        "ota" => Some((OTA_FIELDS, false)),
        "i2c" => Some((I2C_FIELDS, false)),
        "sensor" => Some((SENSOR_FIELDS, true)),
        "switch" => Some((SWITCH_FIELDS, true)),
        "gpio" => Some((GPIO_SWITCH_FIELDS, false)),
        "binary_sensor" => Some((BINARY_SENSOR_FIELDS, true)),
        _ => None,
    }
}

// ── Validation helpers ────────────────────────────────────────────────────────

fn check_field_type(value: &serde_json::Value, expected: FieldType) -> bool {
    match expected {
        FieldType::String => value.is_string(),
        FieldType::Integer => value.is_i64() || value.is_u64(),
        FieldType::Float => value.is_number(),
        FieldType::Boolean => value.is_boolean(),
        FieldType::Object => value.is_object(),
        FieldType::Array => value.is_array(),
        FieldType::Any => true,
    }
}

fn validate_fields(
    config: &serde_json::Value,
    fields: &[FieldDef],
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    let obj = match config.as_object() {
        Some(o) => o,
        None => return,
    };

    for field in fields {
        match obj.get(field.name) {
            None if field.required => {
                errors.push(ValidationError::error(
                    ValidationStage::SchemaValidation,
                    format!("{path}.{}", field.name),
                    format!("required field '{}' is missing", field.name),
                ));
            }
            None => {} // optional and absent — ok
            Some(val) if val.is_null() && field.required => {
                errors.push(ValidationError::error(
                    ValidationStage::SchemaValidation,
                    format!("{path}.{}", field.name),
                    format!("required field '{}' must not be null", field.name),
                ));
            }
            Some(val) => {
                let field_path = format!("{path}.{}", field.name);

                // Type check.
                if !check_field_type(val, field.field_type) {
                    errors.push(ValidationError::error(
                        ValidationStage::SchemaValidation,
                        &field_path,
                        format!(
                            "field '{}' expected {:?} but got {}",
                            field.name,
                            field.field_type,
                            json_type_name(val)
                        ),
                    ));
                    continue;
                }

                // Enum check.
                if let Some(variants) = field.enum_values {
                    if let Some(s) = val.as_str() {
                        if !variants.iter().any(|&v| v.eq_ignore_ascii_case(s)) {
                            errors.push(
                                ValidationError::error(
                                    ValidationStage::SchemaValidation,
                                    &field_path,
                                    format!("invalid value '{}' for '{}'", s, field.name),
                                )
                                .with_suggestion(format!("valid values: {}", variants.join(", "))),
                            );
                        }
                    }
                }

                // Range check.
                if let (Some(min), Some(max)) = (field.min, field.max) {
                    if let Some(n) = val.as_f64() {
                        if n < min || n > max {
                            errors.push(ValidationError::error(
                                ValidationStage::SchemaValidation,
                                &field_path,
                                format!(
                                    "value {n} for '{}' is outside valid range [{min}, {max}]",
                                    field.name
                                ),
                            ));
                        }
                    }
                }
            }
        }
    }
}

fn json_type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

// ── Stage entry point ─────────────────────────────────────────────────────────

/// Stage 7: validate all component configs against their schema definitions.
pub fn stage_7_validate_schemas(
    config: &RawConfig,
    _registry: &ComponentRegistry,
) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    // Group components by type for indexing.
    let mut type_counters: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();

    for comp in &config.components {
        let type_idx = {
            let ctr = type_counters
                .entry(comp.component_type.clone())
                .or_insert(0);
            let idx = *ctr;
            *ctr += 1;
            idx
        };

        let effective_id = effective_component_id(comp);
        let path = format!("{}[{type_idx}]", comp.component_type);

        validate_component(&effective_id, comp, &path, &mut errors);
    }

    errors
}

fn effective_component_id(comp: &ComponentConfig) -> String {
    if let Some(platform) = &comp.platform {
        if !platform.is_empty() {
            return platform.clone();
        }
    }
    comp.component_type.clone()
}

fn validate_component(
    component_id: &str,
    comp: &ComponentConfig,
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    if let Some((fields, has_common)) = fields_for(component_id) {
        if has_common {
            validate_fields(&comp.config, ENTITY_COMMON, path, errors);
        }
        validate_fields(&comp.config, fields, path, errors);
    }
    // Components without a schema entry are skipped (no validation errors).
}

#[cfg(test)]
mod tests {
    use rshome_schema::ComponentRegistry;

    use super::*;
    use crate::raw::{ComponentConfig, EsphomeBlock, RawConfig};
    use serde_json::json;

    fn base_config() -> RawConfig {
        RawConfig {
            esphome: EsphomeBlock {
                name: "test".into(),
                platform: "esp32".into(),
                board: "esp32dev".into(),
                friendly_name: None,
                framework: None,
                includes: vec![],
                libraries: vec![],
                project: None,
                area: None,
                min_version: None,
                profile: None,
                solution: None,
                solution_variant: None,
            },
            packages: vec![],
            substitutions: Default::default(),
            components: vec![],
        }
    }

    fn push(config: &mut RawConfig, typ: &str, platform: Option<&str>, val: serde_json::Value) {
        config.components.push(ComponentConfig {
            component_type: typ.into(),
            platform: platform.map(str::to_owned),
            config: val,
        });
    }

    fn empty_registry() -> ComponentRegistry {
        ComponentRegistry::new()
    }

    #[test]
    fn dht_missing_pin_produces_error() {
        let mut config = base_config();
        push(
            &mut config,
            "sensor",
            Some("dht"),
            json!({"name": "temp", "model": "DHT22"}),
        );
        let errors = stage_7_validate_schemas(&config, &empty_registry());
        assert!(errors
            .iter()
            .any(|e| e.message.contains("pin") && e.is_fatal()));
    }

    #[test]
    fn dht_valid_config_no_errors() {
        let mut config = base_config();
        push(
            &mut config,
            "sensor",
            Some("dht"),
            json!({"name": "temp", "pin": 4, "model": "DHT22"}),
        );
        let errors = stage_7_validate_schemas(&config, &empty_registry());
        assert!(errors.is_empty(), "errors: {:?}", errors);
    }

    #[test]
    fn invalid_enum_value_produces_error() {
        let mut config = base_config();
        push(
            &mut config,
            "logger",
            None,
            json!({"level": "ULTRA_VERBOSE"}),
        );
        let errors = stage_7_validate_schemas(&config, &empty_registry());
        assert!(errors.iter().any(|e| e.message.contains("ULTRA_VERBOSE")));
    }

    #[test]
    fn valid_enum_value_passes() {
        let mut config = base_config();
        push(&mut config, "logger", None, json!({"level": "DEBUG"}));
        let errors = stage_7_validate_schemas(&config, &empty_registry());
        assert!(errors.is_empty(), "errors: {:?}", errors);
    }

    #[test]
    fn pin_out_of_range_produces_error() {
        let mut config = base_config();
        push(
            &mut config,
            "sensor",
            Some("dht"),
            json!({"name": "temp", "pin": 99}),
        );
        let errors = stage_7_validate_schemas(&config, &empty_registry());
        assert!(errors
            .iter()
            .any(|e| e.message.contains("outside valid range")));
    }

    #[test]
    fn wrong_type_produces_error() {
        let mut config = base_config();
        // pin should be integer, not string
        push(
            &mut config,
            "sensor",
            Some("dht"),
            json!({"name": "temp", "pin": "not_a_number"}),
        );
        let errors = stage_7_validate_schemas(&config, &empty_registry());
        assert!(errors
            .iter()
            .any(|e| e.message.contains("expected Integer")));
    }

    #[test]
    fn entity_common_name_required_for_sensor() {
        let mut config = base_config();
        // sensor with entity common but no name
        push(&mut config, "sensor", None, json!({}));
        let errors = stage_7_validate_schemas(&config, &empty_registry());
        assert!(errors
            .iter()
            .any(|e| e.message.contains("name") && e.is_fatal()));
    }

    #[test]
    fn wifi_valid_minimal_config() {
        let mut config = base_config();
        push(
            &mut config,
            "wifi",
            None,
            json!({"provisioning_mode": "nvs"}),
        );
        let errors = stage_7_validate_schemas(&config, &empty_registry());
        assert!(errors.is_empty(), "errors: {:?}", errors);
    }

    #[test]
    fn wifi_rejects_legacy_static_mode() {
        let mut config = base_config();
        push(
            &mut config,
            "wifi",
            None,
            json!({"provisioning_mode": "hardcoded"}),
        );
        let errors = stage_7_validate_schemas(&config, &empty_registry());
        assert!(errors.iter().any(|error| error.message.contains("hardcoded")));
    }

    #[test]
    fn wifi_rejects_legacy_ble_mode() {
        let mut config = base_config();
        push(
            &mut config,
            "wifi",
            None,
            json!({"provisioning_mode": "ble_prov"}),
        );
        let errors = stage_7_validate_schemas(&config, &empty_registry());
        assert!(errors.iter().any(|error| error.message.contains("ble_prov")));
    }

    #[test]
    fn wifi_rejects_legacy_smartconfig_mode() {
        let mut config = base_config();
        push(
            &mut config,
            "wifi",
            None,
            json!({"provisioning_mode": "smartconfig"}),
        );
        let errors = stage_7_validate_schemas(&config, &empty_registry());
        assert!(errors.iter().any(|error| error.message.contains("smartconfig")));
    }

    #[test]
    fn wifi_invalid_provisioning_mode() {
        let mut config = base_config();
        push(
            &mut config,
            "wifi",
            None,
            json!({"provisioning_mode": "zigbee"}),
        );
        let errors = stage_7_validate_schemas(&config, &empty_registry());
        assert!(errors.iter().any(|e| e.message.contains("zigbee")));
    }

    #[test]
    fn api_port_out_of_range() {
        let mut config = base_config();
        push(&mut config, "api", None, json!({"port": 99999}));
        let errors = stage_7_validate_schemas(&config, &empty_registry());
        assert!(errors
            .iter()
            .any(|e| e.message.contains("outside valid range")));
    }

    #[test]
    fn i2c_boolean_scan_field() {
        let mut config = base_config();
        push(
            &mut config,
            "i2c",
            None,
            json!({"sda": 21, "scl": 22, "scan": true}),
        );
        let errors = stage_7_validate_schemas(&config, &empty_registry());
        assert!(errors.is_empty(), "errors: {:?}", errors);
    }

    #[test]
    fn unknown_component_type_skipped_no_error() {
        // Components without schema entries should not produce errors.
        let mut config = base_config();
        push(
            &mut config,
            "custom_component",
            None,
            json!({"anything": "goes"}),
        );
        let errors = stage_7_validate_schemas(&config, &empty_registry());
        assert!(errors.is_empty());
    }
}
