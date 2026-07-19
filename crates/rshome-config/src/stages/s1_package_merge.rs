//! Stage 1 — Package merge.
//!
//! Loads every `PackageRef` listed in the config from the `PackageStore` and
//! merges their component lists into the main config.  Package substitutions
//! are also merged in, but **cannot override** user-defined substitutions.

use crate::error::{ValidationError, ValidationStage};
use crate::raw::{PackageStore, RawConfig};

/// Stage 1: merge referenced packages into `config`.
///
/// Returns all diagnostics produced.  Errors indicate missing packages.
pub fn stage_1_merge_packages(
    config: &mut RawConfig,
    store: &PackageStore,
) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    let packages = config.packages.clone();

    for pkg in &packages {
        match store.get(&pkg.name) {
            None => {
                let path = format!("packages.{}", pkg.name);
                let msg = match &pkg.url {
                    Some(url) => format!(
                        "package '{}' from '{}' not found in package store",
                        pkg.name, url
                    ),
                    None => format!("package '{}' not found in package store", pkg.name),
                };
                errors.push(ValidationError::error(
                    ValidationStage::PackageMerge,
                    path,
                    msg,
                ));
            }
            Some(pkg_config) => {
                // Merge components (package components appended after main).
                for comp in &pkg_config.components {
                    config.components.push(comp.clone());
                }
                // Merge substitutions — package values do NOT override main.
                for (k, v) in &pkg_config.substitutions {
                    config
                        .substitutions
                        .entry(k.clone())
                        .or_insert_with(|| v.clone());
                }
            }
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::{ComponentConfig, EsphomeBlock, PackageRef, PackageStore, RawConfig};
    use serde_json::json;

    fn base_esphome() -> EsphomeBlock {
        EsphomeBlock {
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
        }
    }

    fn base_config() -> RawConfig {
        RawConfig {
            esphome: base_esphome(),
            packages: vec![],
            substitutions: Default::default(),
            components: vec![],
        }
    }

    fn pkg_ref(name: &str) -> PackageRef {
        PackageRef {
            name: name.into(),
            url: None,
            file: None,
            git_ref: None,
            config_path: None,
        }
    }

    fn sensor_comp(platform: &str) -> ComponentConfig {
        ComponentConfig {
            component_type: "sensor".into(),
            platform: Some(platform.into()),
            config: json!({"pin": 4}),
        }
    }

    #[test]
    fn empty_packages_list_noop() {
        let mut config = base_config();
        let store = PackageStore::new();
        let errors = stage_1_merge_packages(&mut config, &store);
        assert!(errors.is_empty());
        assert!(config.components.is_empty());
    }

    #[test]
    fn missing_package_produces_fatal_error() {
        let mut config = base_config();
        config.packages.push(pkg_ref("nonexistent"));
        let errors = stage_1_merge_packages(&mut config, &PackageStore::new());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].is_fatal());
        assert!(errors[0].path.contains("nonexistent"));
        assert!(errors[0].message.contains("nonexistent"));
    }

    #[test]
    fn url_package_missing_includes_url_in_message() {
        let mut config = base_config();
        config.packages.push(PackageRef {
            name: "remote".into(),
            url: Some("https://github.com/user/repo".into()),
            file: None,
            git_ref: None,
            config_path: None,
        });
        let errors = stage_1_merge_packages(&mut config, &PackageStore::new());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("github.com"));
    }

    #[test]
    fn package_components_merged_into_main() {
        let mut config = base_config();
        config.packages.push(pkg_ref("common"));

        let mut pkg = base_config();
        pkg.components.push(sensor_comp("dht"));

        let mut store = PackageStore::new();
        store.insert("common", pkg);

        let errors = stage_1_merge_packages(&mut config, &store);
        assert!(errors.is_empty());
        assert_eq!(config.components.len(), 1);
        assert_eq!(config.components[0].component_type, "sensor");
    }

    #[test]
    fn package_substitutions_do_not_override_main() {
        let mut config = base_config();
        config
            .substitutions
            .insert("key".into(), "main_value".into());
        config.packages.push(pkg_ref("pkg"));

        let mut pkg = base_config();
        pkg.substitutions.insert("key".into(), "pkg_value".into());
        pkg.substitutions.insert("new".into(), "added".into());

        let mut store = PackageStore::new();
        store.insert("pkg", pkg);

        stage_1_merge_packages(&mut config, &store);
        assert_eq!(config.substitutions["key"], "main_value");
        assert_eq!(config.substitutions["new"], "added");
    }

    #[test]
    fn multiple_packages_merged_in_order() {
        let mut config = base_config();
        config.packages.push(pkg_ref("a"));
        config.packages.push(pkg_ref("b"));

        let mut store = PackageStore::new();
        for name in ["a", "b"] {
            let mut pkg = base_config();
            pkg.components.push(ComponentConfig {
                component_type: name.into(),
                platform: None,
                config: json!({}),
            });
            store.insert(name, pkg);
        }

        let errors = stage_1_merge_packages(&mut config, &store);
        assert!(errors.is_empty());
        assert_eq!(config.components.len(), 2);
        assert_eq!(config.components[0].component_type, "a");
        assert_eq!(config.components[1].component_type, "b");
    }

    #[test]
    fn partial_failure_collects_all_errors() {
        let mut config = base_config();
        config.packages.push(pkg_ref("missing_a"));
        config.packages.push(pkg_ref("missing_b"));
        let errors = stage_1_merge_packages(&mut config, &PackageStore::new());
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn main_components_preserved_after_merge() {
        let mut config = base_config();
        config.components.push(sensor_comp("adc"));
        config.packages.push(pkg_ref("extra"));

        let mut pkg = base_config();
        pkg.components.push(sensor_comp("dht"));

        let mut store = PackageStore::new();
        store.insert("extra", pkg);

        stage_1_merge_packages(&mut config, &store);
        // Main component first, then package component.
        assert_eq!(config.components.len(), 2);
        assert_eq!(config.components[0].platform.as_deref(), Some("adc"));
        assert_eq!(config.components[1].platform.as_deref(), Some("dht"));
    }
}
