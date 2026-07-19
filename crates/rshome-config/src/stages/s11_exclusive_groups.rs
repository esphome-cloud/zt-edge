//! Stage 11: Exclusive group validation.
//!
//! Ensures that at most one component from each exclusive group is selected.

use std::collections::HashMap;

use rshome_schema::ComponentRegistry;

use crate::error::{ValidationError, ValidationStage};
use crate::raw::RawConfig;

/// Check that no exclusive group has more than one selected component.
pub fn stage_11_check_exclusive_groups(
    config: &RawConfig,
    registry: &ComponentRegistry,
) -> Vec<ValidationError> {
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

    let expanded = registry.resolve_auto_load(&selected);

    // Build group → members mapping.
    let mut groups: HashMap<String, Vec<String>> = HashMap::new();
    for id in &expanded {
        if let Some(def) = registry.get(id) {
            if let Some(group) = &def.exclusive_group {
                groups.entry(group.clone()).or_default().push(id.clone());
            }
        }
    }

    // Flag any group with more than one member.
    for (group, members) in &groups {
        if members.len() > 1 {
            let member_list = members.join(", ");
            errors.push(ValidationError::error(
                ValidationStage::ExclusiveGroup,
                "components",
                format!(
                    "exclusive group '{}' has multiple selections: [{}]",
                    group, member_list
                ),
            ));
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use rshome_schema::{ComponentDefinition, ComponentRegistry, InstancePolicy};

    use super::*;
    use crate::raw::{ComponentConfig, EsphomeBlock, RawConfig};
    use serde_json::json;

    fn make_config(components: Vec<&str>) -> RawConfig {
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
    fn exclusive_group_is_checked_for_public_extensions() {
        let mut reg = ComponentRegistry::new();
        for id in ["radio_2g4", "radio_5g"] {
            reg.register(ComponentDefinition {
                id: id.into(),
                exclusive_group: Some("radio_band".into()),
                instance_policy: InstancePolicy::ExclusiveGroup("radio_band".into()),
                ..Default::default()
            });
        }
        let config = make_config(vec!["radio_2g4", "radio_5g"]);
        let errors = stage_11_check_exclusive_groups(&config, &reg);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].stage, ValidationStage::ExclusiveGroup);
        assert!(errors[0].message.contains("radio_band"));
    }
}
