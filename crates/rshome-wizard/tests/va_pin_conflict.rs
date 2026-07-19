//! Pin-conflict lint per
//! Task 0.6 acceptance #1 + #2 + #5, expanding va-residuals Phase 5 T5.3.
//!
//! Three layers of coverage:
//! 1. Real-registry smoke tests (existing).
//! 2. **Proptest 10K cases** of synthetic pin-collision scenarios —
//!    replaces the PRD's "50-fixture conflict corpus + 50 clean
//!    corpus" with 10K randomly-generated 2-solution pairs. Each case
//!    asserts the boundary predicate: same GPIO + different function
//!    is the only configuration that produces exactly one conflict;
//!    all other configurations produce zero.
//! 3. **Shared-bus invariant** — 100 programmatically-generated
//!    profiles all assigning the same GPIO to the same function name
//!    (e.g., `i2c_sda`) on a single chip produce zero conflicts. PRD
//!    acceptance #5.

use proptest::prelude::*;

use rshome_schema::platform::{ChipFamilyKind, DomainKind, PinAssignment};
use rshome_schema::solution::{
    default_solution_registry, ComponentBundle, NetworkTopology, RuntimeBinding, SchedulingPolicy,
    SolutionDefinition, SolutionKind,
};
use rshome_wizard::pin_conflict::detect_conflicts;

/// Every pair of V&A solutions that both declare `pin_assignments` and are
/// both chip_coverage-compatible with S3 must be reported cleanly by the
/// validator — no panics, stable output, deterministic ordering.
#[test]
fn registry_pairs_are_analyzable_without_panic() {
    let reg = default_solution_registry();

    let va_with_pins: Vec<_> = reg
        .all()
        .filter(|s| s.domain == Some(DomainKind::VehicleAircraftControl))
        .filter(|s| s.pin_assignments.is_some())
        .collect();

    assert!(
        !va_with_pins.is_empty(),
        "expected at least one V&A solution with pin_assignments"
    );

    for a in &va_with_pins {
        for b in &va_with_pins {
            if a.id >= b.id {
                continue;
            }
            // Just assert no panic + stable ordering.
            let conflicts = detect_conflicts(&[a, b], ChipFamilyKind::Esp32S3);
            for c in &conflicts {
                assert!(
                    c.solutions.0 <= c.solutions.1,
                    "PinConflict.solutions must be alphabetically ordered: {:?}",
                    c
                );
                assert_eq!(c.chip, ChipFamilyKind::Esp32S3);
            }
        }
    }
}

/// A control_board solution and its receiver-direct-drive counterpart share
/// the same `vehicle_control_pins()` helper. In the co-resident scenario,
/// any collision would be same-function (shared wiring) — so zero real
/// conflicts are expected.
#[test]
fn vehicle_control_helpers_same_gpio_same_function_tolerated() {
    let reg = default_solution_registry();
    let quad = reg.get("quad_stabilizer_solution").expect("quad");
    let fixedwing = reg.get("fixedwing_stabilizer_solution").expect("fixedwing");

    let conflicts = detect_conflicts(&[quad, fixedwing], ChipFamilyKind::Esp32S3);
    assert!(
        conflicts.is_empty(),
        "quad + fixedwing use `vehicle_control_pins()` — same GPIOs map to \
         same functions, which is tolerated as shared wiring, not a conflict. \
         Got: {:?}",
        conflicts
    );
}

/// Solutions marked `chip_coverage = insufficient` for a chip are skipped
/// when analyzing that chip — they're not expected to run there.
#[test]
fn insufficient_chip_skipped() {
    let reg = default_solution_registry();
    let quad = reg.get("quad_stabilizer_solution").expect("quad");
    // quad has esp32_c6 = insufficient. Ask for C6 conflicts; quad gets skipped.
    let fixedwing = reg.get("fixedwing_stabilizer_solution").expect("fixedwing");
    let conflicts = detect_conflicts(&[quad, fixedwing], ChipFamilyKind::Esp32C6);
    assert!(
        conflicts.is_empty(),
        "both quad and fixedwing are c6-insufficient; expected empty conflicts"
    );
}

// ── Synthetic-solution helper for proptest + shared-bus tests ──────────────

