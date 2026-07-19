//! Native (non-Wasm) binding implementations.
//!
//! These functions contain the core logic shared between the native and
//! Wasm export surfaces.  The Wasm surface in `wasm_bindings.rs` is a thin
//! wrapper that delegates here.

use rshome_config::pipeline::{PartialConfig, ValidationPipeline, ValidationResult};
use rshome_config::raw::{ComponentConfig, EsphomeBlock, PackageStore, RawConfig};
use rshome_schema::feature_flags::FeatureFlagSet;
use rshome_schema::module::default_module_registry;
use rshome_schema::platform::PlatformCatalog;
use rshome_schema::registry::{ComponentId, ComponentRegistry};
use rshome_schema::solution::default_solution_registry;
use rshome_schema::ChipTarget;

use crate::pin_map::chip_pin_info;
use crate::types::{ComponentInfo, ModuleInfo, PinInfo, SolutionInfo};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn default_registry() -> ComponentRegistry {
    ComponentRegistry::default_registry()
}

fn parse_chip_target(target: &str) -> Option<ChipTarget> {
    match target.to_lowercase().as_str() {
        "esp32" => Some(ChipTarget::Esp32),
        "esp32s2" | "esp32_s2" | "esp32-s2" => Some(ChipTarget::Esp32S2),
        "esp32s3" | "esp32_s3" | "esp32-s3" => Some(ChipTarget::Esp32S3),
        "esp32c2" | "esp32_c2" | "esp32-c2" => Some(ChipTarget::Esp32C2),
        "esp32c3" | "esp32_c3" | "esp32-c3" => Some(ChipTarget::Esp32C3),
        "esp32c5" | "esp32_c5" | "esp32-c5" => Some(ChipTarget::Esp32C5),
        "esp32c6" | "esp32_c6" | "esp32-c6" => Some(ChipTarget::Esp32C6),
        "esp32c61" | "esp32_c61" | "esp32-c61" => Some(ChipTarget::Esp32C61),
        "esp32h2" | "esp32_h2" | "esp32-h2" => Some(ChipTarget::Esp32H2),
        "esp32p4" | "esp32_p4" | "esp32-p4" => Some(ChipTarget::Esp32P4),
        _ => None,
    }
}

// ── Core API functions ────────────────────────────────────────────────────────

/// Validate a full `RawConfig` JSON string.
///
/// Returns `(valid, summary_or_errors)` where summary contains `active_flags`
/// and `chip_target` on success.
pub fn validate_config_native(config_json: &str) -> serde_json::Value {
    let config: RawConfig = match serde_json::from_str(config_json) {
        Ok(c) => c,
        Err(e) => {
            return serde_json::json!({
                "valid": false,
                "errors": [{"path": "", "message": format!("JSON parse error: {e}"), "severity": "error"}]
            });
        }
    };

    let registry = default_registry();
    let pipeline = ValidationPipeline::new(registry);
    let store = PackageStore::new();

    match pipeline.validate(config, &store) {
        ValidationResult::Valid(validated) => {
            serde_json::json!({
                "valid": true,
                "errors": [],
                "active_flags": validated.active_flags,
                "chip_target": format!("{:?}", validated.esphome.chip_target),
                "component_count": validated.components.len(),
                "pin_allocations": validated.pin_allocations.len(),
            })
        }
        ValidationResult::Invalid(errors) => {
            serde_json::json!({
                "valid": false,
                "errors": errors,
            })
        }
    }
}

/// Validate a partial config for real-time browser feedback.
pub fn validate_partial_native(partial_json: &str) -> serde_json::Value {
    #[derive(serde::Deserialize)]
    struct PartialInput {
        esphome: Option<EsphomeBlock>,
        #[serde(default)]
        components: Vec<ComponentConfig>,
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
            }]);
        }
    };

    let registry = default_registry();
    let pipeline = ValidationPipeline::new(registry);

    let errors = pipeline.validate_partial(PartialConfig {
        esphome: input.esphome,
        components: input.components,
        substitutions: input.substitutions,
    });

    serde_json::to_value(errors).unwrap_or(serde_json::json!([]))
}

/// List available components, optionally filtered by entity type category.
pub fn list_components_native(target: Option<&str>, category: Option<&str>) -> Vec<ComponentInfo> {
    let registry = default_registry();
    let mut components: Vec<ComponentInfo> = registry
        .all_definitions()
        .map(ComponentInfo::from)
        .collect();

    // Filter by entity type category if provided.
    if let Some(cat) = category {
        let cat_lower = cat.to_lowercase();
        components.retain(|c| {
            c.entity_type
                .as_deref()
                .map(|e| e == cat_lower)
                .unwrap_or(false)
        });
    }

    // Future: filter by chip target when per-target component metadata is added.
    let _ = target;

    components.sort_by(|a, b| a.id.cmp(&b.id));
    components
}

