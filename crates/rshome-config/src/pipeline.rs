//! Pipeline orchestrator — runs all 13 validation stages in order.

use rshome_schema::{ComponentRegistry, FeatureFlagSet};

use crate::error::ValidationError;
use crate::raw::{ComponentConfig, PackageStore, RawConfig};
use crate::stages::{
    s10_pin_conflicts::stage_10_check_pin_conflicts,
    s11_exclusive_groups::stage_11_check_exclusive_groups,
    s12_profile_validation::stage_12_validate_profile,
    s13_secret_fields::stage_13_check_secret_fields,
    s1_package_merge::stage_1_merge_packages,
    s2_substitutions::stage_2_apply_substitutions,
    s3_5_variant_resolution::stage_3_5_resolve_variant,
    s3_extend_remove::stage_3_process_extend_remove,
    s4_external_components::{stage_4_resolve_external_components, AllowList},
    s5_preload::stage_5_preload_esphome_block,
    s6_load_components::stage_6_load_components,
    s7_schema_validation::stage_7_validate_schemas,
    s8_id_resolution::stage_8_resolve_ids,
    s9_final_validation::stage_9_final_validation,
};
use crate::validated::{
    DependencyGraph, ValidatedComponent, ValidatedConfig, ValidatedEsphomeBlock,
    ValidatedProjectConfig,
};

// ── Result type ───────────────────────────────────────────────────────────────

/// Output of [`ValidationPipeline::validate`].
pub enum ValidationResult {
    /// All 10 stages passed (may still include warnings/info).
    Valid(Box<ValidatedConfig>),
    /// At least one fatal error prevented completion.
    Invalid(Vec<ValidationError>),
}

impl ValidationResult {
    /// Returns `true` if the config is valid.
    pub fn is_valid(&self) -> bool {
        matches!(self, Self::Valid(_))
    }

    /// Returns the validated config, or `None` if invalid.
    pub fn ok(self) -> Option<ValidatedConfig> {
        match self {
            Self::Valid(v) => Some(*v),
            Self::Invalid(_) => None,
        }
    }

    /// Returns the error list, or an empty vec if valid.
    pub fn errors(self) -> Vec<ValidationError> {
        match self {
            Self::Valid(_) => vec![],
            Self::Invalid(e) => e,
        }
    }

    /// Collect all diagnostics (warnings + errors) regardless of outcome.
    pub fn all_errors(&self) -> &[ValidationError] {
        match self {
            Self::Valid(_) => &[],
            Self::Invalid(e) => e,
        }
    }
}

// ── Partial config (for real-time browser feedback) ───────────────────────────

/// A partial config for incremental browser-side validation.
///
/// Only stages that make sense for partial data are run (stages 2, 5, 7).
pub struct PartialConfig {
    /// Optional esphome block (for platform validation).
    pub esphome: Option<crate::raw::EsphomeBlock>,
    /// Component instances added so far.
    pub components: Vec<ComponentConfig>,
    /// Substitutions defined so far.
    pub substitutions: std::collections::HashMap<String, String>,
}

// ── Pipeline ──────────────────────────────────────────────────────────────────

/// The 10-stage validation pipeline.
pub struct ValidationPipeline {
    /// Component registry used for AUTO_LOAD, DEPENDENCIES, CONFLICTS_WITH.
    registry: ComponentRegistry,
    /// External source allow-list for Stage 4.
    allow_list: AllowList,
}

impl ValidationPipeline {
    /// Create a pipeline with the given registry and a default (empty) allow-list.
    pub fn new(registry: ComponentRegistry) -> Self {
        Self {
            registry,
            allow_list: AllowList::new(),
        }
    }

    /// Create a pipeline with a custom allow-list for external sources.
    pub fn with_allow_list(registry: ComponentRegistry, allow_list: AllowList) -> Self {
        Self {
            registry,
            allow_list,
        }
    }

