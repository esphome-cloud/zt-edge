//! Stage 2 — Substitution resolution.
//!
//! Replaces `${var}` and `${var:=default}` placeholders in all string fields
//! of every component's config.  Substitutions are defined in `config.substitutions`.

use std::collections::HashMap;

use crate::error::{ValidationError, ValidationStage};
use crate::raw::RawConfig;

// ── Core substitution engine ──────────────────────────────────────────────────

/// Resolve all `${…}` placeholders in a single string value.
///
/// - `${var}` → value from `subs`; warning if undefined.
/// - `${var:=default}` → value from `subs` if present, otherwise `default`.
fn resolve_string(
    s: &str,
    subs: &HashMap<String, String>,
    errors: &mut Vec<ValidationError>,
    path: &str,
) -> String {
    let mut result = String::with_capacity(s.len());
    let mut remaining = s;

    while let Some(start) = remaining.find("${") {
        result.push_str(&remaining[..start]);
        remaining = &remaining[start + 2..];

        if let Some(end) = remaining.find('}') {
            let expr = &remaining[..end];
            remaining = &remaining[end + 1..];

            let (var_name, default) = if let Some(sep) = expr.find(":=") {
                (&expr[..sep], Some(&expr[sep + 2..]))
            } else {
                (expr, None)
            };

            match subs.get(var_name) {
                Some(val) => result.push_str(val),
                None => match default {
                    Some(d) => result.push_str(d),
                    None => {
                        errors.push(ValidationError::warning(
                            ValidationStage::Substitutions,
                            path,
                            format!("substitution variable '${{{var_name}}}' is undefined"),
                        ));
                        // Preserve the original placeholder so downstream stages can see it.
                        result.push_str("${");
                        result.push_str(expr);
                        result.push('}');
                    }
                },
            }
        } else {
            // Unclosed `${` — pass through unchanged.
            result.push_str("${");
        }
    }

    result.push_str(remaining);
    result
}

/// Recursively apply substitutions to a JSON value in place.
fn apply_to_value(
    value: &mut serde_json::Value,
    subs: &HashMap<String, String>,
    errors: &mut Vec<ValidationError>,
    path: &str,
) {
    match value {
        serde_json::Value::String(s) => {
            *s = resolve_string(s, subs, errors, path);
        }
        serde_json::Value::Array(arr) => {
            for (i, item) in arr.iter_mut().enumerate() {
                apply_to_value(item, subs, errors, &format!("{path}[{i}]"));
            }
        }
        serde_json::Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                apply_to_value(v, subs, errors, &format!("{path}.{k}"));
            }
        }
        // Numbers, booleans, null — no substitution needed.
        _ => {}
    }
}

// ── Stage entry point ─────────────────────────────────────────────────────────

/// Stage 2: apply all substitutions to component configs.
pub fn stage_2_apply_substitutions(config: &mut RawConfig) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    let subs = config.substitutions.clone();

    for (i, comp) in config.components.iter_mut().enumerate() {
        let path = format!("components[{i}]");
        apply_to_value(&mut comp.config, &subs, &mut errors, &path);

        // Also substitute in the `platform` field string.
        if let Some(platform) = comp.platform.take() {
            comp.platform = Some(resolve_string(&platform, &subs, &mut errors, &path));
        }
    }

    errors
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

    fn push_comp(config: &mut RawConfig, typ: &str, val: serde_json::Value) {
        config.components.push(ComponentConfig {
            component_type: typ.into(),
            platform: None,
            config: val,
        });
    }

    #[test]
    fn no_substitutions_noop() {
        let mut config = base_config();
        push_comp(&mut config, "sensor", json!({"pin": 4, "name": "temp"}));
        let errors = stage_2_apply_substitutions(&mut config);
        assert!(errors.is_empty());
        assert_eq!(config.components[0].config["pin"], 4);
        assert_eq!(config.components[0].config["name"], "temp");
    }

    #[test]
    fn simple_string_substitution() {
        let mut config = base_config();
        config
            .substitutions
            .insert("sensor_name".into(), "Temperature".into());
        push_comp(&mut config, "sensor", json!({"name": "${sensor_name}"}));
        let errors = stage_2_apply_substitutions(&mut config);
        assert!(errors.is_empty());
        assert_eq!(config.components[0].config["name"], "Temperature");
    }

    #[test]
    fn default_used_when_variable_missing() {
        let mut config = base_config();
        push_comp(
            &mut config,
            "sensor",
            json!({"name": "${missing:=fallback}"}),
        );
        let errors = stage_2_apply_substitutions(&mut config);
        assert!(errors.is_empty());
        assert_eq!(config.components[0].config["name"], "fallback");
    }

    #[test]
    fn explicit_value_takes_precedence_over_default() {
        let mut config = base_config();
        config
            .substitutions
            .insert("my_var".into(), "actual".into());
        push_comp(
            &mut config,
            "sensor",
            json!({"name": "${my_var:=fallback}"}),
        );
        let errors = stage_2_apply_substitutions(&mut config);
        assert!(errors.is_empty());
        assert_eq!(config.components[0].config["name"], "actual");
    }

    #[test]
    fn undefined_var_without_default_produces_warning() {
        let mut config = base_config();
        push_comp(&mut config, "sensor", json!({"name": "${undef}"}));
        let errors = stage_2_apply_substitutions(&mut config);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].severity, crate::error::Severity::Warning);
        assert!(errors[0].message.contains("undef"));
        // Placeholder preserved.
        assert_eq!(config.components[0].config["name"], "${undef}");
    }

    #[test]
    fn substitution_in_nested_object() {
        let mut config = base_config();
        config.substitutions.insert("sda_pin".into(), "21".into());
        push_comp(
            &mut config,
            "i2c",
            json!({"sda": "${sda_pin}", "scl": "22"}),
        );
        let errors = stage_2_apply_substitutions(&mut config);
        assert!(errors.is_empty());
        assert_eq!(config.components[0].config["sda"], "21");
        assert_eq!(config.components[0].config["scl"], "22");
    }

    #[test]
    fn substitution_in_array_elements() {
        let mut config = base_config();
        config.substitutions.insert("room".into(), "kitchen".into());
        push_comp(&mut config, "logger", json!({"tags": ["${room}", "fixed"]}));
        let errors = stage_2_apply_substitutions(&mut config);
        assert!(errors.is_empty());
        assert_eq!(config.components[0].config["tags"][0], "kitchen");
        assert_eq!(config.components[0].config["tags"][1], "fixed");
    }

    #[test]
    fn platform_field_substitution() {
        let mut config = base_config();
        config.substitutions.insert("plat".into(), "dht".into());
        config.components.push(ComponentConfig {
            component_type: "sensor".into(),
            platform: Some("${plat}".into()),
            config: json!({}),
        });
        let errors = stage_2_apply_substitutions(&mut config);
        assert!(errors.is_empty());
        assert_eq!(config.components[0].platform.as_deref(), Some("dht"));
    }

    #[test]
    fn multiple_placeholders_in_one_string() {
        let mut config = base_config();
        config.substitutions.insert("device".into(), "mydev".into());
        config
            .substitutions
            .insert("suffix".into(), "_sensor".into());
        push_comp(&mut config, "sensor", json!({"name": "${device}${suffix}"}));
        let errors = stage_2_apply_substitutions(&mut config);
        assert!(errors.is_empty());
        assert_eq!(config.components[0].config["name"], "mydev_sensor");
    }
}
