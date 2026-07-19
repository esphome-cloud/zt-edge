//! Registry export — build the `registry-data.json` shape as a `serde_json::Value`.
//!
//! Extracted from `bin/export-registry.rs` so the bench harness (RG-2 B5)
//! can measure the same code path the binary runs. Single source of truth:
//! the binary is now a thin `println!` wrapper over [`build_registry_export`].

use rshome_schema::platform::{default_form_factor_meta, default_topology_meta};
use rshome_schema::registry::ComponentRegistry;
use rshome_schema::ChipTarget;

use crate::bindings::{
    get_platform_catalog_native, list_components_native, list_modules_native, list_profiles_native,
    list_solutions_native,
};
use crate::pin_map::chip_pin_info;

/// Build the `registry-data.json` payload as a `serde_json::Value`.
///
/// Mirrors `bin/export-registry`'s `main()` exactly, minus the trailing
/// `println!`. The binary calls this and then pretty-prints. The bench
/// calls this and measures wall-clock.
///
/// Output shape (top-level keys):
/// - `components` — list_components_native(None, None)
/// - `pin_maps` — per-`ChipTarget` pin info, keyed by serde snake_case name
/// - `profiles` — list_profiles_native()
/// - `modules` — list_modules_native(None)
/// - `solutions` — list_solutions_native(None)
/// - `platform_catalog` — get_platform_catalog_native()
/// - `full_dependency_dag` — component DAG over all registered components
/// - `solution_orchestration_dags` — per-solution orchestration DAG keyed by solution id
/// - `topology_metadata` — `default_topology_meta()` (3 entries, ADR-022 Phase 1)
/// - `form_factor_metadata` — `default_form_factor_meta()` (64 entries, ADR-022 Phase 1)
pub fn build_registry_export() -> serde_json::Value {
    let components = list_components_native(None, None);
    let profiles = list_profiles_native();
    let modules = list_modules_native(None);
    let solutions = list_solutions_native(None);
    let platform_catalog = get_platform_catalog_native();

    let mut pin_maps = serde_json::Map::new();
    for target in ChipTarget::all() {
        let key = serde_json::to_value(target)
            .expect("ChipTarget serializes to JSON")
            .as_str()
            .expect("ChipTarget serializes as string")
            .to_string();
        pin_maps.insert(
            key,
            serde_json::to_value(chip_pin_info(*target)).expect("PinInfo serializes"),
        );
    }

    let registry = ComponentRegistry::default_registry();
    let all_ids: Vec<String> = registry.all_ids().map(|s| s.to_owned()).collect();
    let full_dependency_dag = match registry.build_dag(&all_ids) {
        Ok(dag) => dag.to_json(),
        Err(e) => serde_json::json!({ "error": format!("{e}") }),
    };

    let sol_reg = rshome_schema::solution::default_solution_registry();
    let mut solution_dags = serde_json::Map::new();
    for def in sol_reg.all() {
        if let Ok(dag) = rshome_schema::OrchestrationDag::from_steps(&def.fixed_orchestration) {
            solution_dags.insert(def.id.clone(), dag.to_json());
        }
    }

    let topology_metadata = default_topology_meta();
    let form_factor_metadata = default_form_factor_meta();

    serde_json::json!({
        "components": components,
        "pin_maps": pin_maps,
        "profiles": profiles,
        "modules": modules,
        "solutions": solutions,
        "platform_catalog": platform_catalog,
        "full_dependency_dag": full_dependency_dag,
        "solution_orchestration_dags": solution_dags,
        "topology_metadata": topology_metadata,
        "form_factor_metadata": form_factor_metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_registry_export_has_all_top_level_keys() {
        let v = build_registry_export();
        let obj = v.as_object().expect("export is a JSON object");
        for key in [
            "components",
            "pin_maps",
            "profiles",
            "modules",
            "solutions",
            "platform_catalog",
            "full_dependency_dag",
            "solution_orchestration_dags",
            "topology_metadata",
            "form_factor_metadata",
        ] {
            assert!(obj.contains_key(key), "missing top-level key: {key}");
        }
    }

    #[test]
    fn build_registry_export_metadata_arrays_have_expected_cardinality() {
        let v = build_registry_export();
        // Phase 1 ADR-022 + RG-2 B2 invariant: 3 topology + 64 form-factor entries.
        assert_eq!(v["topology_metadata"].as_array().unwrap().len(), 3);
        assert_eq!(v["form_factor_metadata"].as_array().unwrap().len(), 64);
    }
}
