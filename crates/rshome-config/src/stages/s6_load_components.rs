//! Stage 6 — Component loading and AUTO_LOAD chain resolution.
//!
//! - Looks up every component referenced in the config in the `ComponentRegistry`.
//! - Expands AUTO_LOAD chains to the fixed point.
//! - Validates DEPENDENCIES are met.
//! - Checks CONFLICTS_WITH pairs.

use rshome_schema::ComponentRegistry;

use crate::error::{ValidationError, ValidationStage};
use crate::raw::RawConfig;

// ── Stage entry point ─────────────────────────────────────────────────────────

/// Stage 6: load and validate all components via the registry.
///
/// Performs:
/// 1. Verifies every referenced component/platform exists in `registry`.
/// 2. Expands AUTO_LOAD chains.
/// 3. Validates DEPENDENCIES are present.
/// 4. Checks CONFLICTS_WITH.
///
/// Returns the **resolved** component ID list (original + auto-loaded, sorted).
pub fn stage_6_load_components(
    config: &RawConfig,
    registry: &ComponentRegistry,
) -> (Vec<String>, Vec<ValidationError>) {
    let mut errors = Vec::new();

    // Collect directly referenced component IDs.
    let mut direct: Vec<String> = Vec::new();
    for (i, comp) in config.components.iter().enumerate() {
        let path = format!("components[{i}]");
        let id = component_effective_id(comp);

        match registry.get(&id) {
            None => {
                let suggestion =
                    closest_match(&id, registry).map(|m| format!("Did you mean '{m}'?"));
                let mut err = ValidationError::error(
                    ValidationStage::ComponentLoading,
                    &path,
                    format!("unknown component '{id}'"),
                );
                if let Some(s) = suggestion {
                    err = err.with_suggestion(s);
                }
                errors.push(err);
            }
            Some(def) => {
                // For platform components, verify the parent platform exists too.
                if let Some(platform) = &comp.platform {
                    if !def.child_components.contains(platform) && registry.get(platform).is_none()
                    {
                        errors.push(ValidationError::error(
                            ValidationStage::ComponentLoading,
                            format!("{path}.platform"),
                            format!(
                                "unknown platform '{platform}' for component '{}'",
                                comp.component_type
                            ),
                        ));
                    }
                }
                if !direct.contains(&id) {
                    direct.push(id);
                }
            }
        }
    }

    if errors.iter().any(|e| e.is_fatal()) {
        // Can't expand AUTO_LOAD if some components are unknown — return early.
        return (direct, errors);
    }

    // Expand AUTO_LOAD chains.
    let resolved = registry.resolve_auto_load(&direct);

    // Check DEPENDENCIES.
    for id in &resolved {
        if let Some(def) = registry.get(id) {
            for dep in &def.dependencies {
                if !resolved.contains(dep) {
                    errors.push(ValidationError::error(
                        ValidationStage::ComponentLoading,
                        id.as_str(),
                        format!("component '{id}' requires '{dep}' which is not in the config"),
                    ));
                }
            }
        }
    }

    // Check CONFLICTS_WITH.
    for id in &resolved {
        if let Some(def) = registry.get(id) {
            for conflict in &def.conflicts_with {
                if resolved.contains(conflict) {
                    errors.push(ValidationError::error(
                        ValidationStage::ComponentLoading,
                        id.as_str(),
                        format!("component '{id}' conflicts with '{conflict}'"),
                    ));
                }
            }
        }
    }

    (resolved, errors)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Compute the effective component ID for a `ComponentConfig`.
///
/// For platform components (e.g. sensor/dht), the effective ID is the platform
/// name ("dht") because that's what's registered in the registry.  For top-level
/// components (wifi, logger, etc.) it's the component_type itself.
fn component_effective_id(comp: &crate::raw::ComponentConfig) -> String {
    if let Some(platform) = &comp.platform {
        if !platform.is_empty() {
            return platform.clone();
        }
    }
    comp.component_type.clone()
}

/// Find the closest matching registered component ID using edit-distance.
/// Returns `None` if no reasonable match is found.
fn closest_match(id: &str, registry: &ComponentRegistry) -> Option<String> {
    let threshold = (id.len() / 3).max(1);
    registry
        .all_ids()
        .filter(|&candidate| edit_distance(id, candidate) <= threshold)
        .min_by_key(|&candidate| edit_distance(id, candidate))
        .map(str::to_owned)
}

/// Simple character-level edit distance (Levenshtein).
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let n = a.len();
    let m = b.len();
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    #[allow(clippy::needless_range_loop)]
    for i in 0..=n {
        dp[i][0] = i;
    }
    for (j, row) in dp[0].iter_mut().enumerate().take(m + 1) {
        *row = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            dp[i][j] = if a[i - 1] == b[j - 1] {
                dp[i - 1][j - 1]
            } else {
                1 + dp[i - 1][j].min(dp[i][j - 1]).min(dp[i - 1][j - 1])
            };
        }
    }
    dp[n][m]
}

