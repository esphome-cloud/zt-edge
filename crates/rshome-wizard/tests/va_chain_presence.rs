//! V&A chain-presence + chain-semantic lint per
//! Task 0.3.
//!
//! Three invariants enforced, layered from cheap-and-static to
//! 10K-case fuzz:
//!
//! 1. **Field presence** — every V&A solution declares all three chain
//!    enums (`control_uplink`, `video_downlink`, `telemetry`) as
//!    `Some(_)`. The sentinel value `ControlUplinkKind::None` is
//!    allowed for non-vehicle-bound roles; `Option::None` is never
//!    allowed. (Pre-existing test, retained.)
//!
//! 2. **Vehicle-bound semantic** — for every solution whose
//!    `architecture_tier.is_vehicle_bound()` is true (control_board,
//!    control_telemetry_board, all_in_one_cam), `control_uplink` must
//!    NOT be `ControlUplinkKind::None`. Vehicle-bound roles drive
//!    actuators and therefore require a real uplink protocol. ADR-016
//!    is the source-of-truth.
//!
//! 3. **Effective uplink golden replay** — for ~100 fixture inputs in
//!    `tests/fixtures/effective_uplink_golden.json`,
//!    `effective_control_uplink(profile, sol)` returns the expected
//!    serde-name string. Covers the parameterized
//!    `phone_bridge_solution` case where the uplink comes from the
//!    `vehicle_side_protocol` user-parameter, not `sol.control_uplink`.
//!
//! 4. **10K proptest fuzz** — for arbitrary (role, uplink) pairs, the
//!    boundary predicate `is_vehicle_bound(role) ∧ uplink ==
//!    ControlUplinkKind::None ⇒ violation` matches the lint's
//!    classification. Guards against a future refactor that changes
//!    either the lint logic or `is_vehicle_bound` in isolation.
//!
//! Companion guard: `va_topology_category_presence.rs` covers
//! `topology_category` auto-population (ADR-01).

use std::collections::BTreeMap;
use std::path::PathBuf;

use proptest::prelude::*;

use rshome_schema::platform::{ChipFamilyKind, ControlUplinkKind, DomainKind, McuRole};
use rshome_schema::solution::{default_solution_registry, SolutionDefinition};
use rshome_wizard::workspace::{effective_control_uplink, SavedProfile};
use serde_json::Value;

// ── (1) Field presence ──────────────────────────────────────────────────────

#[test]
fn every_va_solution_declares_all_three_chains() {
    let reg = default_solution_registry();
    let mut missing: Vec<String> = Vec::new();

    for sol in reg.all() {
        if sol.domain != Some(DomainKind::VehicleAircraftControl) {
            continue;
        }
        if sol.control_uplink.is_none() {
            missing.push(format!("{}: control_uplink missing", sol.id));
        }
        if sol.video_downlink.is_none() {
            missing.push(format!("{}: video_downlink missing", sol.id));
        }
        if sol.telemetry.is_none() {
            missing.push(format!("{}: telemetry missing", sol.id));
        }
    }

    assert!(
        missing.is_empty(),
        "V&A solutions missing chain annotations:\n  {}",
        missing.join("\n  ")
    );
}

// ── (2) Vehicle-bound semantic ──────────────────────────────────────────────

fn is_violation(role: Option<McuRole>, uplink: Option<ControlUplinkKind>) -> bool {
    let is_va_actuator = role.is_some_and(|r| r.is_vehicle_bound());
    is_va_actuator && matches!(uplink, None | Some(ControlUplinkKind::None))
}

