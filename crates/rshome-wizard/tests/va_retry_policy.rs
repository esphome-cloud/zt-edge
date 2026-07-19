//! `RetryPolicy` + `OrchestrationStep` schema lint per
//! Task 2.1.
//!
//! Three contract layers:
//!
//! 1. **Schema invariants** — `RetryPolicy::new()` accepts only well-
//!    formed (max_attempts, initial, max, budget) tuples. Hand-written
//!    edge cases below + 10K proptest fuzz exercise the boundary.
//! 2. **No-regression on V&A registry** — the 37 existing V&A
//!    solutions ship `retry_policy: None` and `parallel_group: None`
//!    on every `OrchestrationStep` (boot-once legacy semantics).
//!    Their serialized JSON must NOT contain `retry_policy` or
//!    `parallel_group` keys.
//! 3. **Size guarantee** — `size_of::<RetryPolicy>() == 16` enforced
//!    at compile time via `const _: () = assert!(...)` in
//!    `rshome-schema::orchestration`. This lint just round-trips a
//!    concrete instance to confirm the layout works end-to-end.

use proptest::prelude::*;

use rshome_schema::orchestration::{RetryPolicy, RetryPolicyError};
use rshome_schema::platform::DomainKind;
use rshome_schema::solution::default_solution_registry;

// ── (1) Schema invariants ───────────────────────────────────────────────────

#[test]
fn retry_policy_size_is_16() {
    assert_eq!(std::mem::size_of::<RetryPolicy>(), 16);
}

#[test]
fn retry_policy_serde_roundtrip() {
    let p = RetryPolicy::new(3, 5, 50, 200).unwrap();
    let json = serde_json::to_string(&p).unwrap();
    let back: RetryPolicy = serde_json::from_str(&json).unwrap();
    assert_eq!(p, back);
}

#[test]
fn retry_policy_rejects_known_violations() {
    // Boundary cases the schema must reject.
    assert!(matches!(
        RetryPolicy::new(0, 10, 100, 1000).unwrap_err(),
        RetryPolicyError::ZeroMaxAttempts
    ));
    assert!(matches!(
        RetryPolicy::new(3, 200, 100, 1000).unwrap_err(),
        RetryPolicyError::InitialExceedsMax { .. }
    ));
    assert!(matches!(
        RetryPolicy::new(5, 50, 250, 1000).unwrap_err(),
        RetryPolicyError::MaxBackoffExceedsPerAttemptBudget { .. }
    ));
}

// ── (2) No-regression on V&A registry ───────────────────────────────────────

#[test]
fn every_va_orchestration_step_has_no_retry_policy_today() {
    // Phase 2 Task 2.1 lands the SCHEMA. Phase 2 Task 2.2+ wires the
    // runtime + the firmware codegen template; until then NO existing
    // V&A solution should populate retry_policy or parallel_group.
    let reg = default_solution_registry();
    let mut violations: Vec<String> = Vec::new();
    for sol in reg.all() {
        if sol.domain != Some(DomainKind::VehicleAircraftControl) {
            continue;
        }
        for (idx, step) in sol.fixed_orchestration.iter().enumerate() {
            if step.retry_policy.is_some() {
                violations.push(format!(
                    "{}::fixed_orchestration[{}] = {:?} — retry_policy set unexpectedly",
                    sol.id, idx, step.id,
                ));
            }
            if step.parallel_group.is_some() {
                violations.push(format!(
                    "{}::fixed_orchestration[{}] = {:?} — parallel_group set unexpectedly",
                    sol.id, idx, step.id,
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "Phase 2 Task 2.1 expects no V&A solution to use retry_policy / parallel_group yet — \
         the firmware runtime support lands in Task 2.2. Violations:\n  {}",
        violations.join("\n  "),
    );
}

#[test]
fn va_orchestration_step_json_omits_unset_optional_fields() {
    // Backward-compat guard: serializing a default OrchestrationStep
    // must NOT include `retry_policy` or `parallel_group` keys in the
    // JSON output. Catches a regression that drops the
    // `skip_serializing_if = "Option::is_none"` attribute.
    let reg = default_solution_registry();
    let mut violations: Vec<String> = Vec::new();
    for sol in reg.all() {
        if sol.domain != Some(DomainKind::VehicleAircraftControl) {
            continue;
        }
        for step in &sol.fixed_orchestration {
            let json = serde_json::to_string(step).expect("step must serialize");
            for forbidden in ["\"retry_policy\"", "\"parallel_group\""] {
                if json.contains(forbidden) {
                    violations.push(format!(
                        "{}: step {:?} serialized JSON contains {} — \
                         skip_serializing_if dropped?",
                        sol.id, step.id, forbidden,
                    ));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "OrchestrationStep optional fields leaked into JSON:\n  {}",
        violations.join("\n  "),
    );
}

// ── (3) 10K proptest fuzz on RetryPolicy::new() ─────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    /// For arbitrary (attempts, initial, max, budget) inputs,
    /// `RetryPolicy::new()` either returns `Ok` (and the returned
    /// policy round-trips through serde) or returns `Err` with a
    /// specific error variant matching one of the three failure
    /// conditions. No panics, no Err-without-cause.
    #[test]
    fn retry_policy_new_classification_is_total(
        attempts in any::<u8>(),
        initial in any::<u32>(),
        max in any::<u32>(),
        budget in any::<u32>(),
    ) {
        let result = RetryPolicy::new(attempts, initial, max, budget);
        match result {
            Ok(p) => {
                // Returned Ok ⇒ all 3 invariants hold.
                prop_assert!(p.max_attempts >= 1);
                prop_assert!(p.backoff_ms_initial <= p.backoff_ms_max);
                let per_attempt = p.total_budget_ms / p.max_attempts as u32;
                prop_assert!(p.backoff_ms_max <= per_attempt);
                // Round-trip cleanly.
                let json = serde_json::to_string(&p).unwrap();
                let back: RetryPolicy = serde_json::from_str(&json).unwrap();
                prop_assert_eq!(p, back);
            }
            Err(RetryPolicyError::ZeroMaxAttempts) => {
                prop_assert_eq!(attempts, 0);
            }
            Err(RetryPolicyError::InitialExceedsMax { initial: i, max: m }) => {
                prop_assert_eq!(i, initial);
                prop_assert_eq!(m, max);
                prop_assert!(initial > max);
                prop_assert!(attempts > 0); // ZeroMaxAttempts would have fired first
            }
            Err(RetryPolicyError::MaxBackoffExceedsPerAttemptBudget {
                max: m,
                budget: b,
                attempts: a,
                per_attempt_budget: pab,
            }) => {
                prop_assert_eq!(m, max);
                prop_assert_eq!(b, budget);
                prop_assert_eq!(a, attempts);
                prop_assert_eq!(pab, budget / attempts as u32);
                prop_assert!(max > pab);
                prop_assert!(attempts > 0);
                prop_assert!(initial <= max);
            }
        }
    }

    /// Generative invariant: when proptest builds a (initial, max,
    /// budget, attempts) tuple satisfying the invariants by
    /// construction, `RetryPolicy::new()` accepts it.
    #[test]
    fn retry_policy_new_accepts_well_formed_inputs(
        attempts in 1u8..=255,
        budget in 1u32..=u32::MAX / 256,
    ) {
        // Construct (initial, max) inside [0, budget/attempts].
        let per_attempt = budget / attempts as u32;
        let max = per_attempt;
        let initial = per_attempt / 2;
        prop_assert!(RetryPolicy::new(attempts, initial, max, budget).is_ok());
    }
}
