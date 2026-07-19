//! Stage 3.5 — Variant resolution.
//!
//! If the user's `RawConfig.esphome` declares a `solution` whose
//! registry definition has a non-empty `variants[]`, this stage reads
//! `esphome.solution_variant`, looks up the matching
//! [`SolutionVariantDefinition`], and produces a
//! [`VariantResolution`] that downstream pipeline steps (chiefly the
//! `active_flags` computation at the end of the pipeline) consume to
//! apply variant-level deltas.
//!
//! Three error kinds land here:
//!
//! | Kind | Severity | Condition |
//! |---|---|---|
//! | `MissingRequiredVariantId` | Error | solution declares variants but `solution_variant` is `None` |
//! | `UnknownVariantId` | Error | `solution_variant` set but not in `solution.variants[]` |
//! | `VariantAppliedToNoVariantSolution` | Warning | `solution_variant` set on a solution with no variants |
//!
//! Unknown `solution` ids are **not** reported here — that's the job
//! of Stage 9 (`FinalValidation`); double-reporting would just noise
//! up the error list.
//!
//! Capability validation (variant's `required_caps` vs. the selected
//! module's capability set) is intentionally deferred to a later pass
//! — at stage 3.5 the module has not yet been resolved. The PRD calls
//! this out as Phase 2b followup work.
//!
//! Added by the rshome-codegen-variants PRD Phase 1 T1.2.

use rshome_schema::solution::SolutionRegistry;

use crate::error::{ValidationError, ValidationStage};
use crate::raw::{ComponentConfig, RawConfig};

/// Variant-resolution output propagated forward through the pipeline.
/// Empty by default; populated only when a solution declares variants
/// and the user picked a valid one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VariantResolution {
    /// Mirrors `config.esphome.solution`. Propagated to `ValidatedConfig`
    /// for snapshot-naming + diagnostics.
    pub solution_id: Option<String>,
    /// Mirrors `config.esphome.solution_variant` after validation. Only
    /// non-`None` when the id was validated against the solution's
    /// `variants[]`.
    pub variant_id: Option<String>,
    /// Active-flag additions from the resolved variant. Applied during
    /// the `ValidatedConfig.active_flags` assembly step.
    pub active_flag_add: Vec<String>,
    /// Active-flag removals from the resolved variant. Applied during
    /// the same assembly step; removals win over additions within this
    /// same variant to avoid ambiguous deltas.
    pub active_flag_remove: Vec<String>,
}

