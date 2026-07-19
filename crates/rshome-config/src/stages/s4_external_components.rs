//! Stage 4 — External component trust model.
//!
//! Validates that all external dependency sources are either trusted by default
//! (Espressif registry, local paths) or explicitly allowed by the caller.
//!
//! # Trust model
//!
//! | Source | Default trust |
//! |--------|---------------|
//! | `EspressifRegistry` | Trusted |
//! | `Local(path)` | Trusted (user's own code) |
//! | `GitHub(url)` | Requires explicit allow |
//! | `GitLab(url)` | Requires explicit allow |
//! | `Gitee(url)` | Requires explicit allow |

use crate::error::{ValidationError, ValidationStage};
use crate::raw::RawConfig;

// ── DependencySource ──────────────────────────────────────────────────────────

/// Source / origin of an external component dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencySource {
    /// Official Espressif component registry — trusted by default.
    EspressifRegistry,
    /// GitHub repository — requires explicit allow.
    GitHub(String),
    /// GitLab repository — requires explicit allow.
    GitLab(String),
    /// Gitee repository — requires explicit allow.
    Gitee(String),
    /// Local filesystem path — always trusted.
    Local(String),
}

impl DependencySource {
    /// Parse a dependency source from a URL or path string.
    pub fn from_url(url: &str) -> Self {
        if url.is_empty() || url.starts_with('.') || url.starts_with('/') {
            return Self::Local(url.to_owned());
        }
        if url.contains("github.com") {
            return Self::GitHub(url.to_owned());
        }
        if url.contains("gitlab.com") {
            return Self::GitLab(url.to_owned());
        }
        if url.contains("gitee.com") {
            return Self::Gitee(url.to_owned());
        }
        if url.contains("components.espressif.com") || !url.contains("://") {
            return Self::EspressifRegistry;
        }
        // Unknown external URL — treat as GitHub-like (requires allow).
        Self::GitHub(url.to_owned())
    }

    /// Returns `true` if this source is trusted without any explicit allow.
    pub fn is_trusted_by_default(&self) -> bool {
        matches!(self, Self::EspressifRegistry | Self::Local(_))
    }

    /// Human-readable source label.
    pub fn label(&self) -> &str {
        match self {
            Self::EspressifRegistry => "espressif_registry",
            Self::GitHub(_) => "github",
            Self::GitLab(_) => "gitlab",
            Self::Gitee(_) => "gitee",
            Self::Local(_) => "local",
        }
    }
}

// ── AllowList ─────────────────────────────────────────────────────────────────

/// Explicit allow-list for untrusted external sources.
///
/// Callers populate this before running Stage 4 to permit specific external URLs.
#[derive(Debug, Default)]
pub struct AllowList {
    /// URL prefixes that are explicitly allowed.
    pub allowed_prefixes: Vec<String>,
}

impl AllowList {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allow all URLs that start with `prefix`.
    pub fn allow_prefix(&mut self, prefix: impl Into<String>) {
        self.allowed_prefixes.push(prefix.into());
    }

    /// Returns `true` if `url` is covered by an explicit allow entry.
    pub fn is_allowed(&self, url: &str) -> bool {
        self.allowed_prefixes
            .iter()
            .any(|p| url.starts_with(p.as_str()))
    }
}

// ── Stage entry point ─────────────────────────────────────────────────────────

/// Stage 4: validate external component sources against the trust model.
///
/// Checks all `framework.components` IDF deps and any component `git` / `url`
/// fields for untrusted sources.  Untrusted sources not covered by `allow_list`
/// produce fatal errors.
pub fn stage_4_resolve_external_components(
    config: &RawConfig,
    allow_list: &AllowList,
) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    // Check framework-level IDF component dependencies.
    if let Some(fw) = &config.esphome.framework {
        for (idx, idf_comp) in fw.components.iter().enumerate() {
            let path = format!("esphome.framework.components[{idx}]");

            if let Some(git_url) = &idf_comp.git {
                check_url(git_url, &path, allow_list, &mut errors);
            }
            // Local paths are always trusted; registry names have no URL.
        }
    }

    // Check component-level external references.
    for (i, comp) in config.components.iter().enumerate() {
        let path = format!("components[{i}]");

        // Look for git/url fields in the component config JSON.
        if let Some(git_url) = comp.config.get("git").and_then(|v| v.as_str()) {
            check_url(git_url, &format!("{path}.git"), allow_list, &mut errors);
        }
        if let Some(url) = comp.config.get("url").and_then(|v| v.as_str()) {
            check_url(url, &format!("{path}.url"), allow_list, &mut errors);
        }
    }

    errors
}