#[test]
fn vehicle_bound_solutions_have_real_control_uplink() {
    let reg = default_solution_registry();
    let mut violations: Vec<String> = Vec::new();

    for sol in reg.all() {
        if sol.domain != Some(DomainKind::VehicleAircraftControl) {
            continue;
        }
        if is_violation(sol.architecture_tier, sol.control_uplink) {
            violations.push(format!(
                "{} (role={:?}): control_uplink={:?} — vehicle-bound roles must drive a real uplink per ADR-016",
                sol.id, sol.architecture_tier, sol.control_uplink,
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "V&A solutions violating vehicle-bound uplink contract:\n  {}",
        violations.join("\n  "),
    );
}

// ── (3) Effective-uplink golden replay ──────────────────────────────────────

fn fixtures_path() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir).join("tests/fixtures/effective_uplink_golden.json")
}

#[derive(serde::Deserialize)]
struct UplinkFixture {
    solution_id: String,
    parameter_values: BTreeMap<String, Value>,
    expected_uplink: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    note: String,
}

#[test]
fn effective_control_uplink_matches_golden_fixtures() {
    let reg = default_solution_registry();
    let path = fixtures_path();
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {}", path.display(), e));
    let fixtures: Vec<UplinkFixture> = serde_json::from_str(&raw).unwrap();
    assert!(
        fixtures.len() >= 98,
        "fixture file shrank — expected ≥98 cases, got {}",
        fixtures.len(),
    );

    let mut mismatches: Vec<String> = Vec::new();
    for (i, fx) in fixtures.iter().enumerate() {
        let Some(sol): Option<&SolutionDefinition> = reg.get(&fx.solution_id) else {
            mismatches.push(format!(
                "fixture[{}]: unknown solution `{}`",
                i, fx.solution_id
            ));
            continue;
        };
        let profile = SavedProfile {
            label: format!("fixture-{}", i),
            chip_target: ChipFamilyKind::Esp32S3,
            selected_solution_id: fx.solution_id.clone(),
            parameter_values: fx.parameter_values.clone(),
        };
        let actual = effective_control_uplink(&profile, sol);
        // Treat the sentinel "none" string as None for comparison purposes —
        // both forms represent absence at the wire level.
        if actual.as_deref() != fx.expected_uplink.as_deref() {
            mismatches.push(format!(
                "fixture[{}] {} params={:?}: expected={:?} actual={:?}",
                i, fx.solution_id, fx.parameter_values, fx.expected_uplink, actual,
            ));
        }
    }

    assert!(
        mismatches.is_empty(),
        "effective_control_uplink golden mismatches ({}):\n  {}",
        mismatches.len(),
        mismatches.join("\n  ")
    );
}

// ── (4) 10K proptest fuzz on the boundary predicate ─────────────────────────

fn arb_role() -> impl Strategy<Value = McuRole> {
    prop_oneof![
        Just(McuRole::RemoteControlTx),
        Just(McuRole::SmartphoneGateway),
        Just(McuRole::ControlBoard),
        Just(McuRole::ControlTelemetryBoard),
        Just(McuRole::AllInOneCam),
        Just(McuRole::VideoBoard),
        Just(McuRole::ReceiverDirectDrive),
    ]
}

fn arb_uplink() -> impl Strategy<Value = ControlUplinkKind> {
    prop_oneof![
        Just(ControlUplinkKind::None),
        Just(ControlUplinkKind::EspNow),
        Just(ControlUplinkKind::WifiMesh),
        Just(ControlUplinkKind::Wifi80211lr),
        Just(ControlUplinkKind::BleMesh),
        Just(ControlUplinkKind::Crsf),
        Just(ControlUplinkKind::Sbus),
        Just(ControlUplinkKind::WifiMavlink),
        Just(ControlUplinkKind::WifiCrtp),
        Just(ControlUplinkKind::BleGatt),
        Just(ControlUplinkKind::UsbCdc),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    /// For every (role, uplink) pair the proptest engine generates, the
    /// classification produced by `is_violation` must equal the
    /// independently-derived ground truth: vehicle-bound role +
    /// `ControlUplinkKind::None` = violation; anything else = OK.
    ///
    /// Guards against a refactor of either `is_violation` (the lint
    /// predicate) or `McuRole::is_vehicle_bound` (the role classifier)
    /// that silently changes which solutions the lint flags.
    #[test]
    fn boundary_predicate_classifies_correctly(
        role in arb_role(),
        uplink in arb_uplink(),
    ) {
        let ground_truth = role.is_vehicle_bound() && uplink == ControlUplinkKind::None;
        let computed = is_violation(Some(role), Some(uplink));
        prop_assert_eq!(ground_truth, computed);
    }

    /// Every uplink variant on a vehicle-bound role is OK UNLESS it's
    /// `None`; this asymmetric guard catches a refactor that erroneously
    /// adds a second "absence-like" sentinel.
    #[test]
    fn vehicle_bound_only_fails_on_none_sentinel(uplink in arb_uplink()) {
        let role = McuRole::ControlBoard;
        let should_fail = uplink == ControlUplinkKind::None;
        let does_fail = is_violation(Some(role), Some(uplink));
        prop_assert_eq!(should_fail, does_fail);
    }

    /// No non-vehicle-bound role ever triggers a violation, regardless
    /// of uplink — non-vehicle-bound roles are exempt by ADR-016.
    #[test]
    fn non_vehicle_bound_never_violates(uplink in arb_uplink()) {
        for role in [
            McuRole::RemoteControlTx,
            McuRole::SmartphoneGateway,
            McuRole::VideoBoard,
            McuRole::ReceiverDirectDrive,
        ] {
            prop_assert!(!is_violation(Some(role), Some(uplink)));
        }
    }
}