/// Stage 3.5 entry point — see module-level docs.
///
/// Takes `&mut RawConfig` because a happy-path resolution applies the
/// variant's `add_components` + `remove_components` deltas to
/// `config.components`, so downstream stages (Stage 6 AUTO_LOAD in
/// particular) walk the post-overlay component list.
pub fn stage_3_5_resolve_variant(
    config: &mut RawConfig,
    solution_registry: &SolutionRegistry,
) -> (VariantResolution, Vec<ValidationError>) {
    let mut errors = Vec::new();
    let mut resolution = VariantResolution::default();

    let Some(solution_id) = config.esphome.solution.as_ref() else {
        return (resolution, errors);
    };
    resolution.solution_id = Some(solution_id.clone());

    // Unknown solution is reported by Stage 9 (FinalValidation) — don't
    // double-report here. Just drop out.
    let Some(sol) = solution_registry.get(solution_id) else {
        return (resolution, errors);
    };

    let declared_variant = config.esphome.solution_variant.as_ref();

    if sol.variants.is_empty() {
        if let Some(vid) = declared_variant {
            errors.push(ValidationError::warning(
                ValidationStage::VariantResolution,
                "esphome.solution_variant",
                format!(
                    "solution '{solution_id}' declares no variants — ignoring \
                     variant id '{vid}'",
                ),
            ));
        }
        return (resolution, errors);
    }

    let available: Vec<String> = sol.variants.iter().map(|v| v.id.clone()).collect();

    let Some(variant_id) = declared_variant else {
        errors.push(
            ValidationError::error(
                ValidationStage::VariantResolution,
                "esphome.solution_variant",
                format!(
                    "solution '{solution_id}' requires a variant selection \
                     but `solution_variant` is not set",
                ),
            )
            .with_suggestion(format!("available variants: {}", available.join(", "))),
        );
        return (resolution, errors);
    };

    let Some(variant) = sol.variants.iter().find(|v| &v.id == variant_id) else {
        errors.push(
            ValidationError::error(
                ValidationStage::VariantResolution,
                "esphome.solution_variant",
                format!("solution '{solution_id}' does not declare a variant '{variant_id}'",),
            )
            .with_suggestion(format!("available variants: {}", available.join(", "))),
        );
        // Keep `variant_id` out of the resolution — the caller should
        // treat "unknown variant" the same as "no variant".
        return (resolution, errors);
    };

    resolution.variant_id = Some(variant.id.clone());
    resolution.active_flag_add = variant.active_flag_add.clone();
    resolution.active_flag_remove = variant.active_flag_remove.clone();

    // Apply `add_components` / `remove_components` to `config.components`
    // so Stage 6's AUTO_LOAD walk sees the post-overlay component list.
    // rshome-codegen-variants PRD Phase 5 T5.2.
    //
    // Semantics:
    //   - `add_components` — each entry is a component_type name. Append a
    //     fresh empty-config ComponentConfig iff no existing entry shares
    //     that `component_type`. User-supplied configs always win.
    //   - `remove_components` — drop the FIRST entry whose
    //     `component_type` matches. Subsequent entries with the same
    //     type (rare; shouldn't happen pre-stage-3) are left alone to
    //     stay consistent with `s3_extend_remove`'s semantics.
    for comp_type in &variant.add_components {
        let already_present = config
            .components
            .iter()
            .any(|c| &c.component_type == comp_type);
        if !already_present {
            config.components.push(ComponentConfig {
                component_type: comp_type.clone(),
                platform: None,
                config: serde_json::Value::Object(serde_json::Map::new()),
            });
        }
    }
    for comp_type in &variant.remove_components {
        if let Some(pos) = config
            .components
            .iter()
            .position(|c| &c.component_type == comp_type)
        {
            config.components.remove(pos);
        }
    }

    (resolution, errors)
}