fn check_url(url: &str, path: &str, allow_list: &AllowList, errors: &mut Vec<ValidationError>) {
    let source = DependencySource::from_url(url);
    if source.is_trusted_by_default() {
        return;
    }
    if allow_list.is_allowed(url) {
        return;
    }
    errors.push(
        ValidationError::error(
            ValidationStage::ExternalComponents,
            path,
            format!(
                "untrusted external source '{}' ({}) requires explicit allow-list entry",
                url,
                source.label()
            ),
        )
        .with_suggestion("Add the URL prefix to AllowList::allow_prefix() to permit it"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::{ComponentConfig, EsphomeBlock, FrameworkConfig, IdfComponentRef, RawConfig};
    use serde_json::json;

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

    #[test]
    fn no_external_components_clean() {
        let config = base_config();
        let errors = stage_4_resolve_external_components(&config, &AllowList::new());
        assert!(errors.is_empty());
    }

    #[test]
    fn github_url_without_allow_list_rejected() {
        let mut config = base_config();
        config.esphome.framework = Some(FrameworkConfig {
            framework_type: "esp-idf".into(),
            version: None,
            components: vec![IdfComponentRef {
                name: "my_component".into(),
                version: None,
                git: Some("https://github.com/user/repo".into()),
                path: None,
            }],
            sdkconfig_options: Default::default(),
        });
        let errors = stage_4_resolve_external_components(&config, &AllowList::new());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].is_fatal());
        assert!(errors[0].message.contains("github.com"));
    }

    #[test]
    fn github_url_with_allow_prefix_passes() {
        let mut config = base_config();
        config.esphome.framework = Some(FrameworkConfig {
            framework_type: "esp-idf".into(),
            version: None,
            components: vec![IdfComponentRef {
                name: "my_component".into(),
                version: None,
                git: Some("https://github.com/user/repo".into()),
                path: None,
            }],
            sdkconfig_options: Default::default(),
        });
        let mut allow = AllowList::new();
        allow.allow_prefix("https://github.com/user/");
        let errors = stage_4_resolve_external_components(&config, &allow);
        assert!(errors.is_empty());
    }

    #[test]
    fn espressif_registry_always_trusted() {
        let source = DependencySource::from_url("components.espressif.com/led_strip");
        assert!(source.is_trusted_by_default());
    }

    #[test]
    fn local_path_always_trusted() {
        let source = DependencySource::from_url("./my_component");
        assert!(source.is_trusted_by_default());
        let source2 = DependencySource::from_url("/absolute/path");
        assert!(source2.is_trusted_by_default());
    }

    #[test]
    fn gitee_url_rejected_without_allow() {
        let source = DependencySource::from_url("https://gitee.com/user/repo");
        assert!(!source.is_trusted_by_default());
        assert_eq!(source.label(), "gitee");
    }

    #[test]
    fn gitlab_url_rejected_without_allow() {
        let source = DependencySource::from_url("https://gitlab.com/user/repo");
        assert!(!source.is_trusted_by_default());
        assert_eq!(source.label(), "gitlab");
    }

    #[test]
    fn component_git_field_checked() {
        let mut config = base_config();
        config.components.push(ComponentConfig {
            component_type: "external_component".into(),
            platform: None,
            config: json!({"git": "https://github.com/user/external"}),
        });
        let errors = stage_4_resolve_external_components(&config, &AllowList::new());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("github.com"));
    }
}
