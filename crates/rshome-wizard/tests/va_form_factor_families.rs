//! Form-factor-families lint per design doc §"Verification" line 673:
//! "every solution declares `form_factor_families[] ⊆` the canonical
//! `FormFactorKind` enum."
//!
//! Concretely: every actuator-bearing V&A solution must declare a
//! non-empty `form_factor_families` so the wizard can validate
//! `(solution, form_factor)` pairs at step 7 / step 9
//! (see `type-driven-ui/src/components/rshome/wizard/navigation.ts:226-234`).
//! Solutions without actuators (TX, video, gateway, bridge, passthrough
//! with no fixed form factor) are exempt.

use rshome_schema::platform::DomainKind;
use rshome_schema::solution::default_solution_registry;

#[test]
fn actuator_va_solutions_declare_form_factor_families() {
    let reg = default_solution_registry();

    let mut missing: Vec<String> = Vec::new();
    for sol in reg.all() {
        if sol.domain != Some(DomainKind::VehicleAircraftControl) {
            continue;
        }
        // Skip non-vehicle-bound roles (TX / gateway / video-only / passthrough).
        if !sol
            .architecture_tier
            .map(|r| r.is_vehicle_bound())
            .unwrap_or(false)
        {
            continue;
        }
        // One explicit exception: mcu_sbc_bridge_solution is a ControlBoard
        // role (vehicle-bound) but is intentionally form-factor-agnostic —
        // it's a generic MCU↔SBC bridge that works with any airframe. The
        // bridge itself doesn't pin motor/servo mixing.
        if sol.id == "mcu_sbc_bridge_solution" {
            continue;
        }
        match &sol.form_factor_families {
            Some(ffs) if !ffs.is_empty() => {}
            _ => missing.push(sol.id.clone()),
        }
    }

    assert!(
        missing.is_empty(),
        "V&A actuator solutions missing form_factor_families:\n  {}",
        missing.join("\n  ")
    );
}