#[cfg(test)]
mod tests {
    use rshome_schema::{ComponentDefinition, ComponentRegistry};

    use super::*;
    use crate::raw::{ComponentConfig, EsphomeBlock, RawConfig};
    use serde_json::json;

    fn make_registry() -> ComponentRegistry {
        let mut reg = ComponentRegistry::new();

        // wifi — no deps, no conflicts
        reg.register(ComponentDefinition {
            id: "wifi".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: None,
            ..Default::default()
        });

        // sensor — platform parent
        reg.register(ComponentDefinition {
            id: "sensor".into(),
            is_family: true,
            child_components: vec!["dht".into(), "adc".into()],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(rshome_schema::EntityType::Sensor),
            ..Default::default()
        });

        // dht — auto-loads sensor
        reg.register(ComponentDefinition {
            id: "dht".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec!["sensor".into()],
            dependencies: vec![],
            conflicts_with: vec![],
            entity_type: Some(rshome_schema::EntityType::Sensor),
            ..Default::default()
        });

        // api — requires wifi
        reg.register(ComponentDefinition {
            id: "api".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec![],
            dependencies: vec!["wifi".into()],
            conflicts_with: vec![],
            entity_type: None,
            ..Default::default()
        });

        // mqtt — conflicts with api
        reg.register(ComponentDefinition {
            id: "mqtt".into(),
            is_family: false,
            child_components: vec![],
            auto_load: vec![],
            dependencies: vec![],
            conflicts_with: vec!["api".into()],
            entity_type: None,
            ..Default::default()
        });

        reg
    }

    fn base_config() -> RawConfig {
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
            components: vec![],
        }
    }

    fn push_comp(config: &mut RawConfig, typ: &str, platform: Option<&str>) {
        config.components.push(ComponentConfig {
            component_type: typ.into(),
            platform: platform.map(str::to_owned),
            config: json!({}),
        });
    }

    #[test]
    fn known_component_resolves_ok() {
        let mut config = base_config();
        push_comp(&mut config, "wifi", None);
        let (resolved, errors) = stage_6_load_components(&config, &make_registry());
        assert!(errors.is_empty());
        assert!(resolved.contains(&"wifi".to_string()));
    }

    #[test]
    fn unknown_component_produces_error() {
        let mut config = base_config();
        push_comp(&mut config, "nonexistent_comp", None);
        let (_, errors) = stage_6_load_components(&config, &make_registry());
        assert!(!errors.is_empty());
        assert!(errors[0].is_fatal());
        assert!(errors[0].message.contains("nonexistent_comp"));
    }

    #[test]
    fn auto_load_chain_resolved() {
        let mut config = base_config();
        // dht should auto-load sensor
        config.components.push(ComponentConfig {
            component_type: "sensor".into(),
            platform: Some("dht".into()),
            config: json!({}),
        });
        let (resolved, errors) = stage_6_load_components(&config, &make_registry());
        assert!(errors.is_empty(), "errors: {:?}", errors);
        assert!(resolved.contains(&"dht".to_string()));
        assert!(resolved.contains(&"sensor".to_string()));
    }

    #[test]
    fn missing_dependency_produces_error() {
        let mut config = base_config();
        // api requires wifi, but wifi not in config
        push_comp(&mut config, "api", None);
        let (_, errors) = stage_6_load_components(&config, &make_registry());
        assert!(errors.iter().any(|e| e.message.contains("wifi")));
    }

    #[test]
    fn conflict_detected() {
        let mut config = base_config();
        push_comp(&mut config, "wifi", None);
        push_comp(&mut config, "api", None);
        push_comp(&mut config, "mqtt", None);
        let (_, errors) = stage_6_load_components(&config, &make_registry());
        assert!(errors.iter().any(|e| e.message.contains("conflicts")));
    }

    #[test]
    fn empty_config_resolves_empty() {
        let config = base_config();
        let (resolved, errors) = stage_6_load_components(&config, &make_registry());
        assert!(errors.is_empty());
        assert!(resolved.is_empty());
    }

    #[test]
    fn api_with_wifi_satisfies_dependency() {
        let mut config = base_config();
        push_comp(&mut config, "wifi", None);
        push_comp(&mut config, "api", None);
        let (_, errors) = stage_6_load_components(&config, &make_registry());
        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
    }

    #[test]
    fn edit_distance_exact_match() {
        assert_eq!(edit_distance("wifi", "wifi"), 0);
    }

    #[test]
    fn edit_distance_one_substitution() {
        assert_eq!(edit_distance("wifii", "wifi"), 1);
    }
}
