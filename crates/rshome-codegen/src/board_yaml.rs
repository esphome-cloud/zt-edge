//! Board YAML generator — renders `HardwareAssemblyDefinition` into
//! `esp_board_manager` YAML files (board_info, board_peripherals, board_devices).

use std::path::{Path, PathBuf};

use rshome_schema::assembly::HardwareAssemblyDefinition;

use crate::error::CodegenError;
use crate::generator::write_file;

// ── YAML value formatting ──────────────────────────────────────────────────

/// Render a `serde_json::Value` as a YAML scalar. Aborts on non-scalar
/// values — callers needing nested trees should use [`render_yaml_value`]
/// which routes objects/arrays through block-style emission.
fn yaml_scalar(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => {
            if s.contains(' ') || s.contains(':') || s.contains('#') {
                format!("\"{}\"", s)
            } else {
                s.clone()
            }
        }
        serde_json::Value::Null => "null".into(),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            // Should never reach here from `render_yaml_value`; leaving a
            // debuggable fallback rather than unreachable!() in case a new
            // caller slips in later.
            v.to_string()
        }
    }
}

/// Render a `serde_json::Value` into YAML — scalars inline, maps and
/// arrays as block-style on their own lines. The caller passes `indent`
/// (spaces) for nested content.
///
/// When the value is an object or array, the returned string starts with
/// `\n` so the caller can write `format!("{key}:{rendered}")` and get
/// well-formed block YAML. Scalars are returned inline (no leading
/// newline) and the caller writes `format!("{key}: {rendered}")`.
fn render_yaml_value(v: &serde_json::Value, indent: usize) -> String {
    match v {
        serde_json::Value::Object(map) if map.is_empty() => " {}".into(),
        serde_json::Value::Object(map) => {
            let pad = " ".repeat(indent);
            let mut out = String::new();
            for (k, val) in map {
                out.push('\n');
                out.push_str(&pad);
                out.push_str(k);
                out.push(':');
                match val {
                    serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                        out.push_str(&render_yaml_value(val, indent + 2));
                    }
                    _ => {
                        out.push(' ');
                        out.push_str(&yaml_scalar(val));
                    }
                }
            }
            out
        }
        serde_json::Value::Array(arr) if arr.is_empty() => " []".into(),
        serde_json::Value::Array(arr) => {
            let pad = " ".repeat(indent);
            let mut out = String::new();
            for item in arr {
                out.push('\n');
                out.push_str(&pad);
                out.push_str("- ");
                match item {
                    serde_json::Value::Object(map) if !map.is_empty() => {
                        // List-of-maps emits the first key inline with the dash
                        // and subsequent keys at (indent + 2) so the YAML lines
                        // up as:   - key_a: val
                        //            key_b: val
                        let mut first = true;
                        for (k, val) in map {
                            if !first {
                                out.push('\n');
                                out.push_str(&" ".repeat(indent + 2));
                            }
                            first = false;
                            out.push_str(k);
                            out.push(':');
                            match val {
                                serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                                    out.push_str(&render_yaml_value(val, indent + 4));
                                }
                                _ => {
                                    out.push(' ');
                                    out.push_str(&yaml_scalar(val));
                                }
                            }
                        }
                    }
                    serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                        // Nested container after the dash — emit block-style
                        // continuation at a deeper indent.
                        out.push_str(render_yaml_value(item, indent + 2).trim_start_matches('\n'));
                    }
                    _ => out.push_str(&yaml_scalar(item)),
                }
            }
            out
        }
        _ => yaml_scalar(v),
    }
}

// ── Device-type translation ────────────────────────────────────────────────

// esp_board_manager v0.5.x ships these device-parser directories
// (see espressif/esp-gmf/packages/esp_board_manager/devices/).
const ESP_BOARD_MANAGER_BUILTIN_DEVICE_TYPES: &[&str] = &[
    "audio_codec",
    "button",
    "camera",
    "custom",
    "display_lcd",
    "fs_fat",
    "fs_spiffs",
    "gpio_ctrl",
    "gpio_expander",
    "lcd_touch_i2c",
    "ledc_ctrl",
    "power_ctrl",
    // Legacy rshome-codegen types already shipped; keep them passing
    // through unchanged so existing fixture assemblies still parse.
    "i2c_sensor",
    "onewire_sensor",
    "ds18b20",
    "dht",
];