/// Get the GPIO pin map for a chip target.
pub fn get_pin_map_native(target: &str) -> Result<Vec<PinInfo>, String> {
    let chip = parse_chip_target(target).ok_or_else(|| {
        format!(
            "unknown target '{target}'; expected one of: esp32, esp32s2, esp32s3, esp32c2, \
             esp32c3, esp32c5, esp32c6, esp32c61, esp32h2, esp32p4"
        )
    })?;
    Ok(chip_pin_info(chip))
}

/// Compute USE_* feature flags for a list of component IDs.
pub fn compute_feature_flags_native(ids_json: &str) -> serde_json::Value {
    let ids: Vec<ComponentId> = match serde_json::from_str(ids_json) {
        Ok(v) => v,
        Err(e) => {
            return serde_json::json!({"error": format!("invalid JSON: {e}")});
        }
    };

    let registry = default_registry();
    let ffs = FeatureFlagSet::from_components(&ids, &registry);

    let mut flags: Vec<&str> = ffs.iter_flags().collect();
    flags.sort();

    serde_json::json!({
        "flags": flags,
        "c_defines": ffs.to_c_defines(),
        "cargo_features": ffs.to_cargo_features(),
    })
}

/// List all registered public profiles.
pub fn list_profiles_native() -> serde_json::Value {
    use rshome_schema::profile::ProfileRegistry;

    let registry = ProfileRegistry::default_profiles();
    let profiles: Vec<_> = registry
        .all_ids()
        .filter_map(|id| registry.get(id))
        .collect();

    serde_json::to_value(profiles)
        .unwrap_or_else(|e| serde_json::json!({"error": format!("serialization error: {e}")}))
}

/// Get the `ComponentDefinition` for a single component ID.
pub fn get_component_schema_native(component_id: &str) -> serde_json::Value {
    let registry = default_registry();
    match registry.get(component_id) {
        Some(def) => serde_json::to_value(def)
            .unwrap_or_else(|e| serde_json::json!({"error": format!("serialization error: {e}")})),
        None => serde_json::json!({"error": format!("component '{component_id}' not found")}),
    }
}

/// Generate the full ESP-IDF project source tree in memory and return it as a
/// JSON object.
///
/// # Input
/// JSON-encoded `RawConfig` object (same shape as [`validate_config_native`]).
///
/// # Output
/// On success:
/// ```json
/// {
///   "valid": true,
///   "device_name": "my_device",
///   "chip_target": "Esp32",
///   "file_count": 47,
///   "files": [
///     {"path": "CMakeLists.txt", "content_b64": "Y21ha2VfbWlu..."},
///     {"path": "main/main.c", "content_b64": "I2luY2x1ZGUg..."}
///   ]
/// }
/// ```
///
/// On validation failure:
/// ```json
/// {"valid": false, "errors": [...]}
/// ```
///
/// File contents are base64-encoded so the JSON envelope can carry arbitrary
/// generated bytes without escape-encoding overhead.
///
/// This function does **no** filesystem I/O. It is intended to run in
/// `wasm32-unknown-unknown` so the rshome browser wizard can offer
/// "Download ESP-IDF source" without contacting an external service.
pub fn codegen_export_native(config_json: &str) -> serde_json::Value {
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};

    let config: RawConfig = match serde_json::from_str(config_json) {
        Ok(c) => c,
        Err(e) => {
            return serde_json::json!({
                "valid": false,
                "errors": [{
                    "path": "",
                    "message": format!("JSON parse error: {e}"),
                    "severity": "error",
                }],
            });
        }
    };

    let registry = default_registry();
    let pipeline = ValidationPipeline::new(registry);
    let store = PackageStore::new();

    let validated = match pipeline.validate(config, &store) {
        ValidationResult::Valid(v) => v,
        ValidationResult::Invalid(errors) => {
            return serde_json::json!({
                "valid": false,
                "errors": errors,
            });
        }
    };

    let device_name = validated.esphome.name.clone();
    let chip_target = format!("{:?}", validated.esphome.chip_target);

    let project =
        match rshome_codegen::generator::ProjectGenerator::new(&validated).generate_in_memory() {
            Ok(p) => p,
            Err(e) => {
                return serde_json::json!({
                    "valid": false,
                    "errors": [{
                        "path": "",
                        "message": format!("codegen error: {e}"),
                        "severity": "error",
                    }],
                });
            }
        };

    let files: Vec<serde_json::Value> = project
        .into_iter()
        .map(|(path, bytes)| {
            serde_json::json!({
                "path": path.to_string_lossy(),
                "content_b64": B64.encode(&bytes),
            })
        })
        .collect();

    serde_json::json!({
        "valid": true,
        "device_name": device_name,
        "chip_target": chip_target,
        "file_count": files.len(),
        "files": files,
    })
}