fn synth_solution(id: &str, pins: Vec<(u8, &str)>) -> SolutionDefinition {
    SolutionDefinition {
        id: id.into(),
        label: id.into(),
        label_zh: None,
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec![],
        fixed_inputs: vec![],
        fixed_outputs: vec![],
        fixed_orchestration: vec![],
        scheduling: SchedulingPolicy {
            id: "synth".into(),
            label: "synth".into(),
            decisions: vec![],
        },
        user_parameters: vec![],
        feedback_paths: vec![],
        variants: vec![],
        component_bundle: ComponentBundle::default(),
        runtime_binding: RuntimeBinding::default(),
        external_contracts: vec![],
        network_topology: NetworkTopology::default(),
        domain: None,
        architecture_tier: None,
        communication_chains: None,
        pin_assignments: Some(
            pins.into_iter()
                .map(|(gpio, func)| PinAssignment {
                    function: func.into(),
                    default_gpio: gpio,
                    alternatives: vec![],
                    capability: "synth".into(),
                })
                .collect(),
        ),
        family: None,
        form_factor_families: None,
        control_uplink: None,
        video_downlink: None,
        telemetry: None,
        sensor_tier_min: None,
        actuator_family: None,
        power_rails: None,
        failsafe: None,
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: None,
    }
}

// ── Shared-bus invariant (PRD acceptance #5) ───────────────────────────────

#[test]
fn shared_bus_100_profiles_zero_false_positives() {
    // Generate 100 synthetic solutions all assigning GPIO 21 → "i2c_sda"
    // (canonical shared-bus scenario). detect_conflicts must produce
    // zero PinConflict entries.
    let solutions: Vec<SolutionDefinition> = (0..100)
        .map(|i| synth_solution(&format!("synth_{:03}", i), vec![(21, "i2c_sda")]))
        .collect();
    let refs: Vec<&SolutionDefinition> = solutions.iter().collect();
    let conflicts = detect_conflicts(&refs, ChipFamilyKind::Esp32S3);
    assert!(
        conflicts.is_empty(),
        "100-profile shared bus produced {} false-positive conflicts; first 3: {:?}",
        conflicts.len(),
        conflicts.iter().take(3).collect::<Vec<_>>(),
    );
}

// ── Proptest 10K cases (PRD acceptances #1 + #2) ───────────────────────────

fn arb_gpio() -> impl Strategy<Value = u8> {
    // ESP32-S3 has GPIO 0..48; we'll generate the same range. Constraining
    // catches off-by-one indexing bugs while keeping the input space small
    // enough that random clashes are common (good for the proptest).
    0u8..=48
}

fn arb_function() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("motor_a_pwm".to_string()),
        Just("motor_b_pwm".to_string()),
        Just("i2c_sda".to_string()),
        Just("i2c_scl".to_string()),
        Just("uart_tx".to_string()),
        Just("uart_rx".to_string()),
        Just("status_led".to_string()),
        Just("button".to_string()),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    /// For a 2-solution pair with single pin each, the conflict predicate
    /// is exactly: same GPIO ∧ different function. Any other input
    /// produces zero conflicts.
    #[test]
    fn two_solution_pin_predicate_holds(
        gpio_a in arb_gpio(),
        gpio_b in arb_gpio(),
        func_a in arb_function(),
        func_b in arb_function(),
    ) {
        let a = synth_solution("alpha", vec![(gpio_a, func_a.as_str())]);
        let b = synth_solution("beta", vec![(gpio_b, func_b.as_str())]);
        let conflicts = detect_conflicts(&[&a, &b], ChipFamilyKind::Esp32S3);

        let expected_conflict = gpio_a == gpio_b && func_a != func_b;
        if expected_conflict {
            prop_assert_eq!(conflicts.len(), 1, "expected exactly 1 conflict for (gpio_a={}, gpio_b={}, func_a={}, func_b={})", gpio_a, gpio_b, func_a, func_b);
            prop_assert_eq!(conflicts[0].gpio, gpio_a);
        } else {
            prop_assert!(
                conflicts.is_empty(),
                "expected zero conflicts for (gpio_a={}, gpio_b={}, func_a={}, func_b={}) but got {:?}",
                gpio_a, gpio_b, func_a, func_b, conflicts,
            );
        }
    }

    /// Output ordering is stable regardless of input order — swapping
    /// alpha and beta produces the same alphabetically-ordered conflict.
    /// Same-function cases are vacuously passed (no conflict either way)
    /// to keep the proptest reject budget unconsumed.
    #[test]
    fn conflict_ordering_is_stable_under_input_swap(
        gpio in arb_gpio(),
        func_a in arb_function(),
        func_b in arb_function(),
    ) {
        let a = synth_solution("alpha", vec![(gpio, func_a.as_str())]);
        let b = synth_solution("beta", vec![(gpio, func_b.as_str())]);
        let conflicts_ab = detect_conflicts(&[&a, &b], ChipFamilyKind::Esp32S3);
        let conflicts_ba = detect_conflicts(&[&b, &a], ChipFamilyKind::Esp32S3);
        prop_assert_eq!(&conflicts_ab, &conflicts_ba);
    }
}
