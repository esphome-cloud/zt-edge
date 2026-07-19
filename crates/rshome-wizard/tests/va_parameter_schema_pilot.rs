//! Parameter-schema pilot test per va-residuals Phase 6 T6.4 / F7.1.
//!
//! The pilot solution `elrs_crsf_brushless_solution` carries the canonical
//! set of wizard-configurable parameters for the ADR-driven V&A Phase 6
//! skeleton. If any new wizard-facing knob arrives, it should land here
//! first, then propagate to sibling solutions in a follow-on PRD.
//!
//! This test is the guard: if the pilot loses a required parameter, the
//! skeleton regresses.

use rshome_schema::solution::default_solution_registry;

#[test]
fn pilot_declares_all_required_wizard_parameters() {
    let reg = default_solution_registry();
    let sol = reg
        .get("elrs_crsf_brushless_solution")
        .expect("elrs_crsf_brushless_solution must be in registry");

    // Every ID below is part of the Phase 6 pilot contract. Adding one is
    // fine; removing one requires a ADR + doc update.
    let required_ids = [
        "vehicle_type",
        "imu_axis_tier",
        "imu_chip",
        "control_rate_hz",
        "failsafe_timeout_ms",
        "crsf_uart_rx_gpio",
        "esc_protocol",
        "imu_i2c_addr",
    ];

    let actual_ids: Vec<&str> = sol.user_parameters.iter().map(|p| p.id.as_str()).collect();

    for id in required_ids {
        assert!(
            actual_ids.contains(&id),
            "pilot parameter '{}' missing. Present: {:?}",
            id,
            actual_ids,
        );
    }

    // Phase 6 exit: pilot has ≥ 3 wizard-configurable params (PRD T6.1 / §3).
    assert!(
        sol.user_parameters.len() >= 3,
        "pilot should declare ≥ 3 user_parameters; found {}",
        sol.user_parameters.len(),
    );
}

#[test]
fn pilot_parameter_dependencies_are_consistent() {
    // Every `depends_on.parameter_id` must reference a parameter that
    // actually exists in the same solution's list — otherwise the wizard
    // can't evaluate the condition.
    let reg = default_solution_registry();
    let sol = reg
        .get("elrs_crsf_brushless_solution")
        .expect("pilot missing");

    let ids: std::collections::HashSet<&str> =
        sol.user_parameters.iter().map(|p| p.id.as_str()).collect();

    for p in &sol.user_parameters {
        if let Some(dep) = &p.depends_on {
            assert!(
                ids.contains(dep.parameter_id.as_str()),
                "parameter '{}' depends_on '{}' but that parameter is not in the same solution",
                p.id,
                dep.parameter_id,
            );
        }
    }
}

#[test]
fn pilot_enum_values_have_non_empty_labels() {
    // Guards against a silent regression where `enum_values` land with empty
    // labels — the wizard dropdown would render blank items.
    let reg = default_solution_registry();
    let sol = reg
        .get("elrs_crsf_brushless_solution")
        .expect("pilot missing");

    for p in &sol.user_parameters {
        let Some(opts) = &p.enum_values else { continue };
        for o in opts {
            assert!(
                !o.label.is_empty(),
                "parameter '{}' enum option '{}' has empty label",
                p.id,
                o.value,
            );
            assert!(
                !o.value.is_empty(),
                "parameter '{}' has an enum option with empty value",
                p.id,
            );
        }
    }
}
