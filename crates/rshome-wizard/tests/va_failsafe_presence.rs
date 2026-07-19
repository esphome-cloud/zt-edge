//! V&A failsafe contract + watchdog-ladder lint per
//! Task 0.4.
//!
//! The watchdog ladder (ADR-011) fixes four allowed values —
//! **{100, 250, 500, 1000}** ms — chosen so the firmware's failsafe
//! loop can resolve real failures inside the dynamics window of each
//! vehicle class:
//!
//! | Class                          | Watchdog | rx_loss_behavior  |
//! |--------------------------------|----------|-------------------|
//! | Multirotor (`quad_mix`)        | 100      | motor_cutoff      |
//! | Helicopter (`heli_swashplate`) | 100      | motor_cutoff      |
//! | VTOL (`vtol_transition`)       | 100      | hover_hold        |
//! | Fixed-wing                     | 250      | glide_trim        |
//! | Light/responsive ground        | 250      | motor_cutoff/rth  |
//! | General ground / marine        | 500      | varies            |
//! | Slow LTA / full-size marine    | 1000     | glide_trim / rth  |
//!
//! Passthrough exception (allow-list): `receiver_direct_drive_solution`
//! and `sbus_passthrough_solution` carry `watchdog_ms: null` +
//! `rx_loss_behavior: passthrough_last` — failsafe is deferred to the
//! RX itself, no firmware watchdog applies.

use std::collections::BTreeSet;

use proptest::prelude::*;

use rshome_schema::platform::{ActuatorFamily, DomainKind, RxLossBehavior};
use rshome_schema::solution::default_solution_registry;

// ── (1) Existing: legacy-passthrough warning marker ─────────────────────────

#[test]
fn legacy_passthrough_solutions_carry_warning_marker_in_label() {
    let reg = default_solution_registry();
    let legacy_passthroughs = [
        "receiver_direct_drive_solution",
        "sbus_passthrough_solution",
    ];
    for id in legacy_passthroughs {
        let sol = reg
            .get(id)
            .unwrap_or_else(|| panic!("legacy passthrough '{}' should be in registry", id));
        assert!(
            sol.label.contains('⚠'),
            "legacy passthrough '{}' must carry ⚠️ marker in `label`; current label = {:?}",
            id,
            sol.label
        );
    }
}

// ── (2) Existing: every actuator-bearing solution has a killswitch source ───

#[test]
fn every_actuator_va_solution_has_killswitch_source() {
    let reg = default_solution_registry();
    let mut missing: Vec<String> = Vec::new();
    for sol in reg.all() {
        if sol.domain != Some(DomainKind::VehicleAircraftControl) {
            continue;
        }
        if !sol
            .architecture_tier
            .map(|r| r.is_vehicle_bound())
            .unwrap_or(false)
        {
            continue;
        }
        let fs = match &sol.failsafe {
            Some(fs) => fs,
            None => {
                missing.push(format!("{}: failsafe = None", sol.id));
                continue;
            }
        };
        if fs.killswitch_source.is_empty() {
            missing.push(format!("{}: killswitch_source is empty", sol.id));
        }
    }
    assert!(
        missing.is_empty(),
        "V&A actuator solutions missing killswitch_source:\n  {}",
        missing.join("\n  ")
    );
}

// ── (3) Watchdog ladder: vehicle-bound watchdog ∈ {100, 250, 500, 1000} ────

const ALLOWED_WATCHDOG_LADDER: &[u32] = &[100, 250, 500, 1000];

fn passthrough_allowlist() -> BTreeSet<&'static str> {
    [
        "receiver_direct_drive_solution",
        "sbus_passthrough_solution",
    ]
    .into_iter()
    .collect()
}