    /// Run the full 10-stage pipeline on `config`.
    ///
    /// Stages are run in order.  Fatal errors at any stage stop the pipeline
    /// (remaining stages are skipped).  Non-fatal errors and warnings are
    /// accumulated and returned in the `Invalid` variant.
    pub fn validate(
        &self,
        mut config: RawConfig,
        package_store: &PackageStore,
    ) -> ValidationResult {
        let mut all_errors: Vec<ValidationError> = Vec::new();

        // ── Stage 1: package merge ────────────────────────────────────────────
        let s1_errs = stage_1_merge_packages(&mut config, package_store);
        let s1_fatal = s1_errs.iter().any(|e| e.is_fatal());
        all_errors.extend(s1_errs);
        if s1_fatal {
            return ValidationResult::Invalid(all_errors);
        }

        // ── Stage 2: substitutions ────────────────────────────────────────────
        let s2_errs = stage_2_apply_substitutions(&mut config);
        all_errors.extend(s2_errs);
        // Substitution warnings are non-fatal — continue.

        // ── Stage 3: extend/remove ────────────────────────────────────────────
        let s3_errs = stage_3_process_extend_remove(&mut config);
        let s3_fatal = s3_errs.iter().any(|e| e.is_fatal());
        all_errors.extend(s3_errs);
        if s3_fatal {
            return ValidationResult::Invalid(all_errors);
        }

        // ── Stage 3.5: variant resolution ────────────────────────────────────
        // Slots between extend/remove and external components because the
        // variant overlay may add components that should participate in
        // AUTO_LOAD (stage 6) and pin-conflict detection (stage 10).
        // rshome-codegen-variants PRD Phase 1 T1.3.
        let solution_registry = rshome_schema::solution::default_solution_registry();
        let (variant_resolution, s3_5_errs) =
            stage_3_5_resolve_variant(&mut config, &solution_registry);
        let s3_5_fatal = s3_5_errs.iter().any(|e| e.is_fatal());
        all_errors.extend(s3_5_errs);
        if s3_5_fatal {
            return ValidationResult::Invalid(all_errors);
        }

        // ── Stage 4: external component trust ─────────────────────────────────
        let s4_errs = stage_4_resolve_external_components(&config, &self.allow_list);
        let s4_fatal = s4_errs.iter().any(|e| e.is_fatal());
        all_errors.extend(s4_errs);
        if s4_fatal {
            return ValidationResult::Invalid(all_errors);
        }

        // ── Stage 5: preload (platform detection) ─────────────────────────────
        let preload_ctx = match stage_5_preload_esphome_block(&config) {
            Ok(ctx) => ctx,
            Err(errs) => {
                all_errors.extend(errs);
                return ValidationResult::Invalid(all_errors);
            }
        };

        // ── Stage 6: component loading + AUTO_LOAD ────────────────────────────
        let (resolved_ids, s6_errs) = stage_6_load_components(&config, &self.registry);
        let s6_fatal = s6_errs.iter().any(|e| e.is_fatal());
        all_errors.extend(s6_errs);
        if s6_fatal {
            return ValidationResult::Invalid(all_errors);
        }

        // ── Stage 7: schema validation ────────────────────────────────────────
        let s7_errs = stage_7_validate_schemas(&config, &self.registry);
        let s7_fatal = s7_errs.iter().any(|e| e.is_fatal());
        all_errors.extend(s7_errs);
        if s7_fatal {
            return ValidationResult::Invalid(all_errors);
        }

        // ── Stage 8: ID resolution ────────────────────────────────────────────
        let (_id_table, s8_errs) = stage_8_resolve_ids(&mut config);
        let s8_fatal = s8_errs.iter().any(|e| e.is_fatal());
        all_errors.extend(s8_errs);
        if s8_fatal {
            return ValidationResult::Invalid(all_errors);
        }

        // ── Stage 9: final cross-component validation ─────────────────────────
        let s9_errs = stage_9_final_validation(&config, &self.registry);
        let s9_fatal = s9_errs.iter().any(|e| e.is_fatal());
        all_errors.extend(s9_errs);
        if s9_fatal {
            return ValidationResult::Invalid(all_errors);
        }

        // ── Stage 10: pin conflicts ────────────────────────────────────────────
        let (tracker, s10_errs) = stage_10_check_pin_conflicts(&config, preload_ctx.chip_target);
        let s10_fatal = s10_errs.iter().any(|e| e.is_fatal());
        all_errors.extend(s10_errs);
        if s10_fatal {
            return ValidationResult::Invalid(all_errors);
        }

        // ── Stage 11: exclusive group validation ──────────────────────────────
        let s11_errs = stage_11_check_exclusive_groups(&config, &self.registry);
        let s11_fatal = s11_errs.iter().any(|e| e.is_fatal());
        all_errors.extend(s11_errs);
        if s11_fatal {
            return ValidationResult::Invalid(all_errors);
        }

        // ── Stage 12: profile validation ──────────────────────────────────────
        let s12_errs = stage_12_validate_profile(&config, &self.registry);
        let s12_fatal = s12_errs.iter().any(|e| e.is_fatal());
        all_errors.extend(s12_errs);
        if s12_fatal {
            return ValidationResult::Invalid(all_errors);
        }

        // ── Stage 13: secret field validation ─────────────────────────────────
        let s13_errs = stage_13_check_secret_fields(&config, &self.registry);
        let s13_fatal = s13_errs.iter().any(|e| e.is_fatal());
        all_errors.extend(s13_errs);
        if s13_fatal {
            return ValidationResult::Invalid(all_errors);
        }

        // ── Build ValidatedConfig ─────────────────────────────────────────────
        let feature_flags = FeatureFlagSet::from_components(&resolved_ids, &self.registry);

        let mut active_flags: Vec<String> = resolved_ids
            .iter()
            .filter_map(|id| {
                // Collect USE_* flags by checking well-known flag names.
                if feature_flags.contains(&format!("USE_{}", id.to_uppercase())) {
                    Some(format!("USE_{}", id.to_uppercase()))
                } else {
                    None
                }
            })
            .collect();

        // Also add entity-level flags (USE_SENSOR, USE_SWITCH, etc.)
        for flag_candidate in &[
            "USE_SENSOR",
            "USE_BINARY_SENSOR",
            "USE_SWITCH",
            "USE_LIGHT",
            "USE_CLIMATE",
            "USE_FAN",
            "USE_COVER",
            "USE_LOCK",
            "USE_MEDIA_PLAYER",
            "USE_NUMBER",
            "USE_SELECT",
            "USE_TEXT",
            "USE_BUTTON",
            "USE_EVENT",
            "USE_TEXT_SENSOR",
            "USE_ALARM_CONTROL_PANEL",
            "USE_WIFI",
            "USE_API",
            "USE_MQTT",
            "USE_OTA",
            "USE_LOGGER",
            "USE_I2C",
            "USE_SPI",
            "USE_UART",
            "USE_DHT",
        ] {
            if feature_flags.contains(flag_candidate)
                && !active_flags.contains(&flag_candidate.to_string())
            {
                active_flags.push(flag_candidate.to_string());
            }
        }
        active_flags.sort();
        active_flags.dedup();

        // Apply variant-level active_flag deltas captured by stage 3.5.
        // Additions are layered on top; removals win last so a variant
        // that both adds and removes the same flag ends up without it
        // (documented in `stage_3_5_variant_resolution.rs`).
        // rshome-codegen-variants PRD Phase 1 T1.2.
        for flag in &variant_resolution.active_flag_add {
            if !active_flags.contains(flag) {
                active_flags.push(flag.clone());
            }
        }
        if !variant_resolution.active_flag_remove.is_empty() {
            active_flags.retain(|f| !variant_resolution.active_flag_remove.contains(f));
        }
        active_flags.sort();
        active_flags.dedup();

        // Build dependency graph via petgraph DAG.
        let dep_graph = match self.registry.build_dag(&resolved_ids) {
            Ok(dag) => DependencyGraph::from_dag(dag),
            Err(_) => {
                // Fallback to manual construction if cycle detected.
                let mut g = DependencyGraph::new();
                for id in &resolved_ids {
                    if let Some(def) = self.registry.get(id) {
                        for dep in &def.dependencies {
                            g.add_dependency(id, dep);
                        }
                        for auto in &def.auto_load {
                            g.add_dependency(id, auto);
                        }
                    }
                }
                g.compute_order();
                g
            }
        };

        // Build validated components list.
        let mut type_counters: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let mut validated_components: Vec<ValidatedComponent> = Vec::new();

        for comp in &config.components {
            let idx = {
                let ctr = type_counters
                    .entry(comp.component_type.clone())
                    .or_insert(0);
                let i = *ctr;
                *ctr += 1;
                i
            };
            let component_id = comp
                .platform
                .clone()
                .filter(|p| !p.is_empty())
                .unwrap_or_else(|| comp.component_type.clone());

            let entity_id = comp
                .config
                .get("id")
                .and_then(|v| v.as_str())
                .map(str::to_owned);

            validated_components.push(ValidatedComponent {
                component_id,
                platform_type: comp.platform.as_ref().map(|_| comp.component_type.clone()),
                index: idx,
                entity_id,
                config: comp.config.clone(),
                auto_loaded: false,
            });
        }

        // Add auto-loaded components not explicitly in config.
        let explicit_ids: std::collections::HashSet<String> = config
            .components
            .iter()
            .map(|c| {
                c.platform
                    .clone()
                    .filter(|p| !p.is_empty())
                    .unwrap_or_else(|| c.component_type.clone())
            })
            .collect();

        for auto_id in resolved_ids.iter().filter(|id| !explicit_ids.contains(*id)) {
            validated_components.push(ValidatedComponent {
                component_id: auto_id.clone(),
                platform_type: None,
                index: 0,
                entity_id: None,
                config: serde_json::Value::Object(serde_json::Map::new()),
                auto_loaded: true,
            });
        }

        let esphome = ValidatedEsphomeBlock {
            name: config.esphome.name.clone(),
            chip_target: preload_ctx.chip_target,
            board: preload_ctx.board.clone(),
            friendly_name: config.esphome.friendly_name.clone(),
            framework_type: preload_ctx.framework_type,
            project: config
                .esphome
                .project
                .as_ref()
                .map(|p| ValidatedProjectConfig {
                    name: p.name.clone(),
                    version: p.version.clone(),
                }),
            solution_id: config.esphome.solution.clone(),
            solution_variant_id: variant_resolution.variant_id.clone(),
        };

        // Any non-fatal accumulated diagnostics are included in the result
        // but don't prevent returning ValidatedConfig.
        if all_errors.iter().any(|e| e.is_fatal()) {
            return ValidationResult::Invalid(all_errors);
        }

        ValidationResult::Valid(Box::new(ValidatedConfig {
            esphome,
            components: validated_components,
            active_flags,
            pin_allocations: tracker.pin_allocations().to_vec(),
            dependency_graph: dep_graph,
        }))
    }

