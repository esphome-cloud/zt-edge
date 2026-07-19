//! Stage 9 — Final cross-component validation.
//!
//! Checks invariants that span multiple components:
//! - `api:` configured → `wifi:` must be present.
//! - `mqtt:` + `api:` together → warning about potential conflict.
//! - `ota:` → at least one connectivity component (wifi / ethernet).
//! - `deep_sleep:` → API keepalive may not work; warn.
//! - Climate visual settings consistency (min < max, step > 0).

use rshome_schema::ComponentRegistry;

use crate::error::{ValidationError, ValidationStage};
use crate::raw::RawConfig;

// ── Stage entry point ─────────────────────────────────────────────────────────

/// Stage 9: cross-component invariant checks.
pub fn stage_9_final_validation(
    config: &RawConfig,
    _registry: &ComponentRegistry,
) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    let types: std::collections::HashSet<&str> = config
        .components
        .iter()
        .map(|c| c.component_type.as_str())
        .collect();

    let has_wifi = types.contains("wifi") || types.contains("ethernet");
    let has_api = types.contains("api");
    let has_mqtt = types.contains("mqtt");
    let has_ota = types.contains("ota");
    let has_deep_sleep = types.contains("deep_sleep");
    let has_logger = types.contains("logger");
    let has_climate = types.contains("climate");

    // Rule 1: api requires wifi or ethernet.
    if has_api && !has_wifi {
        errors.push(
            ValidationError::error(
                ValidationStage::FinalValidation,
                "api",
                "the 'api:' component requires 'wifi:' or 'ethernet:' to be configured",
            )
            .with_suggestion("Add a 'wifi:' block with your SSID and password"),
        );
    }

    // Rule 2: mqtt + api together → warning.
    if has_mqtt && has_api {
        errors.push(ValidationError::warning(
            ValidationStage::FinalValidation,
            "mqtt",
            "both 'mqtt:' and 'api:' are configured; \
             Home Assistant native API and MQTT can conflict — consider using only one",
        ));
    }

    // Rule 3: ota requires connectivity.
    if has_ota && !has_wifi {
        errors.push(
            ValidationError::error(
                ValidationStage::FinalValidation,
                "ota",
                "the 'ota:' component requires 'wifi:' or 'ethernet:' to be configured",
            )
            .with_suggestion("OTA updates are delivered over the network"),
        );
    }

    // Rule 4: deep_sleep + api → warn about keepalive.
    if has_deep_sleep && has_api {
        errors.push(ValidationError::warning(
            ValidationStage::FinalValidation,
            "deep_sleep",
            "deep_sleep and api are both configured; \
             the device will disconnect from Home Assistant during sleep cycles",
        ));
    }

    // Rule 5: deep_sleep + logger with non-zero baud → warn.
    if has_deep_sleep && has_logger {
        // Check if baud_rate is non-zero.
        for comp in &config.components {
            if comp.component_type == "logger" {
                if let Some(baud) = comp.config.get("baud_rate").and_then(|v| v.as_u64()) {
                    if baud > 0 {
                        errors.push(ValidationError::info(
                            ValidationStage::FinalValidation,
                            "logger",
                            "logger is active during deep_sleep wakeup; \
                             consider setting baud_rate: 0 to disable serial logging",
                        ));
                    }
                }
            }
        }
    }

    // Rule 6: climate visual settings consistency.
    if has_climate {
        validate_climate_visuals(config, &mut errors);
    }

    // Rule 7: warn if no connectivity at all but has api/ota/mqtt.
    if !has_wifi && (has_api || has_ota || has_mqtt) {
        // Already covered by rules 1 and 3 — skip duplicate warning.
    }

    // Rule 8: mqtt requires wifi.
    if has_mqtt && !has_wifi {
        errors.push(
            ValidationError::error(
                ValidationStage::FinalValidation,
                "mqtt",
                "the 'mqtt:' component requires 'wifi:' or 'ethernet:' to be configured",
            )
            .with_suggestion("Add a 'wifi:' block with your SSID and password"),
        );
    }

    // Rule 9: solution-aware validation.
    if let Some(ref solution_id) = config.esphome.solution {
        validate_solution(solution_id, &types, &mut errors);
        // Rule 9b — orchestration budget check (Phase 2 Task 2.1 #3 / ADR-023 D2).
        // The schema-level invariants in `RetryPolicy::new`
        // (initial ≤ max ≤ budget/attempts) are insufficient for safety:
        // a step's `total_budget_ms` could still exceed the host
        // solution's watchdog half-window and mask a real failure.
        // This rule reads the solution's `failsafe.watchdog_ms` and
        // rejects any step where `retry_policy.total_budget_ms >
        // watchdog_ms / 2`. ADR-023 D2 + master design §10.2.
        validate_orchestration_budget(solution_id, &mut errors);
    }

    // Rule 10: sigrok component validation.
    if types.contains("sigrok") {
        validate_sigrok(config, &mut errors);
    }

    errors
}

