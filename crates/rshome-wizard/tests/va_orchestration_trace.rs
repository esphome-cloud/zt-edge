//! `OrchestrationTrace` schema lint per
//! Task 2.3 (schema-side; firmware lab tests deferred).
//!
//! Three contract layers:
//!
//! 1. **Wire shape stability** — snapshot the JSON serialization of a
//!    canonical trace and assert structural anchors. Browser workspace
//!    UI subscribes via the WebRTC `logs` channel and unmarshals against
//!    this exact shape — any silent rename breaks the panel.
//! 2. **Invariants** — `OrchestrationStepTrace.attempts` is never 0 for
//!    a row in the trace (PRD Task 2.3 acceptance #2).
//!    `OrchestrationTrace.invalid_events()` returns the offending ids.
//! 3. **10K proptest** — arbitrary traces round-trip through serde
//!    without value drift.
//!
//! What this lint does NOT cover (deferred follow-ups per scope cap):
//!
//!   - `e2e_orchestration_trace.c` — 1000 init runs (firmware, hardware).
//!   - `bench_orchestration_trace_overhead.c` — 5ms p95 overhead on
//!     ESP32-S3 @ 240 MHz (hardware bench).
//!   - `e2e_workspace_orchestration_panel.spec.ts` — Playwright e2e
//!     against the Tier-S hardware lab + browser WebRTC subscription.

use proptest::collection::vec as prop_vec;
use proptest::prelude::*;

use rshome_schema::orchestration::{
    OrchestrationStepOutcome, OrchestrationStepTrace, OrchestrationTrace,
};

// ── (1) Wire shape stability ────────────────────────────────────────────────

fn canonical_trace() -> OrchestrationTrace {
    OrchestrationTrace {
        solution_id: "wheeled_4wd_diff_solution".into(),
        events: vec![
            OrchestrationStepTrace {
                step_id: "i2c_bus_init".into(),
                outcome: OrchestrationStepOutcome::Ok,
                attempts: 1,
                total_ms: 4,
            },
            OrchestrationStepTrace {
                step_id: "imu_init".into(),
                outcome: OrchestrationStepOutcome::RetrySucceeded,
                attempts: 2,
                total_ms: 27,
            },
            OrchestrationStepTrace {
                step_id: "motor_pwm_init".into(),
                outcome: OrchestrationStepOutcome::Ok,
                attempts: 1,
                total_ms: 9,
            },
        ],
    }
}

#[test]
fn wire_shape_is_stable() {
    let trace = canonical_trace();
    let json = serde_json::to_string(&trace).expect("serialize trace");

    // Anchors the browser workspace UI relies on. If any of these change,
    // the panel renderer must be updated in lockstep.
    let required = [
        "\"solution_id\":\"wheeled_4wd_diff_solution\"",
        "\"events\":[",
        "\"step_id\":\"i2c_bus_init\"",
        "\"outcome\":\"ok\"",
        "\"outcome\":\"retry_succeeded\"",
        "\"attempts\":2",
        "\"total_ms\":27",
    ];
    for anchor in required {
        assert!(
            json.contains(anchor),
            "wire shape missing anchor {:?} in: {}",
            anchor,
            json,
        );
    }
}

#[test]
fn outcome_snake_case_serialization() {
    // Each variant must serialize to its documented snake_case string.
    let cases = [
        (OrchestrationStepOutcome::Ok, "\"ok\""),
        (
            OrchestrationStepOutcome::RetrySucceeded,
            "\"retry_succeeded\"",
        ),
        (
            OrchestrationStepOutcome::FailsafeBudget,
            "\"failsafe_budget\"",
        ),
        (
            OrchestrationStepOutcome::FailsafeAttempts,
            "\"failsafe_attempts\"",
        ),
    ];
    for (variant, expected) in cases {
        let json = serde_json::to_string(&variant).unwrap();
        assert_eq!(json, expected, "outcome {:?} drift", variant);
        let back: OrchestrationStepOutcome = serde_json::from_str(&json).unwrap();
        assert_eq!(back, variant);
    }
}

#[test]
fn outcome_is_failsafe_classifies_correctly() {
    assert!(!OrchestrationStepOutcome::Ok.is_failsafe());
    assert!(!OrchestrationStepOutcome::RetrySucceeded.is_failsafe());
    assert!(OrchestrationStepOutcome::FailsafeBudget.is_failsafe());
    assert!(OrchestrationStepOutcome::FailsafeAttempts.is_failsafe());
}