    /// Validate a partial config for real-time browser feedback.
    ///
    /// Runs a subset of stages appropriate for incomplete config:
    /// Stage 2 (substitutions), Stage 5 (platform detection), Stage 7 (schema).
    pub fn validate_partial(&self, partial: PartialConfig) -> Vec<ValidationError> {
        let mut errors = Vec::new();

        // Build a minimal RawConfig from the partial data.
        let esphome = partial.esphome.unwrap_or_else(|| crate::raw::EsphomeBlock {
            name: "partial".into(),
            platform: "esp32".into(),
            board: "esp32dev".into(),
            friendly_name: None,
            framework: None,
            includes: vec![],
            libraries: vec![],
            project: None,
            area: None,
            min_version: None,
            profile: None,
            solution: None,
            solution_variant: None,
        });

        let mut config = RawConfig {
            esphome,
            packages: vec![],
            substitutions: partial.substitutions,
            components: partial.components,
        };

        // Stage 2 — substitutions (quick, no dependencies).
        errors.extend(stage_2_apply_substitutions(&mut config));

        // Stage 5 — platform detection (validates esphome block).
        if let Err(errs) = stage_5_preload_esphome_block(&config) {
            errors.extend(errs);
        }

        // Stage 7 — schema validation (type checking per component).
        errors.extend(stage_7_validate_schemas(&config, &self.registry));

        errors
    }
}

