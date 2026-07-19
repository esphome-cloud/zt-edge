//! Stage 8 — ID resolution and cross-reference validation.
//!
//! - Collects all entity IDs from component configs.
//! - Detects duplicate IDs.
//! - Auto-generates IDs for entities that omit them.
//! - Validates cross-references (template sensors referencing other entities).

use std::collections::HashMap;

use crate::error::{ValidationError, ValidationStage};
use crate::raw::RawConfig;

// ── ID table ──────────────────────────────────────────────────────────────────

/// A resolved entity ID entry.
#[derive(Debug, Clone)]
pub struct IdEntry {
    /// The stable ID string.
    pub id: String,
    /// Component index in the config that owns this ID.
    pub component_index: usize,
    /// Component type that owns this ID.
    pub component_type: String,
}

// ── Stage entry point ─────────────────────────────────────────────────────────

/// Stage 8: resolve entity IDs and check cross-references.
///
/// Returns the populated ID table on success (errors may still be present for
/// non-fatal issues).
pub fn stage_8_resolve_ids(
    config: &mut RawConfig,
) -> (HashMap<String, IdEntry>, Vec<ValidationError>) {
    let mut errors = Vec::new();
    let mut id_table: HashMap<String, IdEntry> = HashMap::new();

    // First pass: collect all explicitly set IDs and detect duplicates.
    for (i, comp) in config.components.iter().enumerate() {
        if let Some(id_val) = comp.config.get("id") {
            if let Some(id_str) = id_val.as_str() {
                let id = id_str.to_owned();
                if let Some(existing) = id_table.get(&id) {
                    errors.push(ValidationError::error(
                        ValidationStage::IdResolution,
                        format!("components[{i}].id"),
                        format!(
                            "duplicate ID '{}': already used by {} at index {}",
                            id, existing.component_type, existing.component_index
                        ),
                    ));
                } else {
                    id_table.insert(
                        id.clone(),
                        IdEntry {
                            id,
                            component_index: i,
                            component_type: comp.component_type.clone(),
                        },
                    );
                }
            } else if !id_val.is_null() {
                errors.push(ValidationError::error(
                    ValidationStage::IdResolution,
                    format!("components[{i}].id"),
                    "entity 'id' field must be a string".to_owned(),
                ));
            }
        }
    }

    // Second pass: auto-generate IDs for entities that need one (have a name).
    for (i, comp) in config.components.iter_mut().enumerate() {
        if comp.config.get("id").is_none() {
            // Generate ID from name if present.
            if let Some(name) = comp.config.get("name").and_then(|v| v.as_str()) {
                let generated = slugify(name);
                // Ensure uniqueness.
                let unique_id = make_unique(&generated, &id_table);
                id_table.insert(
                    unique_id.clone(),
                    IdEntry {
                        id: unique_id.clone(),
                        component_index: i,
                        component_type: comp.component_type.clone(),
                    },
                );
                // Inject the generated ID into the config.
                if let Some(obj) = comp.config.as_object_mut() {
                    obj.insert("id".into(), serde_json::Value::String(unique_id));
                }
            }
        }
    }

    // Third pass: validate cross-references (e.g. `sensor_id` fields).
    for (i, comp) in config.components.iter().enumerate() {
        let path = format!("components[{i}]");
        check_cross_refs(&comp.config, &id_table, &path, &mut errors);
    }

    (id_table, errors)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Convert a human-readable name into a valid identifier slug.
fn slugify(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_owned()
}

/// Append a numeric suffix to make `base` unique in `table`.
fn make_unique(base: &str, table: &HashMap<String, IdEntry>) -> String {
    if !table.contains_key(base) {
        return base.to_owned();
    }
    let mut n = 2usize;
    loop {
        let candidate = format!("{base}_{n}");
        if !table.contains_key(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Walk a JSON value looking for `*_id` fields and verify they resolve.
fn check_cross_refs(
    value: &serde_json::Value,
    id_table: &HashMap<String, IdEntry>,
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    if let Some(obj) = value.as_object() {
        for (key, val) in obj {
            let field_path = format!("{path}.{key}");
            // Fields ending in `_id` that are strings are cross-references.
            if key.ends_with("_id") && key != "id" {
                if let Some(ref_id) = val.as_str() {
                    if !ref_id.is_empty() && !id_table.contains_key(ref_id) {
                        errors.push(ValidationError::error(
                            ValidationStage::IdResolution,
                            &field_path,
                            format!("cross-reference '{ref_id}' not found in ID table"),
                        ));
                    }
                }
            }
            // Recurse into nested objects.
            check_cross_refs(val, id_table, &field_path, errors);
        }
    } else if let Some(arr) = value.as_array() {
        for (i, item) in arr.iter().enumerate() {
            check_cross_refs(item, id_table, &format!("{path}[{i}]"), errors);
        }
    }
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
    fn explicit_ids_collected() {
        let mut config = base_config();
        push(
            &mut config,
            "sensor",
            json!({"id": "my_sensor", "name": "temp"}),
        );
        let (table, errors) = stage_8_resolve_ids(&mut config);
        assert!(errors.is_empty());
        assert!(table.contains_key("my_sensor"));
    }

    #[test]
    fn duplicate_id_produces_error() {
        let mut config = base_config();
        push(&mut config, "sensor", json!({"id": "dup", "name": "A"}));
        push(&mut config, "sensor", json!({"id": "dup", "name": "B"}));
        let (_, errors) = stage_8_resolve_ids(&mut config);
        assert!(errors
            .iter()
            .any(|e| e.message.contains("duplicate ID") && e.is_fatal()));
    }

    #[test]
    fn id_auto_generated_from_name() {
        let mut config = base_config();
        push(&mut config, "sensor", json!({"name": "Temperature Sensor"}));
        let (table, errors) = stage_8_resolve_ids(&mut config);
        assert!(errors.is_empty());
        // Should contain a slugified version of the name.
        assert!(table.keys().any(|k| k.contains("temperature")));
    }

    #[test]
    fn auto_generated_id_injected_into_config() {
        let mut config = base_config();
        push(&mut config, "sensor", json!({"name": "Humidity"}));
        stage_8_resolve_ids(&mut config);
        // The component's config should now have an `id` field.
        assert!(config.components[0].config.get("id").is_some());
    }

    #[test]
    fn unique_suffix_appended_for_name_collision() {
        let mut config = base_config();
        push(&mut config, "sensor", json!({"name": "temp"}));
        push(&mut config, "sensor", json!({"name": "temp"}));
        let (table, _) = stage_8_resolve_ids(&mut config);
        // Both should have entries: "temp" and "temp_2"
        assert!(table.contains_key("temp"));
        assert!(table.contains_key("temp_2"));
    }

    #[test]
    fn cross_reference_to_known_id_passes() {
        let mut config = base_config();
        push(
            &mut config,
            "sensor",
            json!({"id": "my_sensor", "name": "temp"}),
        );
        push(
            &mut config,
            "template",
            json!({"sensor_id": "my_sensor", "name": "derived"}),
        );
        let (_, errors) = stage_8_resolve_ids(&mut config);
        assert!(errors.is_empty(), "errors: {:?}", errors);
    }

    #[test]
    fn cross_reference_to_unknown_id_produces_error() {
        let mut config = base_config();
        push(
            &mut config,
            "template",
            json!({"sensor_id": "nonexistent_sensor", "name": "derived"}),
        );
        let (_, errors) = stage_8_resolve_ids(&mut config);
        assert!(errors
            .iter()
            .any(|e| e.message.contains("nonexistent_sensor")));
    }

    #[test]
    fn slugify_spaces_to_underscores() {
        assert_eq!(slugify("Living Room Temp"), "living_room_temp");
    }

    #[test]
    fn slugify_special_chars_to_underscores() {
        assert_eq!(slugify("temp-sensor/1"), "temp_sensor_1");
    }
}