fn validate_solution(
    solution_id: &str,
    component_types: &std::collections::HashSet<&str>,
    errors: &mut Vec<ValidationError>,
) {
    let sol_reg = rshome_schema::solution::default_solution_registry();
    let Some(sol) = sol_reg.get(solution_id) else {
        errors.push(ValidationError::error(
            ValidationStage::FinalValidation,
            "esphome.solution",
            format!("unknown solution '{solution_id}'; not found in solution registry"),
        ));
        return;
    };

    // Check all required components are present.
    for required in &sol.component_bundle.required {
        if !component_types.contains(required.as_str()) {
            errors.push(
                ValidationError::error(
                    ValidationStage::FinalValidation,
                    "esphome.solution",
                    format!(
                        "solution '{solution_id}' requires component '{required}' but it is not configured"
                    ),
                )
                .with_suggestion(format!("Add a '{required}:' block to your configuration")),
            );
        }
    }

    // Warn about components not in required or optional bundles.
    let all_known: std::collections::HashSet<&str> = sol
        .component_bundle
        .required
        .iter()
        .chain(sol.component_bundle.optional.iter())
        .map(|s| s.as_str())
        .collect();

    for comp_type in component_types {
        if !all_known.contains(comp_type) {
            errors.push(ValidationError::warning(
                ValidationStage::FinalValidation,
                "esphome.solution",
                format!(
                    "component '{comp_type}' is not part of solution '{solution_id}' bundle; \
                     it may not be compatible"
                ),
            ));
        }
    }
}

/// Phase 2 Task 2.1 acceptance #3 / ADR-023 D2 — config-level orchestration
/// budget check.
///
/// Iterates the host solution's `fixed_orchestration` and rejects any step
/// whose `retry_policy.total_budget_ms > watchdog_ms / 2`. Solutions without
/// a `failsafe.watchdog_ms` (GCS-side, RC-TX, passthrough; see ADR-016 scope)
/// skip the check — there's no watchdog to bound against.
///
/// Complementary to the schema-level invariants in `RetryPolicy::new`
/// (which are bounded but per-step, not aware of the host watchdog).
/// Counterpart of the firmware-side budget-check in
/// `orchestrator.c.tera`'s pre-sleep guard.
fn validate_orchestration_budget(solution_id: &str, errors: &mut Vec<ValidationError>) {
    let sol_reg = rshome_schema::solution::default_solution_registry();
    let Some(sol) = sol_reg.get(solution_id) else {
        return;
    };
    let Some(failsafe) = sol.failsafe.as_ref() else {
        return;
    };
    let Some(watchdog_ms) = failsafe.watchdog_ms else {
        return;
    };
    check_orchestration_budget(solution_id, watchdog_ms, &sol.fixed_orchestration, errors);
}