#[test]
fn entered_failsafe_returns_true_when_any_step_failsafes() {
    let mut trace = canonical_trace();
    assert!(
        !trace.entered_failsafe(),
        "happy-path trace should not be failsafe"
    );

    trace.events.push(OrchestrationStepTrace {
        step_id: "encoder_init".into(),
        outcome: OrchestrationStepOutcome::FailsafeBudget,
        attempts: 2,
        total_ms: 80,
    });
    assert!(
        trace.entered_failsafe(),
        "trace with failsafe step should report it"
    );
}

// ── (2) Invariants ──────────────────────────────────────────────────────────

#[test]
fn invalid_events_flags_zero_attempts() {
    let trace = OrchestrationTrace {
        solution_id: "synthetic".into(),
        events: vec![
            OrchestrationStepTrace {
                step_id: "good_step".into(),
                outcome: OrchestrationStepOutcome::Ok,
                attempts: 1,
                total_ms: 5,
            },
            OrchestrationStepTrace {
                step_id: "bad_step".into(),
                outcome: OrchestrationStepOutcome::Ok,
                attempts: 0,
                total_ms: 5,
            },
        ],
    };
    let invalid = trace.invalid_events();
    assert_eq!(invalid, vec!["bad_step"]);
}

#[test]
fn canonical_trace_passes_invariant_check() {
    let trace = canonical_trace();
    assert!(
        trace.invalid_events().is_empty(),
        "canonical trace should pass invariants, got: {:?}",
        trace.invalid_events(),
    );
}

// ── (3) Serde round-trip via proptest ───────────────────────────────────────

fn arb_outcome() -> impl Strategy<Value = OrchestrationStepOutcome> {
    prop_oneof![
        Just(OrchestrationStepOutcome::Ok),
        Just(OrchestrationStepOutcome::RetrySucceeded),
        Just(OrchestrationStepOutcome::FailsafeBudget),
        Just(OrchestrationStepOutcome::FailsafeAttempts),
    ]
}

fn arb_step_id() -> impl Strategy<Value = String> {
    // Lower-case ASCII identifiers like real step ids.
    proptest::string::string_regex("[a-z][a-z0-9_]{0,32}").unwrap()
}

fn arb_step_trace() -> impl Strategy<Value = OrchestrationStepTrace> {
    (arb_step_id(), arb_outcome(), 1u8..=255, any::<u32>()).prop_map(
        |(step_id, outcome, attempts, total_ms)| OrchestrationStepTrace {
            step_id,
            outcome,
            attempts,
            total_ms,
        },
    )
}

fn arb_trace() -> impl Strategy<Value = OrchestrationTrace> {
    (arb_step_id(), prop_vec(arb_step_trace(), 0..16)).prop_map(|(solution_id, events)| {
        OrchestrationTrace {
            solution_id,
            events,
        }
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    /// For arbitrary `OrchestrationTrace` values built with valid
    /// `attempts >= 1`, serde round-trip must preserve every field.
    #[test]
    fn trace_serde_roundtrip(trace in arb_trace()) {
        let json = serde_json::to_string(&trace).unwrap();
        let back: OrchestrationTrace = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(trace, back);
    }

    /// `invalid_events()` is total: it returns exactly the step_ids
    /// with `attempts == 0`. Confirmed by independently re-computing
    /// the set.
    #[test]
    fn invalid_events_is_total(events in prop_vec(arb_step_trace_unchecked(), 0..20)) {
        let trace = OrchestrationTrace {
            solution_id: "test".into(),
            events: events.clone(),
        };
        let computed: Vec<&str> = trace.invalid_events();
        let expected: Vec<&str> = events
            .iter()
            .filter(|e| e.attempts == 0)
            .map(|e| e.step_id.as_str())
            .collect();
        prop_assert_eq!(computed, expected);
    }
}

/// Like `arb_step_trace` but allows `attempts == 0` so the
/// `invalid_events_is_total` proptest can exercise the failing path.
fn arb_step_trace_unchecked() -> impl Strategy<Value = OrchestrationStepTrace> {
    (arb_step_id(), arb_outcome(), any::<u8>(), any::<u32>()).prop_map(
        |(step_id, outcome, attempts, total_ms)| OrchestrationStepTrace {
            step_id,
            outcome,
            attempts,
            total_ms,
        },
    )
}
