//! Shared output types for the wizard API.
//!
//! These structs are returned by the WASM-exported wizard surface
//! (`crates/rshome-wizard/src/wasm_bindings.rs`) and serialized to the
//! browser via the `registry-data.json` export
//! (`cargo run -p rshome-wizard --bin export-registry`).
//!
//! Most fields are passthroughs of the foundational `rshome-schema`
//! definitions — see imports below for the canonical source of each
//! enum (`ChipTarget`, `McuRole`, `FormFactorKind`, etc.).
//!
//! Browser clients consume the same serialized shapes through the registry
//! export and the Wasm bindings.

use serde::{Deserialize, Serialize};

use rshome_schema::module::ModuleDefinition;
use rshome_schema::platform::{
    ActuatorFamily, ChipCoverageStatus, ChipFamilyKind, CompanionLinkKind, ControlUplinkKind,
    FailsafeInfo, FormFactorKind, ImplementationFamily, PlatformTargetDefinition, PowerRailKind,
    SensorRequirement, SensorTierKind, TelemetryKind, TopologyKind, VideoDownlinkKind,
};
use rshome_schema::registry::ComponentDefinition;
use rshome_schema::solution::{
    OrchestrationStep, SolutionDefinition, SolutionVariantDefinition, UserParameterDefinition,
};
use rshome_schema::{ChipTarget, EntityType};
use std::collections::BTreeMap;

// ── ComponentInfo ─────────────────────────────────────────────────────────────

/// Enriched component info returned by `list_components`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentInfo {
    pub id: String,
    pub description: String,
    pub is_family: bool,
    pub entity_type: Option<String>,
    pub auto_load: Vec<String>,
    pub dependencies: Vec<String>,
    pub conflicts_with: Vec<String>,
    pub child_components: Vec<String>,
}

impl From<&ComponentDefinition> for ComponentInfo {
    fn from(def: &ComponentDefinition) -> Self {
        Self {
            id: def.id.clone(),
            description: def.description.clone(),
            is_family: def.is_family,
            entity_type: def.entity_type.map(entity_type_name),
            auto_load: def.auto_load.clone(),
            dependencies: def.dependencies.clone(),
            conflicts_with: def.conflicts_with.clone(),
            child_components: def.child_components.clone(),
        }
    }
}

/// Serialize a `#[serde(rename_all = "snake_case")]` enum variant to its string name.
fn serde_enum_name<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value)
        .expect("enum serializes")
        .trim_matches('"')
        .to_string()
}

fn entity_type_name(entity_type: EntityType) -> String {
    serde_enum_name(&entity_type)
}

// ── PinInfo ───────────────────────────────────────────────────────────────────

/// GPIO pin descriptor returned by `get_pin_map`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinInfo {
    /// GPIO number (0-based).
    pub gpio_num: u8,
    /// `true` if the pin can only be used as input.
    pub input_only: bool,
    /// `true` if the pin is reserved for internal flash/PSRAM.
    pub flash_reserved: bool,
    /// `true` if the pin affects boot mode when driven at startup.
    pub is_strapping: bool,
    /// Supported modes for this pin.
    pub supported_modes: Vec<String>,
    /// Human-readable description.
    pub description: String,
}

/// Build the GPIO pin map for a chip target.
pub fn build_pin_map(target: ChipTarget) -> Vec<PinInfo> {
    use crate::pin_map::chip_pin_info;
    chip_pin_info(target)
}

// ── ValidConfigSummary ────────────────────────────────────────────────────────

/// Successful validation result returned by `validate_config`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidConfigSummary {
    pub valid: bool,
    pub active_flags: Vec<String>,
    pub chip_target: String,
    pub component_count: usize,
    pub pin_allocations: usize,
}

// ── ModuleInfo ────────────────────────────────────────────────────────────────

/// Hardware module descriptor for the wizard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleInfo {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_zh: Option<String>,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_zh: Option<String>,
    pub target: String,
    pub hardware_caps: Vec<String>,
    pub constraints: Vec<String>,
    pub compatible_solutions: Vec<String>,
    /// Target domain for wizard scoping. `None` means visible in all domains.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}

