//! Motor-backend dispatch + variant-conflict lint per
//! Task 3.2 (dispatch + conflict-detection scope; firmware lab tests
//! deferred).
//!
//! Two layers of coverage:
//!
//! 1. **Dispatch correctness** — `motor_control_backend(active_flags)`
//!    returns the expected `MotorBackend` for each well-formed input.
//!    `USE_BDSHOT` wins over `USE_DSHOT`; default is PWM. Existing
//!    `tests/motor_control_backend.rs` covers self-hosted path; this
//!    file adds proptest coverage on the predicate itself.
//!
//! 2. **Variant conflict detection** — `detect_variant_flag_conflicts`
//!    flags any active-flag combination where 2+ flags from the same
//!    mutually-exclusive group coexist. Today there's one group
//!    (`USE_DSHOT` × `USE_BDSHOT`); the detector is structured so
//!    adding future groups (e.g., camera backends) only requires
//!    extending `MUTUALLY_EXCLUSIVE_FLAG_GROUPS`.
//!
//!    50-fixture corpus at `tests/fixtures/variant_conflicts.json`
//!    (30 conflict + 20 clean cases) drives the
//!    `conflict_corpus_classification_total` test.
//!
//! What this lint does NOT cover (deferred per scope cap):
//!
//!   - Brookesia codegen path variant awareness (firmware codegen).
//!   - insta snapshots of generated projects per variant
//!     (`brookesia_quad_stabilizer_dshot.snap`, etc.).
//!   - Hardware symbol-presence check on built firmware
//!     (`e2e_brookesia_variant_dispatch.rs` requires Tier-S lab).
//!   - Pipeline-level wiring of `detect_variant_flag_conflicts` into
//!     rshome-config Stage 3.5 (analogous to the stage9
//!     orchestration-budget-check deferral from Task 2.1).

use std::path::PathBuf;

use proptest::collection::vec as prop_vec;
use proptest::prelude::*;
use serde::Deserialize;

use rshome_codegen::generator::{
    detect_variant_flag_conflicts, motor_control_backend, MotorBackend, VariantFlagConflict,
    MUTUALLY_EXCLUSIVE_FLAG_GROUPS,
};

// ── (1) Dispatch correctness ────────────────────────────────────────────────

#[test]
fn dispatch_returns_pwm_when_no_motor_backend_flag() {
    let backend = motor_control_backend(&[]);
    assert_eq!(backend, MotorBackend::Pwm);
}

#[test]
fn dispatch_returns_dshot_for_use_dshot() {
    let backend = motor_control_backend(&["USE_DSHOT".to_string()]);
    assert_eq!(backend, MotorBackend::Dshot);
}

#[test]
fn dispatch_returns_bdshot_for_use_bdshot() {
    let backend = motor_control_backend(&["USE_BDSHOT".to_string()]);
    assert_eq!(backend, MotorBackend::Bdshot);
}

#[test]
fn dispatch_precedence_bdshot_beats_dshot() {
    let backend = motor_control_backend(&["USE_DSHOT".to_string(), "USE_BDSHOT".to_string()]);
    assert_eq!(backend, MotorBackend::Bdshot);
}

#[test]
fn dispatch_ignores_unrelated_flags() {
    let backend = motor_control_backend(&[
        "USE_DSHOT".to_string(),
        "USE_WIFI".to_string(),
        "USE_API".to_string(),
    ]);
    assert_eq!(backend, MotorBackend::Dshot);
}

// ── (2) Conflict-detection corpus replay ────────────────────────────────────

#[derive(Deserialize)]
struct ConflictGroupExpect {
    group: Vec<String>,
    present: Vec<String>,
}

#[derive(Deserialize)]
struct ConflictFixture {
    case: String,
    active_flags: Vec<String>,
    expected_conflicts: Vec<ConflictGroupExpect>,
    #[serde(default)]
    #[allow(dead_code)]
    note: String,
}

fn corpus_path() -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest).join("tests/fixtures/variant_conflicts.json")
}

