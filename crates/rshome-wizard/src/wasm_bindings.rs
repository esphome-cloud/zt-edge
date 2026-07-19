//! `wasm-bindgen` browser export surface for `rshome-wizard`.
//!
//! All functions accept and return JSON strings.  Delegates to the
//! native implementations in `bindings.rs`.
//!
//! Build with:
//! ```bash
//! wasm-pack build crates/rshome-wizard --target web -- --features wasm
//! ```

use wasm_bindgen::prelude::*;

use crate::bindings;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn to_json_str(value: &serde_json::Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

// ── Exported functions ────────────────────────────────────────────────────────

/// Validate a full rshome device configuration.
///
/// # Input
/// JSON-encoded `RawConfig` object.
///
/// # Output
/// ```json
/// {"valid": true, "errors": [], "active_flags": [...], "chip_target": "Esp32"}
/// ```
/// or on failure:
/// ```json
/// {"valid": false, "errors": [{...}, ...]}
/// ```
#[wasm_bindgen]
pub fn validate_config(config_json: &str) -> String {
    to_json_str(&bindings::validate_config_native(config_json))
}

/// Validate a partial config for real-time field feedback.
///
/// # Input
/// JSON object with optional `esphome`, `components`, `substitutions` fields.
///
/// # Output
/// JSON array of `ValidationError` objects (empty = no errors).
#[wasm_bindgen]
pub fn validate_partial(partial_json: &str) -> String {
    to_json_str(&bindings::validate_partial_native(partial_json))
}

/// List available components filtered by target and/or category.
///
/// # Input
/// JSON object: `{"target": "esp32", "category": "sensor"}` — all fields optional.
///
/// # Output
/// JSON array of `ComponentInfo` objects, sorted by ID.
#[wasm_bindgen]
pub fn list_components(options_json: &str) -> String {
    #[derive(serde::Deserialize, Default)]
    struct Options {
        target: Option<String>,
        category: Option<String>,
    }

    let opts: Options = serde_json::from_str(options_json).unwrap_or_default();
    let components =
        bindings::list_components_native(opts.target.as_deref(), opts.category.as_deref());

    serde_json::to_string(&components).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

/// Get the GPIO pin map for a chip target.
///
/// # Input
/// A chip target string: `"esp32"`, `"esp32s3"`, or `"esp32c6"`.
///
/// # Output
/// JSON array of `PinInfo` objects, one per GPIO pin.
#[wasm_bindgen]
pub fn get_pin_map(target: &str) -> String {
    match bindings::get_pin_map_native(target) {
        Ok(pins) => {
            serde_json::to_string(&pins).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
        }
        Err(msg) => format!("{{\"error\":\"{msg}\"}}"),
    }
}

/// Compute USE_* feature flags for a component ID list.
///
/// # Input
/// JSON array of component ID strings: `["wifi", "dht", "sensor"]`.
///
/// # Output
/// ```json
/// {"flags": ["USE_WIFI", ...], "c_defines": "#define USE_WIFI\n...", "cargo_features": [...]}
/// ```
#[wasm_bindgen]
pub fn compute_feature_flags(ids_json: &str) -> String {
    to_json_str(&bindings::compute_feature_flags_native(ids_json))
}

/// Get the schema definition for a single component.
///
/// # Input
/// Component ID string, e.g. `"dht"`, `"wifi"`.
///
/// # Output
/// JSON `ComponentDefinition` object or `{"error": "..."}`.
#[wasm_bindgen]
pub fn get_component_schema(component_id: &str) -> String {
    to_json_str(&bindings::get_component_schema_native(component_id))
}

/// Preview files that would be generated for a config without writing anything.
///
/// # Input
/// JSON-encoded `RawConfig`.
///
/// # Output
/// ```json
/// {"valid": true, "generated_files": [...], "active_flags": [...], "chip_target": "Esp32"}
/// ```
#[wasm_bindgen]
pub fn codegen_preview(config_json: &str) -> String {
    to_json_str(&bindings::codegen_preview_native(config_json))
}

/// Generate the full ESP-IDF project source tree in memory and return it as
/// a JSON object containing every file (path + base64-encoded content).
///
/// Used by the rshome wizard's "Download ESP-IDF source" button to produce
/// a zippable project on the client side without contacting a build service.
///
/// # Input
/// JSON-encoded `RawConfig`.
///
/// # Output
/// On success: `{"valid": true, "device_name": "...", "chip_target": "...",
///   "file_count": N, "files": [{"path": "...", "content_b64": "..."}, ...]}`.
///
/// On validation/codegen failure: `{"valid": false, "errors": [...]}`.
#[wasm_bindgen]
pub fn codegen_export(config_json: &str) -> String {
    to_json_str(&bindings::codegen_export_native(config_json))
}

/// List available hardware modules with optional filtering.
///
/// # Input
/// JSON object: `{"target": "esp32s3"}` — all fields optional.
///
/// # Output
/// JSON array of `ModuleInfo` objects, sorted by ID.
#[wasm_bindgen]
pub fn list_modules(options_json: &str) -> String {
    #[derive(serde::Deserialize, Default)]
    struct Options {
        target: Option<String>,
    }

    let opts: Options = serde_json::from_str(options_json).unwrap_or_default();
    let modules = bindings::list_modules_native(opts.target.as_deref());
    serde_json::to_string(&modules).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

/// List available solutions with optional filtering.
///
/// # Input
/// JSON object: `{"module_id": "esp32s3_generic_wifi_board"}` — all fields optional.
///
/// # Output
/// JSON array of `SolutionInfo` objects, sorted by ID.
#[wasm_bindgen]
pub fn list_solutions(options_json: &str) -> String {
    #[derive(serde::Deserialize, Default)]
    struct Options {
        module_id: Option<String>,
    }

    let opts: Options = serde_json::from_str(options_json).unwrap_or_default();
    let solutions = bindings::list_solutions_native(opts.module_id.as_deref());
    serde_json::to_string(&solutions).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

/// Get the full definition of a specific solution.
///
/// # Input
/// Solution ID string, e.g. `"camera_stream"`.
///
/// # Output
/// JSON `SolutionDefinition` object or `{"error": "..."}`.
#[wasm_bindgen]
pub fn get_solution_detail(solution_id: &str) -> String {
    to_json_str(&bindings::get_solution_detail_native(solution_id))
}

/// Get the full platform capability catalog.
///
/// # Output
/// JSON `PlatformCatalog` object with all target definitions.
#[wasm_bindgen]
pub fn get_platform_catalog() -> String {
    to_json_str(&bindings::get_platform_catalog_native())
}

/// Get auto-derived board assembly for a module.
///
/// # Input
/// Module ID string, e.g. `"esp32s3_generic_wifi_board"`.
///
/// # Output
/// JSON `HardwareAssemblyDefinition` object or `{"error": "..."}`.
#[wasm_bindgen]
pub fn get_assembly_for_module(module_id: &str) -> String {
    to_json_str(&bindings::get_assembly_for_module_native(module_id))
}

/// Get HA entity export definitions for a solution.
///
/// # Input
/// Solution ID string, e.g. `"sensor_hub"`.
///
/// # Output
/// JSON array of `HaEntityExportDefinition` objects.
#[wasm_bindgen]
pub fn get_ha_entities_for_solution(solution_id: &str) -> String {
    to_json_str(&bindings::get_ha_entities_native(solution_id))
}

/// Get network topology for a solution.
///
/// # Input
/// Solution ID string, e.g. `"sensor_hub"`.
///
/// # Output
/// JSON string of `NetworkTopology` value (e.g. `"star"`, `"mesh"`).
#[wasm_bindgen]
pub fn get_network_topology(solution_id: &str) -> String {
    to_json_str(&bindings::get_network_topology_native(solution_id))
}

/// Validate a workspace (multiple saved profiles) for cross-profile compatibility.
///
/// # Input
/// JSON-encoded `Workspace` object: `{"profiles": [{...SavedProfile}, ...]}`.
///
/// # Output
/// - On success: JSON array of `WorkspaceError` objects; empty array means
///   the workspace is internally consistent. Each error carries `{kind,
///   message, profiles}` where `kind` is one of `UnknownSolution`,
///   `UnmatchedTxUplink`, `UnmatchedRxUplink`, `ChainTelemetryMismatch`,
///   `PinConflict`, `ParameterizedUplinkInvalid`.
/// - On parse failure: `{"parse_error": "..."}`.
///
/// Closes va-residuals Phase 8 T8.4 transport gap (workspace-ux PRD
/// Phase 0 T0.2).
#[wasm_bindgen]
pub fn validate_workspace(workspace_json: &str) -> String {
    to_json_str(&bindings::validate_workspace_native(workspace_json))
}

/// Generate a JSON-IR (Intermediate Representation) from a device config.
///
/// Replaces JS `generateDeviceIR()` in `lib/json-ir-generator.ts`.
///
/// # Input
/// JSON-encoded `RawConfig` object.
///
/// # Output
/// See [`bindings::generate_device_ir_native`] for the output shape.
#[wasm_bindgen]
pub fn generate_device_ir(device_config_json: &str) -> String {
    to_json_str(&bindings::generate_device_ir_native(device_config_json))
}

/// Unpack a codegen export JSON response into a structured file tree.
///
/// Replaces JS `exportRshomeProject()` base64 handling in
/// `lib/rshome-codegen-export.ts`. Takes the flat file array from
/// [`codegen_export`] and reorganizes it into a hierarchical directory
/// tree with canonical base64-encoded file contents.
///
/// # Input
/// JSON response from `codegen_export`.
///
/// # Output
/// See [`bindings::unpack_codegen_archive_native`] for the output shape.
#[wasm_bindgen]
pub fn unpack_codegen_archive(codegen_json: &str) -> String {
    to_json_str(&bindings::unpack_codegen_archive_native(codegen_json))
}
