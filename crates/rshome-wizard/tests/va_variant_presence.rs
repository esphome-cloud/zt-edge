//! Variant-presence lint per rshome-codegen-variants PRD Phase 5 T5.1 / F6.1.
//!
//! Iterates the registry; for every solution declaring a non-empty
//! `variants[]`, asserts:
//!
//! 1. **Distinct ids** — two variants on the same solution cannot share
//!    an `id`. The wizard's `selectedVariantId` + the pipeline's
//!    `find(|v| v.id == variant_id)` both assume uniqueness.
//! 2. **Non-empty labels** — the wizard renders variant pickers with
//!    `label` as the display string; blank label = broken UI.
//! 3. **`required_caps` ⊆ union of supported modules' `hardware_caps`** —
//!    a variant cannot demand a capability that none of the solution's
//!    `supported_modules[]` claim to provide. Otherwise the backend's
//!    Stage 3.5 variant-cap check (deferred to Phase 2b) would
//!    eventually reject every module.
//! 4. **`active_flag_add` and `active_flag_remove` disjoint** — a
//!    variant that both adds and removes the same flag is ambiguous;
//!    the pipeline applies removals last (ending without the flag)
//!    which is almost never what the author intended.
//!
//! Current consumers covered: `composite_device_firmware` (shipped
//! pre-PRD) + `quad_stabilizer_solution` (Phase 4 collapse). Any future
//! solution that gains variants is automatically subject to these
//! invariants.

use std::collections::HashSet;

use rshome_schema::module::default_module_registry;
use rshome_schema::platform::Capability;
use rshome_schema::solution::default_solution_registry;

#[test]
fn every_variant_has_unique_id_within_its_solution() {
    let reg = default_solution_registry();
    let mut offenders: Vec<String> = Vec::new();

    for sol in reg.all() {
        if sol.variants.is_empty() {
            continue;
        }
        let mut seen: HashSet<&str> = HashSet::new();
        for v in &sol.variants {
            if !seen.insert(v.id.as_str()) {
                offenders.push(format!("{}: variant id '{}' duplicated", sol.id, v.id));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "variant-id uniqueness violations:\n  {}",
        offenders.join("\n  ")
    );
}

#[test]
fn every_variant_has_a_non_empty_label() {
    let reg = default_solution_registry();
    let mut offenders: Vec<String> = Vec::new();

    for sol in reg.all() {
        for v in &sol.variants {
            if v.label.trim().is_empty() {
                offenders.push(format!("{}:{}", sol.id, v.id));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "variants with empty labels:\n  {}",
        offenders.join("\n  ")
    );
}

#[test]
fn variant_required_caps_are_satisfiable_by_some_supported_module() {
    let sol_reg = default_solution_registry();
    let mod_reg = default_module_registry();
    let mut offenders: Vec<String> = Vec::new();

    for sol in sol_reg.all() {
        if sol.variants.is_empty() {
            continue;
        }
        // Union of capabilities across the solution's supported modules.
        let mut union: HashSet<Capability> = HashSet::new();
        for mid in &sol.supported_modules {
            if let Some(m) = mod_reg.get(mid) {
                union.extend(m.hardware_caps.iter().copied());
            }
        }
        for v in &sol.variants {
            for cap in &v.required_caps {
                if !union.contains(cap) {
                    offenders.push(format!(
                        "{}:{} requires cap {:?} which none of its \
                         supported_modules provide",
                        sol.id, v.id, cap,
                    ));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "variant required_cap violations:\n  {}",
        offenders.join("\n  ")
    );
}

#[test]
fn variant_active_flag_add_and_remove_are_disjoint() {
    let reg = default_solution_registry();
    let mut offenders: Vec<String> = Vec::new();

    for sol in reg.all() {
        for v in &sol.variants {
            let adds: HashSet<&str> = v.active_flag_add.iter().map(|s| s.as_str()).collect();
            for rem in &v.active_flag_remove {
                if adds.contains(rem.as_str()) {
                    offenders.push(format!(
                        "{}:{} has {} in both active_flag_add and \
                         active_flag_remove",
                        sol.id, v.id, rem,
                    ));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "ambiguous active-flag deltas:\n  {}",
        offenders.join("\n  ")
    );
}

#[test]
fn expected_variant_carriers_exist() {
    // Anchor the lint to the two solutions that SHOULD carry variants
    // today. If a future edit accidentally drops `variants[]` on either,
    // this test fires — preserving the schema invariant that Phase 4's
    // collapse relies on.
    let reg = default_solution_registry();

    let quad = reg
        .get("quad_stabilizer_solution")
        .expect("quad_stabilizer_solution must be registered");
    assert!(
        !quad.variants.is_empty(),
        "quad_stabilizer_solution must retain its Phase-4 variants[]",
    );

    let composite = reg
        .get("composite_device_firmware")
        .expect("composite_device_firmware must be registered");
    assert!(
        !composite.variants.is_empty(),
        "composite_device_firmware must retain its profile_a + profile_b variants",
    );
}