// ── Tests (T1.6 — 4 cases per PRD) ───────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rshome_schema::platform::Capability;
    use rshome_schema::solution::{
        default_solution_registry, CodegenPath, ComponentBundle, NetworkTopology, RuntimeBinding,
        SchedulingPolicy, SolutionDefinition, SolutionKind, SolutionRegistry,
        SolutionVariantDefinition,
    };
    use std::collections::BTreeMap;

    use crate::raw::{EsphomeBlock, RawConfig};

    /// Build a minimal solution with N variants for fixture use. Each
    /// variant's `active_flag_add`/`active_flag_remove` distinguishes it
    /// by id so the happy-path test can distinguish the branches.
    fn solution_with_variants(id: &str, variant_ids: &[&str]) -> SolutionDefinition {
        SolutionDefinition {
            id: id.into(),
            label: format!("Solution {id}"),
            label_zh: None,
            kind: SolutionKind::FirmwareAppliance,
            supported_modules: vec!["fake_module".into()],
            fixed_inputs: vec![],
            fixed_outputs: vec![],
            fixed_orchestration: vec![],
            scheduling: SchedulingPolicy {
                id: "test".into(),
                label: "test".into(),
                decisions: vec![],
            },
            user_parameters: vec![],
            feedback_paths: vec![],
            variants: variant_ids
                .iter()
                .map(|vid| SolutionVariantDefinition {
                    id: (*vid).into(),
                    label: format!("Variant {vid}"),
                    label_zh: None,
                    required_caps: vec![Capability::Wifi],
                    parameter_defaults: BTreeMap::new(),
                    add_components: vec![],
                    remove_components: vec![],
                    add_external_contracts: vec![],
                    active_flag_add: vec![format!("USE_{}", vid.to_uppercase())],
                    active_flag_remove: vec![format!("USE_NOT_{}", vid.to_uppercase())],
                    user_parameter_overrides: vec![],
                    runtime_binding_override: None,
                })
                .collect(),
            component_bundle: ComponentBundle::default(),
            runtime_binding: RuntimeBinding {
                codegen_path: CodegenPath::SelfHosted,
                ..Default::default()
            },
            external_contracts: vec![],
            network_topology: NetworkTopology::default(),
            domain: None,
            architecture_tier: None,
            communication_chains: None,
            pin_assignments: None,
            family: None,
            form_factor_families: None,
            control_uplink: None,
            video_downlink: None,
            telemetry: None,
            sensor_tier_min: None,
            actuator_family: None,
            power_rails: None,
            failsafe: None,
            topology_category: None,
            required_sensors: vec![],
            companion_link: None,
            chip_coverage: None,
        }
    }

    fn raw_config_with(solution: Option<&str>, variant: Option<&str>) -> RawConfig {
        RawConfig {
            esphome: EsphomeBlock {
                name: "test".into(),
                platform: "esp32_s3".into(),
                board: "esp32-s3-devkitc-1".into(),
                friendly_name: None,
                framework: None,
                includes: vec![],
                libraries: vec![],
                project: None,
                area: None,
                min_version: None,
                profile: None,
                solution: solution.map(String::from),
                solution_variant: variant.map(String::from),
            },
            packages: vec![],
            substitutions: Default::default(),
            components: vec![],
        }
    }

    fn registry_with(sol: SolutionDefinition) -> SolutionRegistry {
        let mut reg = default_solution_registry();
        reg.register(sol);
        reg
    }

    // (1) no solution picked → no error, no resolution.
    #[test]
    fn no_solution_is_noop() {
        let mut config = raw_config_with(None, Some("dshot"));
        let registry = default_solution_registry();
        let (resolution, errors) = stage_3_5_resolve_variant(&mut config, &registry);
        assert!(errors.is_empty());
        assert_eq!(resolution, VariantResolution::default());
    }

    // (2) variant required but missing → MissingRequiredVariantId error.
    #[test]
    fn missing_required_variant_id_errors() {
        let sol = solution_with_variants("multi_var_sol", &["pwm", "dshot"]);
        let registry = registry_with(sol);
        let mut config = raw_config_with(Some("multi_var_sol"), None);
        let (resolution, errors) = stage_3_5_resolve_variant(&mut config, &registry);
        assert_eq!(errors.len(), 1);
        let err = &errors[0];
        assert_eq!(err.stage, ValidationStage::VariantResolution);
        assert!(err.is_fatal());
        assert!(err.message.contains("multi_var_sol"));
        let suggestion = err.suggestion.as_deref().unwrap_or_default();
        assert!(
            suggestion.contains("pwm"),
            "suggestion lists pwm: {suggestion}"
        );
        assert!(
            suggestion.contains("dshot"),
            "suggestion lists dshot: {suggestion}"
        );
        // Resolution captured solution_id for diagnostics but not variant_id.
        assert_eq!(resolution.solution_id.as_deref(), Some("multi_var_sol"));
        assert!(resolution.variant_id.is_none());
    }

    // (3) unknown variant id → UnknownVariantId error.
    #[test]
    fn unknown_variant_id_errors() {
        let sol = solution_with_variants("multi_var_sol", &["pwm", "dshot"]);
        let registry = registry_with(sol);
        let mut config = raw_config_with(Some("multi_var_sol"), Some("bdshot"));
        let (resolution, errors) = stage_3_5_resolve_variant(&mut config, &registry);
        assert_eq!(errors.len(), 1);
        let err = &errors[0];
        assert_eq!(err.stage, ValidationStage::VariantResolution);
        assert!(err.is_fatal());
        assert!(err.message.contains("bdshot"));
        let suggestion = err.suggestion.as_deref().unwrap_or_default();
        assert!(suggestion.contains("pwm") && suggestion.contains("dshot"));
        assert!(resolution.variant_id.is_none());
    }

    // (4) variant set on no-variants solution → VariantAppliedToNoVariantSolution warn.
    #[test]
    fn variant_on_no_variants_warns_not_fatal() {
        let sol = solution_with_variants("single_path_sol", &[]);
        let registry = registry_with(sol);
        let mut config = raw_config_with(Some("single_path_sol"), Some("extra"));
        let (resolution, errors) = stage_3_5_resolve_variant(&mut config, &registry);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].stage, ValidationStage::VariantResolution);
        assert!(
            !errors[0].is_fatal(),
            "should be a warning, not a fatal: {:?}",
            errors[0]
        );
        assert!(errors[0].message.contains("extra"));
        assert_eq!(resolution.solution_id.as_deref(), Some("single_path_sol"));
        assert!(resolution.variant_id.is_none());
    }

    // (5) happy path: selection matches, active_flag deltas captured.
    #[test]
    fn happy_path_captures_active_flag_deltas() {
        let sol = solution_with_variants("multi_var_sol", &["pwm", "dshot"]);
        let registry = registry_with(sol);
        let mut config = raw_config_with(Some("multi_var_sol"), Some("dshot"));
        let (resolution, errors) = stage_3_5_resolve_variant(&mut config, &registry);
        assert!(errors.is_empty(), "expected no errors, got {errors:?}");
        assert_eq!(resolution.solution_id.as_deref(), Some("multi_var_sol"));
        assert_eq!(resolution.variant_id.as_deref(), Some("dshot"));
        assert_eq!(resolution.active_flag_add, vec!["USE_DSHOT".to_string()]);
        assert_eq!(
            resolution.active_flag_remove,
            vec!["USE_NOT_DSHOT".to_string()]
        );
    }

    // (6) unknown solution id is silently passed through — Stage 9 reports it.
    #[test]
    fn unknown_solution_is_silent_passthrough() {
        let registry = default_solution_registry();
        let mut config = raw_config_with(Some("nope_not_here"), Some("unused"));
        let (resolution, errors) = stage_3_5_resolve_variant(&mut config, &registry);
        assert!(
            errors.is_empty(),
            "stage 3.5 should not double-report unknown solution ids: {errors:?}",
        );
        assert_eq!(resolution.solution_id.as_deref(), Some("nope_not_here"));
        assert!(resolution.variant_id.is_none());
    }

    // ── T5.2: component overlay + Stage-6 AUTO_LOAD interaction ─────────

    fn solution_with_component_adding_variant(
        sol_id: &str,
        variant_id: &str,
        add_components: Vec<String>,
        remove_components: Vec<String>,
    ) -> SolutionDefinition {
        SolutionDefinition {
            id: sol_id.into(),
            label: format!("Solution {sol_id}"),
            label_zh: None,
            kind: SolutionKind::FirmwareAppliance,
            supported_modules: vec!["fake_module".into()],
            fixed_inputs: vec![],
            fixed_outputs: vec![],
            fixed_orchestration: vec![],
            scheduling: SchedulingPolicy {
                id: "test".into(),
                label: "test".into(),
                decisions: vec![],
            },
            user_parameters: vec![],
            feedback_paths: vec![],
            variants: vec![SolutionVariantDefinition {
                id: variant_id.into(),
                label: format!("Variant {variant_id}"),
                label_zh: None,
                required_caps: vec![],
                parameter_defaults: BTreeMap::new(),
                add_components,
                remove_components,
                add_external_contracts: vec![],
                active_flag_add: vec![],
                active_flag_remove: vec![],
                user_parameter_overrides: vec![],
                runtime_binding_override: None,
            }],
            component_bundle: ComponentBundle::default(),
            runtime_binding: RuntimeBinding {
                codegen_path: CodegenPath::SelfHosted,
                ..Default::default()
            },
            external_contracts: vec![],
            network_topology: NetworkTopology::default(),
            domain: None,
            architecture_tier: None,
            communication_chains: None,
            pin_assignments: None,
            family: None,
            form_factor_families: None,
            control_uplink: None,
            video_downlink: None,
            telemetry: None,
            sensor_tier_min: None,
            actuator_family: None,
            power_rails: None,
            failsafe: None,
            topology_category: None,
            required_sensors: vec![],
            companion_link: None,
            chip_coverage: None,
        }
    }

    /// When a variant declares `add_components: ["wifi"]`, Stage 3.5
    /// must append a matching `ComponentConfig` to `config.components`
    /// so Stage 6's AUTO_LOAD pass walks the post-overlay tree. This
    /// is the load-bearing post-condition of the Phase 5 T5.2 test.
    #[test]
    fn add_components_is_appended_to_raw_config() {
        let sol = solution_with_component_adding_variant(
            "overlay_sol",
            "v1",
            vec!["wifi".into()],
            vec![],
        );
        let registry = registry_with(sol);
        let mut config = raw_config_with(Some("overlay_sol"), Some("v1"));
        assert!(config.components.is_empty(), "fixture starts empty");

        let (_resolution, errors) = stage_3_5_resolve_variant(&mut config, &registry);
        assert!(errors.is_empty(), "expected no errors, got {errors:?}");
        assert_eq!(config.components.len(), 1);
        assert_eq!(config.components[0].component_type, "wifi");
    }

    /// `add_components` must NOT overwrite an existing user-supplied
    /// ComponentConfig of the same type. User config always wins.
    #[test]
    fn add_components_skips_when_user_already_configured_the_type() {
        let sol = solution_with_component_adding_variant(
            "overlay_sol",
            "v1",
            vec!["wifi".into()],
            vec![],
        );
        let registry = registry_with(sol);
        let mut config = raw_config_with(Some("overlay_sol"), Some("v1"));
        // User already picked wifi with their own config.
        config.components.push(ComponentConfig {
            component_type: "wifi".into(),
            platform: None,
            config: serde_json::json!({"ssid": "user_pick"}),
        });

        let (_resolution, errors) = stage_3_5_resolve_variant(&mut config, &registry);
        assert!(errors.is_empty());
        assert_eq!(config.components.len(), 1, "no dup appended");
        assert_eq!(
            config.components[0].config["ssid"].as_str(),
            Some("user_pick"),
            "user-supplied config preserved",
        );
    }

    /// `remove_components` drops the first matching entry. Used by
    /// variants that disable a base-solution component (e.g.
    /// composite_device_firmware's profile_b removes wifi).
    #[test]
    fn remove_components_drops_matching_entry() {
        let sol = solution_with_component_adding_variant(
            "overlay_sol",
            "v1",
            vec![],
            vec!["wifi".into()],
        );
        let registry = registry_with(sol);
        let mut config = raw_config_with(Some("overlay_sol"), Some("v1"));
        config.components.push(ComponentConfig {
            component_type: "wifi".into(),
            platform: None,
            config: serde_json::json!({}),
        });
        config.components.push(ComponentConfig {
            component_type: "ota".into(),
            platform: None,
            config: serde_json::json!({}),
        });

        let (_resolution, errors) = stage_3_5_resolve_variant(&mut config, &registry);
        assert!(errors.is_empty());
        assert_eq!(config.components.len(), 1);
        assert_eq!(config.components[0].component_type, "ota");
    }
}
