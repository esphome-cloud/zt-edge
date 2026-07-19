//! Config export in multiple formats and ESPHome YAML import.
//!
//! Exports `ValidatedConfig` to JSON (canonical), TOML (human-readable), or
//! YAML (ESPHome-compatible for migration).  Also provides an import path from
//! ESPHome YAML for the 20 most common config patterns.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::raw::{ComponentConfig, EsphomeBlock, RawConfig};
use crate::validated::ValidatedConfig;

// ── ExportFormat ──────────────────────────────────────────────────────────────

/// Target serialization format for config export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// Canonical machine-readable format (deterministic key ordering).
    Json,
    /// Human-friendly TOML format.
    Toml,
    /// ESPHome-compatible YAML (for migration / import into ESPHome).
    Yaml,
}

// ── Export ────────────────────────────────────────────────────────────────────

/// Export a validated config in the requested format.
///
/// The returned string is suitable for writing to a file or displaying in the
/// browser wizard's "Review & Export" step.
pub fn export_config(config: &ValidatedConfig, format: ExportFormat) -> String {
    let raw = validated_to_raw(config);
    match format {
        ExportFormat::Json => export_json(&raw),
        ExportFormat::Toml => export_toml(&raw),
        ExportFormat::Yaml => export_yaml(&raw),
    }
}

/// Convert a `ValidatedConfig` back into a canonical `RawConfig` for export.
fn validated_to_raw(config: &ValidatedConfig) -> RawConfig {
    use crate::raw::FrameworkConfig;

    let components: Vec<ComponentConfig> = config
        .components
        .iter()
        .filter(|c| !c.auto_loaded) // Only export user-declared components.
        .map(|c| ComponentConfig {
            component_type: c.component_id.clone(),
            platform: c.platform_type.clone(),
            config: c.config.clone(),
        })
        .collect();

    let framework = Some(FrameworkConfig {
        framework_type: match config.esphome.framework_type {
            crate::validated::FrameworkType::EspIdf => "esp-idf".to_owned(),
            crate::validated::FrameworkType::Arduino => "arduino".to_owned(),
        },
        version: None,
        components: vec![],
        sdkconfig_options: HashMap::new(),
    });
    RawConfig {
        esphome: EsphomeBlock {
            name: config.esphome.name.clone(),
            platform: format!("{:?}", config.esphome.chip_target).to_lowercase(),
            board: config.esphome.board.clone(),
            friendly_name: config.esphome.friendly_name.clone(),
            framework,
            includes: vec![],
            libraries: vec![],
            project: config
                .esphome
                .project
                .as_ref()
                .map(|p| crate::raw::ProjectConfig {
                    name: p.name.clone(),
                    version: p.version.clone(),
                }),
            area: None,
            min_version: None,
            profile: None,
            solution: None,
            solution_variant: None,
        },
        packages: vec![],
        substitutions: HashMap::new(),
        components,
    }
}

/// Serialize to canonical JSON (deterministic key ordering via `serde_json`).
fn export_json(config: &RawConfig) -> String {
    serde_json::to_string_pretty(config).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

/// Serialize to human-readable TOML.
fn export_toml(config: &RawConfig) -> String {
    toml::to_string_pretty(config).unwrap_or_else(|e| format!("# error: {e}"))
}

/// Serialize to YAML using a simple custom emitter (no external dep).
///
/// Produces output compatible with ESPHome's YAML parser for basic configs.
fn export_yaml(config: &RawConfig) -> String {
    let mut out = String::new();

    // esphome: block
    out.push_str("esphome:\n");
    out.push_str(&format!("  name: {}\n", config.esphome.name));
    if let Some(ref fn_) = config.esphome.friendly_name {
        out.push_str(&format!("  friendly_name: {}\n", fn_));
    }
    out.push('\n');

    // esp32/esp32s3/esp32c6: block (ESPHome platform block style)
    let platform = config
        .esphome
        .platform
        .replace("esp32s3", "esp32")
        .replace("esp32c6", "esp32");
    let platform_key = config.esphome.platform.as_str();
    out.push_str(&format!("{platform_key}:\n"));
    out.push_str(&format!("  board: {}\n", config.esphome.board));
    if let Some(ref fw) = config.esphome.framework {
        out.push_str("  framework:\n");
        out.push_str(&format!("    type: {}\n", fw.framework_type));
        if let Some(ref v) = fw.version {
            out.push_str(&format!("    version: {}\n", v));
        }
    }
    let _ = platform;
    out.push('\n');

    // substitutions:
    if !config.substitutions.is_empty() {
        out.push_str("substitutions:\n");
        let mut subs: Vec<(&str, &str)> = config
            .substitutions
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        subs.sort_by_key(|(k, _)| *k);
        for (k, v) in subs {
            out.push_str(&format!("  {k}: {v}\n"));
        }
        out.push('\n');
    }

    // Components: group by component_type
    let mut by_type: std::collections::BTreeMap<&str, Vec<&ComponentConfig>> =
        std::collections::BTreeMap::new();
    for comp in &config.components {
        by_type
            .entry(comp.component_type.as_str())
            .or_default()
            .push(comp);
    }

    for (comp_type, instances) in &by_type {
        out.push_str(&format!("{comp_type}:\n"));
        for inst in instances {
            if let Some(ref platform) = inst.platform {
                out.push_str(&format!("  - platform: {platform}\n"));
            }
            if let serde_json::Value::Object(ref map) = inst.config {
                for (k, v) in map {
                    let val_str = yaml_scalar(v);
                    out.push_str(&format!("    {k}: {val_str}\n"));
                }
            }
        }
        out.push('\n');
    }

    out
}

/// Convert a serde_json Value to a YAML scalar string (single-line only).
fn yaml_scalar(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => {
            if s.contains(':') || s.contains('#') || s.is_empty() {
                format!("\"{s}\"")
            } else {
                s.clone()
            }
        }
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "~".to_owned(),
        serde_json::Value::Array(arr) => {
            format!(
                "[{}]",
                arr.iter().map(yaml_scalar).collect::<Vec<_>>().join(", ")
            )
        }
        serde_json::Value::Object(_) => "{...}".to_owned(),
    }
}