#[cfg(test)]
mod tests {
    use rshome_schema::{ComponentDefinition, ComponentRegistry};

    use super::*;
    use crate::error::ValidationStage;
    use crate::raw::{ComponentConfig, EsphomeBlock, PackageStore, RawConfig};
    use serde_json::json;

    fn make_registry() -> ComponentRegistry {
        let mut reg = ComponentRegistry::new();
        for id in [
            "wifi", "api", "ota", "logger", "sensor", "dht", "i2c", "uart",
        ] {
            reg.register(ComponentDefinition {
                id: id.into(),
                is_family: id == "sensor",
                child_components: if id == "sensor" {
                    vec!["dht".into()]
                } else {
                    vec![]
                },
                auto_load: if id == "dht" {
                    vec!["sensor".into()]
                } else {
                    vec![]
                },
                dependencies: if id == "api" {
                    vec!["wifi".into()]
                } else {
                    vec![]
                },
                conflicts_with: vec![],
                entity_type: None,
                ..Default::default()
            });
        }
        reg
    }

    fn minimal_config(platform: &str) -> RawConfig {
        RawConfig {
            esphome: EsphomeBlock {
                name: "testdev".into(),
                platform: platform.into(),
                board: "esp32dev".into(),
                friendly_name: None,
                framework: None,
                includes: vec![],
                libraries: vec![],
                project: None,
                area: None,
                min_version: None,
                profile: None,
                solution: None,
                solution_variant: None,
            },
            packages: vec![],
            substitutions: Default::default(),
            components: vec![],
        }
    }