/// Inner workhorse. Factored out so tests can synthesize
/// `OrchestrationStep` fixtures without registering them in the
/// (immutable) default registry.
fn check_orchestration_budget(
    solution_id: &str,
    watchdog_ms: u32,
    steps: &[rshome_schema::solution::OrchestrationStep],
    errors: &mut Vec<ValidationError>,
) {
    let half_watchdog = watchdog_ms / 2;
    for (i, step) in steps.iter().enumerate() {
        let Some(policy) = step.retry_policy.as_ref() else {
            continue;
        };
        if policy.total_budget_ms > half_watchdog {
            errors.push(
                ValidationError::error(
                    ValidationStage::FinalValidation,
                    format!(
                        "esphome.solution.fixed_orchestration[{i}].retry_policy.total_budget_ms"
                    ),
                    format!(
                        "OrchestrationBudgetExceeded: solution '{solution_id}' step '{step_id}' \
                         total_budget_ms ({total}) > watchdog_ms / 2 ({half}); the retry budget \
                         would consume more than half the watchdog window and could mask a real \
                         persistent failure",
                        step_id = step.id,
                        total = policy.total_budget_ms,
                        half = half_watchdog,
                    ),
                )
                .with_suggestion(format!(
                    "Reduce retry_policy.total_budget_ms to ≤ {half_watchdog} ms (half of \
                     watchdog_ms = {watchdog_ms} ms), or shorten max_attempts / backoff so \
                     the schedule fits"
                )),
            );
        }
    }
}

fn validate_climate_visuals(config: &RawConfig, errors: &mut Vec<ValidationError>) {
    for (i, comp) in config.components.iter().enumerate() {
        if comp.component_type != "climate" {
            continue;
        }
        let path = format!("climate[{i}]");

        if let Some(visual) = comp.config.get("visual").and_then(|v| v.as_object()) {
            let min_temp = visual.get("min_temperature").and_then(|v| v.as_f64());
            let max_temp = visual.get("max_temperature").and_then(|v| v.as_f64());
            let step = visual.get("temperature_step").and_then(|v| v.as_f64());

            if let (Some(min), Some(max)) = (min_temp, max_temp) {
                if min >= max {
                    errors.push(ValidationError::error(
                        ValidationStage::FinalValidation,
                        format!("{path}.visual"),
                        format!(
                            "climate visual: min_temperature ({min}) must be less than \
                             max_temperature ({max})"
                        ),
                    ));
                }
            }

            if let Some(s) = step {
                if s <= 0.0 {
                    errors.push(ValidationError::error(
                        ValidationStage::FinalValidation,
                        format!("{path}.visual.temperature_step"),
                        format!("climate visual: temperature_step must be positive, got {s}"),
                    ));
                }
            }
        }
    }
}

// ── Sigrok component validation ──────────────────────────────────────────────

/// ESP32-S3 strapping pins that should not be used as logic analyzer channels.
const STRAPPING_PINS: &[u64] = &[0, 3, 45, 46];
/// USB Serial/JTAG pins — reserved when using USB Serial/JTAG transport.
const USB_JTAG_PINS: &[u64] = &[19, 20];
/// SPI flash pins on ESP32-S3 (MSPI bus).
const FLASH_PINS: &[u64] = &[26, 27, 28, 29, 30, 31, 32];
/// Maximum sample rate (hardware limit for dedic_gpio approach).
const MAX_SAMPLE_RATE_HZ: u64 = 20_000_000;
/// Maximum buffer depth (SRAM budget, 200 KB).
const MAX_BUFFER_DEPTH: u64 = 200_000;

