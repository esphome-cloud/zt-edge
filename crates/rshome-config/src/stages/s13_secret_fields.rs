//! Stage 13: Secret field validation.
//!
//! Ensures that secret fields (API keys, tokens, etc.) are not present as
//! plaintext values in the config. They must be provisioned via NVS or
//! referenced with `!secret` syntax.

use rshome_schema::ComponentRegistry;

use crate::error::{ValidationError, ValidationStage};
use crate::raw::RawConfig;

/// Check that no secret field contains a plaintext value.
pub fn stage_13_check_secret_fields(
    config: &RawConfig,
    registry: &ComponentRegistry,
) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    for comp in &config.components {
        let component_id = comp
            .platform
            .clone()
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| comp.component_type.clone());

        let def = match registry.get(&component_id) {
            Some(d) => d,
            None => continue,
        };

        for field_name in &def.secret_fields {
            if let Some(value) = comp.config.get(field_name).and_then(|v| v.as_str()) {
                // Allow empty strings (not yet configured).
                if value.is_empty() {
                    continue;
                }
                // Allow ESPHome secret references.
                if value.starts_with("!secret ") {
                    continue;
                }
                // Plaintext value found — reject.
                errors.push(ValidationError::error(
                    ValidationStage::SecretFields,
                    format!("{}.{}", component_id, field_name),
                    format!(
                        "secret field '{}' for component '{}' must not contain a plaintext value; provision via NVS",
                        field_name, component_id
                    ),
                ));
            }
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use rshome_schema::{ComponentDefinition, ComponentRegistry};

    use super::*;
    use crate::raw::{ComponentConfig, EsphomeBlock, RawConfig};
    use serde_json::json;

    fn make_config(components: Vec<ComponentConfig>) -> RawConfig {
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
                profile: None,
                solution: None,
                solution_variant: None,
            },
            packages: vec![],
            substitutions: Default::default(),
            components,
        }
    }

    #[test]
    fn registered_secret_field_rejects_plaintext() {
        let mut reg = ComponentRegistry::new();
        reg.register(ComponentDefinition {
            id: "cloud_connector".into(),
            secret_fields: vec!["access_token".into()],
            ..Default::default()
        });
        let config = make_config(vec![ComponentConfig {
            component_type: "cloud_connector".into(),
            platform: None,
            config: json!({"access_token": "plaintext"}),
        }]);
        let errors = stage_13_check_secret_fields(&config, &reg);
        assert_eq!(errors[0].stage, ValidationStage::SecretFields);
        assert!(errors[0].message.contains("access_token"));
    }
}