    #[test]
    fn empty_valid_config_passes() {
        let pipeline = ValidationPipeline::new(make_registry());
        let config = minimal_config("esp32");
        let result = pipeline.validate(config, &PackageStore::new());
        assert!(result.is_valid());
    }

    #[test]
    fn unknown_platform_produces_invalid() {
        let pipeline = ValidationPipeline::new(make_registry());
        let config = minimal_config("rp2040");
        let result = pipeline.validate(config, &PackageStore::new());
        assert!(!result.is_valid());
    }

    #[test]
    fn valid_wifi_component_passes() {
        let pipeline = ValidationPipeline::new(make_registry());
        let mut config = minimal_config("esp32");
        config.components.push(ComponentConfig {
            component_type: "wifi".into(),
            platform: None,
            config: json!({"provisioning_mode": "nvs"}),
        });
        let result = pipeline.validate(config, &PackageStore::new());
        assert!(result.is_valid(), "result should be valid");
    }

    #[test]
    fn api_without_wifi_fails() {
        let pipeline = ValidationPipeline::new(make_registry());
        let mut config = minimal_config("esp32");
        config.components.push(ComponentConfig {
            component_type: "api".into(),
            platform: None,
            config: json!({}),
        });
        let result = pipeline.validate(config, &PackageStore::new());
        assert!(!result.is_valid());
    }

    // ── Variant resolution pipeline integration (Phase 1 T1.6) ──────────

