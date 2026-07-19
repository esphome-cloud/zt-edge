//! Stage 10 — Pin conflict detection.
//!
//! Walks every component's config, extracts GPIO pin assignments, and feeds
//! them into the `ResourceTracker` from `rshome-schema`.  Conflicts (same GPIO
//! allocated by two components) and chip-capability violations (input-only pin
//! used as output, flash-reserved pins, out-of-range GPIO) are surfaced as
//! `ValidationError`s.

use rshome_schema::{ChipTarget, PinAllocation, PinMode, PullMode, ResourceTracker};

use crate::error::{ValidationError, ValidationStage};
use crate::raw::RawConfig;

// ── Pin extraction ────────────────────────────────────────────────────────────

/// A GPIO usage declaration extracted from a component's config.
#[derive(Debug)]
struct PinUsage {
    gpio_num: u8,
    mode: PinMode,
    pull_mode: Option<PullMode>,
    inverted: bool,
    component_id: String,
    path: String,
}

/// Extract all GPIO pin usages from a component's JSON config.
///
/// Recognises common field names: `pin`, `sda`, `scl`, `mosi`, `miso`, `clk`,
/// `cs`, `tx_pin`, `rx_pin`, `trigger_pin`, `echo_pin`, `output`, `input`.
fn extract_pins(config: &serde_json::Value, component_id: &str, base_path: &str) -> Vec<PinUsage> {
    let mut pins = Vec::new();

    let obj = match config.as_object() {
        Some(o) => o,
        None => return pins,
    };

    // Map of config field → (PinMode, is_input).
    let field_modes: &[(&str, PinMode)] = &[
        ("pin", PinMode::InputOutput),
        ("sda", PinMode::I2cSda),
        ("scl", PinMode::I2cScl),
        ("mosi", PinMode::SpiMosi),
        ("miso", PinMode::SpiMiso),
        ("clk", PinMode::SpiClk),
        ("cs", PinMode::SpiCs),
        ("tx_pin", PinMode::Uart),
        ("rx_pin", PinMode::Uart),
        ("trigger_pin", PinMode::Output),
        ("echo_pin", PinMode::Input),
        ("output", PinMode::Output),
        ("input_pin", PinMode::Input),
    ];

    for (field, mode) in field_modes {
        if let Some(val) = obj.get(*field) {
            // GPIO can be specified as:
            // - a plain integer: `pin: 4`
            // - an object with a `number` key: `pin: {number: 4, mode: INPUT_PULLUP}`
            let (gpio_num, pull_mode, inverted) = match val {
                serde_json::Value::Number(n) => {
                    if let Some(num) = n.as_u64().and_then(|v| u8::try_from(v).ok()) {
                        (Some(num), None, false)
                    } else {
                        (None, None, false)
                    }
                }
                serde_json::Value::Object(pin_obj) => {
                    let num = pin_obj
                        .get("number")
                        .and_then(|v| v.as_u64())
                        .and_then(|v| u8::try_from(v).ok());
                    let pull = pin_obj.get("mode").and_then(|v| v.as_str()).and_then(|s| {
                        if s.contains("PULLUP") || s.contains("INPUT_PULLUP") {
                            Some(PullMode::Up)
                        } else if s.contains("PULLDOWN") {
                            Some(PullMode::Down)
                        } else {
                            None
                        }
                    });
                    let inv = pin_obj
                        .get("inverted")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    (num, pull, inv)
                }
                _ => (None, None, false),
            };

            if let Some(gpio) = gpio_num {
                pins.push(PinUsage {
                    gpio_num: gpio,
                    mode: *mode,
                    pull_mode,
                    inverted,
                    component_id: component_id.to_owned(),
                    path: format!("{base_path}.{field}"),
                });
            }
        }
    }

    pins
}

// ── Stage entry point ─────────────────────────────────────────────────────────