/// Map our logical device_type string to the concrete `type:` value emitted
/// into board_devices.yaml. Anything not in the esp_board_manager built-in
/// set gets coerced to `custom` — the built-in no-op device type that
/// registers the device for `get_device_handle` without running any driver
/// init.
fn resolve_device_type_yaml(device_type: &str) -> &str {
    if ESP_BOARD_MANAGER_BUILTIN_DEVICE_TYPES.contains(&device_type) {
        device_type
    } else {
        "custom"
    }
}

// ── Generator ──────────────────────────────────────────────────────────────

/// Generates `esp_board_manager` YAML files from a `HardwareAssemblyDefinition`.
pub struct BoardYamlGenerator<'a> {
    assembly: &'a HardwareAssemblyDefinition,
}

impl<'a> BoardYamlGenerator<'a> {
    pub fn new(assembly: &'a HardwareAssemblyDefinition) -> Self {
        Self { assembly }
    }

    /// Generate `board_info.yaml` content.
    pub fn generate_board_info(&self) -> String {
        let a = self.assembly;
        let mut lines = vec!["# Auto-generated by rshome-codegen. Do not edit.".into()];
        lines.push(format!("board_name: \"{}\"", a.label));
        lines.push(format!("chip: {}", a.target.to_idf_target()));
        lines.push("version: \"1.0\"".into());
        lines.push(format!("description: \"{}\"", a.description));
        lines.join("\n")
    }

    /// Generate `board_peripherals.yaml` content.
    pub fn generate_peripherals(&self) -> String {
        let mut out = String::from("# Auto-generated by rshome-codegen. Do not edit.\n");
        out.push_str("peripherals:");

        for p in &self.assembly.peripherals {
            out.push_str(&format!("\n  - name: {}", p.name));
            out.push_str(&format!("\n    type: {}", p.periph_type));
            out.push_str(&format!("\n    role: {}", p.role));
            if !p.config.is_empty() {
                out.push_str("\n    config:");
                for (k, v) in &p.config {
                    out.push_str(&format!("\n      {}:", k));
                    match v {
                        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                            // Block-style nested value (leading \n built in).
                            out.push_str(&render_yaml_value(v, 8));
                        }
                        _ => {
                            out.push(' ');
                            out.push_str(&yaml_scalar(v));
                        }
                    }
                }
            }
        }

