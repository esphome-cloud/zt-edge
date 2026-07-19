//! Migration shim integration lint per
//! Task 4.2.
//!
//! Three contract layers:
//!
//! 1. **Detection** — `has_legacy_implementation_family()` and
//!    `read_legacy_implementation_family()` correctly classify each
//!    of 50 legacy + 100 modern fixtures.
//! 2. **Migration** — `accept_legacy_solution_json()` strips the
//!    legacy key while preserving everything else. Re-serializing the
//!    cleaned value never includes `implementation_family`.
//! 3. **Idempotence** — the shim is safe to call multiple times; the
//!    second call is a no-op on already-cleaned input.
//!
//! What this lint does NOT cover (deferred per scope cap):
//!
//!   - `metrics` crate counter (`legacy_implementation_family_observed_total`)
//!     — would require adding the `metrics` dep + a `metrics-test`
//!     subscriber. The tracing warning is in place; the counter is a
//!     follow-up.
//!   - `tracing-test` subscriber assertions on warning emission per
//!     fixture — tracing tests sufficient via the unit tests in
//!     `solution_legacy.rs`.

use std::path::PathBuf;

use serde_json::Value;

use rshome_schema::solution_legacy::{
    accept_legacy_solution_json, has_legacy_implementation_family,
    read_legacy_implementation_family,
};

fn fixture_path(name: &str) -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest).join("tests/fixtures").join(name)
}

fn load_fixtures(name: &str) -> Vec<Value> {
    let raw = std::fs::read_to_string(fixture_path(name))
        .unwrap_or_else(|e| panic!("read {}: {}", name, e));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {}: {}", name, e))
}

// ── (1) Detection across 50 legacy + 100 modern fixtures ────────────────────

#[test]
fn legacy_corpus_all_carry_implementation_family() {
    let legacy = load_fixtures("legacy_implementation_family_inputs.json");
    assert_eq!(
        legacy.len(),
        50,
        "legacy corpus must have exactly 50 fixtures"
    );
    for (i, v) in legacy.iter().enumerate() {
        assert!(
            has_legacy_implementation_family(v),
            "legacy fixture {} missing implementation_family: {}",
            i,
            v,
        );
        let value = read_legacy_implementation_family(v);
        assert!(
            value.is_some(),
            "legacy fixture {} carries implementation_family but read returned None",
            i,
        );
    }
}

#[test]
fn modern_corpus_carries_no_implementation_family() {
    let modern = load_fixtures("v1_solutions_corpus.json");
    assert_eq!(
        modern.len(),
        100,
        "modern corpus must have exactly 100 fixtures"
    );
    let mut leakers: Vec<usize> = Vec::new();
    for (i, v) in modern.iter().enumerate() {
        if has_legacy_implementation_family(v) {
            leakers.push(i);
        }
    }
    assert!(
        leakers.is_empty(),
        "modern fixtures unexpectedly contain implementation_family: {:?}",
        leakers,
    );
}

// ── (2) Migration: shim strips the legacy key ──────────────────────────────

#[test]
fn shim_strips_legacy_key_on_all_50_legacy_fixtures() {
    let legacy = load_fixtures("legacy_implementation_family_inputs.json");
    for (i, v) in legacy.iter().enumerate() {
        // Capture the original size and non-legacy keys for comparison.
        let original_obj = v.as_object().expect("fixture must be an object");
        let other_keys: std::collections::BTreeSet<&str> = original_obj
            .keys()
            .filter(|k| k.as_str() != "implementation_family")
            .map(|k| k.as_str())
            .collect();

        let cleaned = accept_legacy_solution_json(v.clone());

        // The legacy key is gone.
        assert!(
            !has_legacy_implementation_family(&cleaned),
            "fixture {} retained legacy key after shim",
            i,
        );
        // Every other key is preserved.
        let cleaned_obj = cleaned.as_object().unwrap();
        let cleaned_keys: std::collections::BTreeSet<&str> =
            cleaned_obj.keys().map(|k| k.as_str()).collect();
        assert_eq!(
            cleaned_keys, other_keys,
            "fixture {}: non-legacy keys not preserved",
            i,
        );
    }
}

#[test]
fn shim_round_trip_to_json_never_contains_legacy_key() {
    let legacy = load_fixtures("legacy_implementation_family_inputs.json");
    for (i, v) in legacy.iter().enumerate() {
        let cleaned = accept_legacy_solution_json(v.clone());
        let serialized = serde_json::to_string(&cleaned).unwrap();
        assert!(
            !serialized.contains("\"implementation_family\""),
            "fixture {} re-serialized JSON contains forbidden key: {}",
            i,
            serialized,
        );
    }
}

// ── (3) Idempotence ─────────────────────────────────────────────────────────

#[test]
fn shim_is_idempotent_on_modern_inputs() {
    let modern = load_fixtures("v1_solutions_corpus.json");
    for (i, v) in modern.iter().enumerate() {
        let cleaned = accept_legacy_solution_json(v.clone());
        assert_eq!(
            cleaned, *v,
            "modern fixture {} unexpectedly modified by shim",
            i,
        );
    }
}

#[test]
fn shim_is_idempotent_on_pre_cleaned_legacy_inputs() {
    let legacy = load_fixtures("legacy_implementation_family_inputs.json");
    for (i, v) in legacy.iter().enumerate() {
        let cleaned_once = accept_legacy_solution_json(v.clone());
        let cleaned_twice = accept_legacy_solution_json(cleaned_once.clone());
        assert_eq!(
            cleaned_once, cleaned_twice,
            "shim not idempotent on fixture {} (cleaned-once differs from cleaned-twice)",
            i,
        );
    }
}