#[test]
fn watchdog_ladder_match() {
    let reg = default_solution_registry();
    let allow = passthrough_allowlist();
    let mut violations: Vec<String> = Vec::new();

    for sol in reg.all() {
        if sol.domain != Some(DomainKind::VehicleAircraftControl) {
            continue;
        }
        let is_vehicle_bound = sol
            .architecture_tier
            .map(|r| r.is_vehicle_bound())
            .unwrap_or(false);
        let is_passthrough = allow.contains(sol.id.as_str());
        let Some(fs) = &sol.failsafe else {
            // Non-vehicle-bound roles legitimately have no failsafe block.
            if is_vehicle_bound && !is_passthrough {
                violations.push(format!("{}: failsafe = None on vehicle-bound role", sol.id));
            }
            continue;
        };

        match fs.watchdog_ms {
            None => {
                if is_vehicle_bound && !is_passthrough {
                    violations.push(format!(
                        "{}: watchdog_ms = null but vehicle-bound and NOT in passthrough allow-list",
                        sol.id,
                    ));
                }
            }
            Some(ms) => {
                if !ALLOWED_WATCHDOG_LADDER.contains(&ms) {
                    violations.push(format!(
                        "{}: watchdog_ms = {} — not in ladder {:?} (ADR-011)",
                        sol.id, ms, ALLOWED_WATCHDOG_LADDER,
                    ));
                }
                if is_passthrough {
                    violations.push(format!(
                        "{}: in passthrough allow-list but watchdog_ms = {} (must be null)",
                        sol.id, ms,
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "watchdog-ladder violations ({}):\n  {}",
        violations.len(),
        violations.join("\n  "),
    );
}

// ── (4) Multirotor specifically: quad_mix → 100 exactly ─────────────────────

fn is_multirotor_violation(actuator: Option<ActuatorFamily>, watchdog_ms: Option<u32>) -> bool {
    actuator == Some(ActuatorFamily::QuadMix) && watchdog_ms != Some(100)
}

#[test]
fn no_multirotor_over_100ms() {
    let reg = default_solution_registry();
    let mut violations: Vec<String> = Vec::new();
    for sol in reg.all() {
        if sol.domain != Some(DomainKind::VehicleAircraftControl) {
            continue;
        }
        let wd = sol.failsafe.as_ref().and_then(|f| f.watchdog_ms);
        if is_multirotor_violation(sol.actuator_family, wd) {
            violations.push(format!(
                "{}: actuator_family = QuadMix but watchdog_ms = {:?} (must be 100)",
                sol.id, wd,
            ));
        }
    }
    assert!(
        violations.is_empty(),
        "multirotor watchdog violations:\n  {}",
        violations.join("\n  "),
    );
}

// ── (5) Passthrough-last rx_loss only on legacy allow-list ─────────────────

#[test]
fn passthrough_only_for_legacy() {
    let reg = default_solution_registry();
    let allow = passthrough_allowlist();
    let mut violations: Vec<String> = Vec::new();
    for sol in reg.all() {
        if sol.domain != Some(DomainKind::VehicleAircraftControl) {
            continue;
        }
        let Some(fs) = &sol.failsafe else { continue };
        if fs.rx_loss_behavior == Some(RxLossBehavior::PassthroughLast)
            && !allow.contains(sol.id.as_str())
        {
            violations.push(format!(
                "{}: rx_loss_behavior = passthrough_last but solution is NOT in legacy allow-list {:?}",
                sol.id, allow,
            ));
        }
    }
    assert!(
        violations.is_empty(),
        "passthrough_last appearing on non-legacy solutions:\n  {}",
        violations.join("\n  "),
    );
}

// ── (6) Watchdog ladder CSV golden — drift detector ─────────────────────────

#[test]
fn watchdog_ladder_csv_matches_registry() {
    // The CSV at `tests/fixtures/watchdog_ladder.csv` is a snapshot derived
    // from `scripts/va-solution-table.tsv` columns
    // `(id, actuator_family, watchdog_ms, rx_loss_behavior)`. Any row
    // disagreeing with the live registry indicates either (a) Rust-source
    // drift not refreshed, or (b) a deliberate registry mutation that
    // needs `--update-snapshot`.
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let csv_path = format!("{}/tests/fixtures/watchdog_ladder.csv", manifest_dir);
    let csv = std::fs::read_to_string(&csv_path)
        .unwrap_or_else(|e| panic!("cannot read {}: {}", csv_path, e));

    let reg = default_solution_registry();
    let mut mismatches: Vec<String> = Vec::new();

    for (idx, line) in csv.lines().enumerate() {
        if idx == 0 {
            // header row
            continue;
        }
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() < 4 {
            mismatches.push(format!("line {}: malformed `{}`", idx + 1, line));
            continue;
        }
        let (id, csv_af, csv_ms, csv_rx) = (parts[0], parts[1], parts[2], parts[3]);
        let Some(sol) = reg.get(id) else {
            mismatches.push(format!("line {}: unknown solution `{}`", idx + 1, id));
            continue;
        };

        let actual_af = sol
            .actuator_family
            .map(|a| {
                serde_json::to_value(a)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .unwrap_or_default();
        if actual_af != csv_af {
            mismatches.push(format!(
                "{}: actuator_family csv={:?} actual={:?}",
                id, csv_af, actual_af
            ));
        }
        let actual_ms = sol
            .failsafe
            .as_ref()
            .and_then(|f| f.watchdog_ms)
            .map(|n| n.to_string())
            .unwrap_or_else(|| "null".to_string());
        if actual_ms != csv_ms {
            mismatches.push(format!(
                "{}: watchdog_ms csv={:?} actual={:?}",
                id, csv_ms, actual_ms
            ));
        }
        let actual_rx = sol
            .failsafe
            .as_ref()
            .and_then(|f| f.rx_loss_behavior)
            .map(|r| {
                serde_json::to_value(r)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .unwrap_or_else(|| "null".to_string());
        if actual_rx != csv_rx {
            mismatches.push(format!(
                "{}: rx_loss_behavior csv={:?} actual={:?}",
                id, csv_rx, actual_rx
            ));
        }
    }

    assert!(
        mismatches.is_empty(),
        "watchdog_ladder.csv drift ({}):\n  {}",
        mismatches.len(),
        mismatches.join("\n  "),
    );
}

// ── (7) 10K proptest fuzz on multirotor-violation predicate ────────────────

fn arb_actuator() -> impl Strategy<Value = ActuatorFamily> {
    prop_oneof![
        Just(ActuatorFamily::BrushedHbridge),
        Just(ActuatorFamily::BrushlessEscPwm),
        Just(ActuatorFamily::BrushlessEscDshot),
        Just(ActuatorFamily::SteeringServo),
        Just(ActuatorFamily::MixedDiffDrive),
        Just(ActuatorFamily::MixedAckermann),
        Just(ActuatorFamily::QuadMix),
        Just(ActuatorFamily::HydraulicJoint),
        Just(ActuatorFamily::PneumaticChamber),
        Just(ActuatorFamily::TendonCable),
        Just(ActuatorFamily::ThrusterVector),
        Just(ActuatorFamily::HeliSwashplate),
        Just(ActuatorFamily::FixedwingSurfaces),
        Just(ActuatorFamily::VtolTransition),
    ]
}

fn arb_watchdog() -> impl Strategy<Value = Option<u32>> {
    prop_oneof![
        Just(None),
        Just(Some(100u32)),
        Just(Some(250u32)),
        Just(Some(500u32)),
        Just(Some(1000u32)),
        // intentionally include off-ladder values to exercise the violation case
        any::<u32>().prop_map(Some),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    /// For arbitrary (actuator_family, watchdog_ms) inputs, the predicate
    /// `is_multirotor_violation` must equal the independently-derived
    /// ground truth: QuadMix paired with anything other than Some(100)
    /// is a violation.
    #[test]
    fn multirotor_violation_predicate_classifies(
        af in arb_actuator(),
        wd in arb_watchdog(),
    ) {
        let ground_truth = af == ActuatorFamily::QuadMix && wd != Some(100);
        prop_assert_eq!(ground_truth, is_multirotor_violation(Some(af), wd));
    }

    /// Non-QuadMix actuators never trigger a multirotor violation regardless
    /// of watchdog — the rule is scoped to multirotor specifically.
    #[test]
    fn non_quadmix_never_multirotor_violates(wd in arb_watchdog()) {
        for af in [
            ActuatorFamily::BrushedHbridge,
            ActuatorFamily::HeliSwashplate,
            ActuatorFamily::FixedwingSurfaces,
            ActuatorFamily::MixedDiffDrive,
            ActuatorFamily::ThrusterVector,
        ] {
            prop_assert!(!is_multirotor_violation(Some(af), wd));
        }
    }
}