/// Return a summary of what codegen would produce without writing files.
pub fn codegen_preview_native(config_json: &str) -> serde_json::Value {
    let config: RawConfig = match serde_json::from_str(config_json) {
        Ok(c) => c,
        Err(e) => {
            return serde_json::json!({
                "error": format!("JSON parse error: {e}")
            });
        }
    };

    let registry = default_registry();
    let pipeline = ValidationPipeline::new(registry);
    let store = PackageStore::new();

    match pipeline.validate(config, &store) {
        ValidationResult::Valid(validated) => {
            match rshome_codegen::generator::ProjectGenerator::new(&validated).generate_in_memory()
            {
                Ok(project) => {
                    let files: Vec<String> = project
                        .keys()
                        .map(|path| path.to_string_lossy().into_owned())
                        .collect();
                    serde_json::json!({
                        "valid": true,
                        "device_name": validated.esphome.name,
                        "chip_target": format!("{:?}", validated.esphome.chip_target),
                        "active_flags": validated.active_flags,
                        "component_count": validated.components.len(),
                        "generated_files": files,
                        "file_count": project.len(),
                    })
                }
                Err(error) => serde_json::json!({
                    "valid": false,
                    "errors": [{
                        "path": "",
                        "message": format!("codegen error: {error}"),
                        "severity": "error",
                    }],
                }),
            }
        }
        ValidationResult::Invalid(errors) => {
            serde_json::json!({
                "valid": false,
                "errors": errors,
            })
        }
    }
}

// ── Module & Solution API ─────────────────────────────────────────────────────

/// List available hardware modules, optionally filtered by chip target.
pub fn list_modules_native(target: Option<&str>) -> Vec<ModuleInfo> {
    let registry = default_module_registry();
    let mut modules: Vec<ModuleInfo> = match target {
        Some(t) => {
            let chip = match parse_chip_target(t) {
                Some(c) => c,
                None => return vec![],
            };
            registry
                .for_target(chip)
                .into_iter()
                .map(ModuleInfo::from)
                .collect()
        }
        None => registry.all().map(ModuleInfo::from).collect(),
    };
    modules.sort_by(|a, b| a.id.cmp(&b.id));
    modules
}

/// List available solutions, optionally filtered by module ID.
pub fn list_solutions_native(module_id: Option<&str>) -> Vec<SolutionInfo> {
    let registry = default_solution_registry();
    let mut solutions: Vec<SolutionInfo> = match module_id {
        Some(mid) => registry
            .for_module(mid)
            .into_iter()
            .map(SolutionInfo::from)
            .collect(),
        None => registry.all().map(SolutionInfo::from).collect(),
    };
    solutions.sort_by(|a, b| a.id.cmp(&b.id));
    solutions
}

/// Get the full definition of a specific solution.
pub fn get_solution_detail_native(solution_id: &str) -> serde_json::Value {
    let registry = default_solution_registry();
    match registry.get(solution_id) {
        Some(def) => serde_json::to_value(def)
            .unwrap_or_else(|e| serde_json::json!({"error": format!("serialization error: {e}")})),
        None => serde_json::json!({"error": format!("solution '{solution_id}' not found")}),
    }
}

/// Get the auto-derived assembly for a module.
pub fn get_assembly_for_module_native(module_id: &str) -> serde_json::Value {
    let mod_reg = default_module_registry();
    match mod_reg.get(module_id) {
        Some(module) => {
            let assembly = rshome_schema::assembly::HardwareAssemblyDefinition::from_module(module);
            serde_json::to_value(assembly).unwrap_or_else(
                |e| serde_json::json!({"error": format!("serialization error: {e}")}),
            )
        }
        None => serde_json::json!({"error": format!("module '{module_id}' not found")}),
    }
}

/// Get HA entity export definitions for a solution.
pub fn get_ha_entities_native(solution_id: &str) -> serde_json::Value {
    let sol_reg = default_solution_registry();
    match sol_reg.get(solution_id) {
        Some(def) => serde_json::to_value(&def.runtime_binding.ha_entities)
            .unwrap_or_else(|e| serde_json::json!({"error": format!("serialization error: {e}")})),
        None => serde_json::json!({"error": format!("solution '{solution_id}' not found")}),
    }
}

