//! Stage 3 — Extend / Remove directives.
//!
//! ESPHome supports two config-level directives:
//!
//! - `!extend <type>` — deep-merges the directive's config into the first
//!   existing component of that type.
//! - `!remove <type>` — removes the first component of that type.
//!
//! Directives are represented as [`ComponentConfig`](crate::raw::ComponentConfig)
//! entries whose `component_type` starts with `"!"`.  After processing, all
//! directive entries are consumed and removed from the component list.

use crate::error::{ValidationError, ValidationStage};
use crate::raw::RawConfig;

// ── Deep merge ────────────────────────────────────────────────────────────────

/// Recursively merge `src` into `dst`.
///
/// - Object keys in `src` override corresponding keys in `dst`.
/// - Nested objects are merged recursively.
/// - Arrays in `src` replace arrays in `dst` entirely.
fn deep_merge(dst: &mut serde_json::Value, src: &serde_json::Value) {
    match (dst, src) {
        (serde_json::Value::Object(d), serde_json::Value::Object(s)) => {
            for (key, sv) in s {
                let entry = d.entry(key.clone()).or_insert(serde_json::Value::Null);
                if entry.is_object() && sv.is_object() {
                    deep_merge(entry, sv);
                } else {
                    *entry = sv.clone();
                }
            }
        }
        // For non-object destinations, src replaces dst entirely.
        (dst, src) => *dst = src.clone(),
    }
}

// ── Stage entry point ─────────────────────────────────────────────────────────

/// Stage 3: process `!extend` and `!remove` directives.
///
/// All directive entries are removed from `config.components` during processing.
pub fn stage_3_process_extend_remove(config: &mut RawConfig) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    let mut i = 0;

    while i < config.components.len() {
        let comp = &config.components[i];

        if comp.is_extend() {
            let target = comp.directive_target().map(str::to_owned);
            let extend_config = comp.config.clone();
            config.components.remove(i);
            // i now points to the next element — don't increment.

            match target {
                None => {
                    errors.push(ValidationError::error(
                        ValidationStage::ExtendRemove,
                        "components",
                        "!extend directive is missing a target component type",
                    ));
                }
                Some(ref target_type) => {
                    match config
                        .components
                        .iter_mut()
                        .find(|c| &c.component_type == target_type)
                    {
                        None => {
                            errors.push(ValidationError::error(
                                ValidationStage::ExtendRemove,
                                format!("!extend {target_type}"),
                                format!(
                                    "!extend target '{target_type}' not found in component list"
                                ),
                            ));
                        }
                        Some(existing) => {
                            deep_merge(&mut existing.config, &extend_config);
                        }
                    }
                }
            }
        } else if comp.is_remove() {
            let target = comp.directive_target().map(str::to_owned);
            config.components.remove(i);
            // i now points to the next element — don't increment.

            match target {
                None => {
                    errors.push(ValidationError::error(
                        ValidationStage::ExtendRemove,
                        "components",
                        "!remove directive is missing a target component type",
                    ));
                }
                Some(ref target_type) => {
                    match config
                        .components
                        .iter()
                        .position(|c| &c.component_type == target_type)
                    {
                        None => {
                            errors.push(ValidationError::warning(
                                ValidationStage::ExtendRemove,
                                format!("!remove {target_type}"),
                                format!(
                                    "!remove target '{target_type}' not found; nothing removed"
                                ),
                            ));
                        }
                        Some(j) => {
                            config.components.remove(j);
                        }
                    }
                }
            }
        } else {
            i += 1;
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

    fn comp(typ: &str, val: serde_json::Value) -> ComponentConfig {
        ComponentConfig {
            component_type: typ.into(),
            platform: None,
            config: val,
        }
    }

    #[test]
    fn no_directives_noop() {
        let mut config = base_config();
        config.components.push(comp("wifi", json!({"ssid": "Net"})));
        let errors = stage_3_process_extend_remove(&mut config);
        assert!(errors.is_empty());
        assert_eq!(config.components.len(), 1);
    }

    #[test]
    fn extend_merges_new_keys() {
        let mut config = base_config();
        config.components.push(comp("wifi", json!({"ssid": "Net"})));
        config
            .components
            .push(comp("!extend wifi", json!({"password": "secret"})));
        let errors = stage_3_process_extend_remove(&mut config);
        assert!(errors.is_empty());
        assert_eq!(config.components.len(), 1);
        assert_eq!(config.components[0].config["ssid"], "Net");
        assert_eq!(config.components[0].config["password"], "secret");
    }

    #[test]
    fn extend_overrides_existing_key() {
        let mut config = base_config();
        config
            .components
            .push(comp("wifi", json!({"ssid": "OldNet"})));
        config
            .components
            .push(comp("!extend wifi", json!({"ssid": "NewNet"})));
        let errors = stage_3_process_extend_remove(&mut config);
        assert!(errors.is_empty());
        assert_eq!(config.components[0].config["ssid"], "NewNet");
    }

    #[test]
    fn extend_missing_target_produces_error() {
        let mut config = base_config();
        config
            .components
            .push(comp("!extend nonexistent", json!({"x": 1})));
        let errors = stage_3_process_extend_remove(&mut config);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].is_fatal());
        assert!(errors[0].message.contains("nonexistent"));
    }

    #[test]
    fn remove_deletes_target_component() {
        let mut config = base_config();
        config.components.push(comp("logger", json!({})));
        config.components.push(comp("wifi", json!({"ssid": "Net"})));
        config.components.push(comp("!remove logger", json!({})));
        let errors = stage_3_process_extend_remove(&mut config);
        assert!(errors.is_empty());
        assert_eq!(config.components.len(), 1);
        assert_eq!(config.components[0].component_type, "wifi");
    }

    #[test]
    fn remove_missing_target_produces_warning() {
        let mut config = base_config();
        config
            .components
            .push(comp("!remove nonexistent", json!({})));
        let errors = stage_3_process_extend_remove(&mut config);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].severity, crate::error::Severity::Warning);
        assert!(errors[0].message.contains("nonexistent"));
    }

    #[test]
    fn directives_not_present_in_final_component_list() {
        let mut config = base_config();
        config.components.push(comp("wifi", json!({"ssid": "Net"})));
        config
            .components
            .push(comp("!extend wifi", json!({"password": "pw"})));
        stage_3_process_extend_remove(&mut config);
        assert!(config
            .components
            .iter()
            .all(|c| !c.component_type.starts_with('!')));
    }

    #[test]
    fn deep_merge_nested_objects() {
        let mut config = base_config();
        config.components.push(comp(
            "logger",
            json!({"level": "INFO", "logs": {"sensor": "DEBUG"}}),
        ));
        config
            .components
            .push(comp("!extend logger", json!({"logs": {"switch": "WARN"}})));
        let errors = stage_3_process_extend_remove(&mut config);
        assert!(errors.is_empty());
        assert_eq!(config.components[0].config["level"], "INFO");
        // Nested merge: both sensor and switch keys should be present.
        assert_eq!(config.components[0].config["logs"]["sensor"], "DEBUG");
        assert_eq!(config.components[0].config["logs"]["switch"], "WARN");
    }
}
