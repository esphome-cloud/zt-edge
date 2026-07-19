//! `wasm-bindgen` browser export surface for `rshome-schema`.
//!
//! Provides thin JSON-in / JSON-out functions that wrap the native Rust API so
//! TypeScript/JavaScript code can call them without custom JS type definitions.
//!
//! Build with:
//! ```bash
//! wasm-pack build crates/rshome-schema --target web -- --features wasm
//! ```

use wasm_bindgen::prelude::*;

use crate::feature_flags::FeatureFlagSet;
use crate::registry::{ComponentId, ComponentRegistry};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn default_registry() -> ComponentRegistry {
    ComponentRegistry::default_registry()
}

fn ok_json(value: impl serde::Serialize) -> String {
    serde_json::to_string(&value).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

fn err_json(msg: &str) -> String {
    format!("{{\"ok\":false,\"errors\":[{:?}]}}", msg)
}

// ── Exported functions ────────────────────────────────────────────────────────

/// Validate a JSON array of component IDs for dependency satisfaction and conflicts.
///
/// # Input
/// A JSON array of component ID strings, e.g. `["wifi","dht","ota"]`.
///
/// # Output
/// ```json
/// { "ok": true, "errors": [] }
/// ```
/// or on failure:
/// ```json
/// { "ok": false, "errors": ["component 'ota' requires 'wifi' which is not selected", ...] }
/// ```
#[wasm_bindgen]
pub fn validate_component_ids(ids_json: &str) -> String {
    let ids: Vec<ComponentId> = match serde_json::from_str(ids_json) {
        Ok(v) => v,
        Err(e) => return err_json(&format!("invalid JSON: {e}")),
    };

    let reg = default_registry();
    let mut errors: Vec<String> = Vec::new();

    if let Err(dep_errors) = reg.check_dependencies(&ids) {
        for e in dep_errors {
            errors.push(e.to_string());
        }
    }
    if let Err(conflicts) = reg.check_conflicts(&ids) {
        for e in conflicts {
            errors.push(e.to_string());
        }
    }

    let ok = errors.is_empty();
    ok_json(serde_json::json!({ "ok": ok, "errors": errors }))
}

/// Expand a JSON array of component IDs with their AUTO_LOAD dependencies.
///
/// # Input
/// A JSON array of component ID strings.
///
/// # Output
/// A JSON array of component ID strings (sorted, deduplicated, auto-loads included).
#[wasm_bindgen]
pub fn resolve_auto_load(ids_json: &str) -> String {
    let ids: Vec<ComponentId> = match serde_json::from_str(ids_json) {
        Ok(v) => v,
        Err(e) => return err_json(&format!("invalid JSON: {e}")),
    };

    let reg = default_registry();
    let expanded = reg.resolve_auto_load(&ids);
    ok_json(expanded)
}

/// Compute feature flags for a JSON array of component IDs.
///
/// # Input
/// A JSON array of component ID strings.
///
/// # Output
/// ```json
/// {
///   "c_defines": "#define USE_WIFI\n#define USE_SENSOR\n...",
///   "flags": ["USE_WIFI", "USE_SENSOR", ...],
///   "cargo_features": ["wifi", "sensor", ...]
/// }
/// ```
#[wasm_bindgen]
pub fn compute_feature_flags(ids_json: &str) -> String {
    let ids: Vec<ComponentId> = match serde_json::from_str(ids_json) {
        Ok(v) => v,
        Err(e) => return err_json(&format!("invalid JSON: {e}")),
    };

    let reg = default_registry();
    let ffs = FeatureFlagSet::from_components(&ids, &reg);

    let flags: Vec<&str> = {
        let mut v: Vec<&str> = ffs.iter_flags().collect();
        v.sort();
        v
    };
    let cargo = ffs.to_cargo_features();
    let c_defines = ffs.to_c_defines();

    ok_json(serde_json::json!({
        "c_defines": c_defines,
        "flags": flags,
        "cargo_features": cargo,
    }))
}

/// Return the full JSON Schema for all entity types.
///
/// # Output
/// A pretty-printed JSON Schema string (draft 7 compatible).
#[wasm_bindgen]
pub fn get_json_schema() -> String {
    let reg = default_registry();
    reg.to_json_schema_string()
}

// ── Tests (run only on native, not wasm32) ────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_component_ids_valid_input() {
        let result = validate_component_ids(r#"["wifi","logger"]"#);
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(
            v["ok"].as_bool().unwrap(),
            "expected ok=true, got: {result}"
        );
        assert!(v["errors"].as_array().unwrap().is_empty());
    }

    #[test]
    fn validate_component_ids_ota_missing_wifi() {
        let result = validate_component_ids(r#"["ota"]"#);
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(
            !v["ok"].as_bool().unwrap(),
            "expected ok=false, got: {result}"
        );
        assert!(!v["errors"].as_array().unwrap().is_empty());
    }

    #[test]
    fn validate_component_ids_wifi_ethernet_conflict() {
        let result = validate_component_ids(r#"["wifi","ethernet"]"#);
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(!v["ok"].as_bool().unwrap());
    }

    #[test]
    fn validate_component_ids_invalid_json_returns_error() {
        let result = validate_component_ids("not json");
        assert!(result.contains("error") || result.contains("false"));
    }

    #[test]
    fn resolve_auto_load_expands_dht() {
        let result = resolve_auto_load(r#"["dht"]"#);
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let ids: Vec<&str> = v
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect();
        assert!(ids.contains(&"sensor"), "sensor should be auto-loaded");
        assert!(ids.contains(&"dht"));
    }

    #[test]
    fn resolve_auto_load_empty_returns_empty_array() {
        let result = resolve_auto_load(r#"[]"#);
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(v.as_array().unwrap().is_empty());
    }

    #[test]
    fn compute_feature_flags_wifi_logger() {
        let result = compute_feature_flags(r#"["wifi","logger"]"#);
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let defines = v["c_defines"].as_str().unwrap();
        assert!(defines.contains("USE_WIFI"));
        assert!(defines.contains("USE_LOGGER"));
        let flags = v["flags"].as_array().unwrap();
        assert!(flags.iter().any(|f| f.as_str() == Some("USE_WIFI")));
    }

    #[test]
    fn compute_feature_flags_invalid_json() {
        let result = compute_feature_flags("{bad json}");
        assert!(result.contains("error") || result.contains("false"));
    }

    #[test]
    fn get_json_schema_returns_valid_json() {
        let schema = get_json_schema();
        let v: serde_json::Value =
            serde_json::from_str(&schema).expect("get_json_schema should return valid JSON");
        assert!(v.is_object());
    }

    #[test]
    fn get_json_schema_contains_entity_types() {
        let schema = get_json_schema();
        assert!(schema.contains("sensor"));
        assert!(schema.contains("climate"));
    }
}