/// Get network topology for a solution.
pub fn get_network_topology_native(solution_id: &str) -> serde_json::Value {
    let sol_reg = default_solution_registry();
    match sol_reg.get(solution_id) {
        Some(def) => serde_json::to_value(def.network_topology)
            .unwrap_or_else(|e| serde_json::json!({"error": format!("serialization error: {e}")})),
        None => serde_json::json!({"error": format!("solution '{solution_id}' not found")}),
    }
}

/// Get the full platform capability catalog.
pub fn get_platform_catalog_native() -> serde_json::Value {
    let catalog = PlatformCatalog::esp_idf_default();
    serde_json::to_value(catalog)
        .unwrap_or_else(|e| serde_json::json!({"error": format!("serialization error: {e}")}))
}

// ── DAG bindings ─────────────────────────────────────────────────────────────

/// Build the component dependency DAG for a set of component IDs.
///
/// Input: JSON array of component ID strings, e.g. `["wifi", "dht", "sensor"]`.
/// Returns JSON `{ "nodes": [...], "edges": [...], "layers": [[...], ...] }`
/// suitable for visualization libraries (dagre, elkjs, D3).
pub fn get_dependency_dag_native(ids_json: &str) -> serde_json::Value {
    let ids: Vec<ComponentId> = match serde_json::from_str(ids_json) {
        Ok(v) => v,
        Err(e) => return serde_json::json!({"error": format!("invalid JSON: {e}")}),
    };
    let registry = ComponentRegistry::default_registry();
    match registry.build_dag(&ids) {
        Ok(dag) => dag.to_json(),
        Err(e) => serde_json::json!({"error": format!("{e}")}),
    }
}

/// Build the orchestration DAG for a solution.
///
/// Returns JSON `{ "nodes": [...], "edges": [...], "layers": [[...], ...] }`.
pub fn get_orchestration_dag_native(solution_id: &str) -> serde_json::Value {
    let sol_reg = default_solution_registry();
    match sol_reg.get(solution_id) {
        Some(def) => match rshome_schema::OrchestrationDag::from_steps(&def.fixed_orchestration) {
            Ok(dag) => dag.to_json(),
            Err(e) => serde_json::json!({"error": format!("{e}")}),
        },
        None => serde_json::json!({"error": format!("solution '{solution_id}' not found")}),
    }
}

/// Build the build pipeline DAG for a set of component IDs.
///
/// Input: JSON array of component ID strings.
/// Returns JSON `{ "nodes": [...], "edges": [...], "layers": [[...], ...] }`.
pub fn get_build_pipeline_dag_native(ids_json: &str) -> serde_json::Value {
    let ids: Vec<String> = match serde_json::from_str(ids_json) {
        Ok(v) => v,
        Err(e) => return serde_json::json!({"error": format!("invalid JSON: {e}")}),
    };
    let dag = rshome_schema::BuildPipelineDag::from_components(&ids, true);
    dag.to_json()
}

/// Build the signal path DAG for a solution.
///
/// Returns JSON `{ "nodes": [...], "edges": [...] }`.
pub fn get_signal_path_dag_native(solution_id: &str) -> serde_json::Value {
    let sol_reg = default_solution_registry();
    match sol_reg.get(solution_id) {
        Some(def) => {
            // Merge all signal paths from feedback_paths into a single response.
            let dags: Vec<serde_json::Value> = def
                .feedback_paths
                .iter()
                .map(|path| rshome_schema::SignalPathDag::from_signal_path(path).to_json())
                .collect();
            serde_json::json!({
                "solution_id": solution_id,
                "signal_paths": dags,
            })
        }
        None => serde_json::json!({"error": format!("solution '{solution_id}' not found")}),
    }
}

/// Validate a workspace (multiple saved profiles) for cross-profile compatibility.
///
/// Input: JSON-encoded `Workspace` object (`{"profiles": [...]}`).
///
/// Output:
/// - On parse success: JSON array of `WorkspaceError` objects
///   (empty array = workspace is internally consistent).
/// - On parse failure: `{"parse_error": "..."}`.
///
/// Resolves solutions against `default_solution_registry()`; the JSON
/// object does not need to carry the registry inline.
pub fn validate_workspace_native(workspace_json: &str) -> serde_json::Value {
    let ws: crate::workspace::Workspace = match serde_json::from_str(workspace_json) {
        Ok(w) => w,
        Err(e) => {
            return serde_json::json!({
                "parse_error": format!("invalid Workspace JSON: {e}"),
            });
        }
    };
    let reg = default_solution_registry();
    let errors = crate::workspace::validate_workspace(&ws, &reg);
    serde_json::to_value(&errors).unwrap_or_else(|e| {
        serde_json::json!({
            "parse_error": format!("failed to serialize WorkspaceError vec: {e}"),
        })
    })
}