impl From<&ModuleDefinition> for ModuleInfo {
    fn from(def: &ModuleDefinition) -> Self {
        Self {
            id: def.id.clone(),
            label: def.label.clone(),
            label_zh: def.label_zh.clone(),
            description: def.description.clone(),
            description_zh: def.description_zh.clone(),
            target: serde_enum_name(&def.target),
            hardware_caps: def.hardware_caps.iter().map(serde_enum_name).collect(),
            constraints: def.constraints.clone(),
            compatible_solutions: def.compatible_solutions.clone(),
            domain: def.domain.as_ref().map(serde_enum_name),
        }
    }
}

// ── SolutionInfo ──────────────────────────────────────────────────────────────

/// Orchestration step descriptor for the wizard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationStepInfo {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_zh: Option<String>,
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_zh: Option<String>,
    /// IDs of steps that must complete before this step can begin.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
}

impl From<&OrchestrationStep> for OrchestrationStepInfo {
    fn from(step: &OrchestrationStep) -> Self {
        Self {
            id: step.id.clone(),
            label: step.label.clone(),
            label_zh: step.label_zh.clone(),
            description: step.description.clone(),
            description_zh: step.description_zh.clone(),
            depends_on: step.depends_on.clone(),
        }
    }
}

/// A selectable option for enum-typed parameters (wizard export).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumOptionInfo {
    pub value: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Conditional parameter dependency (wizard export).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterDependencyInfo {
    pub parameter_id: String,
    pub when_value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when_not_value: Option<String>,
}

/// GPIO pin assignment for wizard display (wizard export).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolutionPinAssignment {
    pub function: String,
    pub default_gpio: u8,
    #[serde(default)]
    pub alternatives: Vec<u8>,
    pub capability: String,
}

/// User parameter descriptor for the wizard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserParameterInfo {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_zh: Option<String>,
    pub required: bool,
    pub secret: bool,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_zh: Option<String>,
    pub default_value: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<EnumOptionInfo>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<ParameterDependencyInfo>,
}

impl From<&UserParameterDefinition> for UserParameterInfo {
    fn from(param: &UserParameterDefinition) -> Self {
        Self {
            id: param.id.clone(),
            label: param.label.clone(),
            label_zh: param.label_zh.clone(),
            required: param.required,
            secret: param.secret,
            description: param.description.clone(),
            description_zh: param.description_zh.clone(),
            default_value: param.default_value.clone(),
            enum_values: param.enum_values.as_ref().map(|opts| {
                opts.iter()
                    .map(|o| EnumOptionInfo {
                        value: o.value.clone(),
                        label: o.label.clone(),
                        description: o.description.clone(),
                    })
                    .collect()
            }),
            depends_on: param.depends_on.as_ref().map(|d| ParameterDependencyInfo {
                parameter_id: d.parameter_id.clone(),
                when_value: d.when_value.clone(),
                when_not_value: d.when_not_value.clone(),
            }),
        }
    }
}

/// Solution variant descriptor for the wizard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolutionVariantInfo {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_zh: Option<String>,
    pub required_caps: Vec<String>,
}

impl From<&SolutionVariantDefinition> for SolutionVariantInfo {
    fn from(v: &SolutionVariantDefinition) -> Self {
        Self {
            id: v.id.clone(),
            label: v.label.clone(),
            label_zh: v.label_zh.clone(),
            required_caps: v.required_caps.iter().map(serde_enum_name).collect(),
        }
    }
}

