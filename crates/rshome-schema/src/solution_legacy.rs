//! Legacy `implementation_family` migration shim per
//! Task 4.2.
//!
//! `SolutionDefinition.implementation_family: Option<String>` was
//! retired by va-residuals 2026-04-21 in favor of the typed
//! `family: Option<ImplementationFamily>`. Wizard state persisted
//! before that retirement (e.g., browser localStorage) may still carry
//! the legacy field. This module is the one-release-window grace:
//! accept the legacy input, emit a deprecation warning, and strip the
//! field before deserialization so the modern `SolutionDefinition`
//! struct can deserialize cleanly.
//!
//! Lifecycle:
//! - **Phase 4 / Task 4.2 (this file, 2026-05-15):** introduce the shim.
//! - **Phase 4 / Task 4.3 (calendar-bound, +6 weeks):** drop the shim
//!   and any remaining `implementation_family` allow-list entries.
//!
//! Why a helper instead of a custom Deserialize impl: SolutionDefinition
//! has 33+ fields and 200+ literal instantiations throughout
//! `solution.rs`. Modifying the struct to add a hidden
//! `_legacy_implementation_family` field would require bulk-editing
//! every literal — a high-risk operation (see the Task 2.1 OrchestrationStep
//! incident). The helper approach achieves the same migration
//! semantics without touching the struct.
//!
//! Usage:
//!
//! ```ignore
//! use rshome_schema::solution_legacy::accept_legacy_solution_json;
//! use rshome_schema::SolutionDefinition;
//!
//! let cleaned = accept_legacy_solution_json(raw_value);
//! let sol: SolutionDefinition = serde_json::from_value(cleaned)?;
//! ```

use serde_json::Value;

/// Has this JSON value been observed to carry the legacy
/// `implementation_family` key? Read-only; doesn't mutate.
pub fn has_legacy_implementation_family(value: &Value) -> bool {
    value
        .as_object()
        .map(|o| o.contains_key("implementation_family"))
        .unwrap_or(false)
}

/// Read the legacy `implementation_family` value as a string, if
/// present. Returns `None` if the key is absent or the value isn't a
/// JSON string.
pub fn read_legacy_implementation_family(value: &Value) -> Option<String> {
    value
        .as_object()
        .and_then(|o| o.get("implementation_family"))
        .and_then(|v| v.as_str())
        .map(String::from)
}

/// Strip the legacy `implementation_family` key (if present) and emit
/// a deprecation warning. The returned `Value` is safe to feed into
/// `serde_json::from_value::<SolutionDefinition>(...)` without the
/// legacy field appearing in the modern serialized form.
///
/// Idempotent: calling on a value without the legacy key is a no-op
/// (no warning, value returned unchanged).
///
/// **Forbidden behavior:** the warning fires ONLY when the legacy key
/// is present. Modern v1 inputs produce zero warnings.
pub fn accept_legacy_solution_json(mut value: Value) -> Value {
    if let Some(obj) = value.as_object_mut() {
        if let Some(legacy) = obj.remove("implementation_family") {
            // Emit a tracing warning so ops can detect lingering legacy
            // state in the wild. The target is
            // `rshome-schema::deprecated` so subscribers can filter
            // for migration signals across all deprecated fields.
            let legacy_str = legacy
                .as_str()
                .map(String::from)
                .unwrap_or_else(|| legacy.to_string());
            tracing::warn!(
                target: "rshome-schema::deprecated",
                solution_id = %obj
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("<unknown>"),
                legacy_value = %legacy_str,
                "implementation_family is deprecated and will be removed in v0.6.0; \
                 migrating to typed `family` field (Phase 4 Task 4.3)"
            );
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn has_legacy_detects_presence() {
        let v = json!({"implementation_family": "betaflight"});
        assert!(has_legacy_implementation_family(&v));
    }

    #[test]
    fn has_legacy_returns_false_on_modern_input() {
        let v = json!({"family": "betaflight"});
        assert!(!has_legacy_implementation_family(&v));
    }

    #[test]
    fn read_legacy_returns_string_value() {
        let v = json!({"implementation_family": "betaflight"});
        assert_eq!(
            read_legacy_implementation_family(&v),
            Some("betaflight".to_string())
        );
    }

    #[test]
    fn read_legacy_returns_none_when_absent() {
        let v = json!({"family": "ardupilot"});
        assert_eq!(read_legacy_implementation_family(&v), None);
    }

    #[test]
    fn accept_legacy_strips_key() {
        let v = json!({
            "id": "quad_stabilizer_solution",
            "implementation_family": "px4",
            "label": "Quad"
        });
        let cleaned = accept_legacy_solution_json(v);
        assert!(!has_legacy_implementation_family(&cleaned));
        // Other keys preserved.
        assert_eq!(cleaned["id"], "quad_stabilizer_solution");
        assert_eq!(cleaned["label"], "Quad");
    }

    #[test]
    fn accept_legacy_idempotent_on_modern_input() {
        let v = json!({"id": "test", "family": "ardupilot"});
        let cleaned = accept_legacy_solution_json(v.clone());
        assert_eq!(cleaned, v);
    }

    #[test]
    fn accept_legacy_on_non_object_passes_through() {
        let v = json!(42);
        let cleaned = accept_legacy_solution_json(v.clone());
        assert_eq!(cleaned, v);
    }
}
