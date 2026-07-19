//! Topology-category presence lint per va-residuals Phase 2 T2.2 (ADR-01).
//!
//! Every V&A solution must declare non-null `topology_category`. This is
//! auto-populated by `SolutionRegistry::populate_topology_category()` during
//! `default_solution_registry()` construction; this lint guards against a
//! solution that slips past the auto-populate (e.g., chain values the
//! inference table doesn't recognize).
//!
//! Companion wizard-side filter: `isSolutionConsistent` in
//! `type-driven-ui/src/components/rshome/wizard/solution-filter.ts` narrows
//! the V&A catalog by `topology_category === selectedTopology` whenever
//! both are set.

use rshome_schema::platform::DomainKind;
use rshome_schema::solution::default_solution_registry;

#[test]
fn every_va_solution_declares_topology_category() {
    let reg = default_solution_registry();
    let mut missing: Vec<String> = Vec::new();

    for sol in reg.all() {
        if sol.domain != Some(DomainKind::VehicleAircraftControl) {
            continue;
        }
        if sol.topology_category.is_none() {
            missing.push(sol.id.clone());
        }
    }

    assert!(
        missing.is_empty(),
        "V&A solutions missing `topology_category` (chain inference failed — add \
         an exception in `populate_topology_category` or set explicitly):\n  {}",
        missing.join("\n  "),
    );
}

#[test]
fn mcu_sbc_bridge_override_honored() {
    // Per ADR-01: `mcu_sbc_bridge_solution` has `control_uplink = crsf` which
    // would normally infer StandardFpv, but it's an SBC-resident solution and
    // is pinned to ResearchHybrid via an explicit exception in
    // `populate_topology_category`.
    use rshome_schema::platform::TopologyKind;

    let reg = default_solution_registry();
    let sol = reg
        .get("mcu_sbc_bridge_solution")
        .expect("mcu_sbc_bridge_solution must be registered");
    assert_eq!(
        sol.topology_category,
        Some(TopologyKind::ResearchHybrid),
        "mcu_sbc_bridge_solution should be ResearchHybrid despite CRSF uplink",
    );
}
