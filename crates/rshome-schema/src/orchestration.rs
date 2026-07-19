//! Retry policy + parallel-group annotations for `OrchestrationStep`.
//!
//! Phase 2 / Task 2.1 of the vehicle-aircraft-control-design PRD: each
//! `OrchestrationStep` can declare a `RetryPolicy` to recover from
//! transient init failures (I²C EAGAIN, sensor not-yet-ready, etc.)
//! before falling back to `rshome_failsafe_enter()`. A `parallel_group`
//! marker lets independent init steps run concurrently to amortize
//! init latency on multi-component vehicles.
//!
//! The contract is intentionally narrow: this module defines the
//! data shape + invariants. Honoring the policy at firmware run time
//! is Task 2.2; budget-check at config validation time is Task 2.1's
//! `stage9_orchestration_budget_check` follow-up (out of scope here).

use serde::{Deserialize, Serialize};

use schemars::JsonSchema;

/// Bounded-retry policy for a single `OrchestrationStep`.
///
/// Layout: `u8` (1 byte) + 3 padding + 3 × `u32` (12 bytes) = **16 bytes**,
/// guaranteed by the `size_of` assertion below. The compact layout
/// matters for codegen — every step in a 64-step orchestration carries
/// at most this many bytes of retry metadata.
///
/// Invariants enforced by `RetryPolicy::new`:
/// - `max_attempts >= 1`
/// - `backoff_ms_initial <= backoff_ms_max`
/// - `backoff_ms_max <= total_budget_ms / max_attempts` (the average
///   per-attempt budget cannot exceed `backoff_ms_max`, otherwise the
///   step could never complete inside the budget even if every retry
///   succeeds instantly).
///
/// Note: this is the SCHEMA-level invariant. The Phase 2 PRD also
/// asks for a CONFIG-level invariant
/// (`total_budget_ms <= watchdog_ms / 2`) which lives in
/// `rshome-config::stage9_orchestration_budget_check` — out of this
/// crate's scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct RetryPolicy {
    /// Maximum attempts including the initial one. Must be `>= 1`.
    pub max_attempts: u8,
    /// Initial backoff in milliseconds. The first retry waits this long;
    /// subsequent retries double up to `backoff_ms_max`.
    pub backoff_ms_initial: u32,
    /// Cap on per-attempt backoff. Prevents exponential explosion on
    /// `max_attempts >= 10`.
    pub backoff_ms_max: u32,
    /// Total wall-clock budget (across all attempts + backoffs)
    /// before the step is declared failed. Honored by the runtime;
    /// `stage9_orchestration_budget_check` additionally caps this
    /// against the host solution's watchdog.
    pub total_budget_ms: u32,
}

// Compile-time check: PRD Task 2.1 acceptance #1.
const _: () = assert!(
    std::mem::size_of::<RetryPolicy>() == 16,
    "RetryPolicy size drifted from the documented 16 bytes — update the \
     codegen layout assumption or restore the field set.",
);

/// Schema-level invariant violations rejected by `RetryPolicy::new`.
/// Config-level violations (budget vs. host watchdog) live in
/// `rshome-config`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RetryPolicyError {
    #[error("max_attempts must be >= 1, got 0")]
    ZeroMaxAttempts,
    #[error("backoff_ms_initial ({initial}) > backoff_ms_max ({max})")]
    InitialExceedsMax { initial: u32, max: u32 },
    #[error(
        "backoff_ms_max ({max}) > total_budget_ms ({budget}) / max_attempts ({attempts}) = \
         {per_attempt_budget} — per-attempt budget cannot exceed max backoff"
    )]
    MaxBackoffExceedsPerAttemptBudget {
        max: u32,
        budget: u32,
        attempts: u8,
        per_attempt_budget: u32,
    },
}

impl RetryPolicy {
    /// Construct a validated `RetryPolicy`. Returns `Err` if the
    /// schema invariants are violated.
    pub fn new(
        max_attempts: u8,
        backoff_ms_initial: u32,
        backoff_ms_max: u32,
        total_budget_ms: u32,
    ) -> Result<Self, RetryPolicyError> {
        if max_attempts == 0 {
            return Err(RetryPolicyError::ZeroMaxAttempts);
        }
        if backoff_ms_initial > backoff_ms_max {
            return Err(RetryPolicyError::InitialExceedsMax {
                initial: backoff_ms_initial,
                max: backoff_ms_max,
            });
        }
        let per_attempt_budget = total_budget_ms / max_attempts as u32;
        if backoff_ms_max > per_attempt_budget {
            return Err(RetryPolicyError::MaxBackoffExceedsPerAttemptBudget {
                max: backoff_ms_max,
                budget: total_budget_ms,
                attempts: max_attempts,
                per_attempt_budget,
            });
        }
        Ok(Self {
            max_attempts,
            backoff_ms_initial,
            backoff_ms_max,
            total_budget_ms,
        })
    }

    /// Returns `true` if `self` satisfies all schema invariants. Provided
    /// so callers that deserialize from JSON (where invariants aren't
    /// re-checked) can opt into a runtime guard.
    pub fn is_valid(&self) -> bool {
        Self::new(
            self.max_attempts,
            self.backoff_ms_initial,
            self.backoff_ms_max,
            self.total_budget_ms,
        )
        .is_ok()
    }
}

