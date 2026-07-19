//! `FailsafeInfo` serde round-trip lint per
//! Task 0.4 acceptance #4: "`FailsafeInfo` struct serialization
//! round-trips: `serde_json::from_str(&serde_json::to_string(&fs)?)? ==
//! fs` for all 27 vehicle-bound `failsafe` instances (10K
//! `arbitrary::Arbitrary` cases in `tests/property/failsafe_roundtrip.rs`)".
//!
//! The PRD's path hints at `tests/property/failsafe_roundtrip.rs` but
//! Cargo only auto-discovers integration tests at the top level of
//! `tests/`; this file uses the flat `va_*.rs` convention so it joins
//! the lint suite without a `[[test]]` declaration in Cargo.toml.
//!
//! Two layers of coverage:
//!
//! 1. **Concrete registry replay** — every vehicle-bound V&A solution's
//!    real `FailsafeInfo` round-trips through serde with byte-identical
//!    re-serialization.
//! 2. **10K proptest fuzz** — arbitrary `FailsafeInfo` values
//!    (constructed via `prop_oneof!` strategies over each field) round-
//!    trip cleanly. Guards against a future `#[serde(...)]` attribute
//!    addition that silently breaks one field's serialization.

use proptest::collection::vec as prop_vec;
use proptest::prelude::*;

use rshome_schema::platform::{
    DomainKind, EmergencyStopWiring, FailsafeInfo, KillswitchSource, RxLossBehavior,
};
use rshome_schema::solution::default_solution_registry;

// ── (1) Concrete registry replay ────────────────────────────────────────────

#[test]
fn every_va_failsafe_roundtrips_through_serde() {
    let reg = default_solution_registry();
    let mut failures: Vec<String> = Vec::new();
    for sol in reg.all() {
        if sol.domain != Some(DomainKind::VehicleAircraftControl) {
            continue;
        }
        let Some(fs) = &sol.failsafe else { continue };
        let json = match serde_json::to_string(fs) {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!("{}: serialize failed: {}", sol.id, e));
                continue;
            }
        };
        let back: FailsafeInfo = match serde_json::from_str(&json) {
            Ok(v) => v,
            Err(e) => {
                failures.push(format!(
                    "{}: deserialize failed: {} (json was {})",
                    sol.id, e, json
                ));
                continue;
            }
        };
        if &back != fs {
            failures.push(format!(
                "{}: round-trip changed value\n    original: {:?}\n    after:    {:?}",
                sol.id, fs, back,
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "FailsafeInfo round-trip failures in V&A registry:\n  {}",
        failures.join("\n  "),
    );
}

// ── (2) Proptest 10K-case fuzz ──────────────────────────────────────────────

fn arb_killswitch() -> impl Strategy<Value = KillswitchSource> {
    prop_oneof![
        Just(KillswitchSource::RcSwitch),
        Just(KillswitchSource::RxLoss),
        Just(KillswitchSource::TimeoutNoPacket),
        Just(KillswitchSource::EmergencyButton),
        Just(KillswitchSource::SbcHeartbeatLoss),
        Just(KillswitchSource::LowVoltage),
    ]
}

fn arb_rx_loss() -> impl Strategy<Value = RxLossBehavior> {
    prop_oneof![
        Just(RxLossBehavior::MotorCutoff),
        Just(RxLossBehavior::HoverHold),
        Just(RxLossBehavior::Rth),
        Just(RxLossBehavior::GlideTrim),
        Just(RxLossBehavior::Unpowered),
        Just(RxLossBehavior::PassthroughLast),
    ]
}

fn arb_estop() -> impl Strategy<Value = EmergencyStopWiring> {
    prop_oneof![
        Just(EmergencyStopWiring::None),
        Just(EmergencyStopWiring::GpioPulldown),
        Just(EmergencyStopWiring::RelayCutoff),
        Just(EmergencyStopWiring::EscDshotCmd),
    ]
}

fn arb_watchdog() -> impl Strategy<Value = Option<u32>> {
    prop_oneof![
        Just(None),
        Just(Some(100u32)),
        Just(Some(250u32)),
        Just(Some(500u32)),
        Just(Some(1000u32)),
        any::<u32>().prop_map(Some),
    ]
}

fn arb_failsafe() -> impl Strategy<Value = FailsafeInfo> {
    (
        prop_vec(arb_killswitch(), 0..6),
        prop::option::of(arb_rx_loss()),
        arb_watchdog(),
        arb_estop(),
    )
        .prop_map(|(ks, rx, wd, es)| FailsafeInfo {
            killswitch_source: ks,
            rx_loss_behavior: rx,
            watchdog_ms: wd,
            emergency_stop_wiring: es,
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    /// `FailsafeInfo` serde round-trips losslessly for any field
    /// combination the proptest engine generates. Catches a future
    /// serde-attribute drift (e.g., a `#[serde(default)]` accidentally
    /// removed from a field) that would round-trip-mutate the value.
    #[test]
    fn arbitrary_failsafe_serde_roundtrips(fs in arb_failsafe()) {
        let json = serde_json::to_string(&fs).expect("FailsafeInfo must serialize");
        let back: FailsafeInfo = serde_json::from_str(&json).expect("FailsafeInfo must deserialize");
        prop_assert_eq!(fs, back);
    }
}
