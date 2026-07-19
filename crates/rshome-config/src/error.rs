//! Validation error types with source location, severity, and stage metadata.

use serde::{Deserialize, Serialize};

// ── ValidationStage ───────────────────────────────────────────────────────────

/// The pipeline stage that produced a validation error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStage {
    PackageMerge,
    Substitutions,
    ExtendRemove,
    /// Stage 3.5 — resolve a selected solution variant against its parent
    /// solution's `variants[]` and surface its active-flag deltas.
    /// Added by the rshome-codegen-variants PRD Phase 1 T1.4.
    VariantResolution,
    ExternalComponents,
    Preload,
    ComponentLoading,
    SchemaValidation,
    IdResolution,
    FinalValidation,
    PinConflicts,
    ExclusiveGroup,
    ProfileValidation,
    SecretFields,
}

impl std::fmt::Display for ValidationStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::PackageMerge => "package_merge",
            Self::Substitutions => "substitutions",
            Self::ExtendRemove => "extend_remove",
            Self::VariantResolution => "variant_resolution",
            Self::ExternalComponents => "external_components",
            Self::Preload => "preload",
            Self::ComponentLoading => "component_loading",
            Self::SchemaValidation => "schema_validation",
            Self::IdResolution => "id_resolution",
            Self::FinalValidation => "final_validation",
            Self::PinConflicts => "pin_conflicts",
            Self::ExclusiveGroup => "exclusive_group",
            Self::ProfileValidation => "profile_validation",
            Self::SecretFields => "secret_fields",
        };
        f.write_str(s)
    }
}

// ── Severity ──────────────────────────────────────────────────────────────────

/// Severity level for a validation diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Informational note — does not fail validation.
    Info,
    /// Non-blocking warning — validation continues.
    Warning,
    /// Fatal error — pipeline cannot proceed past this stage.
    Error,
}

// ── ValidationError ───────────────────────────────────────────────────────────

/// A structured validation diagnostic with source location and context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    /// JSON-path-like source location, e.g. `"sensor[0].platform.dht.pin"`.
    pub path: String,
    /// Human-readable description of the problem.
    pub message: String,
    /// Severity level.
    pub severity: Severity,
    /// Optional remediation suggestion ("Did you mean 'temperature'?").
    pub suggestion: Option<String>,
    /// Which pipeline stage produced this diagnostic.
    pub stage: ValidationStage,
}

impl ValidationError {
    /// Construct a fatal `Error`-severity diagnostic.
    pub fn error(
        stage: ValidationStage,
        path: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
            severity: Severity::Error,
            suggestion: None,
            stage,
        }
    }

    /// Construct a non-fatal `Warning`-severity diagnostic.
    pub fn warning(
        stage: ValidationStage,
        path: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
            severity: Severity::Warning,
            suggestion: None,
            stage,
        }
    }

    /// Construct an `Info`-severity diagnostic.
    pub fn info(
        stage: ValidationStage,
        path: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
            severity: Severity::Info,
            suggestion: None,
            stage,
        }
    }

    /// Attach a suggestion string and return `self` for chaining.
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }

    /// Returns `true` if this error is fatal (stops pipeline progression).
    pub fn is_fatal(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{:?}@{}] {}: {}",
            self.severity, self.stage, self.path, self.message
        )?;
        if let Some(s) = &self.suggestion {
            write!(f, " (hint: {s})")?;
        }
        Ok(())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Returns `true` if any entry in `errors` has `Severity::Error`.
pub fn has_errors(errors: &[ValidationError]) -> bool {
    errors.iter().any(|e| e.severity == Severity::Error)
}

/// Split `errors` into `(fatal, non_fatal)` partitions.
pub fn partition_errors(
    errors: Vec<ValidationError>,
) -> (Vec<ValidationError>, Vec<ValidationError>) {
    errors
        .into_iter()
        .partition(|e| e.severity == Severity::Error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_constructor_sets_severity() {
        let e = ValidationError::error(ValidationStage::Preload, "esphome.platform", "bad chip");
        assert_eq!(e.severity, Severity::Error);
        assert!(e.is_fatal());
        assert_eq!(e.stage, ValidationStage::Preload);
    }

    #[test]
    fn warning_constructor_not_fatal() {
        let e = ValidationError::warning(ValidationStage::Preload, "x", "minor issue");
        assert_eq!(e.severity, Severity::Warning);
        assert!(!e.is_fatal());
    }

    #[test]
    fn info_constructor_not_fatal() {
        let e = ValidationError::info(ValidationStage::Substitutions, "x", "note");
        assert_eq!(e.severity, Severity::Info);
        assert!(!e.is_fatal());
    }

    #[test]
    fn with_suggestion_attaches_hint() {
        let e = ValidationError::error(ValidationStage::SchemaValidation, "x", "bad")
            .with_suggestion("Did you mean 'temperature'?");
        assert_eq!(e.suggestion.as_deref(), Some("Did you mean 'temperature'?"));
    }

    #[test]
    fn display_includes_path_and_message() {
        let e = ValidationError::error(ValidationStage::PinConflicts, "sensor[0].pin", "conflict");
        let s = e.to_string();
        assert!(s.contains("sensor[0].pin"));
        assert!(s.contains("conflict"));
    }

    #[test]
    fn display_includes_suggestion_when_present() {
        let e = ValidationError::warning(ValidationStage::ComponentLoading, "x", "warn")
            .with_suggestion("try this");
        let s = e.to_string();
        assert!(s.contains("try this"));
    }

    #[test]
    fn has_errors_returns_true_when_error_present() {
        let errors = vec![
            ValidationError::warning(ValidationStage::Preload, "x", "w"),
            ValidationError::error(ValidationStage::Preload, "y", "e"),
        ];
        assert!(has_errors(&errors));
    }

    #[test]
    fn has_errors_returns_false_when_only_warnings() {
        let errors = vec![
            ValidationError::warning(ValidationStage::Preload, "x", "w"),
            ValidationError::info(ValidationStage::Preload, "y", "i"),
        ];
        assert!(!has_errors(&errors));
    }

    #[test]
    fn severity_ordering() {
        assert!(Severity::Error > Severity::Warning);
        assert!(Severity::Warning > Severity::Info);
    }

    #[test]
    fn partition_errors_separates_fatal() {
        let errors = vec![
            ValidationError::error(ValidationStage::Preload, "a", "e"),
            ValidationError::warning(ValidationStage::Preload, "b", "w"),
            ValidationError::info(ValidationStage::Preload, "c", "i"),
        ];
        let (fatal, non_fatal) = partition_errors(errors);
        assert_eq!(fatal.len(), 1);
        assert_eq!(non_fatal.len(), 2);
    }
}