// ── JSON-IR and codegen archive helpers ────────────────────────────────────────

/// Generate a JSON Intermediate Representation (JSON-IR) from a device config.
///
/// Transforms a rshome `RawConfig` into a structured device IR suitable for
/// rendering in the wizard UI and for downstream tool consumption.
///
/// # Input
/// JSON-encoded `RawConfig` object.
///
/// # Output
/// ```json
/// {
///   "device_name": "my_device",
///   "chip_target": "Esp32",
///   "framework": "espidf",
///   "components": [{"id": "wifi", "display_name": "WiFi", "category": "network"}, ...],
///   "pins": [{"gpio": 4, "mode": "output", "component": "led"}, ...],
///   "feature_flags": ["USE_WIFI", "USE_LED", ...],
///   "adapter_kind": "esphome",
///   "ir_version": 1
/// }
/// ```
/// On parse failure: `{"parse_error": "..."}`.
pub fn generate_device_ir_native(device_config_json: &str) -> serde_json::Value {
    let raw: rshome_config::RawConfig = match serde_json::from_str(device_config_json) {
        Ok(r) => r,
        Err(e) => {
            return serde_json::json!({
                "parse_error": format!("invalid RawConfig JSON: {e}"),
            });
        }
    };

    let esphome = &raw.esphome;
    let device_name = &esphome.name;
    let target_str = esphome.platform.as_str();

    // Collect component IDs and derive feature flags
    let comp_ids: Vec<String> = raw
        .components
        .iter()
        .map(|c| c.component_type.clone())
        .collect();
    let flags = compute_feature_flags_native(&serde_json::to_string(&comp_ids).unwrap_or_default());
    let feature_flags = flags
        .get("flags")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    // All ESP32 chips use the esphome adapter kind
    let adapter_kind = "esphome";

    // Build component summaries
    let registry = default_registry();
    let components: Vec<serde_json::Value> = raw
        .components
        .iter()
        .map(|c| {
            let def = registry.get(&c.component_type);
            serde_json::json!({
                "id": c.component_type,
                "display_name": def.map(|d| d.id.as_str()).unwrap_or(&c.component_type),
                "description": def.map(|d| d.description.as_str()).unwrap_or(""),
            })
        })
        .collect();

    // Collect pin assignments from pin_allocations if available
    let pins: Vec<serde_json::Value> = Vec::new();

    serde_json::json!({
        "device_name": device_name,
        "chip_target": target_str,
        "framework": "espidf",
        "components": components,
        "pins": pins,
        "feature_flags": feature_flags,
        "adapter_kind": adapter_kind,
        "ir_version": 1,
    })
}