/// Solution descriptor for the wizard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolutionInfo {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_zh: Option<String>,
    pub kind: String,
    // `implementation_family: Option<String>` retired 2026-04-21 — use
    // the typed `family` field (Option<ImplementationFamily>) below.
    pub supported_modules: Vec<String>,
    pub fixed_orchestration: Vec<OrchestrationStepInfo>,
    pub user_parameters: Vec<UserParameterInfo>,
    pub variants: Vec<SolutionVariantInfo>,
    pub required_components: Vec<String>,
    pub optional_components: Vec<String>,
    pub external_contracts: Vec<String>,
    pub network_topology: String,
    pub has_ha_entities: bool,
    pub codegen_path: String,
    /// Target domain for wizard scoping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Architecture tier: "single_mcu" | "dual_mcu" | "sbc_mcu" | "receiver_direct_drive".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture_tier: Option<String>,
    /// Communication chains this solution implements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub communication_chains: Option<Vec<String>>,
    /// GPIO pin assignments for wizard display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_assignments: Option<Vec<SolutionPinAssignment>>,

    // ── Vehicle & Aircraft Control — extended annotations ──────────────────
    //
    // Parallel to the fields on `SolutionDefinition`; public clients receive
    // these values at the top level rather than in a nested extension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<ImplementationFamily>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_factor_families: Option<Vec<FormFactorKind>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_uplink: Option<ControlUplinkKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_downlink: Option<VideoDownlinkKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<TelemetryKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensor_tier_min: Option<SensorTierKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actuator_family: Option<ActuatorFamily>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power_rails: Option<PowerRailKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failsafe: Option<FailsafeInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chip_coverage: Option<BTreeMap<ChipFamilyKind, ChipCoverageStatus>>,
    /// Topology preset (auto-populated). Added 2026-04-21 per va-residuals Phase 2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topology_category: Option<TopologyKind>,
    /// Non-IMU sensor requirements. Added 2026-04-21 per va-residuals Phase 3 T3.1.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_sensors: Vec<SensorRequirement>,
    /// MCU↔SBC companion link. Added 2026-04-21 per va-residuals Phase 3 T3.2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub companion_link: Option<CompanionLinkKind>,
}

impl From<&SolutionDefinition> for SolutionInfo {
    fn from(def: &SolutionDefinition) -> Self {
        Self {
            id: def.id.clone(),
            label: def.label.clone(),
            label_zh: def.label_zh.clone(),
            kind: serde_enum_name(&def.kind),
            supported_modules: def.supported_modules.clone(),
            fixed_orchestration: def
                .fixed_orchestration
                .iter()
                .map(OrchestrationStepInfo::from)
                .collect(),
            user_parameters: def
                .user_parameters
                .iter()
                .map(UserParameterInfo::from)
                .collect(),
            variants: def.variants.iter().map(SolutionVariantInfo::from).collect(),
            required_components: def.component_bundle.required.clone(),
            optional_components: def.component_bundle.optional.clone(),
            external_contracts: def.external_contracts.clone(),
            network_topology: serde_enum_name(&def.network_topology),
            has_ha_entities: !def.runtime_binding.ha_entities.is_empty(),
            codegen_path: serde_enum_name(&def.runtime_binding.codegen_path),
            domain: def.domain.as_ref().map(serde_enum_name),
            architecture_tier: def.architecture_tier.as_ref().map(serde_enum_name),
            communication_chains: def
                .communication_chains
                .as_ref()
                .map(|chains| chains.iter().map(serde_enum_name).collect()),
            pin_assignments: def.pin_assignments.as_ref().map(|pins| {
                pins.iter()
                    .map(|p| SolutionPinAssignment {
                        function: p.function.clone(),
                        default_gpio: p.default_gpio,
                        alternatives: p.alternatives.clone(),
                        capability: p.capability.clone(),
                    })
                    .collect()
            }),
            family: def.family,
            form_factor_families: def.form_factor_families.clone(),
            control_uplink: def.control_uplink,
            video_downlink: def.video_downlink,
            telemetry: def.telemetry,
            sensor_tier_min: def.sensor_tier_min,
            actuator_family: def.actuator_family,
            power_rails: def.power_rails,
            failsafe: def.failsafe.clone(),
            chip_coverage: def.chip_coverage.clone(),
            topology_category: def.topology_category,
            required_sensors: def.required_sensors.clone(),
            companion_link: def.companion_link,
        }
    }
}

// ── PlatformTargetInfo ────────────────────────────────────────────────────────

/// Platform target descriptor for the wizard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformTargetInfo {
    pub id: String,
    pub target: String,
    pub capabilities: Vec<String>,
    pub supported_inputs: Vec<String>,
    pub supported_outputs: Vec<String>,
}

impl From<&PlatformTargetDefinition> for PlatformTargetInfo {
    fn from(def: &PlatformTargetDefinition) -> Self {
        Self {
            id: def.id.clone(),
            target: serde_enum_name(&def.target),
            capabilities: def
                .capability_profile
                .capabilities
                .iter()
                .map(serde_enum_name)
                .collect(),
            supported_inputs: def.supported_inputs.iter().map(serde_enum_name).collect(),
            supported_outputs: def.supported_outputs.iter().map(serde_enum_name).collect(),
        }
    }
}
