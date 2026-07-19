//! Stage 12: Profile validation.
//!
//! If a `profile` name is specified in the esphome block, validates that:
//! 1. The profile exists in the profile registry.
//! 2. All required components are selected.
//! 3. Exclusive selections are consistent.

use std::collections::HashSet;

use rshome_schema::profile::ProfileRegistry;
use rshome_schema::ComponentRegistry;

use crate::error::{ValidationError, ValidationStage};
use crate::raw::RawConfig;

/// Validate the config against its declared profile (if any).
pub fn stage_12_validate_profile(
    config: &RawConfig,
    component_registry: &ComponentRegistry,
) -> Vec<ValidationError> {
    let profile_name = match &config.esphome.profile {
        Some(name) => name,
        None => return vec![], // No profile specified — no-op.
    };

    let profile_registry = ProfileRegistry::default_profiles();
    let profile = match profile_registry.get(profile_name) {
        Some(p) => p,
        None => {
            return vec![ValidationError::error(
                ValidationStage::ProfileValidation,
                "esphome.profile",
                format!("profile '{}' not found in profile registry", profile_name),
            )];
        }
    };

    let mut errors = Vec::new();

    // Collect selected component IDs (after auto_load expansion).
    let selected: Vec<String> = config
        .components
        .iter()
        .map(|c| {
            c.platform
                .clone()
                .filter(|p| !p.is_empty())
                .unwrap_or_else(|| c.component_type.clone())
        })
        .collect();
    let expanded: HashSet<String> = component_registry
        .resolve_auto_load(&selected)
        .into_iter()
        .collect();

    // Check all required components are present.
    for required in &profile.components {
        if !expanded.contains(required) {
            errors.push(ValidationError::error(
                ValidationStage::ProfileValidation,
                "components",
                format!(
                    "profile '{}' requires component '{}' which is not selected",
                    profile_name, required
                ),
            ));
        }
    }

    // Check exclusive selections are consistent.
    for (group, expected_component) in &profile.exclusive_selections {
        if !expanded.contains(expected_component) {
            errors.push(ValidationError::error(
                ValidationStage::ProfileValidation,
                "components",
                format!(
                    "profile '{}' selects '{}' for group '{}' but it is not in the config",
                    profile_name, expected_component, group
                ),
            ));
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use rshome_schema::ComponentRegistry;

    use super::*;
    use crate::raw::{ComponentConfig, EsphomeBlock, RawConfig};
    use serde_json::json;

    fn make_config_with_profile(profile: Option<&str>, components: Vec<&str>) -> RawConfig {
        RawConfig {
            esphome: EsphomeBlock {
                name: "test".into(),
                platform: "esp32".into(),
                board: "esp32dev".into(),
                friendly_name: None,
                framework: None,
                includes: vec![],
                libraries: vec![],
                project: None,
                area: None,
                min_version: None,
                profile: profile.map(String::from),
                solution: None,
                solution_variant: None,
            },
            packages: vec![],
            substitutions: Default::default(),
            components: components
                .into_iter()
                .map(|c| ComponentConfig {
                    component_type: c.into(),
                    platform: None,
                    config: json!({}),
                })
                .collect(),
        }
    }

    #[test]
    fn no_profile_is_noop() {
        let reg = ComponentRegistry::default_registry();
        let config = make_config_with_profile(None, vec![]);
        let errors = stage_12_validate_profile(&config, &reg);
        assert!(errors.is_empty());
    }

    #[test]
    fn unknown_profile_rejected() {
        let reg = ComponentRegistry::default_registry();
        let config = make_config_with_profile(Some("nonexistent"), vec![]);
        let errors = stage_12_validate_profile(&config, &reg);
        assert!(!errors.is_empty());
        assert!(errors[0].message.contains("nonexistent"));
    }

    #[test]
    fn error_stage_is_profile_validation() {
        let reg = ComponentRegistry::default_registry();
        let config = make_config_with_profile(Some("nonexistent"), vec![]);
        let errors = stage_12_validate_profile(&config, &reg);
        assert_eq!(errors[0].stage, ValidationStage::ProfileValidation);
    }
}