/// Stage 10: detect GPIO pin conflicts and capability violations.
///
/// Returns `(tracker, errors)`.  The tracker holds all successful allocations.
pub fn stage_10_check_pin_conflicts(
    config: &RawConfig,
    target: ChipTarget,
) -> (ResourceTracker, Vec<ValidationError>) {
    let mut tracker = ResourceTracker::new();
    let mut errors = Vec::new();

    // Group by component type for index-based paths.
    let mut type_counters: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();

    for comp in &config.components {
        let idx = {
            let ctr = type_counters
                .entry(comp.component_type.clone())
                .or_insert(0);
            let i = *ctr;
            *ctr += 1;
            i
        };

        let comp_id = if let Some(p) = &comp.platform {
            p.clone()
        } else {
            comp.component_type.clone()
        };

        let base_path = format!("{}[{idx}]", comp.component_type);
        let pins = extract_pins(&comp.config, &comp_id, &base_path);

        for usage in pins {
            let alloc = PinAllocation {
                gpio_num: usage.gpio_num,
                mode: usage.mode,
                component: usage.component_id.clone(),
                pull_mode: usage.pull_mode,
                inverted: usage.inverted,
            };

            match tracker.allocate_pin(alloc) {
                Ok(()) => {}
                Err(conflict) => {
                    errors.push(ValidationError::error(
                        ValidationStage::PinConflicts,
                        &usage.path,
                        format!(
                            "GPIO {} already allocated by '{}' (mode {:?}); \
                             '{}' requested mode {:?}",
                            conflict.gpio_num,
                            conflict.existing_owner,
                            conflict.existing_mode,
                            conflict.new_owner,
                            conflict.new_mode,
                        ),
                    ));
                }
            }
        }
    }

    // Validate chip capabilities.
    if let Err(pin_errors) = tracker.validate_pin_capabilities(target) {
        for pin_err in pin_errors {
            use rshome_schema::pin::PinError;
            let (path, msg, is_warn) = match &pin_err {
                PinError::InputOnlyPinUsedAsOutput {
                    gpio_num,
                    component,
                    ..
                } => (
                    format!("gpio_{gpio_num}"),
                    format!(
                        "GPIO {gpio_num} is input-only on {target:?} but \
                         '{component}' requested output mode"
                    ),
                    false,
                ),
                PinError::GpioOutOfRange {
                    gpio_num, max_gpio, ..
                } => (
                    format!("gpio_{gpio_num}"),
                    format!(
                        "GPIO {gpio_num} does not exist on {target:?} (max GPIO is {max_gpio})"
                    ),
                    false,
                ),
                PinError::StrappingPinWarning {
                    gpio_num,
                    component,
                    ..
                } => (
                    format!("gpio_{gpio_num}"),
                    format!(
                        "GPIO {gpio_num} is a strapping pin on {target:?}; \
                         '{component}' using it may affect boot reliability"
                    ),
                    true, // warning, not fatal
                ),
                PinError::FlashReservedPin {
                    gpio_num,
                    component,
                    ..
                } => (
                    format!("gpio_{gpio_num}"),
                    format!(
                        "GPIO {gpio_num} is reserved for flash on {target:?} and \
                         cannot be used by '{component}'"
                    ),
                    false,
                ),
            };

            let err = if is_warn {
                ValidationError::warning(ValidationStage::PinConflicts, path, msg)
            } else {
                ValidationError::error(ValidationStage::PinConflicts, path, msg)
            };
            errors.push(err);
        }
    }

    (tracker, errors)
}

#[cfg(test)]
mod tests {
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

    fn push(config: &mut RawConfig, typ: &str, val: serde_json::Value) {
        config.components.push(ComponentConfig {
            component_type: typ.into(),
            platform: None,
            config: val,
        });
    }

    #[test]
    fn no_pins_no_errors() {
        let config = base_config();
        let (_, errors) = stage_10_check_pin_conflicts(&config, ChipTarget::Esp32);
        assert!(errors.is_empty());
    }

    #[test]
    fn non_conflicting_pins_pass() {
        let mut config = base_config();
        push(&mut config, "i2c", json!({"sda": 21, "scl": 22}));
        let (_, errors) = stage_10_check_pin_conflicts(&config, ChipTarget::Esp32);
        assert!(errors.is_empty(), "errors: {:?}", errors);
    }

    #[test]
    fn same_gpio_two_components_produces_error() {
        let mut config = base_config();
        push(&mut config, "sensor", json!({"pin": 4}));
        push(&mut config, "switch", json!({"pin": 4}));
        let (_, errors) = stage_10_check_pin_conflicts(&config, ChipTarget::Esp32);
        assert!(errors
            .iter()
            .any(|e| e.is_fatal() && e.message.contains("GPIO 4")));
    }

    #[test]
    fn flash_reserved_pin_rejected() {
        let mut config = base_config();
        push(&mut config, "sensor", json!({"pin": 6})); // GPIO 6 reserved on ESP32
        let (_, errors) = stage_10_check_pin_conflicts(&config, ChipTarget::Esp32);
        assert!(errors
            .iter()
            .any(|e| e.is_fatal() && e.message.contains("reserved for flash")));
    }

    #[test]
    fn strapping_pin_produces_warning() {
        let mut config = base_config();
        push(&mut config, "sensor", json!({"pin": 2})); // GPIO 2 is strapping on ESP32
        let (_, errors) = stage_10_check_pin_conflicts(&config, ChipTarget::Esp32);
        assert!(errors.iter().any(|e| {
            e.severity == crate::error::Severity::Warning && e.message.contains("strapping pin")
        }));
    }

    #[test]
    fn out_of_range_gpio_rejected() {
        let mut config = base_config();
        push(&mut config, "sensor", json!({"pin": 50})); // ESP32C6 max is 30
        let (_, errors) = stage_10_check_pin_conflicts(&config, ChipTarget::Esp32C6);
        assert!(errors
            .iter()
            .any(|e| e.is_fatal() && e.message.contains("does not exist")));
    }

    #[test]
    fn pin_as_object_with_number_key() {
        let mut config = base_config();
        push(&mut config, "sensor", json!({"pin": {"number": 4}}));
        let (_, errors) = stage_10_check_pin_conflicts(&config, ChipTarget::Esp32);
        // GPIO 4 on ESP32 is safe — no errors expected.
        assert!(errors.is_empty(), "errors: {:?}", errors);
    }

    #[test]
    fn esp32s3_flash_reserved_gpio_27_rejected() {
        let mut config = base_config();
        push(&mut config, "sensor", json!({"pin": 27}));
        let (_, errors) = stage_10_check_pin_conflicts(&config, ChipTarget::Esp32S3);
        assert!(errors
            .iter()
            .any(|e| e.is_fatal() && e.message.contains("reserved for flash")));
    }

    #[test]
    fn tracker_returned_with_successful_allocations() {
        let mut config = base_config();
        push(&mut config, "i2c", json!({"sda": 21, "scl": 22}));
        let (tracker, errors) = stage_10_check_pin_conflicts(&config, ChipTarget::Esp32);
        assert!(errors.is_empty(), "errors: {:?}", errors);
        assert_eq!(tracker.pin_allocations().len(), 2);
    }
}