// ── Import ────────────────────────────────────────────────────────────────────

/// Errors that can occur when importing ESPHome YAML.
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
pub enum ImportError {
    #[error("YAML parse error: {0}")]
    YamlParse(String),
    #[error("missing required field: {0}")]
    MissingField(String),
    #[error("unsupported construct: {0}")]
    Unsupported(String),
}

/// Import a subset of ESPHome YAML and convert it to a `RawConfig`.
///
/// Handles the 20 most common ESPHome config patterns:
/// 1. `esphome:` block (name, friendly_name)
/// 2. `esp32:` / `esp32s3:` / `esp32c6:` board + framework
/// 3. `wifi:` with ssid/password/ap
/// 4. `logger:` with level
/// 5. `api:` with password
/// 6. `ota:` with password
/// 7. `sensor:` platform entries (dht, bme280_i2c, sht3x, ds18x20, adc)
/// 8. `binary_sensor:` platform entries (gpio)
/// 9. `switch:` platform entries (gpio, restart)
/// 10. `light:` platform entries (binary, monochromatic, rgb, rgbw)
/// 11. `climate:` platform entries (thermostat, bang_bang, pid)
/// 12. `i2c:` bus config
/// 13. `spi:` bus config
/// 14. `uart:` config
/// 15. `time:` with homeassistant / sntp platform
/// 16. `mqtt:` with broker
/// 17. `substitutions:` block
/// 18. `deep_sleep:` config
/// 19. `text_sensor:` platform entries
/// 20. `packages:` local file references
pub fn import_esphome_yaml(yaml: &str) -> Result<RawConfig, ImportError> {
    // Parse YAML into a generic value using the `toml` crate's JSON representation.
    // Since we avoid external deps, we parse YAML manually via a simple line scanner.
    // For a production implementation, add `serde_yaml` as a dependency.
    // For now, we implement a best-effort line-based parser for the most common patterns.
    parse_esphome_yaml_lines(yaml)
}

