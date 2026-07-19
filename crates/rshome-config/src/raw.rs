//! Raw (unvalidated) config representation.
//!
//! `RawConfig` is the input to the 10-stage pipeline.  It can be parsed from
//! JSON or TOML and represents the user's config exactly as written.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ── Top-level config ──────────────────────────────────────────────────────────

/// Raw config as supplied by the user (JSON / TOML / wizard).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawConfig {
    /// Device identity and target platform.
    pub esphome: EsphomeBlock,
    /// External config packages to merge in (Stage 1).
    #[serde(default)]
    pub packages: Vec<PackageRef>,
    /// Key-value substitutions applied across the config (Stage 2).
    #[serde(default)]
    pub substitutions: HashMap<String, String>,
    /// All component instances (sensor, switch, wifi, …).
    #[serde(default)]
    pub components: Vec<ComponentConfig>,
}

// ── esphome: block ────────────────────────────────────────────────────────────

/// The `esphome:` block — device identity and target platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EsphomeBlock {
    /// Device name shown in Home Assistant.
    pub name: String,
    /// Target chip: `"esp32"`, `"esp32s3"`, `"esp32c6"`.
    pub platform: String,
    /// Board identifier, e.g. `"esp32dev"`, `"esp32-s3-devkitc-1"`.
    pub board: String,
    /// Optional friendly name for the Home Assistant UI.
    #[serde(default)]
    pub friendly_name: Option<String>,
    /// Framework selection and version pinning.
    #[serde(default)]
    pub framework: Option<FrameworkConfig>,
    /// Extra C/C++ include files injected into the generated project.
    #[serde(default)]
    pub includes: Vec<String>,
    /// Additional PlatformIO libraries.
    #[serde(default)]
    pub libraries: Vec<String>,
    /// OTA project identity metadata.
    #[serde(default)]
    pub project: Option<ProjectConfig>,
    /// Physical area shown in HA (e.g. `"Living Room"`).
    #[serde(default)]
    pub area: Option<String>,
    /// Minimum rshome version required by this config.
    #[serde(default)]
    pub min_version: Option<String>,
    /// Optional public profile name registered by an embedding application.
    #[serde(default)]
    pub profile: Option<String>,
    /// Optional solution ID for solution-aware validation (e.g. `"sensor_hub"`).
    #[serde(default)]
    pub solution: Option<String>,
    /// Optional variant id within `solution`'s `variants[]`. Required iff
    /// the referenced solution declares a non-empty `variants[]`; reported
    /// as a `VariantResolution` error at pipeline stage 3.5 when missing.
    /// Ignored (with a warning) for solutions that have no variants.
    ///
    /// Added by the rshome-codegen-variants PRD Phase 1 T1.1.
    #[serde(default)]
    pub solution_variant: Option<String>,
}

/// Framework configuration (ESP-IDF or Arduino).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameworkConfig {
    /// `"esp-idf"` or `"arduino"`.
    #[serde(rename = "type")]
    pub framework_type: String,
    /// Framework version pin (e.g. `"5.3.1"`).  `None` = latest.
    #[serde(default)]
    pub version: Option<String>,
    /// IDF component manager dependencies.
    #[serde(default)]
    pub components: Vec<IdfComponentRef>,
    /// `sdkconfig` key-value overrides.
    #[serde(default)]
    pub sdkconfig_options: HashMap<String, String>,
}

/// An IDF component manager dependency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdfComponentRef {
    /// Component name, e.g. `"espressif/led_strip"`.
    pub name: String,
    /// Version constraint (semver or `"*"`).
    #[serde(default)]
    pub version: Option<String>,
    /// Git URL override (bypasses component registry).
    #[serde(default)]
    pub git: Option<String>,
    /// Local path (relative to project root).
    #[serde(default)]
    pub path: Option<String>,
}

/// OTA / device project identity metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    /// Project name, e.g. `"myco.sensor_board"`.
    pub name: String,
    /// Semantic version, e.g. `"1.0.0"`.
    pub version: String,
}

// ── packages ──────────────────────────────────────────────────────────────────

/// A reference to an external config package to be merged in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageRef {
    /// Local alias for this package.
    pub name: String,
    /// Remote git repository URL.
    #[serde(default)]
    pub url: Option<String>,
    /// Local file path (caller pre-loads content into `PackageStore` for Wasm).
    #[serde(default)]
    pub file: Option<String>,
    /// Git ref (branch / tag / commit SHA).
    #[serde(default)]
    pub git_ref: Option<String>,
    /// Path within the repository to the config file.
    #[serde(default)]
    pub config_path: Option<String>,
}

// ── components ────────────────────────────────────────────────────────────────

/// A single component instance entry in the config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentConfig {
    /// Component type key (`"sensor"`, `"switch"`, `"wifi"`, …).
    ///
    /// May start with `"!"` for directives:
    /// - `"!extend wifi"` — deep-merge config into existing wifi component.
    /// - `"!remove logger"` — remove logger from component list.
    pub component_type: String,
    /// Platform name for platform components, e.g. `"dht"`, `"gpio"`, `"adc"`.
    #[serde(default)]
    pub platform: Option<String>,
    /// Raw config values as a JSON object.
    pub config: serde_json::Value,
}

impl ComponentConfig {
    /// Returns `true` if this is an `!extend` directive.
    pub fn is_extend(&self) -> bool {
        self.component_type.starts_with("!extend")
    }

    /// Returns `true` if this is a `!remove` directive.
    pub fn is_remove(&self) -> bool {
        self.component_type.starts_with("!remove")
    }

    /// Returns the target component type for extend/remove directives.
    ///
    /// `"!extend wifi"` → `Some("wifi")`.
    pub fn directive_target(&self) -> Option<&str> {
        if self.is_extend() || self.is_remove() {
            self.component_type.split(' ').nth(1)
        } else {
            None
        }
    }
}

// ── PackageStore ──────────────────────────────────────────────────────────────

/// A pre-populated store of external package configs.
///
/// The caller is responsible for loading package content (from disk, HTTP,
/// embedded assets, etc.) before invoking the pipeline.  This design keeps
/// `rshome-config` free of I/O and wasm-compilable.
#[derive(Default)]
pub struct PackageStore {
    packages: HashMap<String, RawConfig>,
}

impl PackageStore {
    /// Create an empty package store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a package under `name`.
    pub fn insert(&mut self, name: impl Into<String>, config: RawConfig) {
        self.packages.insert(name.into(), config);
    }

    /// Look up a package by name.
    pub fn get(&self, name: &str) -> Option<&RawConfig> {
        self.packages.get(name)
    }

    /// Returns `true` if a package with this name is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.packages.contains_key(name)
    }

    /// Number of registered packages.
    pub fn len(&self) -> usize {
        self.packages.len()
    }

    /// Returns `true` if the store is empty.
    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }
}
