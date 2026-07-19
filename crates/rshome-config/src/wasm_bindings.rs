//! Wasm-bindgen export surface for `rshome-config`.
//!
//! All functions accept and return JSON strings for maximum interoperability
//! with browser JavaScript.  Errors are returned as JSON-serialised
//! `Vec<ValidationError>`.

use wasm_bindgen::prelude::*;

use crate::pipeline::{PartialConfig, ValidationPipeline};
use crate::raw::RawConfig;
use crate::stages::s4_external_components::AllowList;

/// Validate a full config JSON string.
///
/// `config_json` — JSON-encoded `RawConfig`.
/// `registry_json` — reserved for future use (pass `null`).
///
/// Returns a JSON object: `{"valid": bool, "errors": [...]}`.
#[wasm_bindgen]
pub fn validate_config(config_json: &str) -> String {
    let config: RawConfig = match serde_json::from_str(config_json) {
        Ok(c) => c,
        Err(e) => {
            return serde_json::json!({
                "valid": false,
                "errors": [{"path": "", "message": format!("JSON parse error: {e}"), "severity": "error", "stage": "preload"}]
            })
            .to_string();
        }
    };

    let registry = rshome_schema::ComponentRegistry::default_registry();
    let pipeline = ValidationPipeline::with_allow_list(registry, AllowList::new());
    let store = crate::raw::PackageStore::new();

    match pipeline.validate(config, &store) {
        crate::pipeline::ValidationResult::Valid(validated) => serde_json::json!({
            "valid": true,
            "errors": [],
            "active_flags": validated.active_flags,
            "chip_target": format!("{:?}", validated.esphome.chip_target),
        })
        .to_string(),
        crate::pipeline::ValidationResult::Invalid(errors) => serde_json::json!({
            "valid": false,
            "errors": errors,
        })
        .to_string(),
    }
}

/// Validate a partial config for real-time browser feedback.
///
/// `partial_json` — JSON object with optional `esphome`, `components`, and
/// `substitutions` fields.
///
/// Returns a JSON array of `ValidationError` objects.
#[wasm_bindgen]
pub fn validate_partial(partial_json: &str) -> String {
    #[derive(serde::Deserialize)]
    struct PartialInput {
        esphome: Option<crate::raw::EsphomeBlock>,
        #[serde(default)]
        components: Vec<crate::raw::ComponentConfig>,
        #[serde(default)]
        substitutions: std::collections::HashMap<String, String>,
    }

    let input: PartialInput = match serde_json::from_str(partial_json) {
        Ok(i) => i,
        Err(e) => {
            return serde_json::json!([{
                "path": "",
                "message": format!("JSON parse error: {e}"),
                "severity": "error",
                "stage": "preload"
            }])
            .to_string();
        }
    };

    let registry = rshome_schema::ComponentRegistry::default_registry();
    let pipeline = ValidationPipeline::new(registry);

    let errors = pipeline.validate_partial(PartialConfig {
        esphome: input.esphome,
        components: input.components,
        substitutions: input.substitutions,
    });

    serde_json::to_string(&errors).unwrap_or_else(|_| "[]".to_owned())
}