        out
    }

    /// Generate `board_devices.yaml` content.
    pub fn generate_devices(&self) -> String {
        let mut out = String::from("# Auto-generated by rshome-codegen. Do not edit.\n");
        out.push_str("devices:");

        for d in &self.assembly.devices {
            out.push_str(&format!("\n  - name: {}", d.name));
            out.push_str(&format!(
                "\n    type: {}",
                resolve_device_type_yaml(&d.device_type)
            ));
            if !d.peripherals.is_empty() {
                out.push_str("\n    peripherals:");
                for pref in &d.peripherals {
                    out.push_str(&format!("\n      - {}", pref));
                }
            }
            if !d.config.is_empty() {
                out.push_str("\n    config:");
                for (k, v) in &d.config {
                    out.push_str(&format!("\n      {}:", k));
                    match v {
                        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                            out.push_str(&render_yaml_value(v, 8));
                        }
                        _ => {
                            out.push(' ');
                            out.push_str(&yaml_scalar(v));
                        }
                    }
                }
            }
            out.push_str("\n    dependencies:");
            if d.dependencies.is_empty() {
                out.push_str("\n      []");
            } else {
                for dep in &d.dependencies {
                    out.push_str(&format!("\n      - {}", dep));
                }
            }
        }

        out
    }

    /// Write all three YAML files to `boards_dir/<assembly_id>/`.
    pub fn write_all(&self, boards_dir: &Path) -> Result<Vec<PathBuf>, CodegenError> {
        let dir = boards_dir.join(&self.assembly.id);
        let files = vec![
            write_file(&dir.join("board_info.yaml"), &self.generate_board_info())?,
            write_file(
                &dir.join("board_peripherals.yaml"),
                &self.generate_peripherals(),
            )?,
            write_file(&dir.join("board_devices.yaml"), &self.generate_devices())?,
        ];
        Ok(files)
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rshome_schema::assembly::default_assembly_registry;

    fn gpio_relay_gen() -> (HardwareAssemblyDefinition, BoardYamlGenerator<'static>) {
        // Leak to get 'static — fine in tests
        let reg = default_assembly_registry();
        let asm = reg.get("esp32s3_gpio_relay_assembly").unwrap().clone();
        let asm = Box::leak(Box::new(asm));
        let gen = BoardYamlGenerator::new(asm);
        (asm.clone(), gen)
    }

    fn i2c_sensor_gen() -> (HardwareAssemblyDefinition, BoardYamlGenerator<'static>) {
        let reg = default_assembly_registry();
        let asm = reg.get("esp32s3_i2c_sensor_assembly").unwrap().clone();
        let asm = Box::leak(Box::new(asm));
        let gen = BoardYamlGenerator::new(asm);
        (asm.clone(), gen)
    }

    #[test]
    fn board_info_gpio_relay() {
        let (_, gen) = gpio_relay_gen();
        let yaml = gen.generate_board_info();
        assert!(yaml.contains("board_name: \"ESP32-S3 GPIO Relay Assembly\""));
        assert!(yaml.contains("chip: esp32s3"));
        assert!(yaml.contains("version: \"1.0\""));
        assert!(yaml.contains("description: \"Single-channel GPIO output"));
    }

    #[test]
    fn board_info_i2c_sensor() {
        let (_, gen) = i2c_sensor_gen();
        let yaml = gen.generate_board_info();
        assert!(yaml.contains("chip: esp32s3"));
        assert!(yaml.contains("board_name: \"ESP32-S3 I2C Sensor Assembly\""));
    }

    #[test]
    fn peripherals_gpio_relay() {
        let (_, gen) = gpio_relay_gen();
        let yaml = gen.generate_peripherals();
        assert!(yaml.contains("peripherals:"));
        assert!(yaml.contains("  - name: gpio_relay_output"));
        assert!(yaml.contains("    type: gpio"));
        // `io` matches esp_board_manager's esp_board_periph_role_t enum.
        assert!(yaml.contains("    role: io"));
        assert!(yaml.contains("      pin: 6"));
        assert!(yaml.contains("      mode: GPIO_MODE_OUTPUT"));
        assert!(yaml.contains("      default_level: 0"));
    }

    #[test]
    fn peripherals_i2c_sensor() {
        let (_, gen) = i2c_sensor_gen();
        let yaml = gen.generate_peripherals();
        assert!(yaml.contains("  - name: i2c_master_0"));
        assert!(yaml.contains("    type: i2c"));
        assert!(yaml.contains("    role: master"));
        assert!(yaml.contains("      sda: 8"));
        assert!(yaml.contains("      scl: 9"));
        assert!(yaml.contains("      freq_hz: 400000"));
    }

    #[test]
    fn devices_gpio_relay() {
        let (_, gen) = gpio_relay_gen();
        let yaml = gen.generate_devices();
        assert!(yaml.contains("devices:"));
        assert!(yaml.contains("  - name: gpio_ctrl_0"));
        assert!(yaml.contains("    type: gpio_ctrl"));
        assert!(yaml.contains("      - gpio_relay_output"));
        assert!(yaml.contains("      enabled: true"));
        assert!(yaml.contains("      active_level: 1"));
    }

    #[test]
    fn devices_i2c_sensor() {
        let (_, gen) = i2c_sensor_gen();
        let yaml = gen.generate_devices();
        assert!(yaml.contains("  - name: bme280_0"));
        assert!(yaml.contains("    type: i2c_sensor"));
        assert!(yaml.contains("      - i2c_master_0"));
        assert!(yaml.contains("      address: 0x76"));
        assert!(yaml.contains("      sensor_type: bme280"));
    }

    #[test]
    fn yaml_scalar_types() {
        assert_eq!(yaml_scalar(&serde_json::json!(true)), "true");
        assert_eq!(yaml_scalar(&serde_json::json!(false)), "false");
        assert_eq!(yaml_scalar(&serde_json::json!(42)), "42");
        assert_eq!(yaml_scalar(&serde_json::json!(3.14)), "3.14");
        assert_eq!(yaml_scalar(&serde_json::json!("hello")), "hello");
        assert_eq!(
            yaml_scalar(&serde_json::json!("has space")),
            "\"has space\""
        );
        assert_eq!(yaml_scalar(&serde_json::Value::Null), "null");
    }

    #[test]
    fn render_yaml_value_scalar_inline() {
        assert_eq!(render_yaml_value(&serde_json::json!(42), 4), "42");
        assert_eq!(render_yaml_value(&serde_json::json!("x"), 4), "x");
        assert_eq!(render_yaml_value(&serde_json::Value::Null, 4), "null");
    }

    #[test]
    fn render_yaml_value_empty_containers() {
        assert_eq!(
            render_yaml_value(&serde_json::json!({}), 4),
            " {}",
            "empty map renders as flow-style on the same line"
        );
        assert_eq!(
            render_yaml_value(&serde_json::json!([]), 4),
            " []",
            "empty array renders as flow-style on the same line"
        );
    }

    #[test]
    fn render_yaml_value_nested_map_indents_properly() {
        let v = serde_json::json!({
            "a": 1,
            "b": { "x": 2, "y": "hello" }
        });
        let out = render_yaml_value(&v, 4);
        // Each top-level key at col 4; nested keys at col 6.
        assert!(out.contains("\n    a: 1"), "got: {out:?}");
        assert!(out.contains("\n    b:"), "got: {out:?}");
        assert!(out.contains("\n      x: 2"), "got: {out:?}");
        assert!(out.contains("\n      y: hello"), "got: {out:?}");
    }

    #[test]
    fn render_yaml_value_array_of_maps_matches_list_style() {
        // Mirrors the MCPWM `comparator_configs` shape.
        let v = serde_json::json!([
            { "comparator": 0, "intr_priority": 1 },
            { "comparator": 1 },
        ]);
        let out = render_yaml_value(&v, 4);
        // Each item introduced by `- key: val`; subsequent keys align at +2.
        assert!(out.contains("\n    - comparator: 0"), "got: {out:?}");
        assert!(out.contains("\n      intr_priority: 1"), "got: {out:?}");
        assert!(out.contains("\n    - comparator: 1"), "got: {out:?}");
    }

    #[test]
    fn render_yaml_value_map_with_array_child() {
        // Mirrors the MCPWM shape where `config.comparator_configs` is a list.
        let v = serde_json::json!({
            "timer_config": { "group_id": 0 },
            "comparator_configs": [ { "comparator": 0 } ],
        });
        let out = render_yaml_value(&v, 4);
        assert!(out.contains("\n    timer_config:"), "got: {out:?}");
        assert!(out.contains("\n      group_id: 0"), "got: {out:?}");
        assert!(out.contains("\n    comparator_configs:"), "got: {out:?}");
        assert!(out.contains("\n      - comparator: 0"), "got: {out:?}");
    }

    #[test]
    fn resolve_device_type_passes_builtins_through() {
        // Built-in types stay as-is.
        assert_eq!(resolve_device_type_yaml("gpio_ctrl"), "gpio_ctrl");
        assert_eq!(resolve_device_type_yaml("button"), "button");
        assert_eq!(resolve_device_type_yaml("custom"), "custom");
        // Legacy rshome types also pass through (backward-compat).
        assert_eq!(resolve_device_type_yaml("i2c_sensor"), "i2c_sensor");
        assert_eq!(resolve_device_type_yaml("ds18b20"), "ds18b20");
    }

    #[test]
    fn resolve_device_type_coerces_unknown_to_custom() {
        // V&A device types not in esp_board_manager v0.5.x built-ins
        // collapse to `custom` so esp_board_manager_init() registers the
        // device without running a non-existent driver parser.
        assert_eq!(resolve_device_type_yaml("motor_driver"), "custom");
        assert_eq!(resolve_device_type_yaml("imu_sensor"), "custom");
        assert_eq!(resolve_device_type_yaml("failsafe_ctrl"), "custom");
        assert_eq!(resolve_device_type_yaml("servo_control"), "custom");
        assert_eq!(resolve_device_type_yaml("crsf_link"), "custom");
    }

    #[test]
    fn write_all_creates_directory() {
        let reg = default_assembly_registry();
        let asm = reg.get("esp32s3_gpio_relay_assembly").unwrap();
        let gen = BoardYamlGenerator::new(asm);

        let tmp = std::env::temp_dir().join("rshome_board_yaml_test");
        let _ = std::fs::remove_dir_all(&tmp);

        let files = gen.write_all(&tmp).unwrap();
        assert_eq!(files.len(), 3);

        let board_dir = tmp.join("esp32s3_gpio_relay_assembly");
        assert!(board_dir.join("board_info.yaml").exists());
        assert!(board_dir.join("board_peripherals.yaml").exists());
        assert!(board_dir.join("board_devices.yaml").exists());

        // Verify content
        let info = std::fs::read_to_string(board_dir.join("board_info.yaml")).unwrap();
        assert!(info.contains("chip: esp32s3"));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
