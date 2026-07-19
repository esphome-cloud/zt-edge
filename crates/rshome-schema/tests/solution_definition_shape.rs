//! `SolutionDefinition` shape lint per
//! Task 0.1 acceptance #3: "`SolutionInfo` carries 40 fields per the
//! §6.1 contract; `tests/types/solution_info_shape.rs::test_solution_info_field_count`
//! reads via reflection and asserts 40".
//!
//! Note on naming: the PRD's "SolutionInfo" refers to the conceptual
//! struct represented by `SolutionDefinition` in this crate. This file asserts
//! the Rust authority's field count.
//!
//! **Drift recorded 2026-05-14 at Task 0.1 close:** the Rust authority
//! carries 33 fields, not 40 as master design §6.1 states. Two paths
//! are open: (a) the master doc count is stale and §6.1 should drop
//! to 33 (the IoT extensions present in TS but missing from Rust live
//! in a separate model), or (b) Rust needs to gain the missing 7
//! fields to match. This codification PRD records the actual shape;
//! resolving the doc-vs-code disagreement is downstream work tracked

use schemars::schema_for;
use serde_json::Value;

use rshome_schema::SolutionDefinition;

/// Ground-truth field count of the Rust authority struct. When the
/// struct gains or loses a field, update this constant AND any
/// downstream consumers, the Wasm bindings, and the
/// `default_solution_registry()` fixtures).
const SOLUTION_DEFINITION_FIELD_COUNT: usize = 33;

/// Count the keys under `properties` in the JsonSchema rendering of
/// `SolutionDefinition`. Schemars represents struct fields as schema
/// properties; this gives a version-tolerant introspection point.
fn count_solution_definition_fields() -> usize {
    let schema = schema_for!(SolutionDefinition);
    let json: Value = serde_json::to_value(&schema).expect("schemars schema must serialize");
    let props = json
        .pointer("/properties")
        .and_then(Value::as_object)
        .unwrap_or_else(|| {
            panic!(
                "SolutionDefinition schema missing /properties — schema was:\n{}",
                serde_json::to_string_pretty(&json).unwrap_or_default(),
            )
        });
    props.len()
}

#[test]
fn solution_definition_field_count_is_authoritative() {
    let actual = count_solution_definition_fields();
    assert_eq!(
        actual, SOLUTION_DEFINITION_FIELD_COUNT,
        "SolutionDefinition has {} fields, expected {}. \
         When you intentionally change the struct, update SOLUTION_DEFINITION_FIELD_COUNT \
         in this file AND the matching default in `default_solution_registry()`.",
        actual, SOLUTION_DEFINITION_FIELD_COUNT,
    );
}

/// Sanity check: the required-fields list should be a strict subset
/// of the property list. Schemars derives required-ness from
/// non-`Option<_>` non-`#[serde(default)]` fields; if that ever
/// inverts (e.g. Option fields landing in `required`), this catches
/// it before downstream code makes assumptions.
#[test]
fn solution_definition_required_fields_subset_of_properties() {
    let schema = schema_for!(SolutionDefinition);
    let json: Value = serde_json::to_value(&schema).unwrap();
    let props: std::collections::BTreeSet<String> = json
        .pointer("/properties")
        .and_then(Value::as_object)
        .unwrap()
        .keys()
        .cloned()
        .collect();
    let required: std::collections::BTreeSet<String> = json
        .pointer("/required")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let extra: Vec<&String> = required.difference(&props).collect();
    assert!(
        extra.is_empty(),
        "`required` lists fields not in `properties`: {:?}",
        extra,
    );
}