/// Unpack a `codegen_export` JSON response into a structured file tree.
///
/// Takes the JSON output of [`codegen_export_native`] (flat file array with
/// base64-encoded contents) and reorganizes it into a hierarchical directory
/// tree. Base64 contents are decoded and re-encoded to canonical form.
///
/// # Input
/// JSON response from `codegen_export`, shape:
/// ```json
/// {
///   "valid": true,
///   "device_name": "...",
///   "chip_target": "...",
///   "file_count": N,
///   "files": [{"path": "CMakeLists.txt", "content_b64": "..."}, ...]
/// }
/// ```
///
/// # Output
/// ```json
/// {
///   "device_name": "my_device",
///   "chip_target": "Esp32",
///   "file_count": 47,
///   "tree": {
///     "CMakeLists.txt": {"type": "file", "size": 1024, "content_b64": "..."},
///     "main": {
///       "component.mk": {"type": "file", ...},
///       "main.cpp": {"type": "file", ...}
///     }
///   }
/// }
/// ```
/// On invalid input: `{"unpack_error": "..."}`.
pub fn unpack_codegen_archive_native(codegen_json: &str) -> serde_json::Value {
    let raw: serde_json::Value = match serde_json::from_str(codegen_json) {
        Ok(v) => v,
        Err(e) => {
            return serde_json::json!({
                "unpack_error": format!("invalid JSON: {e}"),
            });
        }
    };

    let valid = raw.get("valid").and_then(|v| v.as_bool()).unwrap_or(false);
    if !valid {
        return serde_json::json!({
            "unpack_error": "codegen export is not valid",
            "errors": raw.get("errors"),
        });
    }

    let device_name = raw
        .get("device_name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let chip_target = raw
        .get("chip_target")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let file_count: u64 = raw.get("file_count").and_then(|v| v.as_u64()).unwrap_or(0);

    let files = match raw.get("files").and_then(|v| v.as_array()) {
        Some(f) => f,
        None => {
            return serde_json::json!({
                "unpack_error": "missing or invalid 'files' array",
            });
        }
    };

    // Decode base64 contents to compute real byte sizes
    let decoded_files: Vec<(String, usize, String)> = files
        .iter()
        .filter_map(|f| {
            let path = f.get("path").and_then(|v| v.as_str())?;
            let b64 = f.get("content_b64").and_then(|v| v.as_str())?;
            use base64::Engine;
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(b64.as_bytes())
                .ok()?;
            let size = decoded.len();
            // Re-encode to canonical base64 (no line wrapping)
            let canonical_b64 = base64::engine::general_purpose::STANDARD.encode(&decoded);
            Some((path.to_string(), size, canonical_b64))
        })
        .collect();

    // Build a nested tree from flat file paths
    let mut tree = serde_json::Map::new();
    for (path, size, content_b64) in &decoded_files {
        let parts: Vec<&str> = path.split('/').collect();
        insert_into_tree(&mut tree, &parts, *size, content_b64);
    }

    serde_json::json!({
        "device_name": device_name,
        "chip_target": chip_target,
        "file_count": file_count,
        "tree": serde_json::Value::Object(tree),
    })
}

/// Insert a file at `path_parts` into the tree map, creating intermediate
/// directory nodes as needed.
fn insert_into_tree(
    tree: &mut serde_json::Map<String, serde_json::Value>,
    path_parts: &[&str],
    size: usize,
    content_b64: &str,
) {
    if path_parts.is_empty() {
        return;
    }

    let name = path_parts[0].to_string();

    if path_parts.len() == 1 {
        // Leaf: file node
        let mut file_node = serde_json::Map::new();
        file_node.insert(
            "type".to_string(),
            serde_json::Value::String("file".to_string()),
        );
        file_node.insert("size".to_string(), serde_json::Value::Number(size.into()));
        file_node.insert(
            "content_b64".to_string(),
            serde_json::Value::String(content_b64.to_string()),
        );
        tree.insert(name, serde_json::Value::Object(file_node));
    } else {
        // Directory: recurse
        let dir = tree.entry(name.clone()).or_insert_with(|| {
            let mut dir_node = serde_json::Map::new();
            dir_node.insert(
                "type".to_string(),
                serde_json::Value::String("dir".to_string()),
            );
            dir_node.insert(
                "children".to_string(),
                serde_json::Value::Object(serde_json::Map::new()),
            );
            serde_json::Value::Object(dir_node)
        });

        if let Some(children) = dir
            .as_object_mut()
            .and_then(|d| d.get_mut("children"))
            .and_then(|c| c.as_object_mut())
        {
            insert_into_tree(children, &path_parts[1..], size, content_b64);
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const EXPECTED_GENERATED_FILES: &[&str] = &[
        "CMakeLists.txt",
        "LICENSE",
        "components/app/CMakeLists.txt",
        "components/app/include/app_composition.h",
        "components/app/src/app_composition.cpp",
        "main/CMakeLists.txt",
        "main/idf_component.yml",
        "main/main.cpp",
        "partitions.csv",
        "sdkconfig.defaults",
    ];

    const VALID_CONFIG: &str = r#"{
        "esphome": {
            "name": "test_device",
            "platform": "esp32",
            "board": "esp32dev"
        },
        "components": [
            {"component_type": "wifi", "config": {"ssid": "MyNet", "password": "pass"}}
        ]
    }"#;

    #[test]
    fn validate_config_valid() {
        let result = validate_config_native(VALID_CONFIG);
        assert!(
            result["valid"].as_bool().unwrap_or(false),
            "expected valid=true, got: {result}"
        );
    }

    #[test]
    fn validate_config_invalid_json() {
        let result = validate_config_native("not json");
        assert!(!result["valid"].as_bool().unwrap_or(true));
    }

    #[test]
    fn validate_partial_returns_array() {
        let partial = r#"{"esphome": {"name": "x", "platform": "esp32", "board": "esp32dev"}}"#;
        let result = validate_partial_native(partial);
        assert!(result.is_array(), "expected array of errors");
    }

    #[test]
    fn list_components_returns_sorted_list() {
        let comps = list_components_native(None, None);
        assert!(!comps.is_empty());
        let ids: Vec<&str> = comps.iter().map(|c| c.id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
        assert!(comps.iter().all(|c| !c.description.trim().is_empty()));
    }

    #[test]
    fn list_components_filter_by_category() {
        let sensors = list_components_native(None, Some("sensor"));
        assert!(!sensors.is_empty());
        for c in &sensors {
            assert_eq!(c.entity_type.as_deref(), Some("sensor"));
        }
    }

    #[test]
    fn list_components_preserves_snake_case_entity_types() {
        let binary = list_components_native(None, Some("binary_sensor"));
        assert!(!binary.is_empty());
        for c in &binary {
            assert_eq!(c.entity_type.as_deref(), Some("binary_sensor"));
        }
    }

    #[test]
    fn list_components_includes_bh1750() {
        let comps = list_components_native(None, Some("sensor"));
        let bh1750 = comps.iter().find(|c| c.id == "bh1750").expect("bh1750");
        assert!(bh1750.auto_load.iter().any(|dep| dep == "sensor"));
        assert!(bh1750.auto_load.iter().any(|dep| dep == "i2c"));
        assert!(bh1750.description.contains("lux"));
    }

    #[test]
    fn get_pin_map_esp32_returns_40_pins() {
        let pins = get_pin_map_native("esp32").unwrap();
        assert_eq!(pins.len(), 40);
    }

    #[test]
    fn get_pin_map_invalid_target_returns_error() {
        assert!(get_pin_map_native("stm32").is_err());
    }

    #[test]
    fn compute_feature_flags_wifi() {
        let result = compute_feature_flags_native(r#"["wifi"]"#);
        let flags = result["flags"].as_array().unwrap();
        assert!(flags.iter().any(|f| f.as_str() == Some("USE_WIFI")));
    }

    #[test]
    fn get_component_schema_wifi() {
        let result = get_component_schema_native("wifi");
        assert_eq!(result["id"].as_str(), Some("wifi"));
        assert!(result["description"].as_str().is_some());
    }

    #[test]
    fn get_component_schema_nonexistent() {
        let result = get_component_schema_native("does_not_exist_xyz");
        assert!(result["error"].is_string());
    }

    #[test]
    fn codegen_preview_valid_config() {
        let result = codegen_preview_native(VALID_CONFIG);
        assert!(result["valid"].as_bool().unwrap_or(false));
        let files: Vec<_> = result["generated_files"]
            .as_array()
            .expect("preview contains generated files")
            .iter()
            .map(|file| file.as_str().expect("generated file path"))
            .collect();
        assert_eq!(files, EXPECTED_GENERATED_FILES);
        assert_eq!(result["file_count"].as_u64(), Some(files.len() as u64));
    }

    #[test]
    fn codegen_export_valid_config() {
        let result = codegen_export_native(VALID_CONFIG);
        assert!(result["valid"].as_bool().unwrap_or(false));
        let files: Vec<_> = result["files"]
            .as_array()
            .expect("export contains generated files")
            .iter()
            .map(|file| file["path"].as_str().expect("generated file path"))
            .collect();
        assert_eq!(files, EXPECTED_GENERATED_FILES);
        assert_eq!(result["file_count"].as_u64(), Some(files.len() as u64));
    }

    // ── Module & Solution tests ─────────────────────────────────────────────

    #[test]
    fn list_modules_returns_all() {
        let modules = list_modules_native(None);
        assert!(
            modules.len() >= 4,
            "expected at least 4 modules, got {}",
            modules.len()
        );
        // Verify sorted by id
        for w in modules.windows(2) {
            assert!(w[0].id <= w[1].id);
        }
    }

    #[test]
    fn list_modules_filter_by_target() {
        let modules = list_modules_native(Some("esp32s3"));
        assert!(!modules.is_empty());
        for m in &modules {
            assert_eq!(m.target, "esp32_s3");
        }
    }

    #[test]
    fn list_modules_unknown_target_returns_empty() {
        let modules = list_modules_native(Some("unknown_chip"));
        assert!(modules.is_empty());
    }

    #[test]
    fn list_solutions_returns_all() {
        let solutions = list_solutions_native(None);
        assert!(
            solutions.len() >= 4,
            "expected at least 4 solutions, got {}",
            solutions.len()
        );
        for w in solutions.windows(2) {
            assert!(w[0].id <= w[1].id);
        }
    }

    #[test]
    fn list_solutions_filter_by_module() {
        let solutions = list_solutions_native(Some("esp32s3_wroom1"));
        assert!(!solutions.is_empty());
        for s in &solutions {
            assert!(s.supported_modules.contains(&"esp32s3_wroom1".to_string()));
        }
    }

    #[test]
    fn get_solution_detail_found() {
        let result = get_solution_detail_native("camera_stream");
        assert!(result["id"].as_str() == Some("camera_stream"));
        assert!(result["component_bundle"]["required"].is_array());
    }

    #[test]
    fn get_solution_detail_not_found() {
        let result = get_solution_detail_native("nonexistent_solution");
        assert!(result["error"].is_string());
    }

    #[test]
    fn get_platform_catalog_has_targets() {
        let result = get_platform_catalog_native();
        assert!(result["platforms"].is_array());
        let platforms = result["platforms"].as_array().unwrap();
        assert!(!platforms.is_empty());
    }

    // ── DAG binding tests ───────────────────────────────────────────────────

    #[test]
    fn get_dependency_dag_valid_input() {
        let result = get_dependency_dag_native(r#"["wifi", "dht"]"#);
        assert!(result["nodes"].is_array());
        assert!(result["edges"].is_array());
        assert!(result["layers"].is_array());
        assert!(result["error"].is_null());
    }

    #[test]
    fn get_dependency_dag_invalid_json() {
        let result = get_dependency_dag_native("not json");
        assert!(result["error"].is_string());
    }

    #[test]
    fn get_orchestration_dag_valid_solution() {
        let result = get_orchestration_dag_native("camera_stream");
        assert!(result["nodes"].is_array());
        assert!(result["edges"].is_array());
        assert!(result["layers"].is_array());
    }

    #[test]
    fn get_orchestration_dag_not_found() {
        let result = get_orchestration_dag_native("nonexistent");
        assert!(result["error"].is_string());
    }

    #[test]
    fn get_build_pipeline_dag_valid() {
        let result = get_build_pipeline_dag_native(r#"["wifi", "sensor"]"#);
        assert!(result["nodes"].is_array());
        assert!(result["layers"].is_array());
    }

    #[test]
    fn get_signal_path_dag_valid() {
        let result = get_signal_path_dag_native("camera_stream");
        assert_eq!(result["solution_id"], "camera_stream");
        assert!(result["signal_paths"].is_array());
    }

    // ── Workspace validator (workspace-ux PRD Phase 0 T0.3) ────────────────

    #[test]
    fn validate_workspace_empty_returns_empty_array() {
        let result = validate_workspace_native(r#"{"profiles":[]}"#);
        assert_eq!(result, serde_json::json!([]));
    }

    #[test]
    fn validate_workspace_parse_error_surfaces_friendly_message() {
        let result = validate_workspace_native("{not_json");
        assert!(
            result.get("parse_error").is_some(),
            "expected parse_error object, got {result:?}"
        );
    }

    #[test]
    fn validate_workspace_unknown_solution_serializes_cleanly() {
        // This test locks the WorkspaceError serde shape: if a future edit
        // renames a field (e.g. `kind` → `error_kind`), this test fires
        // before any TS consumer breaks on the wire format.
        let payload = serde_json::json!({
            "profiles": [{
                "label": "bad",
                "chip_target": "esp32_s3",
                "selected_solution_id": "nonexistent_solution",
                "parameter_values": {}
            }]
        });
        let result = validate_workspace_native(&payload.to_string());
        let arr = result.as_array().expect("expected top-level array");
        assert_eq!(arr.len(), 1, "expected one WorkspaceError, got {arr:?}");
        let err = &arr[0];
        assert_eq!(err["kind"], "UnknownSolution");
        assert!(err["message"]
            .as_str()
            .unwrap()
            .contains("nonexistent_solution"));
        let profiles = err["profiles"].as_array().expect("profiles array");
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0], "bad");
    }

    #[test]
    fn validate_workspace_mismatched_uplink_surfaces_pair_error() {
        // elrs_tx speaks crsf; direct_control speaks wifi_crtp.
        let payload = serde_json::json!({
            "profiles": [
                {
                    "label": "tx",
                    "chip_target": "esp32_s3",
                    "selected_solution_id": "elrs_tx_solution",
                    "parameter_values": {}
                },
                {
                    "label": "rx",
                    "chip_target": "esp32_s3",
                    "selected_solution_id": "direct_control_solution",
                    "parameter_values": {}
                }
            ]
        });
        let result = validate_workspace_native(&payload.to_string());
        let arr = result.as_array().expect("expected top-level array");
        let has_unmatched = arr.iter().any(|e| {
            matches!(
                e["kind"].as_str(),
                Some("UnmatchedTxUplink") | Some("UnmatchedRxUplink")
            )
        });
        assert!(
            has_unmatched,
            "expected Unmatched*Uplink error for crsf/wifi_crtp mismatch. Got: {arr:?}"
        );
    }
}