    /// Happy path: picking a valid variant on `composite_device_firmware`
    /// propagates the variant id into `ValidatedEsphomeBlock.solution_variant_id`.
    /// composite_device_firmware is the only registry fixture shipping
    /// non-empty `variants[]` today (profile_a + profile_b).
    #[test]
    fn variant_resolution_propagates_variant_id_to_validated_config() {
        let pipeline = ValidationPipeline::new(make_registry());
        let mut config = minimal_config("esp32s3");
        config.esphome.solution = Some("composite_device_firmware".into());
        config.esphome.solution_variant = Some("profile_a".into());
        // composite_device_firmware declares `component_bundle.required: [uart]`
        // — wire a uart component so stage 9's final-validation passes.
        config.components.push(ComponentConfig {
            component_type: "uart".into(),
            platform: None,
            config: json!({}),
        });
        let result = pipeline.validate(config, &PackageStore::new());
        let validated = match result {
            ValidationResult::Valid(v) => *v,
            ValidationResult::Invalid(errs) => {
                panic!(
                    "composite_device_firmware + profile_a must validate; got errors:\n{}",
                    errs.iter()
                        .map(|e| format!("  - {e}"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            }
        };
        assert_eq!(
            validated.esphome.solution_id.as_deref(),
            Some("composite_device_firmware")
        );
        assert_eq!(
            validated.esphome.solution_variant_id.as_deref(),
            Some("profile_a")
        );
    }

    /// Unknown variant on a real variant-carrying solution should block
    /// pipeline completion with a VariantResolution error.
    #[test]
    fn variant_resolution_unknown_variant_is_fatal() {
        let pipeline = ValidationPipeline::new(make_registry());
        let mut config = minimal_config("esp32s3");
        config.esphome.solution = Some("composite_device_firmware".into());
        config.esphome.solution_variant = Some("profile_z_not_real".into());
        let result = pipeline.validate(config, &PackageStore::new());
        assert!(!result.is_valid(), "unknown variant should fail");
        let errs = result.errors();
        assert!(
            errs.iter()
                .any(|e| e.stage == ValidationStage::VariantResolution && e.is_fatal()),
            "expected a fatal VariantResolution error; got {errs:?}",
        );
    }

    /// Missing variant on a variant-carrying solution is also fatal.
    #[test]
    fn variant_resolution_missing_variant_is_fatal() {
        let pipeline = ValidationPipeline::new(make_registry());
        let mut config = minimal_config("esp32s3");
        config.esphome.solution = Some("composite_device_firmware".into());
        config.esphome.solution_variant = None;
        let result = pipeline.validate(config, &PackageStore::new());
        assert!(!result.is_valid(), "missing variant must be fatal");
        let errs = result.errors();
        assert!(errs
            .iter()
            .any(|e| e.stage == ValidationStage::VariantResolution && e.is_fatal()));
    }

    #[test]
    fn validated_config_contains_chip_target() {
        let pipeline = ValidationPipeline::new(make_registry());
        let config = minimal_config("esp32s3");
        let validated = pipeline
            .validate(config, &PackageStore::new())
            .ok()
            .expect("should be valid");
        assert_eq!(
            validated.esphome.chip_target,
            rshome_schema::ChipTarget::Esp32S3
        );
    }

    #[test]
    fn partial_validation_bad_platform_returns_error() {
        let pipeline = ValidationPipeline::new(make_registry());
        let partial = PartialConfig {
            esphome: Some(EsphomeBlock {
                name: "test".into(),
                platform: "esp8266".into(),
                board: "nodemcu".into(),
                friendly_name: None,
                framework: None,
                includes: vec![],
                libraries: vec![],
                project: None,
                area: None,
                min_version: None,
                profile: None,
                solution: None,
                solution_variant: None,
            }),
            components: vec![],
            substitutions: Default::default(),
        };
        let errors = pipeline.validate_partial(partial);
        assert!(errors.iter().any(|e| e.path == "esphome.platform"));
    }

    #[test]
    fn partial_validation_good_config_no_errors() {
        let pipeline = ValidationPipeline::new(make_registry());
        let partial = PartialConfig {
            esphome: Some(EsphomeBlock {
                name: "dev".into(),
                platform: "esp32".into(),
                board: "esp32dev".into(),
                friendly_name: None,
                framework: None,
                includes: vec![],
                libraries: vec![],
                project: None,
                area: None,
                min_version: None,
                profile: None,
                solution: None,
                solution_variant: None,
            }),
            components: vec![],
            substitutions: Default::default(),
        };
        let errors = pipeline.validate_partial(partial);
        assert!(
            errors.iter().all(|e| !e.is_fatal()),
            "fatal errors: {:?}",
            errors
        );
    }

    #[test]
    fn validated_config_has_dependency_graph() {
        let pipeline = ValidationPipeline::new(make_registry());
        let mut config = minimal_config("esp32");
        config.components.push(ComponentConfig {
            component_type: "wifi".into(),
            platform: None,
            config: json!({}),
        });
        let validated = pipeline
            .validate(config, &PackageStore::new())
            .ok()
            .expect("valid");
        // Graph may be empty or populated — just ensure it exists.
        let _ = &validated.dependency_graph;
    }
}