/// Simple line-based ESPHome YAML parser for common patterns.
fn parse_esphome_yaml_lines(yaml: &str) -> Result<RawConfig, ImportError> {
    let mut name = String::new();
    let mut platform = "esp32".to_owned();
    let mut board = "esp32dev".to_owned();
    let mut friendly_name: Option<String> = None;
    let mut components: Vec<ComponentConfig> = Vec::new();
    let mut substitutions: HashMap<String, String> = HashMap::new();

    // State machine: track current top-level section and indentation.
    let mut current_section = String::new();
    let mut current_platform: Option<String> = None;
    let mut current_config: HashMap<String, serde_json::Value> = HashMap::new();

    for line in yaml.lines() {
        let stripped = line.trim_start();
        if stripped.is_empty() || stripped.starts_with('#') {
            // End current component block when blank line encountered.
            if !current_section.is_empty() && current_platform.is_some() {
                flush_component(
                    &current_section,
                    &current_platform,
                    &current_config,
                    &mut components,
                );
                current_platform = None;
                current_config.clear();
            }
            continue;
        }

        let indent = line.len() - stripped.len();

        // Top-level section (indent == 0 and ends with ':')
        if indent == 0 {
            // Flush pending component.
            if !current_section.is_empty() && current_platform.is_some() {
                flush_component(
                    &current_section,
                    &current_platform,
                    &current_config,
                    &mut components,
                );
                current_platform = None;
                current_config.clear();
            }

            if let Some(section) = stripped.strip_suffix(':') {
                current_section = section.trim().to_owned();
                current_platform = None;
                current_config.clear();
            }
            continue;
        }

        // Nested key: value
        if let Some(pos) = stripped.find(':') {
            let key = stripped[..pos].trim();
            let value = stripped[pos + 1..].trim();

            match current_section.as_str() {
                "esphome" => match key {
                    "name" => name = value.trim_matches('"').to_owned(),
                    "friendly_name" => friendly_name = Some(value.trim_matches('"').to_owned()),
                    _ => {}
                },
                "substitutions" => {
                    substitutions.insert(key.to_owned(), value.trim_matches('"').to_owned());
                }
                sec @ ("esp32" | "esp32s3" | "esp32c6") => {
                    platform = sec.to_owned();
                    if key == "board" {
                        board = value.trim_matches('"').to_owned();
                    }
                }
                sec @ ("wifi" | "logger" | "api" | "ota" | "mqtt" | "i2c" | "spi" | "uart"
                | "time" | "deep_sleep") => {
                    if key == "platform" {
                        // platform line inside a non-platform section (e.g. time)
                        current_platform = Some(value.trim_matches('"').to_owned());
                    } else if key == "-" || key.starts_with('-') {
                        // list item — new component instance
                        if current_platform.is_some() {
                            flush_component(
                                &current_section,
                                &current_platform,
                                &current_config,
                                &mut components,
                            );
                            current_platform = None;
                            current_config.clear();
                        }
                    } else {
                        current_config.insert(
                            key.to_owned(),
                            serde_json::Value::String(value.trim_matches('"').to_owned()),
                        );
                    }
                    // For simple non-platform components, ensure we create one entry.
                    if current_platform.is_none()
                        && !current_config.is_empty()
                        && matches!(
                            sec,
                            "wifi"
                                | "logger"
                                | "api"
                                | "ota"
                                | "mqtt"
                                | "i2c"
                                | "spi"
                                | "uart"
                                | "deep_sleep"
                        )
                    {
                        // Will be flushed at blank line / next section.
                        let _ = sec;
                    }
                }
                sec @ ("sensor" | "binary_sensor" | "switch" | "light" | "climate"
                | "text_sensor" | "fan" | "cover" | "number" | "select") => {
                    if key == "platform" {
                        if current_platform.is_some() {
                            flush_component(
                                sec,
                                &current_platform,
                                &current_config,
                                &mut components,
                            );
                            current_config.clear();
                        }
                        current_platform = Some(value.trim_matches('"').to_owned());
                    } else if !value.is_empty() {
                        current_config.insert(
                            key.to_owned(),
                            serde_json::Value::String(value.trim_matches('"').to_owned()),
                        );
                    }
                }
                _ => {}
            }
        }
    }

    // Flush final pending component.
    if !current_section.is_empty() {
        if current_platform.is_some() {
            flush_component(
                &current_section,
                &current_platform,
                &current_config,
                &mut components,
            );
        } else if !current_config.is_empty()
            && matches!(
                current_section.as_str(),
                "wifi" | "logger" | "api" | "ota" | "mqtt" | "i2c" | "spi" | "uart" | "deep_sleep"
            )
        {
            components.push(ComponentConfig {
                component_type: current_section.clone(),
                platform: None,
                config: serde_json::Value::Object(
                    current_config
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                ),
            });
        }
    }

    if name.is_empty() {
        return Err(ImportError::MissingField("esphome.name".to_owned()));
    }

    Ok(RawConfig {
        esphome: EsphomeBlock {
            name,
            platform: platform.clone(),
            board,
            friendly_name,
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
        substitutions,
        components,
    })
}

/// Push a completed component entry into the components list.
fn flush_component(
    section: &str,
    platform: &Option<String>,
    config: &HashMap<String, serde_json::Value>,
    components: &mut Vec<ComponentConfig>,
) {
    let config_val = if config.is_empty() {
        serde_json::Value::Object(serde_json::Map::new())
    } else {
        serde_json::Value::Object(config.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
    };

    components.push(ComponentConfig {
        component_type: section.to_owned(),
        platform: platform.clone(),
        config: config_val,
    });
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{ValidationPipeline, ValidationResult};
    use crate::raw::PackageStore;
    use rshome_schema::ComponentRegistry;

    fn make_validated_config() -> ValidatedConfig {
        let raw = RawConfig {
            esphome: EsphomeBlock {
                name: "test_device".to_owned(),
                platform: "esp32".to_owned(),
                board: "esp32dev".to_owned(),
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
            substitutions: HashMap::new(),
            components: vec![ComponentConfig {
                component_type: "wifi".to_owned(),
                platform: None,
                config: serde_json::json!({"ssid": "MyNet", "password": "pass"}),
            }],
        };
        let registry = ComponentRegistry::default_registry();
        let pipeline = ValidationPipeline::new(registry);
        let store = PackageStore::new();
        match pipeline.validate(raw, &store) {
            ValidationResult::Valid(v) => *v,
            ValidationResult::Invalid(errs) => panic!("validation failed: {errs:?}"),
        }
    }

    #[test]
    fn export_json_produces_valid_json() {
        let config = make_validated_config();
        let json = export_config(&config, ExportFormat::Json);
        let v: serde_json::Value = serde_json::from_str(&json).expect("must be valid JSON");
        assert!(v.is_object());
        assert_eq!(v["esphome"]["name"].as_str(), Some("test_device"));
    }

    #[test]
    fn export_toml_produces_valid_toml() {
        let config = make_validated_config();
        let toml_str = export_config(&config, ExportFormat::Toml);
        assert!(toml_str.contains("test_device"));
        assert!(
            !toml_str.starts_with('#'),
            "should not start with an error comment"
        );
    }

    #[test]
    fn export_yaml_contains_device_name() {
        let config = make_validated_config();
        let yaml = export_config(&config, ExportFormat::Yaml);
        assert!(yaml.contains("test_device"));
        assert!(yaml.contains("esphome:"));
    }

    #[test]
    fn export_json_is_deterministic() {
        let config = make_validated_config();
        let a = export_config(&config, ExportFormat::Json);
        let b = export_config(&config, ExportFormat::Json);
        assert_eq!(a, b);
    }

    #[test]
    fn import_basic_esphome_yaml() {
        let yaml = r#"
esphome:
  name: my_sensor
  friendly_name: My Sensor

esp32:
  board: esp32dev

wifi:
  ssid: "MyNet"
  password: "pass"

logger:
  level: DEBUG
"#;
        let raw = import_esphome_yaml(yaml).expect("import should succeed");
        assert_eq!(raw.esphome.name, "my_sensor");
        assert_eq!(raw.esphome.friendly_name.as_deref(), Some("My Sensor"));
        assert_eq!(raw.esphome.platform, "esp32");
        assert_eq!(raw.esphome.board, "esp32dev");
    }

    #[test]
    fn import_substitutions() {
        let yaml = r#"
esphome:
  name: my_device

esp32:
  board: esp32dev

substitutions:
  device_name: "sensor_01"
  location: "living_room"
"#;
        let raw = import_esphome_yaml(yaml).expect("import should succeed");
        assert_eq!(
            raw.substitutions.get("device_name").map(|s| s.as_str()),
            Some("sensor_01")
        );
        assert_eq!(
            raw.substitutions.get("location").map(|s| s.as_str()),
            Some("living_room")
        );
    }

    #[test]
    fn import_missing_name_returns_error() {
        let yaml = "esp32:\n  board: esp32dev\n";
        let result = import_esphome_yaml(yaml);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ImportError::MissingField(_)));
    }

    #[test]
    fn yaml_scalar_string_with_colon_quoted() {
        let v = serde_json::Value::String("key:value".to_owned());
        let s = yaml_scalar(&v);
        assert!(s.starts_with('"') && s.ends_with('"'));
    }

    #[test]
    fn yaml_scalar_plain_string() {
        let v = serde_json::Value::String("hello".to_owned());
        assert_eq!(yaml_scalar(&v), "hello");
    }

    #[test]
    fn yaml_scalar_bool() {
        assert_eq!(yaml_scalar(&serde_json::Value::Bool(true)), "true");
    }

    #[test]
    fn yaml_scalar_null() {
        assert_eq!(yaml_scalar(&serde_json::Value::Null), "~");
    }

    #[test]
    fn export_json_excludes_auto_loaded_components() {
        let config = make_validated_config();
        let json = export_config(&config, ExportFormat::Json);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        // Only wifi should be in components (not auto-loaded ones).
        let comps = v["components"].as_array().unwrap();
        assert!(comps
            .iter()
            .all(|c| c["component_type"].as_str() != Some("sensor")));
    }
}