#[test]
fn conflict_corpus_classification_total() {
    let raw = std::fs::read_to_string(corpus_path()).expect("variant_conflicts.json must exist");
    let corpus: Vec<ConflictFixture> = serde_json::from_str(&raw).expect("fixture must parse");
    assert_eq!(corpus.len(), 50, "corpus must hold exactly 50 fixtures");

    let mut failures: Vec<String> = Vec::new();
    let mut conflict_count = 0usize;
    let mut clean_count = 0usize;

    for fx in &corpus {
        let detected = detect_variant_flag_conflicts(&fx.active_flags);
        let detected_matches_expected =
            match (detected.is_empty(), fx.expected_conflicts.is_empty()) {
                (true, true) => {
                    clean_count += 1;
                    true
                }
                (false, false) => {
                    conflict_count += 1;
                    // Compare detected groups + present sets element-wise.
                    if detected.len() != fx.expected_conflicts.len() {
                        false
                    } else {
                        detected.iter().zip(&fx.expected_conflicts).all(|(d, e)| {
                            let d_group: Vec<&str> = d.group.to_vec();
                            d_group == e.group.iter().map(String::as_str).collect::<Vec<_>>()
                                && d.present
                                    == e.present.iter().map(String::as_str).collect::<Vec<_>>()
                        })
                    }
                }
                _ => false,
            };
        if !detected_matches_expected {
            failures.push(format!(
                "{}: active_flags={:?} expected_conflicts={} detected={:?}",
                fx.case,
                fx.active_flags,
                fx.expected_conflicts.len(),
                detected,
            ));
        }
    }

    assert_eq!(
        conflict_count, 30,
        "expected exactly 30 conflict fixtures, got {}",
        conflict_count
    );
    assert_eq!(
        clean_count, 20,
        "expected exactly 20 clean fixtures, got {}",
        clean_count
    );
    assert!(
        failures.is_empty(),
        "variant-conflict corpus classification drift ({} failures):\n  {}",
        failures.len(),
        failures.join("\n  "),
    );
}

#[test]
fn at_least_one_mutually_exclusive_group_is_defined() {
    // Defensive: catches an accidental empty-array regression that
    // would silently disable the conflict detector.
    assert!(
        !MUTUALLY_EXCLUSIVE_FLAG_GROUPS.is_empty(),
        "MUTUALLY_EXCLUSIVE_FLAG_GROUPS is empty — conflict detector is inert"
    );
    for group in MUTUALLY_EXCLUSIVE_FLAG_GROUPS {
        assert!(
            group.len() >= 2,
            "mutually-exclusive group {:?} has < 2 members — meaningless",
            group,
        );
    }
}

#[test]
fn motor_backend_group_is_present() {
    // The PRD specifically calls out the motor-backend mutual-exclusion.
    // A future refactor that removes this group must update the PRD
    // first.
    let has_motor_group = MUTUALLY_EXCLUSIVE_FLAG_GROUPS.iter().any(|g| {
        let s: std::collections::BTreeSet<&str> = g.iter().copied().collect();
        s.contains("USE_DSHOT") && s.contains("USE_BDSHOT")
    });
    assert!(
        has_motor_group,
        "motor-backend group (USE_DSHOT × USE_BDSHOT) missing from MUTUALLY_EXCLUSIVE_FLAG_GROUPS"
    );
}

// ── (3) 10K proptest fuzz on the detector ────────────────────────────────────

fn arb_flag() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("USE_DSHOT".to_string()),
        Just("USE_BDSHOT".to_string()),
        Just("USE_WIFI".to_string()),
        Just("USE_API".to_string()),
        Just("USE_LOGGER".to_string()),
        Just("USE_MQTT".to_string()),
        Just("USE_OTA".to_string()),
        Just("USE_I2C".to_string()),
    ]
}

fn classify_independently(flags: &[String]) -> Vec<VariantFlagConflict> {
    // Independent re-implementation of the detector against which the
    // production detector is checked. Catches algorithmic drift.
    let mut conflicts = Vec::new();
    for group in MUTUALLY_EXCLUSIVE_FLAG_GROUPS {
        let mut present: Vec<&'static str> = Vec::new();
        for &flag in *group {
            if flags.iter().any(|f| f == flag) {
                present.push(flag);
            }
        }
        if present.len() >= 2 {
            conflicts.push(VariantFlagConflict { group, present });
        }
    }
    conflicts
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    /// For arbitrary feature-flag lists, the detector matches the
    /// independent reference implementation. Catches a refactor that
    /// silently changes the conflict semantics.
    #[test]
    fn detector_matches_reference(flags in prop_vec(arb_flag(), 0..10)) {
        let production = detect_variant_flag_conflicts(&flags);
        let reference = classify_independently(&flags);
        prop_assert_eq!(production, reference);
    }

    /// Dispatch + conflict are independent contracts: a configuration
    /// can be flagged as a conflict AND still dispatch to a specific
    /// backend (per precedence). Verify the dispatch is never
    /// silently broken by a conflict.
    #[test]
    fn dispatch_classification_is_total(flags in prop_vec(arb_flag(), 0..10)) {
        let backend = motor_control_backend(&flags);
        let has_bdshot = flags.iter().any(|f| f == "USE_BDSHOT");
        let has_dshot = flags.iter().any(|f| f == "USE_DSHOT");
        if has_bdshot {
            prop_assert_eq!(backend, MotorBackend::Bdshot);
        } else if has_dshot {
            prop_assert_eq!(backend, MotorBackend::Dshot);
        } else {
            prop_assert_eq!(backend, MotorBackend::Pwm);
        }
    }
}