// ── Phase 2 Task 2.3: orchestration trace event schema ─────────────────────
//
// Mirrors the C-side `rshome_step_trace_t` emitted by the orchestrator
// template (`orchestrator.c.tera`). Each completed init produces one
// `OrchestrationTrace` containing one `OrchestrationStepTrace` per
// declared step. The trace is published to the `rshome_events` bus
// when init finishes (success or failsafe) and consumed by the browser
// workspace UI via WebRTC.

/// Per-step outcome. C-side: `rshome_step_outcome_t`. Serializes as a
/// snake_case string so the JSON shape matches what the workspace UI
/// expects without a custom (de)serializer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OrchestrationStepOutcome {
    /// Succeeded on the first attempt — no retries fired.
    Ok,
    /// Succeeded after at least one retry. Distinguishes "happy path"
    /// from "transient fault that recovered" for the metrics counter
    /// `orchestration_step_retry_succeeded_total{component}`.
    RetrySucceeded,
    /// `total_budget_ms` elapsed before the step could finish.
    /// `rshome_failsafe_enter()` was called after this step.
    FailsafeBudget,
    /// `max_attempts` exhausted (every attempt failed). `rshome_failsafe_enter()`
    /// was called after this step.
    FailsafeAttempts,
}

impl OrchestrationStepOutcome {
    /// `true` if the outcome triggered `rshome_failsafe_enter()`. The
    /// orchestrator handed control back to the failsafe path AFTER
    /// the trace was emitted.
    pub fn is_failsafe(self) -> bool {
        matches!(
            self,
            OrchestrationStepOutcome::FailsafeBudget | OrchestrationStepOutcome::FailsafeAttempts
        )
    }
}

/// One row of the trace — what happened to a single step. Field order
/// matches the C-side struct so consumers can pun the layouts when
/// performance matters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OrchestrationStepTrace {
    /// Stable id matching `OrchestrationStep.id`.
    pub step_id: String,
    /// Final outcome — exactly one of the 4 enum variants.
    pub outcome: OrchestrationStepOutcome,
    /// Attempts made (1..=max_attempts). Invariant: never 0 for a step
    /// that actually executed — Task 2.3 acceptance #2 forbids it.
    pub attempts: u8,
    /// Wall-clock milliseconds from step entry to outcome.
    pub total_ms: u32,
}

/// The full trace for one init run. Emitted on the `rshome_events` bus
/// when the orchestrator's `init_<solution>` driver returns (whether
/// it succeeded or escalated to failsafe).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OrchestrationTrace {
    /// Solution id whose `init_<solution>` driver produced this trace.
    pub solution_id: String,
    /// Per-step traces in the order the orchestrator executed them.
    /// Length equals the solution's declared step count
    /// (`steps.length == events.length` — Task 2.3 acceptance #1).
    pub events: Vec<OrchestrationStepTrace>,
}

impl OrchestrationTrace {
    /// `true` if any step ended in a failsafe outcome.
    pub fn entered_failsafe(&self) -> bool {
        self.events.iter().any(|e| e.outcome.is_failsafe())
    }

    /// Validate the schema-level invariants asserted in PRD §10.2:
    /// - `events.length == steps.length` enforced at the call site;
    /// - every event has `attempts >= 1`;
    /// - `total_ms <= total_budget_ms` enforced at runtime, not here
    ///   (we don't carry the policy in this struct).
    ///
    /// Returns the offending `step_id`s, empty vec when clean.
    pub fn invalid_events(&self) -> Vec<&str> {
        self.events
            .iter()
            .filter(|e| e.attempts == 0)
            .map(|e| e.step_id.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_is_16_bytes() {
        assert_eq!(std::mem::size_of::<RetryPolicy>(), 16);
    }

    #[test]
    fn new_rejects_zero_max_attempts() {
        let err = RetryPolicy::new(0, 10, 100, 1000).unwrap_err();
        assert!(matches!(err, RetryPolicyError::ZeroMaxAttempts));
    }

    #[test]
    fn new_rejects_initial_above_max() {
        let err = RetryPolicy::new(3, 200, 100, 1000).unwrap_err();
        assert!(matches!(err, RetryPolicyError::InitialExceedsMax { .. }));
    }

    #[test]
    fn new_rejects_max_above_per_attempt_budget() {
        // total_budget / max_attempts = 1000 / 5 = 200. backoff_ms_max = 250 > 200.
        let err = RetryPolicy::new(5, 50, 250, 1000).unwrap_err();
        assert!(matches!(
            err,
            RetryPolicyError::MaxBackoffExceedsPerAttemptBudget { .. }
        ));
    }

    #[test]
    fn new_accepts_canonical_values() {
        // Per Phase-2 PRD §"worked example": wheeled_4wd_diff with 5 init
        // steps, watchdog 500ms, total budget 200ms.
        let p = RetryPolicy::new(3, 5, 50, 200).unwrap();
        assert_eq!(p.max_attempts, 3);
        assert_eq!(p.backoff_ms_initial, 5);
        assert_eq!(p.backoff_ms_max, 50);
        assert_eq!(p.total_budget_ms, 200);
    }

    #[test]
    fn serde_roundtrip() {
        let p = RetryPolicy::new(3, 5, 50, 200).unwrap();
        let json = serde_json::to_string(&p).unwrap();
        let back: RetryPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }
}