fn validate_sigrok(config: &RawConfig, errors: &mut Vec<ValidationError>) {
    use crate::sigrok::{SigrokConfig, SigrokTransport};

    let comp = match config
        .components
        .iter()
        .find(|c| c.component_type == "sigrok")
    {
        Some(c) => c,
        None => return,
    };

    // Rule S1: sigrok requires ESP32-S3.
    if config.esphome.platform != "esp32s3" {
        errors.push(
            ValidationError::error(
                ValidationStage::FinalValidation,
                "sigrok",
                format!(
                    "sigrok component requires ESP32-S3 but target is '{}'",
                    config.esphome.platform
                ),
            )
            .with_suggestion("Change platform to 'esp32s3' or remove the sigrok component"),
        );
        return;
    }

    // Deserialize into typed config (with defaults for missing fields).
    let sigrok = match SigrokConfig::from_value(&comp.config) {
        Ok(s) => s,
        Err(e) => {
            errors.push(ValidationError::error(
                ValidationStage::FinalValidation,
                "sigrok",
                format!("invalid sigrok config: {e}"),
            ));
            return;
        }
    };

    // Rule S2: channel count must be 8 or 16.
    if sigrok.channels.len() != 8 && sigrok.channels.len() != 16 {
        errors.push(ValidationError::error(
            ValidationStage::FinalValidation,
            "sigrok.channels",
            format!(
                "sigrok requires exactly 8 or 16 channels, got {}",
                sigrok.channels.len()
            ),
        ));
    }

    // Rule S3: no duplicate GPIOs.
    {
        let mut seen = std::collections::HashSet::new();
        for &pin in &sigrok.channels {
            if !seen.insert(pin) {
                errors.push(ValidationError::error(
                    ValidationStage::FinalValidation,
                    "sigrok.channels",
                    format!("duplicate GPIO {pin} in sigrok channels"),
                ));
            }
        }
    }

    // Rule S4: pin validity — warn on strapping pins, reject reserved pins.
    for &pin in &sigrok.channels {
        let p = pin as u64;
        if STRAPPING_PINS.contains(&p) {
            errors.push(ValidationError::warning(
                ValidationStage::FinalValidation,
                "sigrok.channels",
                format!("GPIO {pin} is a strapping pin — may cause boot issues"),
            ));
        }
        if USB_JTAG_PINS.contains(&p) {
            errors.push(ValidationError::error(
                ValidationStage::FinalValidation,
                "sigrok.channels",
                format!(
                    "GPIO {pin} is used by USB Serial/JTAG (SUMP transport) and \
                     cannot be a sigrok channel"
                ),
            ));
        }
        if FLASH_PINS.contains(&p) {
            errors.push(ValidationError::error(
                ValidationStage::FinalValidation,
                "sigrok.channels",
                format!("GPIO {pin} is a SPI flash pin and cannot be used"),
            ));
        }
    }

    // Rule S5a: USB-OTG PHY conflict warning.
    if sigrok.transport == SigrokTransport::UsbOtgCdc {
        errors.push(ValidationError::warning(
            ValidationStage::FinalValidation,
            "sigrok.transport",
            "USB-OTG mode disables the ESP32-S3's built-in USB Serial/JTAG interface. \
             You will need a USB-UART bridge (e.g., CP2102) on GPIO 43/44 for flashing \
             and serial monitor.",
        ));
    }

    // Rule S5: sample rate ceiling.
    if sigrok.sample_rate_hz as u64 > MAX_SAMPLE_RATE_HZ {
        errors.push(ValidationError::error(
            ValidationStage::FinalValidation,
            "sigrok.sample_rate_hz",
            format!(
                "sample rate {} Hz exceeds the 20 MHz hardware maximum for ESP32-S3",
                sigrok.sample_rate_hz
            ),
        ));
    }

    // Rule S6: buffer depth ceiling (SRAM budget).
    if sigrok.buffer_depth as u64 > MAX_BUFFER_DEPTH {
        errors.push(ValidationError::error(
            ValidationStage::FinalValidation,
            "sigrok.buffer_depth",
            format!(
                "buffer depth {} exceeds the {MAX_BUFFER_DEPTH} sample SRAM limit",
                sigrok.buffer_depth
            ),
        ));
    }
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

    fn push(config: &mut RawConfig, typ: &str, val: serde_json::Value) {
        config.components.push(ComponentConfig {
            component_type: typ.into(),
            platform: None,
            config: val,
        });
    }

    fn reg() -> ComponentRegistry {
        ComponentRegistry::new()
    }

    #[test]
    fn api_without_wifi_produces_error() {
        let mut config = base_config();
        push(&mut config, "api", json!({}));
        let errors = stage_9_final_validation(&config, &reg());
        assert!(errors.iter().any(|e| e.path == "api" && e.is_fatal()));
    }

    #[test]
    fn api_with_wifi_passes() {
        let mut config = base_config();
        push(&mut config, "wifi", json!({"ssid": "Net"}));
        push(&mut config, "api", json!({}));
        let errors = stage_9_final_validation(&config, &reg());
        assert!(errors
            .iter()
            .all(|e| e.severity != crate::error::Severity::Error));
    }

    #[test]
    fn mqtt_and_api_together_produces_warning() {
        let mut config = base_config();
        push(&mut config, "wifi", json!({}));
        push(&mut config, "api", json!({}));
        push(&mut config, "mqtt", json!({"broker": "192.168.1.1"}));
        let errors = stage_9_final_validation(&config, &reg());
        assert!(errors
            .iter()
            .any(|e| e.severity == crate::error::Severity::Warning && e.path == "mqtt"));
    }

    #[test]
    fn ota_without_wifi_produces_error() {
        let mut config = base_config();
        push(&mut config, "ota", json!({}));
        let errors = stage_9_final_validation(&config, &reg());
        assert!(errors.iter().any(|e| e.path == "ota" && e.is_fatal()));
    }

    #[test]
    fn ota_with_wifi_passes() {
        let mut config = base_config();
        push(&mut config, "wifi", json!({}));
        push(&mut config, "ota", json!({}));
        let errors = stage_9_final_validation(&config, &reg());
        assert!(errors.iter().all(|e| !e.is_fatal()));
    }

    #[test]
    fn deep_sleep_with_api_produces_warning() {
        let mut config = base_config();
        push(&mut config, "wifi", json!({}));
        push(&mut config, "api", json!({}));
        push(&mut config, "deep_sleep", json!({"run_duration": "10s"}));
        let errors = stage_9_final_validation(&config, &reg());
        assert!(errors
            .iter()
            .any(|e| e.severity == crate::error::Severity::Warning && e.path == "deep_sleep"));
    }

    #[test]
    fn climate_visual_min_ge_max_produces_error() {
        let mut config = base_config();
        push(
            &mut config,
            "climate",
            json!({"name": "thermo", "visual": {"min_temperature": 30, "max_temperature": 20}}),
        );
        let errors = stage_9_final_validation(&config, &reg());
        assert!(errors
            .iter()
            .any(|e| e.message.contains("min_temperature") && e.is_fatal()));
    }

    #[test]
    fn climate_visual_valid_range_passes() {
        let mut config = base_config();
        push(
            &mut config,
            "climate",
            json!({"name": "thermo", "visual": {
                "min_temperature": 15,
                "max_temperature": 30,
                "temperature_step": 0.5
            }}),
        );
        let errors = stage_9_final_validation(&config, &reg());
        assert!(errors.is_empty(), "errors: {:?}", errors);
    }

    #[test]
    fn climate_zero_step_produces_error() {
        let mut config = base_config();
        push(
            &mut config,
            "climate",
            json!({"name": "thermo", "visual": {"temperature_step": 0.0}}),
        );
        let errors = stage_9_final_validation(&config, &reg());
        assert!(errors
            .iter()
            .any(|e| e.message.contains("temperature_step") && e.is_fatal()));
    }

    #[test]
    fn mqtt_without_wifi_produces_error() {
        let mut config = base_config();
        push(&mut config, "mqtt", json!({"broker": "192.168.1.1"}));
        let errors = stage_9_final_validation(&config, &reg());
        assert!(errors.iter().any(|e| e.path == "mqtt" && e.is_fatal()));
    }

    #[test]
    fn empty_config_no_errors() {
        let config = base_config();
        let errors = stage_9_final_validation(&config, &reg());
        assert!(errors.is_empty());
    }

    // ── Solution-aware validation ───────────────────────────────────────────

    #[test]
    fn no_solution_field_produces_no_errors() {
        let mut config = base_config();
        assert!(config.esphome.solution.is_none());
        push(&mut config, "wifi", json!({"ssid": "test"}));
        let errors = stage_9_final_validation(&config, &reg());
        assert!(errors.is_empty());
    }

    #[test]
    fn unknown_solution_produces_error() {
        let mut config = base_config();
        config.esphome.solution = Some("nonexistent_solution".into());
        push(&mut config, "wifi", json!({"ssid": "test"}));
        let errors = stage_9_final_validation(&config, &reg());
        assert!(errors
            .iter()
            .any(|e| e.message.contains("unknown solution") && e.is_fatal()));
    }

    #[test]
    fn valid_solution_with_all_required_components_passes() {
        let mut config = base_config();
        config.esphome.solution = Some("camera_stream".into());
        push(&mut config, "wifi", json!({"ssid": "test"}));
        let errors = stage_9_final_validation(&config, &reg());
        // No fatal errors from solution validation
        assert!(!errors
            .iter()
            .any(|e| e.path == "esphome.solution" && e.is_fatal()));
    }

    #[test]
    fn solution_missing_required_component_produces_error() {
        let mut config = base_config();
        config.esphome.solution = Some("camera_stream".into());
        let errors = stage_9_final_validation(&config, &reg());
        let solution_errors: Vec<_> = errors
            .iter()
            .filter(|e| e.path == "esphome.solution" && e.is_fatal())
            .collect();
        assert!(
            solution_errors.len() == 1,
            "expected one error for missing wifi"
        );
        assert!(solution_errors.iter().any(|e| e.message.contains("wifi")));
    }

    // ── sigrok tests ────────────────────────────────────────────────────

    #[test]
    fn sigrok_requires_esp32s3() {
        let mut config = base_config();
        config.esphome.platform = "esp32".into();
        push(
            &mut config,
            "sigrok",
            json!({"channels": [4,5,6,7,15,16,17,18]}),
        );
        let errors = stage_9_final_validation(&config, &reg());
        assert!(errors
            .iter()
            .any(|e| e.is_fatal() && e.message.contains("ESP32-S3")));
    }

    #[test]
    fn sigrok_8_channels_accepted() {
        let mut config = base_config();
        config.esphome.platform = "esp32s3".into();
        push(
            &mut config,
            "sigrok",
            json!({"channels": [4,5,6,7,15,16,17,18]}),
        );
        let errors = stage_9_final_validation(&config, &reg());
        assert!(!errors
            .iter()
            .any(|e| e.is_fatal() && e.message.contains("sigrok")));
    }

    #[test]
    fn sigrok_invalid_channel_count_rejected() {
        let mut config = base_config();
        config.esphome.platform = "esp32s3".into();
        push(&mut config, "sigrok", json!({"channels": [4,5,6]}));
        let errors = stage_9_final_validation(&config, &reg());
        assert!(errors
            .iter()
            .any(|e| e.is_fatal() && e.message.contains("8 or 16")));
    }

    #[test]
    fn sigrok_duplicate_pins_rejected() {
        let mut config = base_config();
        config.esphome.platform = "esp32s3".into();
        push(
            &mut config,
            "sigrok",
            json!({"channels": [4,4,6,7,15,16,17,18]}),
        );
        let errors = stage_9_final_validation(&config, &reg());
        assert!(errors
            .iter()
            .any(|e| e.is_fatal() && e.message.contains("duplicate")));
    }

    #[test]
    fn sigrok_strapping_pin_warning() {
        let mut config = base_config();
        config.esphome.platform = "esp32s3".into();
        push(
            &mut config,
            "sigrok",
            json!({"channels": [0,5,6,7,15,16,17,18]}),
        );
        let errors = stage_9_final_validation(&config, &reg());
        assert!(errors
            .iter()
            .any(|e| !e.is_fatal() && e.message.contains("strapping")));
    }

    #[test]
    fn sigrok_rate_ceiling() {
        let mut config = base_config();
        config.esphome.platform = "esp32s3".into();
        push(
            &mut config,
            "sigrok",
            json!({
                "channels": [4,5,6,7,15,16,17,18],
                "sample_rate_hz": 50_000_000
            }),
        );
        let errors = stage_9_final_validation(&config, &reg());
        assert!(errors
            .iter()
            .any(|e| e.is_fatal() && e.message.contains("20 MHz")));
    }

    #[test]
    fn sigrok_depth_ceiling() {
        let mut config = base_config();
        config.esphome.platform = "esp32s3".into();
        push(
            &mut config,
            "sigrok",
            json!({
                "channels": [4,5,6,7,15,16,17,18],
                "buffer_depth": 300_000
            }),
        );
        let errors = stage_9_final_validation(&config, &reg());
        assert!(errors
            .iter()
            .any(|e| e.is_fatal() && e.message.contains("200000")));
    }

    // ── Orchestration budget tests (Phase 2 Task 2.1 #3 / ADR-023) ─────────

    use rshome_schema::orchestration::RetryPolicy;
    use rshome_schema::solution::OrchestrationStep;

    fn step_with_policy(id: &str, policy: Option<RetryPolicy>) -> OrchestrationStep {
        OrchestrationStep {
            id: id.into(),
            label: id.into(),
            label_zh: None,
            description: None,
            description_zh: None,
            depends_on: vec![],
            retry_policy: policy,
            parallel_group: None,
        }
    }

    #[test]
    fn orchestration_budget_passes_when_under_half_watchdog() {
        // watchdog 100 → half = 50. Policy budget = 40 → OK.
        let mut errors: Vec<ValidationError> = Vec::new();
        let policy = RetryPolicy::new(2, 5, 20, 40).unwrap();
        check_orchestration_budget(
            "sol_x",
            100,
            &[step_with_policy("imu_init", Some(policy))],
            &mut errors,
        );
        assert!(
            errors.iter().all(|e| !e.is_fatal()),
            "expected no fatal errors, got: {errors:?}"
        );
    }

    #[test]
    fn orchestration_budget_passes_exactly_at_half_watchdog() {
        // watchdog 100 → half = 50. Policy budget = 50 → OK (≤, not <).
        let mut errors: Vec<ValidationError> = Vec::new();
        let policy = RetryPolicy::new(2, 5, 25, 50).unwrap();
        check_orchestration_budget(
            "sol_x",
            100,
            &[step_with_policy("imu_init", Some(policy))],
            &mut errors,
        );
        assert!(errors.iter().all(|e| !e.is_fatal()));
    }

    #[test]
    fn orchestration_budget_rejects_step_over_half_watchdog() {
        // watchdog 100 → half = 50. Policy budget = 60 → ERROR.
        let mut errors: Vec<ValidationError> = Vec::new();
        let policy = RetryPolicy::new(2, 5, 20, 60).unwrap();
        check_orchestration_budget(
            "sol_x",
            100,
            &[step_with_policy("imu_init", Some(policy))],
            &mut errors,
        );
        let fatals: Vec<&ValidationError> = errors.iter().filter(|e| e.is_fatal()).collect();
        assert_eq!(fatals.len(), 1, "expected exactly 1 fatal error");
        assert!(fatals[0].message.contains("OrchestrationBudgetExceeded"));
        assert!(fatals[0].message.contains("imu_init"));
        assert!(fatals[0].message.contains("60"));
    }

    #[test]
    fn orchestration_budget_skips_steps_without_retry_policy() {
        // Steps with `retry_policy: None` (the 37 existing V&A solutions
        // today) must not produce any error — backward-compat invariant.
        let mut errors: Vec<ValidationError> = Vec::new();
        let steps = vec![
            step_with_policy("step_a", None),
            step_with_policy("step_b", None),
        ];
        check_orchestration_budget("sol_x", 100, &steps, &mut errors);
        assert!(errors.is_empty(), "expected zero errors, got: {errors:?}");
    }

    #[test]
    fn orchestration_budget_flags_multiple_violators_in_one_solution() {
        let mut errors: Vec<ValidationError> = Vec::new();
        let ok = RetryPolicy::new(2, 5, 20, 40).unwrap();
        let too_big = RetryPolicy::new(3, 10, 50, 200).unwrap();
        let steps = vec![
            step_with_policy("step_a", Some(ok)),
            step_with_policy("step_b", Some(too_big)),
            step_with_policy("step_c", Some(too_big)),
        ];
        // watchdog 100 → half = 50; too_big.total_budget_ms = 200 → violators.
        check_orchestration_budget("sol_x", 100, &steps, &mut errors);
        let fatals: Vec<&ValidationError> = errors.iter().filter(|e| e.is_fatal()).collect();
        assert_eq!(fatals.len(), 2, "expected 2 fatal errors (step_b + step_c)");
        assert!(fatals.iter().any(|e| e.message.contains("step_b")));
        assert!(fatals.iter().any(|e| e.message.contains("step_c")));
        // step_a should NOT appear
        assert!(!fatals.iter().any(|e| e.message.contains("'step_a'")));
    }

    #[test]
    fn orchestration_budget_handles_500ms_watchdog_realistic_profile() {
        // wheeled_4wd_diff worked-example shape from PRD §"worked example":
        // 5 init steps, watchdog 500ms; per-step budget 200ms → half = 250 → OK.
        let mut errors: Vec<ValidationError> = Vec::new();
        let policy = RetryPolicy::new(3, 5, 50, 200).unwrap();
        let steps: Vec<OrchestrationStep> = (0..5)
            .map(|i| step_with_policy(&format!("init_step_{i}"), Some(policy)))
            .collect();
        check_orchestration_budget("wheeled_4wd_diff", 500, &steps, &mut errors);
        assert!(errors.iter().all(|e| !e.is_fatal()));
    }

    #[test]
    fn orchestration_budget_skips_when_solution_has_no_failsafe_watchdog() {
        // A GCS solution (mavlink_groundstation_solution) has no
        // `failsafe.watchdog_ms` per ADR-016 scope. The top-level
        // `validate_orchestration_budget` skips lookup-misses cleanly;
        // here we verify via the public top-level entry point.
        let mut errors: Vec<ValidationError> = Vec::new();
        validate_orchestration_budget("mavlink_groundstation_solution", &mut errors);
        assert!(
            errors.is_empty(),
            "GCS solution without watchdog should produce zero errors, got: {errors:?}"
        );
    }

    #[test]
    fn orchestration_budget_skips_unknown_solution_id() {
        // Unknown solutions are reported by `validate_solution` (Rule 9);
        // this rule must not double-report. Verify graceful return.
        let mut errors: Vec<ValidationError> = Vec::new();
        validate_orchestration_budget("no_such_solution_xyz", &mut errors);
        assert!(errors.is_empty());
    }

    #[test]
    fn solution_extra_component_produces_warning() {
        let mut config = base_config();
        config.esphome.solution = Some("camera_stream".into());
        push(&mut config, "wifi", json!({"ssid": "test"}));
        // extra component not in required or optional bundle
        push(&mut config, "i2c", json!({}));
        let errors = stage_9_final_validation(&config, &reg());
        assert!(errors
            .iter()
            .any(|e| e.message.contains("i2c") && !e.is_fatal()));
    }
}
