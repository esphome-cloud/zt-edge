//! Solution definitions and registry.
//!
//! A **solution** is the user-facing entry point — what the user actually selects.
//! Each solution maps to a set of components (a `ComponentBundle`), supports one
//! or more hardware modules, and may offer intra-solution variants.
//!
//! # Variant overlay precedence
//!
//! When a user selects a solution and picks one of its `variants[]`, the
//! `rshome-config` pipeline's Stage 3.5 ("Variant Resolution") produces a
//! merged view by layering the variant deltas on top of the solution base.
//! The rshome-codegen layer consumes only the merged view — it never sees
//! the variant structs directly.
//!
//! Merge order (lowest priority first — later wins):
//!
//! 1. **Solution base** — `SolutionDefinition` defaults: `user_parameters[]`,
//!    `component_bundle.required[]`, `runtime_binding.*`, `external_contracts[]`.
//! 2. **Variant overlay** — `SolutionVariantDefinition` deltas:
//!    - `active_flag_add` / `active_flag_remove` mutate `ValidatedConfig.active_flags`.
//!    - `parameter_defaults` override `user_parameters[*].default_value`.
//!    - `user_parameter_overrides` add / remove / replace entries in `user_parameters[]`.
//!    - `add_components` / `remove_components` diff `component_bundle.required[]`.
//!    - `runtime_binding_override` diffs `runtime_binding.managed_components` +
//!      `custom_components`, and replaces `board_assembly` when set.
//!    - `add_external_contracts` appends to `external_contracts[]`.
//!    - `required_caps` appends to the solution's implicit capability set
//!      (checked against module capabilities at pipeline time).
//! 3. **User raw config** — explicit values in the user's submitted
//!    `RawConfig` always win, including over variant deltas.
//!
//! Introduced by the `rshome-codegen-variants` PRD (Phase 0 T0.1-T0.3).
//! The 4 variant-delta fields below (`active_flag_{add,remove}`,
//! `user_parameter_overrides`, `runtime_binding_override`) default to empty
//! with `skip_serializing_if` so existing `registry-data.json` payloads
//! round-trip unchanged.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::assembly::AssemblyId;
use crate::ha_export::HaEntityExportDefinition;
use crate::platform::{
    ActuatorFamily, ArchitectureTier, Capability, ChipCoverageStatus, ChipFamilyKind,
    CommunicationChainKind, CompanionLinkKind, ControlUplinkKind, DomainKind, EmergencyStopWiring,
    FailsafeInfo, FeedbackSurface, FormFactorKind, ImplementationFamily, InputSurface,
    KillswitchSource, McuRole, OutputSurface, PinAssignment, PowerRailKind, RxLossBehavior,
    SensorRequirement, SensorTierKind, SignalPath, SignalPathStep, TelemetryKind, TopologyKind,
    TransformNode, VideoDownlinkKind,
};

// ── Type aliases ─────────────────────────────────────────────────────────────

/// Unique identifier for a solution.
pub type SolutionId = String;

/// Unique identifier for a solution variant.
pub type VariantId = String;

// ── Enums ────────────────────────────────────────────────────────────────────

/// Codegen path selection — determines which runtime substrate the generated
/// project targets.
///
/// - **BrookesiaManaged** (Path A): Brookesia service manager, custom service
///   registration, managed IDF components via `idf_component.yml`.
/// - **SelfHosted** (Path B): Self-hosted C runtime with rshome_core, rshome_app,
///   scheduler, event bus — the legacy codegen path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CodegenPath {
    /// Brookesia-managed runtime (Path A) — default for new solutions.
    #[default]
    BrookesiaManaged,
    /// Self-hosted C runtime (Path B) — legacy codegen path.
    SelfHosted,
}

/// Network topology for wizard display and documentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum NetworkTopology {
    /// STA mode: devices → AP/router → cloud.
    #[default]
    Star,
    /// ESP-NOW: device A ↔ device B direct.
    PointToPoint,
    /// Mesh-Lite: auto-routing multi-hop.
    Mesh,
    /// AP mode: devices connect to this node's AP.
    Local,
    /// No network (standalone / USB-only).
    None,
}

/// High-level category of a solution.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SolutionKind {
    /// Bare-metal firmware for a single-purpose device.
    FirmwareAppliance,
    /// Network bridge or protocol translator.
    ConnectivityBridge,
    /// Voice-enabled node (wake word + TTS/STT).
    VoiceNode,
}

// ── Supporting structs ───────────────────────────────────────────────────────

/// A fixed step in the solution's orchestration pipeline.
///
/// `retry_policy` and `parallel_group` were added by Phase 2 / Task 2.1
/// of the vehicle-aircraft-control-design PRD. Both default to `None`
/// for backward compatibility — the 37 existing V&A solutions continue
/// to serialize without these fields appearing in `registry-data.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OrchestrationStep {
    pub id: String,
    pub label: String,
    /// Optional Chinese (zh-CN) translation of `label`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_zh: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional Chinese (zh-CN) translation of `description`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_zh: Option<String>,
    /// IDs of steps that must complete before this step can begin.
    /// Empty means this step has no predecessors (can start immediately).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    /// Bounded-retry policy for transient init failures. `None` =
    /// boot-once legacy semantics: any failure calls
    /// `rshome_failsafe_enter()` immediately. Honored by the
    /// firmware runtime (Phase 2 Task 2.2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_policy: Option<crate::orchestration::RetryPolicy>,
    /// Concurrency group for parallel execution. Steps sharing the
    /// same `parallel_group` value run concurrently once their
    /// `depends_on` predecessors complete. `None` = sequential. Phase
    /// 2 Task 2.2 implements the runtime scheduler.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_group: Option<u8>,
}

/// Scheduling policy for the solution's runtime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SchedulingPolicy {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub decisions: Vec<String>,
}

/// A selectable option for enum-typed parameters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EnumOption {
    /// Machine-readable value stored in config, e.g. `"mpu6050"`.
    pub value: String,
    /// Human-readable label shown in the wizard dropdown.
    pub label: String,
    /// Optional tooltip / extended description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Conditional visibility dependency — show this parameter only when a
/// parent parameter has a specific value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ParameterDependency {
    /// The `id` of the parent parameter to watch.
    pub parameter_id: String,
    /// The parent value that makes this parameter visible.
    pub when_value: String,
    /// If set, show this parameter when the parent value does NOT equal this.
    /// Takes precedence over `when_value` when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when_not_value: Option<String>,
}

/// A user-configurable parameter exposed by the solution wizard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UserParameterDefinition {
    pub id: String,
    pub label: String,
    /// Optional Chinese (zh-CN) translation of `label`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_zh: Option<String>,
    pub required: bool,
    #[serde(default)]
    pub secret: bool,
    pub description: String,
    /// Optional Chinese (zh-CN) translation of `description`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_zh: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<serde_json::Value>,
    /// If present, render as a `<select>` dropdown instead of a text input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<EnumOption>>,
    /// If present, this parameter is only visible when the referenced
    /// parent parameter has the specified value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<ParameterDependency>,
}

/// The component bundle required and optionally available for a solution.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct ComponentBundle {
    /// Components that must be included.
    pub required: Vec<String>,
    /// Components the user may optionally add.
    #[serde(default)]
    pub optional: Vec<String>,
}

/// Runtime binding — how a solution maps to actual ESP-IDF build dependencies.
///
/// Separates user-visible contract (SolutionDefinition fields) from implementation
/// details (which components come from Brookesia registry vs. custom codegen templates).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct RuntimeBinding {
    /// Implementation family identifier (e.g. `"esp-drone"`, `"brookesia_service"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    /// Brookesia (or other) managed IDF components to pull via `idf_component.yml`.
    /// Format: `"component_name"` or `"component_name:version_spec"`.
    /// Example: `["brookesia_service_wifi:~0.7", "brookesia_service_nvs:~0.7"]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub managed_components: Vec<ManagedComponentDep>,
    /// Custom components generated from codegen templates (not from any registry).
    /// These are component directory names under `components/` in the generated project.
    /// Example: `["rshome_failsafe", "rshome_imu", "rshome_motor_control"]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_components: Vec<String>,
    /// Maps user parameter IDs to component config fields.
    /// Key: user_parameter.id, Value: target component config path.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub parameter_projection: BTreeMap<String, String>,
    /// Board assembly to use for device/peripheral declarations.
    /// Links to an [`AssemblyId`] in the [`AssemblyRegistry`](crate::assembly::AssemblyRegistry).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board_assembly: Option<AssemblyId>,
    /// Declarative HA entity export definitions (command/state bindings).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ha_entities: Vec<HaEntityExportDefinition>,
    /// Codegen path: Brookesia-managed (Path A, default) or self-hosted (Path B).
    #[serde(default)]
    pub codegen_path: CodegenPath,
}

/// A managed ESP-IDF component dependency (from Espressif Component Registry or Git).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ManagedComponentDep {
    /// Component name as registered (e.g. `"brookesia_service_wifi"`).
    pub name: String,
    /// Version specification (e.g. `"~0.7"`, `">=0.7.0,<1.0"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Git repository URL (for components not in the registry).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<String>,
    /// Component registry namespace (default: `espressif`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

/// What a `UserParameterOverride` does to the base solution's parameter list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UserParameterOverrideOp {
    /// Append a new parameter to the base's `user_parameters[]`.
    /// Pipeline errors if an existing parameter has the same `id`.
    Add,
    /// Delete the base parameter with matching `id`. Pipeline errors if
    /// the `id` is not in the base's `user_parameters[]`.
    Remove,
    /// Replace the base parameter with matching `id`. Pipeline errors if
    /// the `id` is not in the base's `user_parameters[]`. This is useful
    /// when a variant needs to change enum options or conditional
    /// dependencies (not just the default value — use `parameter_defaults`
    /// for default-only tweaks).
    Replace,
}

/// A single structural change a variant applies to the base solution's
/// `user_parameters[]`. Richer than `parameter_defaults` which can only
/// override defaults — this can add, remove, or replace an entry wholesale.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UserParameterOverride {
    /// Operation kind.
    pub op: UserParameterOverrideOp,
    /// Parameter id to act on. For `Add`/`Replace` this must equal
    /// `parameter.id`; for `Remove` it is the sole input.
    pub id: String,
    /// New parameter definition. Required for `Add` + `Replace`; must be
    /// `None` for `Remove`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameter: Option<UserParameterDefinition>,
}

/// Variant-level deltas to a solution's `runtime_binding`. Every field
/// is optional / empty-by-default; only set what the variant actually
/// changes. Merge semantics:
///
/// - `add_managed_components` / `add_custom_components` are appended.
/// - `remove_managed_components` matches by component name; unknown
///   names are silently ignored (variant may want a "remove if present").
/// - `remove_custom_components` matches by exact string.
/// - `board_assembly`, when `Some`, replaces the base's value outright.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct RuntimeBindingOverlay {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_managed_components: Vec<ManagedComponentDep>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remove_managed_components: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_custom_components: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remove_custom_components: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board_assembly: Option<AssemblyId>,
}

/// An intra-solution variant (e.g. Profile A vs Profile B).
///
/// The 4 overlay-delta fields at the bottom (`active_flag_add`,
/// `active_flag_remove`, `user_parameter_overrides`,
/// `runtime_binding_override`) were added by the
/// `rshome-codegen-variants` PRD; see the crate-level doc comment in
/// this module for the full merge-precedence explanation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SolutionVariantDefinition {
    pub id: VariantId,
    pub label: String,
    /// Optional Chinese (zh-CN) translation of `label`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_zh: Option<String>,
    #[serde(default)]
    pub required_caps: Vec<Capability>,
    /// Override default values for parameters that exist in the base
    /// solution's `user_parameters[]`. Keys are parameter ids; values
    /// replace `default_value` only — use `user_parameter_overrides`
    /// to add, remove, or restructure a parameter.
    #[serde(default)]
    pub parameter_defaults: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub add_components: Vec<String>,
    #[serde(default)]
    pub remove_components: Vec<String>,
    #[serde(default)]
    pub add_external_contracts: Vec<String>,
    /// Active-flag deltas — added to `ValidatedConfig.active_flags` at
    /// pipeline stage 3.5. Typically used by mutually-exclusive variant
    /// groups (e.g. PWM variant sets `USE_PWM_OUTPUT`; DShot variant
    /// sets `USE_DSHOT` and removes `USE_PWM_OUTPUT`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_flag_add: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_flag_remove: Vec<String>,
    /// Structural changes to the base's `user_parameters[]` — add,
    /// remove, or replace entries. For default-only changes prefer
    /// `parameter_defaults`; this is for variants that introduce new
    /// params (e.g. DShot variant adds `dshot_rate_khz`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub user_parameter_overrides: Vec<UserParameterOverride>,
    /// Runtime-binding deltas. See [`RuntimeBindingOverlay`] for the
    /// per-field semantics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_binding_override: Option<RuntimeBindingOverlay>,
}

// ── Solution definition ──────────────────────────────────────────────────────

/// A complete solution definition — the primary user-facing selection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SolutionDefinition {
    /// Unique identifier, e.g. `"camera_stream"`.
    pub id: SolutionId,
    /// Human-readable label for the wizard UI.
    pub label: String,
    /// Optional Chinese (zh-CN) translation of `label`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_zh: Option<String>,
    /// Solution category.
    pub kind: SolutionKind,
    /// Module IDs this solution can run on.
    pub supported_modules: Vec<String>,
    /// Fixed input surfaces for this solution.
    #[serde(default)]
    pub fixed_inputs: Vec<InputSurface>,
    /// Fixed output surfaces for this solution.
    #[serde(default)]
    pub fixed_outputs: Vec<OutputSurface>,
    /// Fixed orchestration steps.
    #[serde(default)]
    pub fixed_orchestration: Vec<OrchestrationStep>,
    /// Scheduling policy.
    pub scheduling: SchedulingPolicy,
    /// User-configurable parameters.
    #[serde(default)]
    pub user_parameters: Vec<UserParameterDefinition>,
    /// Signal paths demonstrating the solution's data flow.
    #[serde(default)]
    pub feedback_paths: Vec<SignalPath>,
    /// Intra-solution variants.
    #[serde(default)]
    pub variants: Vec<SolutionVariantDefinition>,
    /// Required and optional components.
    pub component_bundle: ComponentBundle,
    /// Runtime binding — maps this solution to managed IDF components and/or custom templates.
    /// Separates user-visible contract from build-time implementation details.
    #[serde(default)]
    pub runtime_binding: RuntimeBinding,
    /// External system contracts (e.g. "Telegram Bot API", "rshome-ha API").
    #[serde(default)]
    pub external_contracts: Vec<String>,
    /// Network topology for wizard display.
    #[serde(default)]
    pub network_topology: NetworkTopology,
    /// Target domain for wizard scoping. `None` means visible in all domains.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<DomainKind>,
    /// Architecture tier — single MCU, dual MCU, or SBC+MCU.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture_tier: Option<ArchitectureTier>,
    /// Communication chains this solution implements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub communication_chains: Option<Vec<CommunicationChainKind>>,
    /// GPIO pin assignments for this solution (displayed in wizard pin diagram).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_assignments: Option<Vec<PinAssignment>>,

    // ── Vehicle & Aircraft Control — extended annotations ──────────────────
    //
    // The fields below back `docs/vehicle-aircraft-control-dag.md`. They are
    // `Option<_>` + `skip_serializing_if` so they stay invisible in JSON for
    // solutions outside the Vehicle & Aircraft Control domain.
    /// Topology preset — doc §L2 / va-residuals ADR-01. Auto-populated by
    /// `SolutionRegistry::populate_topology_category()` from `control_uplink`
    /// + `video_downlink`; explicit overrides live in that function (e.g.
    ///   `mcu_sbc_bridge_solution` is SBC-resident despite a CRSF uplink and
    ///   is pinned to `ResearchHybrid`). Non-V&A solutions leave it `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topology_category: Option<TopologyKind>,
    /// Firmware lineage used for compatibility filtering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<ImplementationFamily>,
    /// Form-factor families this solution supports. Validates the
    /// `(solution, form_factor)` pair at wizard time per doc §"DAG edge
    /// summary".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_factor_families: Option<Vec<FormFactorKind>>,
    /// Control uplink chain annotation — doc §L5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_uplink: Option<ControlUplinkKind>,
    /// Video downlink chain annotation — doc §L5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_downlink: Option<VideoDownlinkKind>,
    /// Telemetry back-channel annotation — doc §L5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<TelemetryKind>,
    /// Minimum sensor tier this solution requires — doc §L6.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensor_tier_min: Option<SensorTierKind>,
    /// Non-IMU sensor requirements (GPS, depth, pressure, encoders, etc.) —
    /// orthogonal to `sensor_tier_min`. Added 2026-04-21 by va-residuals
    /// Phase 3 T3.1 / ADR-06. Empty vec = no non-IMU sensor required.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_sensors: Vec<SensorRequirement>,
    /// Wired MCU↔SBC companion link (UART / CAN / I²C). `None` = no SBC
    /// companion. Added 2026-04-21 by va-residuals Phase 3 T3.2 / ADR-07.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub companion_link: Option<CompanionLinkKind>,
    /// Actuator family (drives the mixing algorithm) — doc §L6.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actuator_family: Option<ActuatorFamily>,
    /// Power rail scheme — doc §L6.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power_rails: Option<PowerRailKind>,
    /// Failsafe policy — doc §L5.5. `None` for non-actuator solutions
    /// (TX, video board) that have no motors to cut.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failsafe: Option<FailsafeInfo>,
    /// Which ESP32 family satisfies which role for this solution — doc §L3.5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chip_coverage: Option<BTreeMap<ChipFamilyKind, ChipCoverageStatus>>,
}

// ── Registry ─────────────────────────────────────────────────────────────────

/// Registry of all known solutions.
#[derive(Debug, Clone, Default)]
pub struct SolutionRegistry {
    solutions: BTreeMap<SolutionId, SolutionDefinition>,
}

impl SolutionRegistry {
    /// Create an empty solution registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a solution definition.
    pub fn register(&mut self, def: SolutionDefinition) {
        self.solutions.insert(def.id.clone(), def);
    }

    /// Look up a solution by ID.
    pub fn get(&self, id: &str) -> Option<&SolutionDefinition> {
        self.solutions.get(id)
    }

    /// Iterate over all registered solutions.
    pub fn all(&self) -> impl Iterator<Item = &SolutionDefinition> {
        self.solutions.values()
    }

    /// Return all solutions compatible with the given module.
    pub fn for_module(&self, module_id: &str) -> Vec<&SolutionDefinition> {
        self.solutions
            .values()
            .filter(|s| s.supported_modules.iter().any(|m| m == module_id))
            .collect()
    }

    /// Return all solutions of the given kind.
    pub fn for_kind(&self, kind: SolutionKind) -> Vec<&SolutionDefinition> {
        self.solutions.values().filter(|s| s.kind == kind).collect()
    }

    /// Populate `topology_category` on every V&A solution from its chain
    /// annotations. Idempotent — already-set values are preserved so
    /// per-solution overrides (e.g. `mcu_sbc_bridge_solution` pinned to
    /// ResearchHybrid despite CRSF uplink) can live inline in the registry
    /// block.
    ///
    /// Called at the end of `default_solution_registry()` per ADR-01.
    pub fn populate_topology_category(&mut self) {
        for sol in self.solutions.values_mut() {
            if sol.domain != Some(DomainKind::VehicleAircraftControl) {
                continue;
            }
            if sol.topology_category.is_some() {
                continue; // respect explicit per-solution overrides
            }
            // Exceptions: solutions that defy chain-based inference.
            if sol.id == "mcu_sbc_bridge_solution" {
                sol.topology_category = Some(TopologyKind::ResearchHybrid);
                continue;
            }
            // Primary inference: control_uplink → topology.
            let topo = match sol.control_uplink {
                Some(ControlUplinkKind::Crsf) => Some(TopologyKind::StandardFpv),
                Some(
                    ControlUplinkKind::WifiCrtp
                    | ControlUplinkKind::EspNow
                    | ControlUplinkKind::WifiMesh
                    | ControlUplinkKind::Wifi80211lr
                    | ControlUplinkKind::BleMesh
                    | ControlUplinkKind::Sbus
                    | ControlUplinkKind::BleGatt
                    | ControlUplinkKind::UsbCdc,
                ) => Some(TopologyKind::DiyLowcost),
                Some(ControlUplinkKind::WifiMavlink) => Some(TopologyKind::ResearchHybrid),
                Some(ControlUplinkKind::None) | None => None,
            };
            if topo.is_some() {
                sol.topology_category = topo;
                continue;
            }
            // Fallback: infer from video_downlink for video-only solutions
            // (analog_vtx / mjpeg_uart → standard_fpv; mjpeg_http → diy_lowcost;
            // webrtc_sbc → research_hybrid).
            sol.topology_category = match sol.video_downlink {
                Some(VideoDownlinkKind::AnalogVtx | VideoDownlinkKind::MjpegUart) => {
                    Some(TopologyKind::StandardFpv)
                }
                Some(VideoDownlinkKind::MjpegHttp) => Some(TopologyKind::DiyLowcost),
                Some(VideoDownlinkKind::WebrtcSbc) => Some(TopologyKind::ResearchHybrid),
                _ => None,
            };
        }
    }
}

// ── Default registry ─────────────────────────────────────────────────────────

/// Standard Brookesia managed components for WiFi-based firmware solutions.
fn brookesia_wifi_binding() -> RuntimeBinding {
    RuntimeBinding {
        family: Some("brookesia_service".into()),
        managed_components: vec![
            ManagedComponentDep {
                name: "brookesia_lib_utils".into(),
                version: Some("~0.7".into()),
                git: None,
                namespace: None,
            },
            ManagedComponentDep {
                name: "brookesia_service_manager".into(),
                version: Some("~0.7".into()),
                git: None,
                namespace: None,
            },
            ManagedComponentDep {
                name: "brookesia_service_helper".into(),
                version: Some("~0.7".into()),
                git: None,
                namespace: None,
            },
            ManagedComponentDep {
                name: "brookesia_service_wifi".into(),
                version: Some("~0.7".into()),
                git: None,
                namespace: None,
            },
            ManagedComponentDep {
                name: "brookesia_service_nvs".into(),
                version: Some("~0.7".into()),
                git: None,
                namespace: None,
            },
        ],
        custom_components: vec![],
        parameter_projection: BTreeMap::new(),
        ..Default::default()
    }
}

/// Standard Brookesia managed components for vehicle firmware solutions.
/// Includes WiFi base + custom vehicle components from codegen templates.
fn brookesia_vehicle_binding(custom: Vec<String>) -> RuntimeBinding {
    let mut binding = brookesia_wifi_binding();
    binding.family = Some("esp-drone".into());
    binding.custom_components = custom;
    binding
}

// ── Vehicle parameter helpers ────────────────────────────────────────────────

fn vehicle_type_param() -> UserParameterDefinition {
    UserParameterDefinition {
        id: "vehicle_type".into(),
        label: "Vehicle Type".into(),
        label_zh: Some("载具类型".into()),
        required: true,
        secret: false,
        description: "Vehicle type".into(),
        description_zh: Some("载具类型。".into()),
        default_value: Some(serde_json::Value::String("car".into())),
        enum_values: Some(vec![
            EnumOption {
                value: "car".into(),
                label: "Ground Car".into(),
                description: None,
            },
            EnumOption {
                value: "drone".into(),
                label: "Drone / Quadcopter".into(),
                description: None,
            },
            EnumOption {
                value: "boat".into(),
                label: "Boat / ROV".into(),
                description: None,
            },
        ]),
        depends_on: None,
    }
}

fn control_protocol_param() -> UserParameterDefinition {
    UserParameterDefinition {
        id: "control_protocol".into(),
        label: "Control Protocol".into(),
        label_zh: Some("控制协议".into()),
        required: true,
        secret: false,
        description: "Control uplink protocol".into(),
        description_zh: Some("控制上行链路协议。".into()),
        default_value: Some(serde_json::Value::String("wifi_mavlink".into())),
        enum_values: Some(vec![
            EnumOption {
                value: "elrs_crsf".into(),
                label: "ELRS (CRSF)".into(),
                description: Some(
                    "ExpressLRS. Open-source, 2.4/900 MHz, up to 1000 Hz packet rate.".into(),
                ),
            },
            EnumOption {
                value: "crossfire_crsf".into(),
                label: "TBS Crossfire (CRSF)".into(),
                description: Some(
                    "Team BlackSheep. Long-range, bidirectional, proven reliability.".into(),
                ),
            },
            EnumOption {
                value: "sbus".into(),
                label: "SBUS".into(),
                description: Some(
                    "Legacy RC protocol, single-direction. Backward compatibility.".into(),
                ),
            },
            EnumOption {
                value: "esp_now".into(),
                label: "ESP-NOW".into(),
                description: Some(
                    "Connectionless ESP-to-ESP. Best for DIY remotes, <200m range.".into(),
                ),
            },
            EnumOption {
                value: "wifi_lr".into(),
                label: "802.11 LR".into(),
                description: Some("WiFi Long Range. ESP-to-ESP only, >1km range.".into()),
            },
            EnumOption {
                value: "wifi_mavlink".into(),
                label: "WiFi + MAVLink".into(),
                description: Some(
                    "Full-featured over WiFi. Best for debug/telemetry, not sole safety link."
                        .into(),
                ),
            },
        ]),
        depends_on: None,
    }
}

fn imu_axis_tier_param() -> UserParameterDefinition {
    UserParameterDefinition {
        id: "imu_axis_tier".into(),
        label: "IMU Axis Tier".into(),
        label_zh: Some("IMU 轴数".into()),
        required: false,
        secret: false,
        description: "IMU sensor tier. Optional for simple ground vehicles.".into(),
        description_zh: Some("IMU 传感器轴数。简单地面载具可不选。".into()),
        default_value: Some(serde_json::Value::String("none".into())),
        enum_values: Some(vec![
            EnumOption {
                value: "none".into(),
                label: "No IMU".into(),
                description: Some(
                    "No inertial sensing. Suitable for simple ground vehicles.".into(),
                ),
            },
            EnumOption {
                value: "6_axis".into(),
                label: "6-Axis (Accel + Gyro)".into(),
                description: Some("Roll/pitch detection. Yaw drifts over time.".into()),
            },
            EnumOption {
                value: "9_axis".into(),
                label: "9-Axis (+ Magnetometer)".into(),
                description: Some("Stable heading/yaw. Needs magnetic calibration.".into()),
            },
            EnumOption {
                value: "10_axis".into(),
                label: "10-Axis (+ Barometer)".into(),
                description: Some("Adds relative altitude. Best for drones.".into()),
            },
        ]),
        depends_on: None,
    }
}

fn imu_chip_param() -> UserParameterDefinition {
    UserParameterDefinition {
        id: "imu_chip".into(),
        label: "IMU Chip".into(),
        label_zh: Some("IMU 芯片".into()),
        required: false,
        secret: false,
        description: "IMU chip selection".into(),
        description_zh: Some("IMU 芯片型号。".into()),
        default_value: Some(serde_json::Value::String("mpu6050".into())),
        enum_values: Some(vec![
            // 6-axis
            EnumOption {
                value: "mpu6050".into(),
                label: "MPU6050".into(),
                description: Some("6-axis. Classic, cheapest, I2C 0x68.".into()),
            },
            EnumOption {
                value: "bmi270".into(),
                label: "BMI270".into(),
                description: Some("6-axis. Modern, low-power, 6.4kHz gyro ODR.".into()),
            },
            EnumOption {
                value: "icm42688p".into(),
                label: "ICM-42688-P".into(),
                description: Some("6-axis. High performance, drone-grade.".into()),
            },
            // 9-axis
            EnumOption {
                value: "icm20948".into(),
                label: "ICM-20948".into(),
                description: Some("9-axis. Full integrated, TDK.".into()),
            },
            EnumOption {
                value: "lsm9ds1".into(),
                label: "LSM9DS1".into(),
                description: Some("9-axis. ST, dual I2C address.".into()),
            },
            EnumOption {
                value: "bno055".into(),
                label: "BNO055".into(),
                description: Some("9-axis. On-chip fusion, direct quaternion output.".into()),
            },
            // 10-axis combos
            EnumOption {
                value: "icm20948_bmp280".into(),
                label: "ICM-20948 + BMP280".into(),
                description: Some("10-axis. 9-axis IMU + barometer combo.".into()),
            },
            EnumOption {
                value: "bno055_bmp388".into(),
                label: "BNO055 + BMP388".into(),
                description: Some("10-axis. Fusion IMU + high-precision barometer.".into()),
            },
        ]),
        // Hidden when imu_axis_tier = "none"; visible for any other tier
        depends_on: Some(ParameterDependency {
            parameter_id: "imu_axis_tier".into(),
            when_value: String::new(),
            when_not_value: Some("none".into()),
        }),
    }
}

fn actuator_type_param() -> UserParameterDefinition {
    UserParameterDefinition {
        id: "actuator_type".into(),
        label: "Actuator Type".into(),
        label_zh: Some("执行器类型".into()),
        required: true,
        secret: false,
        description: "Motor/servo configuration".into(),
        description_zh: Some("电机/舵机配置。".into()),
        default_value: Some(serde_json::Value::String("h_bridge_2wd".into())),
        enum_values: Some(vec![
            EnumOption {
                value: "h_bridge_2wd".into(),
                label: "H-Bridge 2WD Differential".into(),
                description: Some("Two brushed motors, left/right differential steering.".into()),
            },
            EnumOption {
                value: "h_bridge_4wd".into(),
                label: "H-Bridge 4WD Differential".into(),
                description: Some("Four brushed motors, tank-style steering.".into()),
            },
            EnumOption {
                value: "esc_servo_ackermann".into(),
                label: "ESC + Servo (Ackermann)".into(),
                description: Some("One drive motor + one steering servo. RC car style.".into()),
            },
            EnumOption {
                value: "esc_differential".into(),
                label: "ESC Differential".into(),
                description: Some("Two brushless motors, differential steering.".into()),
            },
            EnumOption {
                value: "brushless_quad".into(),
                label: "4x Brushless (Quadcopter)".into(),
                description: Some("Four brushless motors with ESC. Drone configuration.".into()),
            },
        ]),
        depends_on: None,
    }
}

fn camera_sensor_param() -> UserParameterDefinition {
    UserParameterDefinition {
        id: "camera_sensor".into(),
        label: "Camera Sensor".into(),
        label_zh: Some("摄像头传感器".into()),
        required: false,
        secret: false,
        description: "Camera sensor model".into(),
        description_zh: Some("摄像头传感器型号。".into()),
        default_value: Some(serde_json::Value::String("ov2640".into())),
        enum_values: Some(vec![
            EnumOption {
                value: "ov2640".into(),
                label: "OV2640 (2MP)".into(),
                description: Some("Most common, cheapest, JPEG hardware encode.".into()),
            },
            EnumOption {
                value: "ov3660".into(),
                label: "OV3660 (3MP)".into(),
                description: Some("Higher resolution, auto-focus.".into()),
            },
            EnumOption {
                value: "ov5640".into(),
                label: "OV5640 (5MP)".into(),
                description: Some("Highest resolution, auto-focus, more PSRAM needed.".into()),
            },
        ]),
        depends_on: None,
    }
}

fn frame_size_param() -> UserParameterDefinition {
    UserParameterDefinition {
        id: "frame_size".into(),
        label: "Frame Size".into(),
        label_zh: Some("图像分辨率".into()),
        required: false,
        secret: false,
        description: "Camera resolution".into(),
        description_zh: Some("摄像头分辨率。".into()),
        default_value: Some(serde_json::Value::String("qvga".into())),
        enum_values: Some(vec![
            EnumOption {
                value: "qqvga".into(),
                label: "QQVGA (160x120)".into(),
                description: None,
            },
            EnumOption {
                value: "qvga".into(),
                label: "QVGA (320x240)".into(),
                description: None,
            },
            EnumOption {
                value: "vga".into(),
                label: "VGA (640x480)".into(),
                description: None,
            },
            EnumOption {
                value: "svga".into(),
                label: "SVGA (800x600)".into(),
                description: None,
            },
        ]),
        depends_on: None,
    }
}

#[allow(dead_code)] // Phase 1 R-07 scaffolding: video-quality enum constructor for GCS params rollout.
fn video_level_param(
    id: &str,
    label: &str,
    label_zh: &str,
    desc: &str,
    desc_zh: &str,
) -> UserParameterDefinition {
    UserParameterDefinition {
        id: id.into(),
        label: label.into(),
        label_zh: Some(label_zh.into()),
        required: false,
        secret: false,
        description: desc.into(),
        description_zh: Some(desc_zh.into()),
        default_value: Some(serde_json::Value::String("off".into())),
        enum_values: Some(vec![
            EnumOption {
                value: "off".into(),
                label: "Off".into(),
                description: None,
            },
            EnumOption {
                value: "snapshot".into(),
                label: "Snapshot".into(),
                description: Some("Periodic single-frame capture.".into()),
            },
            EnumOption {
                value: "very_low_fps".into(),
                label: "Very Low FPS".into(),
                description: Some("1-3 fps, minimal bandwidth.".into()),
            },
        ]),
        depends_on: None,
    }
}

fn control_rate_hz_param() -> UserParameterDefinition {
    UserParameterDefinition {
        id: "control_rate_hz".into(),
        label: "Control Rate (Hz)".into(),
        label_zh: Some("控制频率(Hz)".into()),
        required: false,
        secret: false,
        description: "Control loop frequency in Hz".into(),
        description_zh: Some("控制循环频率,单位赫兹。".into()),
        default_value: Some(serde_json::Value::String("100".into())),
        enum_values: Some(vec![
            EnumOption {
                value: "50".into(),
                label: "50 Hz".into(),
                description: Some("Low load. For slow ground vehicles.".into()),
            },
            EnumOption {
                value: "100".into(),
                label: "100 Hz".into(),
                description: Some("Standard. Good for most vehicles.".into()),
            },
            EnumOption {
                value: "250".into(),
                label: "250 Hz".into(),
                description: Some("High rate. For agile drones.".into()),
            },
            EnumOption {
                value: "500".into(),
                label: "500 Hz".into(),
                description: Some("Racing. Maximum responsiveness.".into()),
            },
            EnumOption {
                value: "1000".into(),
                label: "1000 Hz".into(),
                description: Some("Competition. Requires fast IMU.".into()),
            },
        ]),
        depends_on: None,
    }
}

fn failsafe_timeout_ms_param() -> UserParameterDefinition {
    UserParameterDefinition {
        id: "failsafe_timeout_ms".into(),
        label: "Failsafe Timeout (ms)".into(),
        label_zh: Some("失效保护超时(毫秒)".into()),
        required: false,
        secret: false,
        description: "Milliseconds without control packet before failsafe activates".into(),
        description_zh: Some("在多少毫秒未收到控制包后启动失效保护。".into()),
        default_value: Some(serde_json::Value::String("500".into())),
        enum_values: Some(vec![
            EnumOption {
                value: "200".into(),
                label: "200 ms".into(),
                description: Some("Aggressive. For racing/FPV.".into()),
            },
            EnumOption {
                value: "500".into(),
                label: "500 ms".into(),
                description: Some("Standard. Balanced safety.".into()),
            },
            EnumOption {
                value: "1000".into(),
                label: "1000 ms".into(),
                description: Some("Conservative. Tolerates brief dropouts.".into()),
            },
            EnumOption {
                value: "2000".into(),
                label: "2000 ms".into(),
                description: Some("Relaxed. For test/debug.".into()),
            },
        ]),
        depends_on: None,
    }
}

/// Phone-side uplink for `phone_bridge_solution`. The gateway either accepts
/// commands over BLE GATT (most mobile apps) or USB CDC (tethered).
fn phone_side_link_param() -> UserParameterDefinition {
    UserParameterDefinition {
        id: "phone_side_link".into(),
        label: "Phone-side link".into(),
        label_zh: Some("手机侧链路".into()),
        required: true,
        secret: false,
        description: "How the phone talks to the ESP32 gateway".into(),
        description_zh: Some("手机如何与 ESP32 网关通信。".into()),
        default_value: Some(serde_json::Value::String("ble_gatt".into())),
        enum_values: Some(vec![
            EnumOption {
                value: "ble_gatt".into(),
                label: "BLE GATT".into(),
                description: Some("Most common. Works with iOS + Android apps.".into()),
            },
            EnumOption {
                value: "usb_cdc".into(),
                label: "USB CDC".into(),
                description: Some("USB OTG cable. Lowest latency, requires tether.".into()),
            },
        ]),
        depends_on: None,
    }
}

/// Vehicle-side downlink for `phone_bridge_solution`. Picks how the gateway
/// relays to one or many vehicles (point-to-point, multi-hop, long-range, or
/// swarm).
fn vehicle_side_protocol_param() -> UserParameterDefinition {
    UserParameterDefinition {
        id: "vehicle_side_protocol".into(),
        label: "Vehicle-side protocol".into(),
        label_zh: Some("车辆侧协议".into()),
        required: true,
        secret: false,
        description: "Protocol used to reach the vehicle fleet".into(),
        description_zh: Some("网关用于访问车辆群的协议。".into()),
        default_value: Some(serde_json::Value::String("esp_now".into())),
        enum_values: Some(vec![
            EnumOption {
                value: "esp_now".into(),
                label: "ESP-NOW".into(),
                description: Some("Point-to-point or broadcast, lowest latency.".into()),
            },
            EnumOption {
                value: "wifi_mesh".into(),
                label: "Wi-Fi Mesh".into(),
                description: Some(
                    "ESP-WIFI-MESH multi-hop tree for multi-vehicle coverage.".into(),
                ),
            },
            EnumOption {
                value: "wifi_80211lr".into(),
                label: "802.11 LR".into(),
                description: Some("Espressif Long Range mode, ~1 km LOS, ESP32-only.".into()),
            },
            EnumOption {
                value: "ble_mesh".into(),
                label: "BLE Mesh".into(),
                description: Some("Managed flooding for swarm coordination.".into()),
            },
        ]),
        depends_on: None,
    }
}

// ── GCS-side user_parameters helpers (PRD Task 1.1) ────────────────────────
// Shared by mavlink_groundstation_solution, video_board_sbc_companion_solution,
// and web_ui_groundstation_solution. Closes master design §10.1 (GCS
// solutions previously shipped with empty `user_parameters[]`).

fn wifi_ssid_param() -> UserParameterDefinition {
    UserParameterDefinition {
        id: "wifi_ssid".into(),
        label: "Wi-Fi SSID".into(),
        label_zh: Some("Wi-Fi 名称".into()),
        required: true,
        secret: false,
        description: "Network name (1-32 chars) the groundstation joins on boot.".into(),
        description_zh: Some("地面站启动时加入的 Wi-Fi 网络名称 (1-32 字符)。".into()),
        default_value: Some(serde_json::Value::String("rshome-gcs".into())),
        enum_values: None,
        depends_on: None,
    }
}

fn wifi_password_param() -> UserParameterDefinition {
    UserParameterDefinition {
        id: "wifi_password".into(),
        label: "Wi-Fi password".into(),
        label_zh: Some("Wi-Fi 密码".into()),
        required: false,
        secret: true,
        description: "WPA2/3 password (8-63 chars) or empty for open networks.".into(),
        description_zh: Some("WPA2/3 密码 (8-63 字符) 或留空表示开放网络。".into()),
        default_value: Some(serde_json::Value::String("".into())),
        enum_values: None,
        depends_on: None,
    }
}

fn mavlink_udp_port_param() -> UserParameterDefinition {
    UserParameterDefinition {
        id: "mavlink_udp_port".into(),
        label: "MAVLink UDP port".into(),
        label_zh: Some("MAVLink UDP 端口".into()),
        required: true,
        secret: false,
        description: "UDP port the groundstation listens on; QGroundControl defaults to 14550."
            .into(),
        description_zh: Some("地面站监听的 UDP 端口;QGroundControl 默认 14550。".into()),
        default_value: Some(serde_json::Value::String("14550".into())),
        enum_values: None,
        depends_on: None,
    }
}

fn telemetry_rate_hz_param() -> UserParameterDefinition {
    UserParameterDefinition {
        id: "telemetry_rate_hz".into(),
        label: "Telemetry rate (Hz)".into(),
        label_zh: Some("遥测刷新率 (Hz)".into()),
        required: true,
        secret: false,
        description: "Heartbeat / attitude downlink rate (1-50 Hz; 4 Hz is the QGC default)."
            .into(),
        description_zh: Some("心跳 / 姿态下行刷新率 (1-50 Hz;QGC 默认 4 Hz)。".into()),
        default_value: Some(serde_json::Value::String("4".into())),
        enum_values: Some(vec![
            EnumOption {
                value: "1".into(),
                label: "1 Hz".into(),
                description: Some("Low bandwidth (telemetry-link saturation).".into()),
            },
            EnumOption {
                value: "4".into(),
                label: "4 Hz".into(),
                description: Some("Standard QGroundControl default.".into()),
            },
            EnumOption {
                value: "10".into(),
                label: "10 Hz".into(),
                description: Some("Smooth attitude indicator.".into()),
            },
            EnumOption {
                value: "20".into(),
                label: "20 Hz".into(),
                description: Some("Higher rate for tuning sessions.".into()),
            },
            EnumOption {
                value: "50".into(),
                label: "50 Hz".into(),
                description: Some("Diagnostic / log capture only.".into()),
            },
        ]),
        depends_on: None,
    }
}

fn webrtc_signaling_url_param() -> UserParameterDefinition {
    UserParameterDefinition {
        id: "webrtc_signaling_url".into(),
        label: "WebRTC signaling URL".into(),
        label_zh: Some("WebRTC 信令服务器 URL".into()),
        required: false,
        secret: false,
        description: "Signaling endpoint on the SBC (e.g. ws://groundstation.local:8080). Leave blank to auto-discover via mDNS.".into(),
        description_zh: Some("SBC 上的信令端点 (例如 ws://groundstation.local:8080)。留空则通过 mDNS 自动发现。".into()),
        default_value: Some(serde_json::Value::String("".into())),
        enum_values: None,
        depends_on: None,
    }
}

fn safe_stop_timeout_ms_param() -> UserParameterDefinition {
    UserParameterDefinition {
        id: "safe_stop_timeout_ms".into(),
        label: "SAFE_STOP Timeout (ms)".into(),
        label_zh: Some("SAFE_STOP 超时(毫秒)".into()),
        required: false,
        secret: false,
        description: "Milliseconds without any control before emergency brake".into(),
        description_zh: Some("在多少毫秒内未收到任何控制即触发紧急制动。".into()),
        default_value: Some(serde_json::Value::String("1000".into())),
        enum_values: Some(vec![
            EnumOption {
                value: "500".into(),
                label: "500 ms".into(),
                description: Some("Aggressive. For indoor/test.".into()),
            },
            EnumOption {
                value: "1000".into(),
                label: "1000 ms".into(),
                description: Some("Standard. Balanced safety.".into()),
            },
            EnumOption {
                value: "2000".into(),
                label: "2000 ms".into(),
                description: Some("Relaxed. For slow vehicles.".into()),
            },
            EnumOption {
                value: "5000".into(),
                label: "5000 ms".into(),
                description: Some("Very relaxed. For debug.".into()),
            },
        ]),
        depends_on: None,
    }
}

fn direct_probe_threshold_param() -> UserParameterDefinition {
    UserParameterDefinition {
        id: "direct_probe_threshold".into(),
        label: "Direct Probe Threshold".into(),
        label_zh: Some("直连探测阈值".into()),
        required: false,
        secret: false,
        description: "RSSI threshold for considering direct mode viable".into(),
        description_zh: Some("判定可进入直连模式的 RSSI 阈值。".into()),
        default_value: Some(serde_json::Value::String("-60".into())),
        enum_values: Some(vec![
            EnumOption {
                value: "-50".into(),
                label: "-50 dBm".into(),
                description: Some("Very close range only.".into()),
            },
            EnumOption {
                value: "-60".into(),
                label: "-60 dBm".into(),
                description: Some("Standard. Good signal required.".into()),
            },
            EnumOption {
                value: "-70".into(),
                label: "-70 dBm".into(),
                description: Some("Relaxed. Accepts weaker signal.".into()),
            },
            EnumOption {
                value: "-80".into(),
                label: "-80 dBm".into(),
                description: Some("Aggressive. May be unstable.".into()),
            },
        ]),
        depends_on: None,
    }
}

fn fallback_rssi_threshold_param() -> UserParameterDefinition {
    UserParameterDefinition {
        id: "fallback_rssi_threshold".into(),
        label: "Fallback RSSI Threshold".into(),
        label_zh: Some("回退 RSSI 阈值".into()),
        required: false,
        secret: false,
        description: "RSSI below which triggers fallback to relay".into(),
        description_zh: Some("RSSI 低于此值时回退到中继模式。".into()),
        default_value: Some(serde_json::Value::String("-75".into())),
        enum_values: Some(vec![
            EnumOption {
                value: "-65".into(),
                label: "-65 dBm".into(),
                description: Some("Early fallback. More conservative.".into()),
            },
            EnumOption {
                value: "-75".into(),
                label: "-75 dBm".into(),
                description: Some("Standard. Balanced.".into()),
            },
            EnumOption {
                value: "-85".into(),
                label: "-85 dBm".into(),
                description: Some("Late fallback. Pushes range.".into()),
            },
        ]),
        depends_on: None,
    }
}

// ── IoT parameter helpers ───────────────────────────────────────────────────

fn iot_poll_interval_param() -> UserParameterDefinition {
    UserParameterDefinition {
        id: "poll_interval_ms".into(),
        label: "Poll Interval (ms)".into(),
        label_zh: Some("轮询间隔(毫秒)".into()),
        required: false,
        secret: false,
        description: "How often to poll sensors in milliseconds".into(),
        description_zh: Some("传感器轮询间隔,单位毫秒。".into()),
        default_value: Some(serde_json::Value::String("5000".into())),
        enum_values: Some(vec![
            EnumOption {
                value: "1000".into(),
                label: "1 s".into(),
                description: Some("Fast polling.".into()),
            },
            EnumOption {
                value: "5000".into(),
                label: "5 s".into(),
                description: Some("Standard.".into()),
            },
            EnumOption {
                value: "10000".into(),
                label: "10 s".into(),
                description: Some("Low power.".into()),
            },
            EnumOption {
                value: "30000".into(),
                label: "30 s".into(),
                description: Some("Battery saving.".into()),
            },
            EnumOption {
                value: "60000".into(),
                label: "60 s".into(),
                description: Some("Ultra low power.".into()),
            },
        ]),
        depends_on: None,
    }
}

fn uplink_protocol_param() -> UserParameterDefinition {
    UserParameterDefinition {
        id: "uplink_protocol".into(),
        label: "Uplink Protocol".into(),
        label_zh: Some("上行协议".into()),
        required: true,
        secret: false,
        description: "How data is delivered to the backend".into(),
        description_zh: Some("数据如何上报到后端。".into()),
        default_value: Some(serde_json::Value::String("ha_native_api".into())),
        enum_values: Some(vec![
            EnumOption {
                value: "ha_native_api".into(),
                label: "rshome-ha API".into(),
                description: Some("Native API. Direct LAN integration.".into()),
            },
            EnumOption {
                value: "mqtt".into(),
                label: "MQTT".into(),
                description: Some("MQTT broker. WAN-capable, auto-discovery.".into()),
            },
        ]),
        depends_on: None,
    }
}

// ── Vehicle pin assignment helpers ───────────────────────────────────────────

/// Common motor + IMU + RC + status LED + ADC pins for single-MCU vehicle boards.
fn vehicle_control_pins() -> Vec<PinAssignment> {
    vec![
        PinAssignment {
            function: "Motor A PWM".into(),
            default_gpio: 16,
            alternatives: vec![33, 32],
            capability: "MotorControl".into(),
        },
        PinAssignment {
            function: "Motor A DIR".into(),
            default_gpio: 17,
            alternatives: vec![33],
            capability: "MotorControl".into(),
        },
        PinAssignment {
            function: "Motor B PWM".into(),
            default_gpio: 18,
            alternatives: vec![25, 26],
            capability: "MotorControl".into(),
        },
        PinAssignment {
            function: "Motor B DIR".into(),
            default_gpio: 19,
            alternatives: vec![27],
            capability: "MotorControl".into(),
        },
        PinAssignment {
            function: "IMU SDA".into(),
            default_gpio: 21,
            alternatives: vec![13, 14],
            capability: "I2c".into(),
        },
        PinAssignment {
            function: "IMU SCL".into(),
            default_gpio: 22,
            alternatives: vec![14, 13],
            capability: "I2c".into(),
        },
        PinAssignment {
            function: "RC UART RX".into(),
            default_gpio: 4,
            alternatives: vec![36],
            capability: "Uart".into(),
        },
        PinAssignment {
            function: "RC UART TX".into(),
            default_gpio: 5,
            alternatives: vec![39],
            capability: "Uart".into(),
        },
        PinAssignment {
            function: "Status LED".into(),
            default_gpio: 2,
            alternatives: vec![15],
            capability: "Gpio".into(),
        },
        PinAssignment {
            function: "Battery ADC".into(),
            default_gpio: 34,
            alternatives: vec![36, 39],
            capability: "Adc".into(),
        },
    ]
}

/// Vehicle control pins + failsafe relay (for car modules with FailsafeStop).
fn vehicle_car_pins() -> Vec<PinAssignment> {
    let mut pins = vehicle_control_pins();
    pins.push(PinAssignment {
        function: "Failsafe Relay".into(),
        default_gpio: 25,
        alternatives: vec![26],
        capability: "FailsafeStop".into(),
    });
    pins
}

/// Dual-MCU control board pins (motor + IMU + RC + failsafe + inter-board UART).
fn dual_mcu_control_board_pins() -> Vec<PinAssignment> {
    let mut pins = vehicle_car_pins();
    pins.push(PinAssignment {
        function: "Interboard TX".into(),
        default_gpio: 43,
        alternatives: vec![],
        capability: "Uart".into(),
    });
    pins.push(PinAssignment {
        function: "Interboard RX".into(),
        default_gpio: 44,
        alternatives: vec![],
        capability: "Uart".into(),
    });
    pins
}

/// Dual-MCU camera board pins (camera DVP + inter-board UART).
fn dual_mcu_camera_board_pins() -> Vec<PinAssignment> {
    vec![
        PinAssignment {
            function: "CAM PCLK".into(),
            default_gpio: 13,
            alternatives: vec![],
            capability: "Camera".into(),
        },
        PinAssignment {
            function: "CAM VSYNC".into(),
            default_gpio: 6,
            alternatives: vec![],
            capability: "Camera".into(),
        },
        PinAssignment {
            function: "CAM HREF".into(),
            default_gpio: 7,
            alternatives: vec![],
            capability: "Camera".into(),
        },
        PinAssignment {
            function: "CAM XCLK".into(),
            default_gpio: 14,
            alternatives: vec![],
            capability: "Camera".into(),
        },
        PinAssignment {
            function: "CAM SDA".into(),
            default_gpio: 4,
            alternatives: vec![],
            capability: "I2c".into(),
        },
        PinAssignment {
            function: "CAM SCL".into(),
            default_gpio: 5,
            alternatives: vec![],
            capability: "I2c".into(),
        },
        PinAssignment {
            function: "Interboard TX".into(),
            default_gpio: 43,
            alternatives: vec![],
            capability: "Uart".into(),
        },
        PinAssignment {
            function: "Interboard RX".into(),
            default_gpio: 44,
            alternatives: vec![],
            capability: "Uart".into(),
        },
    ]
}

/// All-in-one CAM pins: vehicle control + camera DVP on one board.
/// Camera DVP uses GPIO 6,7,10,11,12,13,14,15 — no conflict with motor GPIO 16-19.
fn all_in_one_cam_pins() -> Vec<PinAssignment> {
    let mut pins = vehicle_control_pins();
    // Camera DVP interface (non-conflicting with motor/IMU pins)
    pins.push(PinAssignment {
        function: "CAM PCLK".into(),
        default_gpio: 13,
        alternatives: vec![],
        capability: "Camera".into(),
    });
    pins.push(PinAssignment {
        function: "CAM VSYNC".into(),
        default_gpio: 6,
        alternatives: vec![],
        capability: "Camera".into(),
    });
    pins.push(PinAssignment {
        function: "CAM HREF".into(),
        default_gpio: 7,
        alternatives: vec![],
        capability: "Camera".into(),
    });
    pins.push(PinAssignment {
        function: "CAM XCLK".into(),
        default_gpio: 14,
        alternatives: vec![],
        capability: "Camera".into(),
    });
    pins.push(PinAssignment {
        function: "CAM SDA (SCCB)".into(),
        default_gpio: 10,
        alternatives: vec![],
        capability: "I2c".into(),
    });
    pins.push(PinAssignment {
        function: "CAM SCL (SCCB)".into(),
        default_gpio: 11,
        alternatives: vec![],
        capability: "I2c".into(),
    });
    pins.push(PinAssignment {
        function: "CAM D4".into(),
        default_gpio: 36,
        alternatives: vec![],
        capability: "Camera".into(),
    });
    pins.push(PinAssignment {
        function: "CAM D5".into(),
        default_gpio: 37,
        alternatives: vec![],
        capability: "Camera".into(),
    });
    pins
}

/// Remote control TX pins (joystick ADC axes + buttons + status LED).
fn remote_control_tx_pins() -> Vec<PinAssignment> {
    vec![
        PinAssignment {
            function: "Joystick X ADC".into(),
            default_gpio: 1,
            alternatives: vec![2, 3],
            capability: "Adc".into(),
        },
        PinAssignment {
            function: "Joystick Y ADC".into(),
            default_gpio: 2,
            alternatives: vec![1, 3],
            capability: "Adc".into(),
        },
        PinAssignment {
            function: "Joystick Z ADC".into(),
            default_gpio: 3,
            alternatives: vec![1, 4],
            capability: "Adc".into(),
        },
        PinAssignment {
            function: "Throttle ADC".into(),
            default_gpio: 4,
            alternatives: vec![5, 6],
            capability: "Adc".into(),
        },
        PinAssignment {
            function: "Button A".into(),
            default_gpio: 5,
            alternatives: vec![6, 7],
            capability: "Gpio".into(),
        },
        PinAssignment {
            function: "Button B".into(),
            default_gpio: 6,
            alternatives: vec![7, 8],
            capability: "Gpio".into(),
        },
        PinAssignment {
            function: "Button C".into(),
            default_gpio: 7,
            alternatives: vec![8, 9],
            capability: "Gpio".into(),
        },
        PinAssignment {
            function: "Button D".into(),
            default_gpio: 8,
            alternatives: vec![9, 10],
            capability: "Gpio".into(),
        },
        PinAssignment {
            function: "Status LED".into(),
            default_gpio: 48,
            alternatives: vec![38],
            capability: "Gpio".into(),
        },
        PinAssignment {
            function: "Battery ADC".into(),
            default_gpio: 9,
            alternatives: vec![10],
            capability: "Adc".into(),
        },
    ]
}

/// Video board pins (camera DVP + inter-board UART + status LED).
fn video_board_pins() -> Vec<PinAssignment> {
    let mut pins = dual_mcu_camera_board_pins();
    pins.push(PinAssignment {
        function: "Status LED".into(),
        default_gpio: 48,
        alternatives: vec![38],
        capability: "Gpio".into(),
    });
    pins
}

/// Receiver direct-drive pins (servo/ESC PWM outputs + RC input + failsafe).
fn receiver_direct_drive_pins() -> Vec<PinAssignment> {
    vec![
        PinAssignment {
            function: "RC UART RX".into(),
            default_gpio: 4,
            alternatives: vec![36],
            capability: "Uart".into(),
        },
        PinAssignment {
            function: "RC UART TX".into(),
            default_gpio: 5,
            alternatives: vec![39],
            capability: "Uart".into(),
        },
        PinAssignment {
            function: "Servo/ESC CH1".into(),
            default_gpio: 16,
            alternatives: vec![17],
            capability: "MotorControl".into(),
        },
        PinAssignment {
            function: "Servo/ESC CH2".into(),
            default_gpio: 17,
            alternatives: vec![18],
            capability: "MotorControl".into(),
        },
        PinAssignment {
            function: "Servo/ESC CH3".into(),
            default_gpio: 18,
            alternatives: vec![19],
            capability: "MotorControl".into(),
        },
        PinAssignment {
            function: "Servo/ESC CH4".into(),
            default_gpio: 19,
            alternatives: vec![20],
            capability: "MotorControl".into(),
        },
        PinAssignment {
            function: "Failsafe Relay".into(),
            default_gpio: 25,
            alternatives: vec![26],
            capability: "FailsafeStop".into(),
        },
        PinAssignment {
            function: "Status LED".into(),
            default_gpio: 2,
            alternatives: vec![15],
            capability: "Gpio".into(),
        },
    ]
}

/// Gateway pins (AP+STA status, LR radio, status LED — no motor/IMU).
#[allow(dead_code)] // Phase 1 scaffolding: smartphone-gateway pin table for ADR-008-driven rollout.
fn gateway_pins() -> Vec<PinAssignment> {
    vec![
        PinAssignment {
            function: "Status LED".into(),
            default_gpio: 2,
            alternatives: vec![15],
            capability: "Gpio".into(),
        },
        PinAssignment {
            function: "UART TX (to vehicle)".into(),
            default_gpio: 43,
            alternatives: vec![],
            capability: "Uart".into(),
        },
        PinAssignment {
            function: "UART RX (from vehicle)".into(),
            default_gpio: 44,
            alternatives: vec![],
            capability: "Uart".into(),
        },
        PinAssignment {
            function: "Battery ADC".into(),
            default_gpio: 34,
            alternatives: vec![36],
            capability: "Adc".into(),
        },
    ]
}

// ── IoT pin assignment helpers ──────────────────────────────────────────────

/// IoT sensor hub pins (I2C sensor bus + optional SPI + status LED + ADC).
fn iot_sensor_pins() -> Vec<PinAssignment> {
    vec![
        PinAssignment {
            function: "I2C SDA".into(),
            default_gpio: 21,
            alternatives: vec![13, 14],
            capability: "I2c".into(),
        },
        PinAssignment {
            function: "I2C SCL".into(),
            default_gpio: 22,
            alternatives: vec![14, 13],
            capability: "I2c".into(),
        },
        PinAssignment {
            function: "SPI MOSI".into(),
            default_gpio: 11,
            alternatives: vec![35],
            capability: "Spi".into(),
        },
        PinAssignment {
            function: "SPI CLK".into(),
            default_gpio: 12,
            alternatives: vec![36],
            capability: "Spi".into(),
        },
        PinAssignment {
            function: "SPI CS (SD)".into(),
            default_gpio: 10,
            alternatives: vec![34],
            capability: "Spi".into(),
        },
        PinAssignment {
            function: "Status LED".into(),
            default_gpio: 48,
            alternatives: vec![38, 2],
            capability: "Gpio".into(),
        },
        PinAssignment {
            function: "Battery ADC".into(),
            default_gpio: 1,
            alternatives: vec![2, 3],
            capability: "Adc".into(),
        },
    ]
}

/// UART debug probe pins (target UART + USB CDC + status LED).
fn iot_debug_probe_pins() -> Vec<PinAssignment> {
    vec![
        PinAssignment {
            function: "Target UART RX".into(),
            default_gpio: 4,
            alternatives: vec![44],
            capability: "Uart".into(),
        },
        PinAssignment {
            function: "Target UART TX".into(),
            default_gpio: 5,
            alternatives: vec![43],
            capability: "Uart".into(),
        },
        PinAssignment {
            function: "Status LED".into(),
            default_gpio: 48,
            alternatives: vec![38, 2],
            capability: "Gpio".into(),
        },
    ]
}

/// I2C bus analyzer pins (I2C bus + status LED).
fn iot_i2c_analyzer_pins() -> Vec<PinAssignment> {
    vec![
        PinAssignment {
            function: "I2C SDA".into(),
            default_gpio: 21,
            alternatives: vec![13, 6],
            capability: "I2c".into(),
        },
        PinAssignment {
            function: "I2C SCL".into(),
            default_gpio: 22,
            alternatives: vec![14, 7],
            capability: "I2c".into(),
        },
        PinAssignment {
            function: "Status LED".into(),
            default_gpio: 48,
            alternatives: vec![38, 2],
            capability: "Gpio".into(),
        },
    ]
}

/// LCD dashboard pins (SPI display + rotary encoder + I2C sensor + status LED).
fn iot_display_pins() -> Vec<PinAssignment> {
    vec![
        PinAssignment {
            function: "SPI MOSI".into(),
            default_gpio: 11,
            alternatives: vec![35],
            capability: "Spi".into(),
        },
        PinAssignment {
            function: "SPI CLK".into(),
            default_gpio: 12,
            alternatives: vec![36],
            capability: "Spi".into(),
        },
        PinAssignment {
            function: "LCD CS".into(),
            default_gpio: 10,
            alternatives: vec![34],
            capability: "Spi".into(),
        },
        PinAssignment {
            function: "LCD DC".into(),
            default_gpio: 9,
            alternatives: vec![33],
            capability: "Gpio".into(),
        },
        PinAssignment {
            function: "LCD RST".into(),
            default_gpio: 8,
            alternatives: vec![18],
            capability: "Gpio".into(),
        },
        PinAssignment {
            function: "LCD Backlight".into(),
            default_gpio: 7,
            alternatives: vec![17],
            capability: "Ledc".into(),
        },
        PinAssignment {
            function: "Encoder A".into(),
            default_gpio: 5,
            alternatives: vec![15],
            capability: "Gpio".into(),
        },
        PinAssignment {
            function: "Encoder B".into(),
            default_gpio: 6,
            alternatives: vec![16],
            capability: "Gpio".into(),
        },
        PinAssignment {
            function: "Encoder Button".into(),
            default_gpio: 4,
            alternatives: vec![0],
            capability: "Gpio".into(),
        },
        PinAssignment {
            function: "I2C SDA".into(),
            default_gpio: 21,
            alternatives: vec![13],
            capability: "I2c".into(),
        },
        PinAssignment {
            function: "I2C SCL".into(),
            default_gpio: 22,
            alternatives: vec![14],
            capability: "I2c".into(),
        },
    ]
}

/// sigrok logic analyzer pins (LCD_CAM parallel data lanes + PCLK + status LED).
/// Default 8-channel assignment on S3; GPIO matrix allows remapping.
fn iot_sigrok_la_pins() -> Vec<PinAssignment> {
    vec![
        PinAssignment {
            function: "LA CH0".into(),
            default_gpio: 4,
            alternatives: vec![6, 15],
            capability: "Gpio".into(),
        },
        PinAssignment {
            function: "LA CH1".into(),
            default_gpio: 5,
            alternatives: vec![7, 16],
            capability: "Gpio".into(),
        },
        PinAssignment {
            function: "LA CH2".into(),
            default_gpio: 6,
            alternatives: vec![8, 17],
            capability: "Gpio".into(),
        },
        PinAssignment {
            function: "LA CH3".into(),
            default_gpio: 7,
            alternatives: vec![9, 18],
            capability: "Gpio".into(),
        },
        PinAssignment {
            function: "LA CH4".into(),
            default_gpio: 15,
            alternatives: vec![4, 35],
            capability: "Gpio".into(),
        },
        PinAssignment {
            function: "LA CH5".into(),
            default_gpio: 16,
            alternatives: vec![5, 36],
            capability: "Gpio".into(),
        },
        PinAssignment {
            function: "LA CH6".into(),
            default_gpio: 17,
            alternatives: vec![37, 38],
            capability: "Gpio".into(),
        },
        PinAssignment {
            function: "LA CH7".into(),
            default_gpio: 18,
            alternatives: vec![33, 34],
            capability: "Gpio".into(),
        },
        PinAssignment {
            function: "Status LED".into(),
            default_gpio: 48,
            alternatives: vec![38, 2],
            capability: "Gpio".into(),
        },
    ]
}

/// Addressable LED strip pins (RMT data + status LED).
fn iot_led_strip_pins() -> Vec<PinAssignment> {
    vec![
        PinAssignment {
            function: "LED Data (RMT)".into(),
            default_gpio: 48,
            alternatives: vec![38, 8],
            capability: "Rmt".into(),
        },
        PinAssignment {
            function: "Status LED".into(),
            default_gpio: 2,
            alternatives: vec![15],
            capability: "Gpio".into(),
        },
    ]
}

// ── IoT Phase 2 pin helpers (2026-04-16) ────────────────────────────────────

/// LD2410 mmWave presence sensor pins (UART2 + status LED).
fn iot_ld2410_pins() -> Vec<PinAssignment> {
    vec![
        PinAssignment {
            function: "LD2410 UART RX".into(),
            default_gpio: 17,
            alternatives: vec![44],
            capability: "Uart".into(),
        },
        PinAssignment {
            function: "LD2410 UART TX".into(),
            default_gpio: 18,
            alternatives: vec![43],
            capability: "Uart".into(),
        },
        PinAssignment {
            function: "LD2410 OUT (raw presence)".into(),
            default_gpio: 4,
            alternatives: vec![5],
            capability: "Gpio".into(),
        },
        PinAssignment {
            function: "Status LED".into(),
            default_gpio: 48,
            alternatives: vec![38, 2],
            capability: "Gpio".into(),
        },
    ]
}

/// CT-clamp power monitor pins (1–3 ADC channels + voltage reference + status LED).
fn iot_ct_clamp_pins() -> Vec<PinAssignment> {
    vec![
        PinAssignment {
            function: "CT Phase A ADC".into(),
            default_gpio: 1,
            alternatives: vec![2, 3],
            capability: "Adc".into(),
        },
        PinAssignment {
            function: "CT Phase B ADC".into(),
            default_gpio: 2,
            alternatives: vec![3, 4],
            capability: "Adc".into(),
        },
        PinAssignment {
            function: "CT Phase C ADC".into(),
            default_gpio: 3,
            alternatives: vec![4, 5],
            capability: "Adc".into(),
        },
        PinAssignment {
            function: "AC Voltage Reference ADC".into(),
            default_gpio: 4,
            alternatives: vec![5, 6],
            capability: "Adc".into(),
        },
        PinAssignment {
            function: "Status LED".into(),
            default_gpio: 48,
            alternatives: vec![38, 2],
            capability: "Gpio".into(),
        },
    ]
}

/// Air-quality station pins (UART for PMS5003 + I²C for SCD40 + status LED).
fn iot_air_quality_pins() -> Vec<PinAssignment> {
    vec![
        PinAssignment {
            function: "PMS UART RX".into(),
            default_gpio: 17,
            alternatives: vec![44],
            capability: "Uart".into(),
        },
        PinAssignment {
            function: "PMS UART TX".into(),
            default_gpio: 18,
            alternatives: vec![43],
            capability: "Uart".into(),
        },
        PinAssignment {
            function: "PMS SET (sleep)".into(),
            default_gpio: 5,
            alternatives: vec![6],
            capability: "Gpio".into(),
        },
        PinAssignment {
            function: "SCD40 I2C SDA".into(),
            default_gpio: 21,
            alternatives: vec![13, 14],
            capability: "I2c".into(),
        },
        PinAssignment {
            function: "SCD40 I2C SCL".into(),
            default_gpio: 22,
            alternatives: vec![14, 13],
            capability: "I2c".into(),
        },
        PinAssignment {
            function: "Status LED".into(),
            default_gpio: 48,
            alternatives: vec![38, 2],
            capability: "Gpio".into(),
        },
    ]
}

/// SX1302 LoRaWAN concentrator pins (SPI + RST + INT + status LED).
fn iot_sx1302_pins() -> Vec<PinAssignment> {
    vec![
        PinAssignment {
            function: "SX1302 SPI MOSI".into(),
            default_gpio: 11,
            alternatives: vec![35],
            capability: "Spi".into(),
        },
        PinAssignment {
            function: "SX1302 SPI MISO".into(),
            default_gpio: 13,
            alternatives: vec![37],
            capability: "Spi".into(),
        },
        PinAssignment {
            function: "SX1302 SPI CLK".into(),
            default_gpio: 12,
            alternatives: vec![36],
            capability: "Spi".into(),
        },
        PinAssignment {
            function: "SX1302 SPI CS".into(),
            default_gpio: 10,
            alternatives: vec![34],
            capability: "Spi".into(),
        },
        PinAssignment {
            function: "SX1302 RST".into(),
            default_gpio: 9,
            alternatives: vec![33],
            capability: "Gpio".into(),
        },
        PinAssignment {
            function: "SX1302 INT".into(),
            default_gpio: 8,
            alternatives: vec![18],
            capability: "Gpio".into(),
        },
        PinAssignment {
            function: "Status LED".into(),
            default_gpio: 48,
            alternatives: vec![38, 2],
            capability: "Gpio".into(),
        },
    ]
}

// ── IoT parameter helpers ──────────────────────────────────────────────────

fn storage_backend_param() -> UserParameterDefinition {
    UserParameterDefinition {
        id: "storage_backend".into(),
        label: "Storage Backend".into(),
        label_zh: Some("存储后端".into()),
        required: true,
        secret: false,
        description: "Where logged data is stored".into(),
        description_zh: Some("记录数据的存储位置。".into()),
        default_value: Some(serde_json::Value::String("sd_card".into())),
        enum_values: Some(vec![
            EnumOption {
                value: "sd_card".into(),
                label: "SD Card (SPI)".into(),
                description: Some("MicroSD via SPI. Large capacity.".into()),
            },
            EnumOption {
                value: "flash".into(),
                label: "Internal Flash".into(),
                description: Some("On-chip NVS/SPIFFS. Limited capacity.".into()),
            },
            EnumOption {
                value: "usb_export".into(),
                label: "USB CDC Export".into(),
                description: Some("Stream directly over USB. No local storage.".into()),
            },
        ]),
        depends_on: None,
    }
}

fn upload_mode_param() -> UserParameterDefinition {
    UserParameterDefinition {
        id: "upload_mode".into(),
        label: "Upload Mode".into(),
        label_zh: Some("上传模式".into()),
        required: false,
        secret: false,
        description: "How logged data is uploaded".into(),
        description_zh: Some("记录数据的上传方式。".into()),
        default_value: Some(serde_json::Value::String("manual".into())),
        enum_values: Some(vec![
            EnumOption {
                value: "manual".into(),
                label: "Manual (USB)".into(),
                description: Some("Connect USB to download data.".into()),
            },
            EnumOption {
                value: "wifi_batch".into(),
                label: "WiFi Batch Upload".into(),
                description: Some("Periodic upload over WiFi.".into()),
            },
        ]),
        depends_on: None,
    }
}

fn display_driver_param() -> UserParameterDefinition {
    UserParameterDefinition {
        id: "display_driver".into(),
        label: "Display Driver".into(),
        label_zh: Some("显示驱动".into()),
        required: true,
        secret: false,
        description: "LCD/OLED driver IC".into(),
        description_zh: Some("LCD/OLED 驱动芯片。".into()),
        default_value: Some(serde_json::Value::String("st7789".into())),
        enum_values: Some(vec![
            EnumOption {
                value: "st7789".into(),
                label: "ST7789 (240x240 / 320x240)".into(),
                description: Some("Color TFT. SPI. Common on dev boards.".into()),
            },
            EnumOption {
                value: "ili9341".into(),
                label: "ILI9341 (320x240)".into(),
                description: Some("Color TFT. SPI. Widely available.".into()),
            },
            EnumOption {
                value: "ssd1306".into(),
                label: "SSD1306 (128x64 OLED)".into(),
                description: Some("Monochrome OLED. I2C or SPI. Low power.".into()),
            },
            EnumOption {
                value: "sh1106".into(),
                label: "SH1106 (128x64 OLED)".into(),
                description: Some("Monochrome OLED. I2C. Similar to SSD1306.".into()),
            },
        ]),
        depends_on: None,
    }
}

fn led_type_param() -> UserParameterDefinition {
    UserParameterDefinition {
        id: "led_type".into(),
        label: "LED Type".into(),
        label_zh: Some("LED 类型".into()),
        required: true,
        secret: false,
        description: "Addressable LED chipset".into(),
        description_zh: Some("可寻址 LED 芯片型号。".into()),
        default_value: Some(serde_json::Value::String("ws2812b".into())),
        enum_values: Some(vec![
            EnumOption {
                value: "ws2812b".into(),
                label: "WS2812B (NeoPixel)".into(),
                description: Some("Single data line. Most common.".into()),
            },
            EnumOption {
                value: "sk6812".into(),
                label: "SK6812 (RGBW)".into(),
                description: Some("RGBW variant with dedicated white LED.".into()),
            },
            EnumOption {
                value: "apa102".into(),
                label: "APA102 (DotStar)".into(),
                description: Some("Clock + data. Higher refresh rate.".into()),
            },
        ]),
        depends_on: None,
    }
}

/// Build a pre-populated solution registry with known solutions.
pub fn default_solution_registry() -> SolutionRegistry {
    let mut r = SolutionRegistry::new();

    // ── 2. Composite Device Firmware ─────────────────────────────────────────

    r.register(SolutionDefinition {
        id: "composite_device_firmware".into(),
        label: "USB Composite Device Firmware".into(),
        label_zh: Some("USB 复合设备固件".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into()],
        fixed_inputs: vec![InputSurface::UsbCdcCommand],
        fixed_outputs: vec![OutputSurface::UsbTx, OutputSurface::StatusLed],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "usb_init".into(),
                label: "Initialize USB composite device".into(),
                label_zh: Some("初始化 USB 复合设备".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "mount_storage".into(),
                label: "Mount mass storage volume".into(),
                label_zh: Some("挂载大容量存储卷".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "cdc_bridge".into(),
                label: "Start CDC serial bridge".into(),
                label_zh: Some("启动 CDC 串口桥".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "dual_task".into(),
            label: "Dual-task (USB + WiFi)".into(),
            decisions: vec!["USB MSC on core 0".into(), "WiFi + CDC on core 1".into()],
        },
        user_parameters: vec![UserParameterDefinition {
            id: "volume_label".into(),
            label: "Volume Label".into(),
            label_zh: Some("卷标".into()),
            required: false,
            secret: false,
            description: "Mass storage volume label".into(),
            description_zh: Some("大容量存储卷的卷标。".into()),
            default_value: Some(serde_json::Value::String("BOOTSTICK".into())),
            enum_values: None,
            depends_on: None,
        }],
        feedback_paths: vec![SignalPath {
            id: "usb_cmd_to_led".into(),
            name: "USB CDC command to status LED".into(),
            source: InputSurface::UsbCdcCommand,
            transforms: vec![SignalPathStep {
                order: 1,
                node: TransformNode::UartProtocolParse,
                label: None,
                description: None,
            }],
            sink: OutputSurface::StatusLed,
            feedback: vec![SignalPathStep {
                order: 1,
                node: FeedbackSurface::UsbSerialJtag,
                label: None,
                description: None,
            }],
            expected_user_result: "Status LED reflects USB command processing state".into(),
        }],
        variants: vec![
            SolutionVariantDefinition {
                id: "profile_a".into(),
                label: "Profile A — Mass Storage + WiFi Bridge".into(),
                label_zh: Some("Profile A — 大容量存储 + Wi-Fi 桥接".into()),
                required_caps: vec![Capability::UsbOtg, Capability::Wifi],
                parameter_defaults: BTreeMap::new(),
                add_components: vec!["wifi".into()],
                remove_components: vec![],
                add_external_contracts: vec![],
                active_flag_add: vec![],
                active_flag_remove: vec![],
                user_parameter_overrides: vec![],
                runtime_binding_override: None,
            },
            SolutionVariantDefinition {
                id: "profile_b".into(),
                label: "Profile B — Mass Storage Only".into(),
                label_zh: Some("Profile B — 仅大容量存储".into()),
                required_caps: vec![Capability::UsbOtg],
                parameter_defaults: BTreeMap::new(),
                add_components: vec![],
                remove_components: vec!["wifi".into()],
                add_external_contracts: vec![],
                active_flag_add: vec![],
                active_flag_remove: vec![],
                user_parameter_overrides: vec![],
                runtime_binding_override: None,
            },
        ],
        component_bundle: ComponentBundle {
            required: vec!["uart".into()],
            optional: vec!["wifi".into(), "ota".into()],
        },
        runtime_binding: RuntimeBinding::default(),
        external_contracts: vec![],
        network_topology: NetworkTopology::None,
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
    });

    // ── 3. Camera Stream (Phone/Browser Video) ──────────────────────────────

    r.register(SolutionDefinition {
        id: "camera_stream".into(),
        label: "Camera MJPEG Stream".into(),
        label_zh: Some("摄像头 MJPEG 推流".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into()],
        fixed_inputs: vec![InputSurface::CameraFrame],
        fixed_outputs: vec![
            OutputSurface::HttpMjpegStream,
            OutputSurface::NetworkApiState,
        ],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "capture".into(),
                label: "Capture camera frame".into(),
                label_zh: Some("采集摄像头帧".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "encode".into(),
                label: "JPEG encode".into(),
                label_zh: Some("JPEG 编码".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "stream".into(),
                label: "HTTP MJPEG stream".into(),
                label_zh: Some("HTTP MJPEG 推流".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "periodic_capture".into(),
            label: "Periodic capture (configurable FPS)".into(),
            decisions: vec!["Capture at target FPS".into()],
        },
        user_parameters: vec![
            frame_size_param(),
            UserParameterDefinition {
                id: "stream_fps".into(),
                label: "Stream FPS".into(),
                label_zh: Some("推流帧率".into()),
                required: false,
                secret: false,
                description: "Target frames per second".into(),
                description_zh: Some("目标每秒帧数。".into()),
                default_value: Some(serde_json::Value::String("15".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "5".into(),
                        label: "5 fps".into(),
                        description: Some("Lowest bandwidth.".into()),
                    },
                    EnumOption {
                        value: "10".into(),
                        label: "10 fps".into(),
                        description: None,
                    },
                    EnumOption {
                        value: "15".into(),
                        label: "15 fps".into(),
                        description: Some("Standard.".into()),
                    },
                    EnumOption {
                        value: "25".into(),
                        label: "25 fps".into(),
                        description: Some("Smooth.".into()),
                    },
                ]),
                depends_on: None,
            },
        ],
        feedback_paths: vec![SignalPath {
            id: "camera_to_stream".into(),
            name: "Camera frame to MJPEG stream".into(),
            source: InputSurface::CameraFrame,
            transforms: vec![SignalPathStep {
                order: 1,
                node: TransformNode::JpegEncode,
                label: None,
                description: None,
            }],
            sink: OutputSurface::HttpMjpegStream,
            feedback: vec![SignalPathStep {
                order: 1,
                node: FeedbackSurface::WebStatus,
                label: None,
                description: None,
            }],
            expected_user_result: "Live camera feed viewable in browser/phone".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["wifi".into()],
            optional: vec!["ota".into()],
        },
        runtime_binding: brookesia_wifi_binding(),
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
    });

    // ── 4. Sensor Hub ────────────────────────────────────────────────────────

    r.register(SolutionDefinition {
        id: "sensor_hub".into(),
        label: "Multi-Sensor Hub".into(),
        label_zh: Some("多传感器中枢".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into(), "esp32c6_wroom1".into()],
        fixed_inputs: vec![
            InputSurface::I2cSensor,
            InputSurface::AdcVoltage,
            InputSurface::ButtonGpio,
        ],
        fixed_outputs: vec![
            OutputSurface::GpioLevel,
            OutputSurface::RelayDrive,
            OutputSurface::NetworkApiState,
        ],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "poll_sensors".into(),
                label: "Poll sensors periodically".into(),
                label_zh: Some("周期性轮询传感器".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "publish_state".into(),
                label: "Publish state to rshome-ha".into(),
                label_zh: Some("向 rshome-ha 上报状态".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "periodic_poll".into(),
            label: "Periodic sensor polling".into(),
            decisions: vec!["Poll interval per sensor".into()],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "primary_sensor".into(),
                label: "Primary Sensor".into(),
                label_zh: Some("主传感器".into()),
                required: true,
                secret: false,
                description: "Physical sensor wired to this hub".into(),
                description_zh: Some("连接到此集线器的物理传感器。".into()),
                default_value: Some(serde_json::Value::String("bme280_i2c".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "bme280_i2c".into(),
                        label: "BME280 (temp/humidity/pressure, I2C 0x76)".into(),
                        description: Some(
                            "Bosch environmental sensor. 3 readings from one chip.".into(),
                        ),
                    },
                    EnumOption {
                        value: "bme680_bsec".into(),
                        label: "BME680 (temp/humidity/pressure/gas, I2C 0x76)".into(),
                        description: Some("Bosch air-quality sensor with VOC gas index.".into()),
                    },
                    EnumOption {
                        value: "sht3x".into(),
                        label: "SHT3x (temp/humidity, I2C 0x44)".into(),
                        description: Some(
                            "Sensirion high-accuracy temperature and humidity.".into(),
                        ),
                    },
                    EnumOption {
                        value: "htu21d".into(),
                        label: "HTU21D (temp/humidity, I2C 0x40)".into(),
                        description: Some("TE Connectivity temperature and humidity.".into()),
                    },
                    EnumOption {
                        value: "bh1750".into(),
                        label: "BH1750 (ambient light, I2C 0x23)".into(),
                        description: Some("Digital ambient light sensor, lux output.".into()),
                    },
                    EnumOption {
                        value: "dht".into(),
                        label: "DHT22 (temp/humidity, single-wire GPIO)".into(),
                        description: Some("Single-bus digital sensor. No I2C needed.".into()),
                    },
                    EnumOption {
                        value: "ds18x20".into(),
                        label: "DS18B20 (temperature, 1-Wire GPIO)".into(),
                        description: Some("Dallas 1-Wire temperature. Chainable.".into()),
                    },
                    EnumOption {
                        value: "adc".into(),
                        label: "ADC (raw voltage, analog pin)".into(),
                        description: Some(
                            "Analog-to-digital input. Thermistor, soil moisture, etc.".into(),
                        ),
                    },
                ]),
                depends_on: None,
            },
            iot_poll_interval_param(),
            UserParameterDefinition {
                id: "secondary_sensor".into(),
                label: "Secondary Sensor (optional)".into(),
                label_zh: Some("副传感器（可选）".into()),
                required: false,
                secret: false,
                description: "Additional sensor on the same bus".into(),
                description_zh: Some("同一总线上的附加传感器。".into()),
                default_value: Some(serde_json::Value::String("none".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "none".into(),
                        label: "None".into(),
                        description: Some("No secondary sensor.".into()),
                    },
                    EnumOption {
                        value: "bh1750".into(),
                        label: "BH1750 (ambient light, I2C 0x23)".into(),
                        description: Some("Add ambient light to an environmental sensor.".into()),
                    },
                    EnumOption {
                        value: "bme280_i2c".into(),
                        label: "BME280 (temp/humidity/pressure, I2C 0x76)".into(),
                        description: Some("Add environmental readings.".into()),
                    },
                    EnumOption {
                        value: "sht3x".into(),
                        label: "SHT3x (temp/humidity, I2C 0x44)".into(),
                        description: Some("Add Sensirion temp/humidity.".into()),
                    },
                    EnumOption {
                        value: "adc".into(),
                        label: "ADC (raw voltage, analog pin)".into(),
                        description: Some("Add analog input (battery, soil, etc.).".into()),
                    },
                ]),
                depends_on: None,
            },
        ],
        feedback_paths: vec![SignalPath {
            id: "sensor_to_api".into(),
            name: "Sensor reading to rshome-ha".into(),
            source: InputSurface::I2cSensor,
            transforms: vec![
                SignalPathStep {
                    order: 1,
                    node: TransformNode::SensorFrameDecode,
                    label: None,
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: TransformNode::Filter,
                    label: None,
                    description: None,
                },
            ],
            sink: OutputSurface::NetworkApiState,
            feedback: vec![SignalPathStep {
                order: 1,
                node: FeedbackSurface::ApiState,
                label: None,
                description: None,
            }],
            expected_user_result: "Sensor readings appear in rshome-ha dashboard".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["wifi".into(), "sensor".into(), "binary_sensor".into()],
            optional: vec![
                "switch".into(),
                "i2c".into(),
                "uart".into(),
                "ota".into(),
                "api".into(),
            ],
        },
        runtime_binding: {
            let mut rb = brookesia_wifi_binding();
            rb.ha_entities = vec![
                crate::ha_export::HaEntityExportDefinition::sensor_entity(
                    "room_temperature",
                    "Room Temperature",
                    "temperature",
                    "°C",
                    crate::ha_export::StateBinding {
                        source_event: "bme280_0.updated".into(),
                        field_map: BTreeMap::from([("temperature".into(), "value".into())]),
                    },
                ),
                crate::ha_export::HaEntityExportDefinition::sensor_entity(
                    "room_humidity",
                    "Room Humidity",
                    "humidity",
                    "%",
                    crate::ha_export::StateBinding {
                        source_event: "bme280_0.updated".into(),
                        field_map: BTreeMap::from([("humidity".into(), "value".into())]),
                    },
                ),
                crate::ha_export::HaEntityExportDefinition::sensor_entity(
                    "room_pressure",
                    "Room Pressure",
                    "pressure",
                    "hPa",
                    crate::ha_export::StateBinding {
                        source_event: "bme280_0.updated".into(),
                        field_map: BTreeMap::from([("pressure".into(), "value".into())]),
                    },
                ),
            ];
            rb
        },
        external_contracts: vec!["rshome-ha Native API".into()],
        network_topology: NetworkTopology::default(),
        domain: Some(DomainKind::IotDeviceTooling),
        architecture_tier: None,
        communication_chains: None,
        pin_assignments: Some(iot_sensor_pins()),
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
    });

    // ── 5. Phone Browser Video (HTTP MJPEG) ───────────────────────────────

    r.register(SolutionDefinition {
        id: "phone_browser_video_solution".into(),
        label: "Phone Browser Video (HTTP MJPEG)".into(),
        label_zh: Some("手机浏览器视频(HTTP MJPEG)".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into()],
        fixed_inputs: vec![InputSurface::CameraFrame],
        fixed_outputs: vec![OutputSurface::HttpMjpegStream],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "wifi_softap".into(),
                label: "Start SoftAP or APSTA".into(),
                label_zh: Some("启动 SoftAP(为手机连接)".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "camera_init".into(),
                label: "Initialize camera sensor".into(),
                label_zh: Some("初始化摄像头传感器".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "http_stream".into(),
                label: "Start HTTP MJPEG stream server".into(),
                label_zh: Some("启动 HTTP MJPEG 推流服务".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "periodic_capture".into(),
            label: "Periodic capture at target FPS".into(),
            decisions: vec!["Capture → JPEG → HTTP multipart boundary".into()],
        },
        user_parameters: vec![
            camera_sensor_param(),
            frame_size_param(),
            UserParameterDefinition {
                id: "jpeg_quality".into(),
                label: "JPEG Quality".into(),
                label_zh: Some("JPEG 质量".into()),
                required: false,
                secret: false,
                description: "JPEG compression quality (10-63, lower = better quality)".into(),
                description_zh: Some("JPEG 压缩质量(10-63,数值越小画质越好)。".into()),
                default_value: Some(serde_json::Value::String("12".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "8".into(),
                        label: "8 (High quality)".into(),
                        description: Some("Best image, most bandwidth.".into()),
                    },
                    EnumOption {
                        value: "12".into(),
                        label: "12 (Standard)".into(),
                        description: Some("Good balance.".into()),
                    },
                    EnumOption {
                        value: "20".into(),
                        label: "20 (Low)".into(),
                        description: Some("Smaller files.".into()),
                    },
                    EnumOption {
                        value: "40".into(),
                        label: "40 (Very low)".into(),
                        description: Some("Minimum bandwidth.".into()),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "stream_fps".into(),
                label: "Stream FPS".into(),
                label_zh: Some("推流帧率".into()),
                required: false,
                secret: false,
                description: "Target frames per second".into(),
                description_zh: Some("目标每秒帧数。".into()),
                default_value: Some(serde_json::Value::String("15".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "5".into(),
                        label: "5 fps".into(),
                        description: Some("Lowest.".into()),
                    },
                    EnumOption {
                        value: "10".into(),
                        label: "10 fps".into(),
                        description: Some("Low.".into()),
                    },
                    EnumOption {
                        value: "15".into(),
                        label: "15 fps".into(),
                        description: Some("Standard.".into()),
                    },
                    EnumOption {
                        value: "25".into(),
                        label: "25 fps".into(),
                        description: Some("Smooth.".into()),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "wifi_mode".into(),
                label: "WiFi Mode".into(),
                label_zh: Some("Wi-Fi 模式".into()),
                required: true,
                secret: false,
                description:
                    "WiFi mode: softap (device hotspot), sta (join router), or ap_sta (bridge)"
                        .into(),
                description_zh: Some(
                    "Wi-Fi 模式:softap(设备热点)、sta(连接路由器)或 ap_sta(桥接)。".into(),
                ),
                default_value: Some(serde_json::Value::String("softap".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "softap".into(),
                        label: "SoftAP".into(),
                        description: Some("Device creates its own hotspot.".into()),
                    },
                    EnumOption {
                        value: "sta".into(),
                        label: "Station".into(),
                        description: Some("Device joins an existing router.".into()),
                    },
                    EnumOption {
                        value: "ap_sta".into(),
                        label: "AP+STA".into(),
                        description: Some("Bridge mode: hotspot + router connection.".into()),
                    },
                ]),
                depends_on: None,
            },
        ],
        feedback_paths: vec![SignalPath {
            id: "camera_to_browser".into(),
            name: "Camera frame to phone browser".into(),
            source: InputSurface::CameraFrame,
            transforms: vec![SignalPathStep {
                order: 1,
                node: TransformNode::JpegEncode,
                label: None,
                description: None,
            }],
            sink: OutputSurface::HttpMjpegStream,
            feedback: vec![SignalPathStep {
                order: 1,
                node: FeedbackSurface::WebStatus,
                label: None,
                description: None,
            }],
            expected_user_result: "Phone browser shows live camera feed from device hotspot".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["wifi".into()],
            optional: vec!["ota".into()],
        },
        runtime_binding: brookesia_wifi_binding(),
        external_contracts: vec![],
        network_topology: NetworkTopology::default(),
        domain: None,
        architecture_tier: None,
        communication_chains: None,
        pin_assignments: None,
        family: Some(ImplementationFamily::EspIotSolution),
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
    });

    // ── 6. Phone RTSP Audio/Video ───────────────────────────────────────────

    r.register(SolutionDefinition {
        id: "phone_rtsp_av_solution".into(),
        label: "Phone RTSP Audio/Video".into(),
        label_zh: Some("手机 RTSP 音视频".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into()],
        fixed_inputs: vec![InputSurface::CameraFrame, InputSurface::AudioInput],
        fixed_outputs: vec![OutputSurface::RtspStream],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "wifi_init".into(),
                label: "Initialize WiFi (SoftAP or STA)".into(),
                label_zh: Some("初始化 Wi-Fi(SoftAP 或 STA)".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "camera_init".into(),
                label: "Initialize camera sensor".into(),
                label_zh: Some("初始化摄像头传感器".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "audio_init".into(),
                label: "Initialize I2S audio input".into(),
                label_zh: Some("初始化 I2S 音频输入".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "rtsp_server".into(),
                label: "Start RTSP server (MJPEG + G711A)".into(),
                label_zh: Some("启动 RTSP 服务(MJPEG + G711A)".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "rtsp_pipeline".into(),
            label: "RTSP pipeline (ADF element chain)".into(),
            decisions: vec![
                "Video: camera → JPEG → RTP".into(),
                "Audio: I2S → G711A → RTP".into(),
            ],
        },
        user_parameters: vec![
            camera_sensor_param(),
            UserParameterDefinition {
                id: "audio_enable".into(),
                label: "Enable Audio".into(),
                label_zh: Some("启用音频".into()),
                required: false,
                secret: false,
                description: "Enable audio capture alongside video".into(),
                description_zh: Some("在视频之外同时采集音频。".into()),
                default_value: Some(serde_json::json!(true)),
                enum_values: None,
                depends_on: None,
            },
            UserParameterDefinition {
                id: "video_fps".into(),
                label: "Video FPS".into(),
                label_zh: Some("视频帧率".into()),
                required: false,
                secret: false,
                description: "Target video frames per second".into(),
                description_zh: Some("目标视频每秒帧数。".into()),
                default_value: Some(serde_json::Value::String("15".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "10".into(),
                        label: "10 fps".into(),
                        description: None,
                    },
                    EnumOption {
                        value: "15".into(),
                        label: "15 fps".into(),
                        description: Some("Standard.".into()),
                    },
                    EnumOption {
                        value: "25".into(),
                        label: "25 fps".into(),
                        description: None,
                    },
                    EnumOption {
                        value: "30".into(),
                        label: "30 fps".into(),
                        description: Some("Smooth.".into()),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "rtsp_port".into(),
                label: "RTSP Port".into(),
                label_zh: Some("RTSP 端口".into()),
                required: false,
                secret: false,
                description: "RTSP server port".into(),
                description_zh: Some("RTSP 服务端口。".into()),
                default_value: Some(serde_json::Value::String("554".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "554".into(),
                        label: "554 (standard)".into(),
                        description: Some("Default RTSP port.".into()),
                    },
                    EnumOption {
                        value: "8554".into(),
                        label: "8554".into(),
                        description: Some("Common alternative.".into()),
                    },
                    EnumOption {
                        value: "8080".into(),
                        label: "8080".into(),
                        description: Some("HTTP fallback port.".into()),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "client_type".into(),
                label: "Client Type".into(),
                label_zh: Some("客户端类型".into()),
                required: false,
                secret: false,
                description: "Expected RTSP client application".into(),
                description_zh: Some("预期的 RTSP 客户端应用。".into()),
                default_value: Some(serde_json::Value::String("vlc".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "vlc".into(),
                        label: "VLC".into(),
                        description: Some("VLC media player.".into()),
                    },
                    EnumOption {
                        value: "ffplay".into(),
                        label: "ffplay".into(),
                        description: Some("FFmpeg playback tool.".into()),
                    },
                    EnumOption {
                        value: "browser".into(),
                        label: "Browser".into(),
                        description: Some("Browser-based RTSP viewer.".into()),
                    },
                ]),
                depends_on: None,
            },
        ],
        feedback_paths: vec![SignalPath {
            id: "av_to_rtsp".into(),
            name: "Camera + audio to RTSP stream".into(),
            source: InputSurface::CameraFrame,
            transforms: vec![
                SignalPathStep {
                    order: 1,
                    node: TransformNode::JpegEncode,
                    label: Some("Video encode".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: TransformNode::AudioEncode,
                    label: Some("Audio G711A encode".into()),
                    description: None,
                },
            ],
            sink: OutputSurface::RtspStream,
            feedback: vec![SignalPathStep {
                order: 1,
                node: FeedbackSurface::RuntimeMetrics,
                label: None,
                description: None,
            }],
            expected_user_result: "VLC/custom app shows live audio+video from device".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["wifi".into()],
            optional: vec!["ota".into()],
        },
        runtime_binding: brookesia_wifi_binding(),
        external_contracts: vec![],
        network_topology: NetworkTopology::default(),
        domain: None,
        architecture_tier: None,
        communication_chains: None,
        pin_assignments: None,
        family: Some(ImplementationFamily::EspAdf),
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
    });

    // ── 7. Direct Vehicle Control (WiFi AP + UDP) ───────────────────────────

    r.register(SolutionDefinition {
        id: "direct_control_solution".into(),
        label: "Vehicle Control Board".into(),
        label_zh: Some("载具控制板".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into()],
        fixed_inputs: vec![InputSurface::WifiEvent],
        fixed_outputs: vec![OutputSurface::MotorDrive, OutputSurface::StatusLed],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "wifi_softap".into(),
                label: "Start SoftAP for phone connection".into(),
                label_zh: Some("启动 SoftAP(为手机连接)".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "udp_listener".into(),
                label: "Start UDP listener on port 2390".into(),
                label_zh: Some("在端口 2390 监听 UDP".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "crtp_dispatch".into(),
                label: "CRTP packet dispatch to commander".into(),
                label_zh: Some("CRTP 包分发到 commander".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "stabilizer_loop".into(),
                label: "Stabilizer task: sensors → controller → motors".into(),
                label_zh: Some("稳定器任务:传感器 → 控制器 → 电机".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "realtime_control".into(),
            label: "Real-time control loop".into(),
            decisions: vec![
                "UDP → CRTP → commander at control_rate_hz".into(),
                "Stabilizer runs at fixed rate independent of command arrival".into(),
                "Failsafe timeout triggers safe stop".into(),
            ],
        },
        user_parameters: vec![
            vehicle_type_param(),
            control_protocol_param(),
            actuator_type_param(),
            imu_axis_tier_param(),
            imu_chip_param(),
            control_rate_hz_param(),
            failsafe_timeout_ms_param(),
            UserParameterDefinition {
                id: "app_compat_mode".into(),
                label: "App Compatibility Mode".into(),
                label_zh: Some("App 兼容模式".into()),
                required: false,
                secret: false,
                description: "Compatible with ESP-Drone mobile app protocol".into(),
                description_zh: Some("兼容 ESP-Drone 手机 App 协议。".into()),
                default_value: Some(serde_json::json!(true)),
                enum_values: None,
                depends_on: None,
            },
        ],
        feedback_paths: vec![SignalPath {
            id: "phone_cmd_to_motor".into(),
            name: "Phone command to motor drive".into(),
            source: InputSurface::WifiEvent,
            transforms: vec![
                SignalPathStep {
                    order: 1,
                    node: TransformNode::CommandDispatch,
                    label: Some("CRTP packet decode".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: TransformNode::PidLoop,
                    label: Some("Stabilizer PID".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 3,
                    node: TransformNode::SafetyInterlock,
                    label: Some("Throttle lock + watchdog".into()),
                    description: None,
                },
            ],
            sink: OutputSurface::MotorDrive,
            feedback: vec![SignalPathStep {
                order: 1,
                node: FeedbackSurface::PhysicalMotion,
                label: None,
                description: None,
            }],
            expected_user_result: "Phone app directly controls vehicle motors via WiFi".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["wifi".into()],
            optional: vec!["ota".into()],
        },
        runtime_binding: {
            let mut b =
                brookesia_vehicle_binding(vec!["rshome_motor_control".into(), "rshome_imu".into()]);
            b.board_assembly = Some("esp32s3_va_wheeled_diff_assembly".into());
            b
        },
        external_contracts: vec!["ESP-Drone App Protocol (UDP/CRTP)".into()],
        network_topology: NetworkTopology::default(),
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::ControlBoard),
        communication_chains: None,
        pin_assignments: Some(vehicle_control_pins()),
        family: Some(ImplementationFamily::EspDrone),
        form_factor_families: Some(vec![
            FormFactorKind::Wheeled2wdDiff,
            FormFactorKind::Wheeled4wdDiff,
            FormFactorKind::Wheeled6wd,
            FormFactorKind::TrackedSkidsteer,
        ]),
        control_uplink: Some(ControlUplinkKind::WifiCrtp),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::CustomUart),
        sensor_tier_min: Some(SensorTierKind::Basic6ax),
        actuator_family: Some(ActuatorFamily::MixedDiffDrive),
        power_rails: Some(PowerRailKind::DualLogicMotor),
        failsafe: Some(FailsafeInfo {
            killswitch_source: vec![KillswitchSource::RxLoss, KillswitchSource::TimeoutNoPacket],
            rx_loss_behavior: Some(RxLossBehavior::MotorCutoff),
            watchdog_ms: Some(500),
            emergency_stop_wiring: EmergencyStopWiring::GpioPulldown,
        }),
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Caveat),
        ])),
    });

    // ── 8. Direct Control + Video ───────────────────────────────────────────

    r.register(SolutionDefinition {
        id: "direct_control_video_solution".into(),
        label: "Direct Control + Video".into(),
        label_zh: Some("直连控制 + 视频".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into()],
        fixed_inputs: vec![InputSurface::WifiEvent, InputSurface::CameraFrame],
        fixed_outputs: vec![
            OutputSurface::MotorDrive,
            OutputSurface::HttpMjpegStream,
            OutputSurface::StatusLed,
        ],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "wifi_softap".into(),
                label: "Start SoftAP".into(),
                label_zh: Some("启动 SoftAP(为手机连接)".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "control_init".into(),
                label: "Initialize UDP/CRTP control chain".into(),
                label_zh: Some("初始化 UDP/CRTP 控制链".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "video_init".into(),
                label: "Initialize camera + HTTP MJPEG (async, after control)".into(),
                label_zh: Some("初始化摄像头 + HTTP MJPEG(异步,在控制之后)".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "control_priority".into(),
            label: "Control-priority dual task".into(),
            decisions: vec![
                "Control loop on high-priority task".into(),
                "Video capture on lower-priority task".into(),
                "Video yields to control under load".into(),
            ],
        },
        user_parameters: vec![
            vehicle_type_param(),
            control_protocol_param(),
            actuator_type_param(),
            imu_axis_tier_param(),
            imu_chip_param(),
            camera_sensor_param(),
            UserParameterDefinition {
                id: "video_protocol".into(),
                label: "Video Protocol".into(),
                label_zh: Some("视频协议".into()),
                required: false,
                secret: false,
                description: "Video streaming protocol".into(),
                description_zh: Some("视频推流协议。".into()),
                default_value: Some(serde_json::Value::String("http_mjpeg".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "http_mjpeg".into(),
                        label: "HTTP MJPEG".into(),
                        description: Some(
                            "Multipart JPEG over HTTP. Simplest, widest compatibility.".into(),
                        ),
                    },
                    EnumOption {
                        value: "rtsp".into(),
                        label: "RTSP".into(),
                        description: Some(
                            "Real Time Streaming Protocol. Better for VLC/media players.".into(),
                        ),
                    },
                ]),
                depends_on: None,
            },
            frame_size_param(),
            UserParameterDefinition {
                id: "video_enable_on_boot".into(),
                label: "Enable Video on Boot".into(),
                label_zh: Some("开机启用视频".into()),
                required: false,
                secret: false,
                description: "Start video stream automatically on boot".into(),
                description_zh: Some("开机时自动启动视频推流。".into()),
                default_value: Some(serde_json::json!(false)),
                enum_values: None,
                depends_on: None,
            },
            // ── Dual-mode relay (optional) ──
            UserParameterDefinition {
                id: "dual_mode_relay".into(),
                label: "Dual-Mode Relay".into(),
                label_zh: Some("双模中继".into()),
                required: false,
                secret: false,
                description:
                    "Enable relay+direct mode switching (phone→gateway→car + phone→car fallback)"
                        .into(),
                description_zh: Some("启用中继+直连模式切换（手机→网关→车 + 手机→车回退）".into()),
                default_value: Some(serde_json::json!(false)),
                enum_values: None,
                depends_on: None,
            },
            UserParameterDefinition {
                id: "default_mode".into(),
                label: "Default Mode".into(),
                label_zh: Some("默认模式".into()),
                required: false,
                secret: false,
                description: "Initial operating mode on boot".into(),
                description_zh: Some("开机时的初始工作模式。".into()),
                default_value: Some(serde_json::Value::String("relay".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "relay".into(),
                        label: "Relay".into(),
                        description: Some("Start in relay mode (phone→gateway→car).".into()),
                    },
                    EnumOption {
                        value: "direct".into(),
                        label: "Direct".into(),
                        description: Some("Start in direct mode (phone→car).".into()),
                    },
                ]),
                depends_on: Some(ParameterDependency {
                    parameter_id: "dual_mode_relay".into(),
                    when_value: "true".into(),
                    when_not_value: None,
                }),
            },
            safe_stop_timeout_ms_param(),
            direct_probe_threshold_param(),
            fallback_rssi_threshold_param(),
        ],
        feedback_paths: vec![
            SignalPath {
                id: "control_path".into(),
                name: "Control: phone to motors".into(),
                source: InputSurface::WifiEvent,
                transforms: vec![
                    SignalPathStep {
                        order: 1,
                        node: TransformNode::CommandDispatch,
                        label: None,
                        description: None,
                    },
                    SignalPathStep {
                        order: 2,
                        node: TransformNode::PidLoop,
                        label: None,
                        description: None,
                    },
                ],
                sink: OutputSurface::MotorDrive,
                feedback: vec![SignalPathStep {
                    order: 1,
                    node: FeedbackSurface::PhysicalMotion,
                    label: None,
                    description: None,
                }],
                expected_user_result: "Phone controls vehicle".into(),
            },
            SignalPath {
                id: "video_path".into(),
                name: "Video: camera to phone browser".into(),
                source: InputSurface::CameraFrame,
                transforms: vec![SignalPathStep {
                    order: 1,
                    node: TransformNode::JpegEncode,
                    label: None,
                    description: None,
                }],
                sink: OutputSurface::HttpMjpegStream,
                feedback: vec![SignalPathStep {
                    order: 1,
                    node: FeedbackSurface::WebStatus,
                    label: None,
                    description: None,
                }],
                expected_user_result: "Phone browser shows live video alongside control".into(),
            },
        ],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["wifi".into()],
            optional: vec!["ota".into()],
        },
        runtime_binding: brookesia_vehicle_binding(vec![
            "rshome_motor_control".into(),
            "rshome_imu".into(),
        ]),
        external_contracts: vec!["ESP-Drone App Protocol (UDP/CRTP)".into()],
        network_topology: NetworkTopology::default(),
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::AllInOneCam),
        communication_chains: None,
        pin_assignments: Some(all_in_one_cam_pins()),
        family: Some(ImplementationFamily::EspDrone),
        form_factor_families: Some(vec![
            FormFactorKind::Wheeled4wdAckermann,
            FormFactorKind::BigfootMonsterTruck,
            FormFactorKind::BigfootRockCrawler,
            FormFactorKind::AtvOffroad,
            FormFactorKind::DriftRallyRacer,
        ]),
        control_uplink: Some(ControlUplinkKind::WifiCrtp),
        video_downlink: Some(VideoDownlinkKind::MjpegHttp),
        telemetry: Some(TelemetryKind::CustomUart),
        sensor_tier_min: Some(SensorTierKind::Basic6ax),
        actuator_family: Some(ActuatorFamily::MixedAckermann),
        power_rails: Some(PowerRailKind::TripleMotorServoLogic),
        failsafe: Some(FailsafeInfo {
            killswitch_source: vec![KillswitchSource::RxLoss, KillswitchSource::TimeoutNoPacket],
            rx_loss_behavior: Some(RxLossBehavior::MotorCutoff),
            watchdog_ms: Some(500),
            emergency_stop_wiring: EmergencyStopWiring::GpioPulldown,
        }),
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Insufficient),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Insufficient),
        ])),
    });

    // ── 9. Direct Control + Telemetry (CRTP + MAVLink WiFi) ──────────────
    //
    // diy_lowcost / control_telemetry_board: ESP-Drone CRTP keeps real-time
    // control on UDP 2390 while a MAVLink-over-WiFi back-channel carries
    // telemetry to QGroundControl. Per
    // `type-driven-ui/docs/vehicle-aircraft-control-dag.md` §L4 family
    // (line 384, esp_drone), §L5 chain (line 468: wifi_crtp / none /
    // mavlink_wifi), §L5.5 failsafe (line 517 row pattern).

    r.register(SolutionDefinition {
        id: "direct_control_telemetry_solution".into(),
        label: "Direct Control + WiFi Telemetry".into(),
        label_zh: Some("直连控制 + Wi-Fi 遥测".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into(), "esp32s3_mini1".into()],
        fixed_inputs: vec![InputSurface::WifiEvent],
        fixed_outputs: vec![
            OutputSurface::MotorDrive,
            OutputSurface::WifiPacket,
            OutputSurface::StatusLed,
        ],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "wifi_softap".into(),
                label: "Start SoftAP for phone + ground-station connection".into(),
                label_zh: Some("启动 SoftAP(供手机与地面站连接)".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "udp_listener".into(),
                label: "Start UDP listener on port 2390 (CRTP control)".into(),
                label_zh: Some("在端口 2390 监听 UDP(CRTP 控制)".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["wifi_softap".into()],
                ..Default::default()
            },
            OrchestrationStep {
                id: "mavlink_listener".into(),
                label: "Start MAVLink UDP server on port 14550 (telemetry)".into(),
                label_zh: Some("在端口 14550 启动 MAVLink UDP 服务(遥测)".into()),
                description: Some("QGroundControl-compatible heartbeat + parameter set + telemetry stream over WiFi.".into()),
                description_zh: Some("通过 Wi-Fi 输出 QGroundControl 兼容的心跳、参数集与遥测流。".into()),
                depends_on: vec!["wifi_softap".into()],
                ..Default::default()
            },
            OrchestrationStep {
                id: "control_loop".into(),
                label: "Stabilizer loop: CRTP setpoint → mix → motors; emit MAVLink telemetry @ telemetry_rate_hz".into(),
                label_zh: Some("稳定器循环:CRTP 设定值 → 混控 → 电机;按 telemetry_rate_hz 输出 MAVLink 遥测".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["udp_listener".into(), "mavlink_listener".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "control_with_telemetry".into(),
            label: "Real-time control + async telemetry".into(),
            decisions: vec![
                "Control loop on a dedicated high-priority task at control_rate_hz".into(),
                "MAVLink telemetry on a lower-priority task; control NEVER yields to telemetry".into(),
                "Failsafe: rx_loss + timeout_no_packet → motor cutoff within watchdog_ms".into(),
            ],
        },
        user_parameters: vec![
            vehicle_type_param(),
            control_protocol_param(),
            actuator_type_param(),
            imu_axis_tier_param(),
            imu_chip_param(),
            control_rate_hz_param(),
            failsafe_timeout_ms_param(),
            UserParameterDefinition {
                id: "mavlink_system_id".into(),
                label: "MAVLink System ID".into(),
                label_zh: Some("MAVLink 系统 ID".into()),
                required: false,
                secret: false,
                description: "MAVLink system ID exposed to QGroundControl (1-250)".into(),
                description_zh: Some("向 QGroundControl 暴露的 MAVLink 系统 ID(1-250)。".into()),
                default_value: Some(serde_json::Value::String("1".into())),
                enum_values: None,
                depends_on: None,
            },
            UserParameterDefinition {
                id: "telemetry_rate_hz".into(),
                label: "Telemetry Rate (Hz)".into(),
                label_zh: Some("遥测发送频率(Hz)".into()),
                required: false,
                secret: false,
                description: "How often to publish MAVLink HEARTBEAT + ATTITUDE + SYS_STATUS".into(),
                description_zh: Some("MAVLink HEARTBEAT/ATTITUDE/SYS_STATUS 的发送频率。".into()),
                default_value: Some(serde_json::Value::String("10".into())),
                enum_values: Some(vec![
                    EnumOption { value: "1".into(), label: "1 Hz".into(), description: Some("Minimum; only HEARTBEAT.".into()) },
                    EnumOption { value: "5".into(), label: "5 Hz".into(), description: Some("Light; OK over weak link.".into()) },
                    EnumOption { value: "10".into(), label: "10 Hz".into(), description: Some("Standard.".into()) },
                    EnumOption { value: "50".into(), label: "50 Hz".into(), description: Some("Tuning / log analysis.".into()) },
                ]),
                depends_on: None,
            },
        ],
        feedback_paths: vec![
            SignalPath {
                id: "phone_cmd_to_motor".into(),
                name: "Phone CRTP command to motor drive".into(),
                source: InputSurface::WifiEvent,
                transforms: vec![
                    SignalPathStep {
                        order: 1,
                        node: TransformNode::CommandDispatch,
                        label: Some("CRTP packet decode".into()),
                        description: None,
                    },
                    SignalPathStep {
                        order: 2,
                        node: TransformNode::PidLoop,
                        label: Some("Stabilizer PID".into()),
                        description: None,
                    },
                    SignalPathStep {
                        order: 3,
                        node: TransformNode::SafetyInterlock,
                        label: Some("RX-loss watchdog (motor_cutoff @ watchdog_ms)".into()),
                        description: None,
                    },
                ],
                sink: OutputSurface::MotorDrive,
                feedback: vec![SignalPathStep {
                    order: 1,
                    node: FeedbackSurface::PhysicalMotion,
                    label: None,
                    description: None,
                }],
                expected_user_result: "Phone app commands drive motors via CRTP".into(),
            },
            SignalPath {
                id: "telemetry_path".into(),
                name: "Vehicle state to QGroundControl over WiFi".into(),
                source: InputSurface::TimerTick,
                transforms: vec![
                    SignalPathStep {
                        order: 1,
                        node: TransformNode::ProtobufEncode,
                        label: Some("MAVLink message build".into()),
                        description: None,
                    },
                    SignalPathStep {
                        order: 2,
                        node: TransformNode::PeriodicTask,
                        label: Some("Emit at telemetry_rate_hz".into()),
                        description: None,
                    },
                ],
                sink: OutputSurface::WifiPacket,
                feedback: vec![SignalPathStep {
                    order: 1,
                    node: FeedbackSurface::WebStatus,
                    label: Some("QGroundControl shows live attitude/battery/RC".into()),
                    description: None,
                }],
                expected_user_result: "QGroundControl auto-discovers the vehicle and shows live telemetry while control loop stays uninterrupted".into(),
            },
        ],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["wifi".into()],
            optional: vec!["ota".into()],
        },
        runtime_binding: brookesia_vehicle_binding(vec![
            "rshome_motor_control".into(),
            "rshome_imu".into(),
            "rshome_failsafe".into(),
        ]),
        external_contracts: vec![
            "ESP-Drone App Protocol (UDP/CRTP)".into(),
            "MAVLink v2 over UDP (port 14550, QGroundControl)".into(),
        ],
        network_topology: NetworkTopology::default(),
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::ControlTelemetryBoard),
        communication_chains: None,
        pin_assignments: Some(vehicle_car_pins()),
        family: Some(ImplementationFamily::EspDrone),
        form_factor_families: Some(vec![
            FormFactorKind::Wheeled2wdDiff,
            FormFactorKind::Wheeled4wdDiff,
            FormFactorKind::Wheeled4wdAckermann,
            FormFactorKind::Wheeled6wd,
            FormFactorKind::TrackedSkidsteer,
        ]),
        control_uplink: Some(ControlUplinkKind::WifiCrtp),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::MavlinkWifi),
        sensor_tier_min: Some(SensorTierKind::Basic6ax),
        actuator_family: Some(ActuatorFamily::MixedDiffDrive),
        power_rails: Some(PowerRailKind::DualLogicMotor),
        failsafe: Some(FailsafeInfo {
            killswitch_source: vec![
                KillswitchSource::RxLoss,
                KillswitchSource::TimeoutNoPacket,
            ],
            rx_loss_behavior: Some(RxLossBehavior::MotorCutoff),
            watchdog_ms: Some(500),
            emergency_stop_wiring: EmergencyStopWiring::GpioPulldown,
        }),
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Caveat),
        ])),
    });

    // ── 10b. Dual-MCU Car (CAM + Control) ────────────────────────────────

    r.register(SolutionDefinition {
        id: "dual_mcu_car_solution".into(),
        label: "Dual-MCU Car (CAM + Control)".into(),
        label_zh: Some("双 MCU 小车(摄像头板 + 控制板)".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec![
            "esp32s3_wroom1".into(),
            "esp32s3_wroom1".into(),
        ],
        fixed_inputs: vec![InputSurface::WifiEvent, InputSurface::CameraFrame],
        fixed_outputs: vec![
            OutputSurface::MotorDrive,
            OutputSurface::HttpMjpegStream,
            OutputSurface::StatusLed,
        ],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "control_board_init".into(),
                label: "Control board: motor + IMU + RC + failsafe init".into(),
                label_zh: Some("控制板:电机 + IMU + 遥控 + 安全停车初始化".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "camera_board_init".into(),
                label: "Camera board: camera + MJPEG + WiFi init".into(),
                label_zh: Some("摄像头板:摄像头 + MJPEG + Wi-Fi 初始化".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "interboard_link".into(),
                label: "Inter-board UART link: heartbeat + vision results".into(),
                label_zh: Some("板间 UART 链路:心跳 + 视觉结果".into()),
                description: Some(
                    "Binary protocol: commands ctrl→cam, vision results cam→ctrl, bidirectional heartbeat"
                        .into(),
                ),
                description_zh: Some(
                    "二进制协议:命令 控制板→摄像头板,视觉结果 摄像头板→控制板,双向心跳。".into(),
                ),
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "control_loop".into(),
                label: "Control loop: RC input → PID → safety → motors".into(),
                label_zh: Some("控制循环:遥控输入 → PID → 安全 → 电机".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "dual_board_split".into(),
            label: "Dual-board task split".into(),
            decisions: vec![
                "Control board: real-time motor/safety loop at control_rate_hz".into(),
                "Camera board: MJPEG streaming at target FPS, async to control".into(),
                "Inter-board: 20ms heartbeat, 200ms link-loss timeout".into(),
                "Camera board failure does NOT affect control board safety".into(),
            ],
        },
        user_parameters: vec![
            vehicle_type_param(),
            control_protocol_param(),
            actuator_type_param(),
            imu_axis_tier_param(),
            imu_chip_param(),
            UserParameterDefinition {
                id: "interboard_baud".into(),
                label: "Inter-Board Baud Rate".into(),
                label_zh: Some("板间 UART 波特率".into()),
                required: false,
                secret: false,
                description: "UART baud rate between control and camera boards".into(),
                description_zh: Some("控制板与摄像头板之间的 UART 波特率。".into()),
                default_value: Some(serde_json::json!(460800)),
                enum_values: Some(vec![
                    EnumOption {
                        value: "115200".into(),
                        label: "115200".into(),
                        description: Some("Standard, lowest CPU overhead.".into()),
                    },
                    EnumOption {
                        value: "230400".into(),
                        label: "230400".into(),
                        description: None,
                    },
                    EnumOption {
                        value: "460800".into(),
                        label: "460800".into(),
                        description: Some("Recommended. Good balance.".into()),
                    },
                    EnumOption {
                        value: "921600".into(),
                        label: "921600".into(),
                        description: Some("Fastest, higher CPU load.".into()),
                    },
                ]),
                depends_on: None,
            },
        ],
        feedback_paths: vec![
            SignalPath {
                id: "control_path".into(),
                name: "RC input to motor drive (control board)".into(),
                source: InputSurface::WifiEvent,
                transforms: vec![
                    SignalPathStep {
                        order: 1,
                        node: TransformNode::CommandDispatch,
                        label: Some("RC protocol decode".into()),
                        description: None,
                    },
                    SignalPathStep {
                        order: 2,
                        node: TransformNode::PidLoop,
                        label: Some("Stabilizer PID".into()),
                        description: None,
                    },
                    SignalPathStep {
                        order: 3,
                        node: TransformNode::SafetyInterlock,
                        label: Some("Failsafe + brake".into()),
                        description: None,
                    },
                ],
                sink: OutputSurface::MotorDrive,
                feedback: vec![SignalPathStep {
                    order: 1,
                    node: FeedbackSurface::PhysicalMotion,
                    label: None,
                    description: None,
                }],
                expected_user_result: "RC commands drive motors via control board".into(),
            },
            SignalPath {
                id: "video_path".into(),
                name: "Camera to MJPEG stream (camera board)".into(),
                source: InputSurface::CameraFrame,
                transforms: vec![SignalPathStep {
                    order: 1,
                    node: TransformNode::JpegEncode,
                    label: Some("JPEG encode".into()),
                    description: None,
                }],
                sink: OutputSurface::HttpMjpegStream,
                feedback: vec![SignalPathStep {
                    order: 1,
                    node: FeedbackSurface::WebStatus,
                    label: None,
                    description: None,
                }],
                expected_user_result: "Camera streams MJPEG to phone/browser via WiFi".into(),
            },
        ],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["wifi".into()],
            optional: vec!["ota".into()],
        },
        runtime_binding: brookesia_vehicle_binding(vec![
            "rshome_motor_control".into(),
            "rshome_imu".into(),
            "rshome_failsafe".into(),
            "rshome_interboard".into(),
        ]),
        external_contracts: vec![
            "Inter-board UART protocol (binary frames)".into(),
            "ESP-Drone App Protocol (UDP/CRTP, optional)".into(),
        ],
        network_topology: NetworkTopology::default(),
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::ControlBoard),
        communication_chains: None,
        pin_assignments: Some(dual_mcu_control_board_pins()),
        family: Some(ImplementationFamily::EspDrone),
        form_factor_families: Some(vec![
            FormFactorKind::Wheeled2wdDiff,
            FormFactorKind::Wheeled4wdDiff,
            FormFactorKind::Wheeled6wd,
            FormFactorKind::TrackedSkidsteer,
        ]),
        control_uplink: Some(ControlUplinkKind::WifiCrtp),
        video_downlink: Some(VideoDownlinkKind::MjpegUart),
        telemetry: Some(TelemetryKind::MavlinkWifi),
        sensor_tier_min: Some(SensorTierKind::Basic6ax),
        actuator_family: Some(ActuatorFamily::MixedDiffDrive),
        power_rails: Some(PowerRailKind::DualLogicMotor),
        failsafe: Some(FailsafeInfo {
            killswitch_source: vec![
                KillswitchSource::RxLoss,
                KillswitchSource::TimeoutNoPacket,
                KillswitchSource::SbcHeartbeatLoss,
            ],
            rx_loss_behavior: Some(RxLossBehavior::MotorCutoff),
            watchdog_ms: Some(500),
            emergency_stop_wiring: EmergencyStopWiring::GpioPulldown,
        }),
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Caveat),
        ])),
    });

    // ── 10c. Balance Stabilizer (inverted-pendulum, 2-wheel) ─────────────
    //
    // Inverted-pendulum control loop for self-balancing 2-wheel platforms
    // (Segway-class) per `type-driven-ui/docs/vehicle-aircraft-control-dag.md`
    // §L1 (balance_2wheel form factor + min standard_9ax tier),
    // §L4 family (line 401, custom — no upstream lineage),
    // §L5 chain (line 485: esp_now / none / custom_uart),
    // §L5.5 failsafe (line 530: motor_cutoff / 100 ms / gpio_pulldown +
    // emergency_button kill source — fall-down recovery is a hard cut).

    r.register(SolutionDefinition {
        id: "balance_stabilizer_solution".into(),
        label: "Self-Balancing 2-Wheel Stabilizer".into(),
        label_zh: Some("两轮自平衡稳定器".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec![
            "esp32s3_wroom1".into(),
            "esp32s3_mini1".into(),
            "esp32c6_wroom1".into(),
            "esp32c6_mini1".into(),
        ],
        fixed_inputs: vec![
            InputSurface::I2cSensor,
            InputSurface::EspNowData,
            InputSurface::ButtonGpio,
        ],
        fixed_outputs: vec![OutputSurface::MotorDrive, OutputSurface::StatusLed],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "imu_calibrate".into(),
                label: "Calibrate IMU at rest (gyro zero, accel pitch reference)".into(),
                label_zh: Some("静止状态下校准 IMU(陀螺零位、加速度俯仰参考)".into()),
                description: Some("Robot must be held upright at the balance point during boot — first 1 s captures the upright reference angle.".into()),
                description_zh: Some("开机的前 1 秒,需将机器人保持在平衡点附近以采集直立参考角。".into()),
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "esp_now_init".into(),
                label: "Initialize ESP-NOW receiver for remote command".into(),
                label_zh: Some("初始化 ESP-NOW 接收以接收遥控命令".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "estop_wire".into(),
                label: "Bind emergency-stop GPIO (active-low pulldown to motor enable)".into(),
                label_zh: Some("绑定急停 GPIO(下拉至电机使能,低电平触发)".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "balance_loop".into(),
                label: "500 Hz IMU loop on core 1: AHRS → outer pendulum PID → inner diff-drive PID → H-bridge".into(),
                label_zh: Some("Core 1 上 500 Hz IMU 循环:AHRS → 外环倒立摆 PID → 内环差速 PID → H 桥".into()),
                description: Some("Outer loop holds tilt angle setpoint; inner loop runs left/right wheel velocities. Fall detection (>30° tilt for 50 ms) cuts H-bridge enable.".into()),
                description_zh: Some("外环维持倾角设定值;内环跑左右轮速度。倾角 >30° 持续 50 ms 判定跌倒,立即切断 H 桥使能。".into()),
                depends_on: vec!["imu_calibrate".into(), "esp_now_init".into(), "estop_wire".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "balance_dual_core".into(),
            label: "Dual-core split: IMU on core 1, comms on core 0".into(),
            decisions: vec![
                "Core 1 pinned IMU/PID loop at 500 Hz — no blocking calls, no logging in the hot path".into(),
                "Core 0 handles ESP-NOW receive + telemetry UART".into(),
                "Fall detection or e-stop fires the gpio_pulldown enable line in the same tick".into(),
            ],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "balance_loop_hz".into(),
                label: "Balance Loop Rate (Hz)".into(),
                label_zh: Some("平衡循环频率(Hz)".into()),
                required: false,
                secret: false,
                description: "IMU + outer-PID rate. ≥500 Hz strongly recommended for stable balance.".into(),
                description_zh: Some("IMU + 外环 PID 频率。建议 ≥500 Hz 才能稳定平衡。".into()),
                default_value: Some(serde_json::Value::String("500".into())),
                enum_values: Some(vec![
                    EnumOption { value: "250".into(), label: "250 Hz".into(), description: Some("Sluggish; only for very stable platforms.".into()) },
                    EnumOption { value: "500".into(), label: "500 Hz".into(), description: Some("Standard for self-balancing 2-wheel.".into()) },
                    EnumOption { value: "1000".into(), label: "1000 Hz".into(), description: Some("Best response; needs tight ISR scheduling.".into()) },
                ]),
                depends_on: None,
            },
            imu_axis_tier_param(),
            imu_chip_param(),
            UserParameterDefinition {
                id: "tilt_pid_kp".into(),
                label: "Tilt PID Kp".into(),
                label_zh: Some("倾角 PID 比例项".into()),
                required: false,
                secret: false,
                description: "Outer-loop pendulum proportional gain".into(),
                description_zh: Some("外环倒立摆比例增益。".into()),
                default_value: Some(serde_json::Value::String("18.0".into())),
                enum_values: None,
                depends_on: None,
            },
            UserParameterDefinition {
                id: "tilt_pid_ki".into(),
                label: "Tilt PID Ki".into(),
                label_zh: Some("倾角 PID 积分项".into()),
                required: false,
                secret: false,
                description: "Outer-loop pendulum integral gain".into(),
                description_zh: Some("外环倒立摆积分增益。".into()),
                default_value: Some(serde_json::Value::String("0.5".into())),
                enum_values: None,
                depends_on: None,
            },
            UserParameterDefinition {
                id: "tilt_pid_kd".into(),
                label: "Tilt PID Kd".into(),
                label_zh: Some("倾角 PID 微分项".into()),
                required: false,
                secret: false,
                description: "Outer-loop pendulum derivative gain".into(),
                description_zh: Some("外环倒立摆微分增益。".into()),
                default_value: Some(serde_json::Value::String("1.2".into())),
                enum_values: None,
                depends_on: None,
            },
            UserParameterDefinition {
                id: "fall_angle_deg".into(),
                label: "Fall-Detection Angle (°)".into(),
                label_zh: Some("跌倒判定倾角(°)".into()),
                required: false,
                secret: false,
                description: "Tilt magnitude that triggers motor cutoff".into(),
                description_zh: Some("达到该倾角后立即切断电机。".into()),
                default_value: Some(serde_json::Value::String("30".into())),
                enum_values: Some(vec![
                    EnumOption { value: "20".into(), label: "20°".into(), description: Some("Conservative — bails early.".into()) },
                    EnumOption { value: "30".into(), label: "30°".into(), description: Some("Standard.".into()) },
                    EnumOption { value: "45".into(), label: "45°".into(), description: Some("Permissive — only cuts on a hard tip-over.".into()) },
                ]),
                depends_on: None,
            },
            failsafe_timeout_ms_param(),
        ],
        feedback_paths: vec![SignalPath {
            id: "imu_to_motors".into(),
            name: "IMU tilt → wheel torque (closed loop)".into(),
            source: InputSurface::I2cSensor,
            transforms: vec![
                SignalPathStep {
                    order: 1,
                    node: TransformNode::Filter,
                    label: Some("AHRS (Mahony / Madgwick)".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: TransformNode::PidLoop,
                    label: Some("Outer pendulum PID (tilt → desired wheel velocity)".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 3,
                    node: TransformNode::PidLoop,
                    label: Some("Inner diff-drive PID (wheel velocity tracking)".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 4,
                    node: TransformNode::SafetyInterlock,
                    label: Some("Fall detection + e-stop GPIO (gpio_pulldown)".into()),
                    description: None,
                },
            ],
            sink: OutputSurface::MotorDrive,
            feedback: vec![
                SignalPathStep {
                    order: 1,
                    node: FeedbackSurface::PhysicalMotion,
                    label: None,
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: FeedbackSurface::LedIndicator,
                    label: Some("Status LED — solid = balanced, blink = fallen".into()),
                    description: None,
                },
            ],
            expected_user_result: "Robot stays upright; lean commands from ESP-NOW remote produce forward/back motion; tip past fall_angle cuts motors hard".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec![],
            optional: vec!["ota".into()],
        },
        runtime_binding: brookesia_vehicle_binding(vec![
            "rshome_motor_control".into(),
            "rshome_imu".into(),
            "rshome_failsafe".into(),
        ]),
        external_contracts: vec![
            "ESP-NOW broadcast frame (4-byte packed control payload)".into(),
        ],
        network_topology: NetworkTopology::PointToPoint,
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::ControlBoard),
        communication_chains: None,
        pin_assignments: Some(vehicle_car_pins()),
        family: Some(ImplementationFamily::Custom),
        form_factor_families: Some(vec![FormFactorKind::Balance2wheel]),
        control_uplink: Some(ControlUplinkKind::EspNow),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::CustomUart),
        sensor_tier_min: Some(SensorTierKind::Standard9ax),
        actuator_family: Some(ActuatorFamily::MixedDiffDrive),
        power_rails: Some(PowerRailKind::DualLogicMotor),
        failsafe: Some(FailsafeInfo {
            killswitch_source: vec![
                KillswitchSource::RxLoss,
                KillswitchSource::TimeoutNoPacket,
                KillswitchSource::EmergencyButton,
            ],
            rx_loss_behavior: Some(RxLossBehavior::MotorCutoff),
            watchdog_ms: Some(100),
            emergency_stop_wiring: EmergencyStopWiring::GpioPulldown,
        }),
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Caveat),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Insufficient),
        ])),
    });

    // ── 11. Remote Control TX ────────────────────────────────────────────

    r.register(SolutionDefinition {
        id: "remote_control_tx_solution".into(),
        label: "Remote Control / Phone Bridge (TX)".into(),
        label_zh: Some("遥控器 / 手机网桥 (TX)".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec![
            "esp32c6_wroom1".into(),
            "esp32c6_mini1".into(),
            "esp32s3_wroom1".into(),
        ],
        fixed_inputs: vec![InputSurface::ButtonGpio],
        fixed_outputs: vec![OutputSurface::WifiPacket],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "init_inputs".into(),
                label: "Initialize joystick ADC + buttons".into(),
                label_zh: Some("初始化摇杆 ADC 和按钮".into()),
                description: Some("Read joystick axes via ADC, buttons via GPIO".into()),
                description_zh: Some("通过 ADC 读取摇杆轴，通过 GPIO 读取按钮".into()),
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "start_tx".into(),
                label: "Start ESP-NOW / WiFi TX link".into(),
                label_zh: Some("启动 ESP-NOW / WiFi 发送链路".into()),
                description: Some("Initialize wireless TX".into()),
                description_zh: Some("初始化无线发送链路".into()),
                depends_on: vec!["init_inputs".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "periodic_tx".into(),
            label: "Periodic TX at control_rate_hz".into(),
            decisions: vec![],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "input_source".into(),
                label: "Input Source".into(),
                label_zh: Some("输入来源".into()),
                required: true,
                secret: false,
                description: "Where control commands come from".into(),
                description_zh: Some("控制命令的来源。".into()),
                default_value: Some(serde_json::Value::String("joystick_adc".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "joystick_adc".into(),
                        label: "Joystick + Buttons (ADC/GPIO)".into(),
                        description: Some(
                            "Handheld remote with physical joysticks and buttons.".into(),
                        ),
                    },
                    EnumOption {
                        value: "usb_phone".into(),
                        label: "USB from Phone".into(),
                        description: Some(
                            "Phone sends commands via USB OTG. Acts as 802.11 LR bridge.".into(),
                        ),
                    },
                    EnumOption {
                        value: "ble_phone".into(),
                        label: "BLE from Phone".into(),
                        description: Some(
                            "Phone sends commands via BLE. Acts as 802.11 LR bridge.".into(),
                        ),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "tx_protocol".into(),
                label: "TX Protocol".into(),
                label_zh: Some("发送协议".into()),
                required: true,
                secret: false,
                description: "Wireless protocol for sending control commands to the vehicle".into(),
                description_zh: Some("向载具发送控制命令的无线协议。".into()),
                default_value: Some(serde_json::Value::String("esp_now".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "esp_now".into(),
                        label: "ESP-NOW".into(),
                        description: Some("Connectionless, <200m, lowest latency.".into()),
                    },
                    EnumOption {
                        value: "wifi_udp".into(),
                        label: "WiFi UDP".into(),
                        description: Some("WiFi AP/STA + UDP packets.".into()),
                    },
                    EnumOption {
                        value: "wifi_lr".into(),
                        label: "802.11 LR".into(),
                        description: Some("WiFi Long Range. ESP-to-ESP only, >1km.".into()),
                    },
                    EnumOption {
                        value: "wifi_mavlink".into(),
                        label: "WiFi + MAVLink".into(),
                        description: Some("Full MAVLink. For QGroundControl.".into()),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "num_channels".into(),
                label: "Control Channels".into(),
                label_zh: Some("控制通道数".into()),
                required: false,
                secret: false,
                description: "Number of RC channels to transmit".into(),
                description_zh: Some("要发送的遥控通道数量。".into()),
                default_value: Some(serde_json::Value::String("4".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "2".into(),
                        label: "2 ch".into(),
                        description: Some("Throttle + steering.".into()),
                    },
                    EnumOption {
                        value: "4".into(),
                        label: "4 ch".into(),
                        description: Some("Standard: throttle, steering, aux1, aux2.".into()),
                    },
                    EnumOption {
                        value: "8".into(),
                        label: "8 ch".into(),
                        description: Some("Full: 4 axes + 4 switches.".into()),
                    },
                ]),
                depends_on: None,
            },
            control_rate_hz_param(),
        ],
        feedback_paths: vec![],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["wifi".into()],
            optional: vec![],
        },
        runtime_binding: RuntimeBinding::default(),
        external_contracts: vec![],
        network_topology: NetworkTopology::PointToPoint,
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::RemoteControlTx),
        communication_chains: None,
        pin_assignments: Some(remote_control_tx_pins()),
        family: Some(ImplementationFamily::Custom),
        form_factor_families: None,
        control_uplink: Some(ControlUplinkKind::Wifi80211lr),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::None),
        sensor_tier_min: None,
        actuator_family: None,
        power_rails: Some(PowerRailKind::SingleLogicOnly),
        failsafe: None,
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Caveat),
        ])),
    });

    // ── 11b. Video Board (dedicated camera/streaming) ─────────────────────

    r.register(SolutionDefinition {
        id: "video_board_solution".into(),
        label: "Video Board (Camera + UART)".into(),
        label_zh: Some("视频板(摄像头 + UART)".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into()],
        fixed_inputs: vec![InputSurface::CameraFrame],
        fixed_outputs: vec![OutputSurface::HttpMjpegStream],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "init_camera".into(),
                label: "Initialize camera + PSRAM".into(),
                label_zh: Some("初始化摄像头 + PSRAM".into()),
                description: Some("Configure camera DVP, allocate frame buffers".into()),
                description_zh: Some("配置摄像头 DVP，分配帧缓冲".into()),
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "start_stream".into(),
                label: "Start HTTP/MJPEG stream + inter-board UART".into(),
                label_zh: Some("启动 HTTP/MJPEG 推流 + 板间 UART".into()),
                description: Some("WiFi AP + MJPEG server + UART link to control board".into()),
                description_zh: Some("WiFi AP + MJPEG 服务 + 与控制板的 UART 链路".into()),
                depends_on: vec!["init_camera".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "camera_loop".into(),
            label: "Camera capture + stream loop".into(),
            decisions: vec![],
        },
        user_parameters: vec![
            camera_sensor_param(),
            frame_size_param(),
            UserParameterDefinition {
                id: "stream_fps".into(),
                label: "Stream FPS".into(),
                label_zh: Some("推流帧率".into()),
                required: false,
                secret: false,
                description: "Target video stream frame rate".into(),
                description_zh: Some("目标视频推流帧率。".into()),
                default_value: Some(serde_json::Value::String("15".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "5".into(),
                        label: "5 fps".into(),
                        description: Some("Lowest bandwidth.".into()),
                    },
                    EnumOption {
                        value: "10".into(),
                        label: "10 fps".into(),
                        description: None,
                    },
                    EnumOption {
                        value: "15".into(),
                        label: "15 fps".into(),
                        description: Some("Standard.".into()),
                    },
                    EnumOption {
                        value: "25".into(),
                        label: "25 fps".into(),
                        description: Some("Smooth.".into()),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "video_output".into(),
                label: "Video Output".into(),
                label_zh: Some("视频输出方式".into()),
                required: true,
                secret: false,
                description: "How the captured video is streamed back to the operator".into(),
                description_zh: Some("捕获的视频如何回传给操作者。".into()),
                default_value: Some(serde_json::Value::String("wifi_mjpeg".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "wifi_mjpeg".into(),
                        label: "WiFi HTTP MJPEG".into(),
                        description: Some(
                            "ESP32 serves MJPEG over WiFi AP. Built-in, no extra hardware.".into(),
                        ),
                    },
                    EnumOption {
                        value: "wifi_rtsp".into(),
                        label: "WiFi RTSP".into(),
                        description: Some(
                            "ESP32 serves RTSP over WiFi. For VLC/media players.".into(),
                        ),
                    },
                    EnumOption {
                        value: "analog_vtx".into(),
                        label: "Analog 5.8G VTX".into(),
                        description: Some(
                            "Camera output to external analog VTX module. Traditional FPV.".into(),
                        ),
                    },
                    EnumOption {
                        value: "digital_vtx".into(),
                        label: "Digital VTX (DJI/HDZero/Walksnail)".into(),
                        description: Some("Camera output to external digital VTX. HD FPV.".into()),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "rx_protocol".into(),
                label: "RX Protocol".into(),
                label_zh: Some("接收协议".into()),
                required: false,
                secret: false,
                description: "How the video board receives commands from the control board".into(),
                description_zh: Some("视频板如何从控制板接收命令。".into()),
                default_value: Some(serde_json::Value::String("uart_binary".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "uart_binary".into(),
                        label: "UART Binary".into(),
                        description: Some("Direct binary protocol over inter-board UART.".into()),
                    },
                    EnumOption {
                        value: "uart_mavlink".into(),
                        label: "UART MAVLink".into(),
                        description: Some("MAVLink over inter-board UART.".into()),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "interboard_baud".into(),
                label: "Inter-Board Baud Rate".into(),
                label_zh: Some("板间 UART 波特率".into()),
                required: false,
                secret: false,
                description: "UART baud rate to control board".into(),
                description_zh: Some("与控制板通信的 UART 波特率。".into()),
                default_value: Some(serde_json::Value::String("460800".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "115200".into(),
                        label: "115200".into(),
                        description: Some("Standard.".into()),
                    },
                    EnumOption {
                        value: "460800".into(),
                        label: "460800".into(),
                        description: Some("Recommended.".into()),
                    },
                    EnumOption {
                        value: "921600".into(),
                        label: "921600".into(),
                        description: Some("Fastest.".into()),
                    },
                ]),
                depends_on: None,
            },
        ],
        feedback_paths: vec![],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["wifi".into()],
            optional: vec![],
        },
        runtime_binding: RuntimeBinding::default(),
        external_contracts: vec![],
        network_topology: NetworkTopology::PointToPoint,
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::VideoBoard),
        communication_chains: None,
        pin_assignments: Some(video_board_pins()),
        family: Some(ImplementationFamily::Custom),
        form_factor_families: None,
        control_uplink: Some(ControlUplinkKind::None),
        video_downlink: Some(VideoDownlinkKind::MjpegUart),
        telemetry: Some(TelemetryKind::None),
        sensor_tier_min: None,
        actuator_family: None,
        power_rails: Some(PowerRailKind::SingleLogicOnly),
        failsafe: None,
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Insufficient),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Insufficient),
        ])),
    });

    // ── 11b.i. Analog VTX Passthrough (camera + 5.8 GHz) ───────────────────
    //
    // Camera-only ESP32 wired to an external analog 5.8 GHz video transmitter
    // module. No JPEG encode, no UART link, no failsafe (no actuators) — the
    // ESP32 just powers/triggers the VTX and optionally exposes camera config.
    // Per `type-driven-ui/docs/vehicle-aircraft-control-dag.md` §L4 family
    // (line 388, custom), §L5 chain (line 473: none / analog_vtx / none), and
    // the standard_fpv × video_board cell of §L2.

    r.register(SolutionDefinition {
        id: "analog_vtx_passthrough_solution".into(),
        label: "Analog VTX Passthrough (Camera + 5.8 GHz)".into(),
        label_zh: Some("模拟图传直通(摄像头 + 5.8 GHz)".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into()],
        fixed_inputs: vec![InputSurface::CameraFrame],
        fixed_outputs: vec![OutputSurface::GpioLevel],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "init_camera".into(),
                label: "Initialize camera + PSRAM frame buffers".into(),
                label_zh: Some("初始化摄像头 + PSRAM 帧缓冲".into()),
                description: Some("Configure DVP and apply sensor exposure/gain defaults — analog VTX takes the raw video signal directly.".into()),
                description_zh: Some("配置 DVP 并应用传感器曝光/增益默认值;模拟图传直接拿到视频原始信号。".into()),
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "init_vtx".into(),
                label: "Power VTX + apply default channel/power".into(),
                label_zh: Some("VTX 上电 + 应用默认频道/功率".into()),
                description: Some("Drive the VTX power-enable GPIO and (if SmartAudio is wired) push the saved channel + power level.".into()),
                description_zh: Some("驱动 VTX 的电源使能 GPIO,若接有 SmartAudio 则下发已保存的频道与功率。".into()),
                depends_on: vec!["init_camera".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "vtx_idle".into(),
            label: "Camera + VTX powered, MCU mostly idle".into(),
            decisions: vec![
                "Camera DVP runs continuously; analog VTX consumes the signal directly".into(),
                "MCU has no realtime obligations once camera + VTX are up".into(),
            ],
        },
        user_parameters: vec![
            camera_sensor_param(),
            UserParameterDefinition {
                id: "vtx_channel".into(),
                label: "VTX Channel".into(),
                label_zh: Some("VTX 频道".into()),
                required: true,
                secret: false,
                description: "5.8 GHz channel for the analog VTX (Raceband)".into(),
                description_zh: Some("模拟图传 5.8 GHz 频道(Raceband)。".into()),
                default_value: Some(serde_json::Value::String("R1".into())),
                enum_values: Some(vec![
                    EnumOption { value: "R1".into(), label: "R1 (5658)".into(), description: None },
                    EnumOption { value: "R2".into(), label: "R2 (5695)".into(), description: None },
                    EnumOption { value: "R3".into(), label: "R3 (5732)".into(), description: None },
                    EnumOption { value: "R4".into(), label: "R4 (5769)".into(), description: None },
                    EnumOption { value: "R5".into(), label: "R5 (5806)".into(), description: None },
                    EnumOption { value: "R6".into(), label: "R6 (5843)".into(), description: None },
                    EnumOption { value: "R7".into(), label: "R7 (5880)".into(), description: None },
                    EnumOption { value: "R8".into(), label: "R8 (5917)".into(), description: None },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "vtx_power_mw".into(),
                label: "VTX Power (mW)".into(),
                label_zh: Some("VTX 发射功率(mW)".into()),
                required: false,
                secret: false,
                description: "Output power; check local regulations".into(),
                description_zh: Some("发射功率,请遵守当地法规。".into()),
                default_value: Some(serde_json::Value::String("25".into())),
                enum_values: Some(vec![
                    EnumOption { value: "25".into(), label: "25 mW (CE/race)".into(), description: Some("Universally legal for racing.".into()) },
                    EnumOption { value: "200".into(), label: "200 mW".into(), description: Some("Mid-range; legal in many regions.".into()) },
                    EnumOption { value: "500".into(), label: "500 mW".into(), description: Some("Long-range; check local regulations.".into()) },
                    EnumOption { value: "800".into(), label: "800 mW".into(), description: Some("Maximum; restricted in many regions.".into()) },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "smartaudio_enabled".into(),
                label: "SmartAudio Control".into(),
                label_zh: Some("SmartAudio 控制".into()),
                required: false,
                secret: false,
                description: "Enable SmartAudio UART link to change channel/power at runtime".into(),
                description_zh: Some("启用 SmartAudio UART 以在运行时切换频道与功率。".into()),
                default_value: Some(serde_json::json!(false)),
                enum_values: None,
                depends_on: None,
            },
        ],
        feedback_paths: vec![SignalPath {
            id: "camera_to_vtx".into(),
            name: "Camera analog signal to 5.8 GHz VTX".into(),
            source: InputSurface::CameraFrame,
            transforms: vec![SignalPathStep {
                order: 1,
                node: TransformNode::OneToOne,
                label: Some("Direct analog signal — no MCU encoding".into()),
                description: None,
            }],
            sink: OutputSurface::GpioLevel,
            feedback: vec![SignalPathStep {
                order: 1,
                node: FeedbackSurface::LedIndicator,
                label: Some("VTX status LED".into()),
                description: None,
            }],
            expected_user_result: "FPV goggles see live analog video on the configured 5.8 GHz channel".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle { required: vec![], optional: vec!["ota".into()] },
        runtime_binding: RuntimeBinding::default(),
        external_contracts: vec!["SmartAudio v2 (optional)".into()],
        network_topology: NetworkTopology::None,
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::VideoBoard),
        communication_chains: None,
        pin_assignments: Some(video_board_pins()),
        family: Some(ImplementationFamily::Custom),
        form_factor_families: None,
        control_uplink: Some(ControlUplinkKind::None),
        video_downlink: Some(VideoDownlinkKind::AnalogVtx),
        telemetry: Some(TelemetryKind::None),
        sensor_tier_min: None,
        actuator_family: None,
        power_rails: Some(PowerRailKind::DualLogicMotor),
        failsafe: None,
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Insufficient),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Insufficient),
        ])),
    });

    // ── 11c. Receiver Direct-Drive ────────────────────────────────────────

    r.register(SolutionDefinition {
        id: "receiver_direct_drive_solution".into(),
        label: "⚠️ Receiver Passthrough (Legacy RX → PWM)".into(),
        label_zh: Some("⚠️ 接收机直通(旧式 RX → PWM)".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32c6_wroom1".into(), "esp32c6_mini1".into()],
        fixed_inputs: vec![InputSurface::WifiEvent],
        fixed_outputs: vec![OutputSurface::GpioLevel],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "init_rx".into(),
                label: "Initialize RX (CRSF/SBUS/ESP-NOW/802.11 LR)".into(),
                label_zh: Some("初始化接收(CRSF/SBUS/ESP-NOW/802.11 LR)".into()),
                description: Some("Start wireless receiver link".into()),
                description_zh: Some("启动无线接收链路".into()),
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "passthrough".into(),
                label: "RX channel → PWM output passthrough + failsafe".into(),
                label_zh: Some("接收通道 → PWM 输出直通 + 安全停机".into()),
                description: Some("No PID, no mixing — each RX channel maps 1:1 to a PWM output. Failsafe watchdog cuts outputs on link loss.".into()),
                description_zh: Some("无 PID、无混控——每个接收通道 1:1 映射到 PWM 输出。链路丢失时看门狗切断输出。".into()),
                depends_on: vec!["init_rx".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "rx_passthrough".into(),
            label: "RX passthrough at packet rate".into(),
            decisions: vec![],
        },
        user_parameters: vec![
            control_protocol_param(),
            UserParameterDefinition {
                id: "output_channels".into(),
                label: "Output Channels".into(),
                label_zh: Some("输出通道数".into()),
                required: true,
                secret: false,
                description: "Number of PWM/servo outputs to drive".into(),
                description_zh: Some("要驱动的 PWM/舵机输出数量。".into()),
                default_value: Some(serde_json::Value::String("4".into())),
                enum_values: Some(vec![
                    EnumOption { value: "2".into(), label: "2 ch".into(), description: Some("Throttle + steering.".into()) },
                    EnumOption { value: "4".into(), label: "4 ch".into(), description: Some("Standard: 2 drive + 2 aux.".into()) },
                    EnumOption { value: "6".into(), label: "6 ch".into(), description: Some("Extended: 4 drive + 2 aux.".into()) },
                    EnumOption { value: "8".into(), label: "8 ch".into(), description: Some("Full: all outputs.".into()) },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "output_type".into(),
                label: "Output Type".into(),
                label_zh: Some("输出类型".into()),
                required: true,
                secret: false,
                description: "Signal type for PWM outputs".into(),
                description_zh: Some("PWM 输出的信号类型。".into()),
                default_value: Some(serde_json::Value::String("servo_pwm".into())),
                enum_values: Some(vec![
                    EnumOption { value: "servo_pwm".into(), label: "Servo PWM (1-2ms)".into(), description: Some("Standard servo signal, 50Hz.".into()) },
                    EnumOption { value: "esc_pwm".into(), label: "ESC PWM".into(), description: Some("ESC throttle signal, 50-500Hz.".into()) },
                    EnumOption { value: "dshot".into(), label: "DShot".into(), description: Some("Digital protocol. DShot150/300/600.".into()) },
                ]),
                depends_on: None,
            },
            failsafe_timeout_ms_param(),
        ],
        feedback_paths: vec![],
        variants: vec![],
        component_bundle: ComponentBundle { required: vec!["wifi".into()], optional: vec![] },
        runtime_binding: RuntimeBinding::default(),
        external_contracts: vec![],
        network_topology: NetworkTopology::PointToPoint,
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::ReceiverDirectDrive),
        communication_chains: None,
        pin_assignments: Some(receiver_direct_drive_pins()),
        family: Some(ImplementationFamily::Custom),
        form_factor_families: None,
        control_uplink: Some(ControlUplinkKind::Crsf),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::CrsfTelemetry),
        sensor_tier_min: None,
        actuator_family: None,
        power_rails: None,
        failsafe: Some(FailsafeInfo {
            killswitch_source: vec![KillswitchSource::RxLoss],
            rx_loss_behavior: Some(RxLossBehavior::PassthroughLast),
            watchdog_ms: None,
            emergency_stop_wiring: EmergencyStopWiring::None,
        }),
        // Receiver-passthrough has minimal MCU workload (UART + PWM fan-out, no
        // PID, no IMU). C6 is the BOM-optimal choice for a dedicated RX board;
        // S3 works but is overkill; D0WD works but lacks modern CRSF-friendly
        // UART features and is legacy-only.
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Caveat),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Caveat),
        ])),
    });

    // ── 11d. SBUS Passthrough (RX → PWM, ⚠️ legacy) ─────────────────────
    //
    // Single-direction SBUS receiver feeding 4–8 PWM outputs with no MCU-
    // level failsafe — recovery is whatever the RX itself does. Per
    // `type-driven-ui/docs/vehicle-aircraft-control-dag.md` §L4 family table
    // (line 396, custom), §L5 chain (line 471: sbus / none / none), §L5.5
    // failsafe (line 538: passthrough_last / null / none, ⚠️ legacy).
    // Kept for backward compatibility with installed bases; the doc itself
    // marks this and receiver_direct_drive as not-recommended for new builds.

    r.register(SolutionDefinition {
        id: "sbus_passthrough_solution".into(),
        label: "⚠️ SBUS Passthrough (Legacy RX → PWM)".into(),
        label_zh: Some("⚠️ SBUS 直通(旧式 RX → PWM)".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec![
            "esp32c6_wroom1".into(),
            "esp32c6_mini1".into(),
            "esp32s3_wroom1".into(),
            "esp32s3_mini1".into(),
        ],
        fixed_inputs: vec![InputSurface::UartRx],
        fixed_outputs: vec![OutputSurface::ServoDrive, OutputSurface::StatusLed],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "sbus_uart_init".into(),
                label: "Initialize SBUS UART (100000 baud, 8E2, inverted)".into(),
                label_zh: Some("初始化 SBUS UART(100000 波特,8E2,反相)".into()),
                description: Some("Open UART with inverted-RX line discipline; SBUS frames at 7 ms or 14 ms cadence.".into()),
                description_zh: Some("打开 UART 并启用反相接收;SBUS 帧周期 7 ms 或 14 ms。".into()),
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "passthrough".into(),
                label: "Decode SBUS frame → fan out to PWM channels (no PID, no mixing)".into(),
                label_zh: Some("解码 SBUS 帧 → 直接分发到 PWM 通道(无 PID、无混控)".into()),
                description: Some("Each SBUS channel maps 1:1 to a PWM output. RX failsafe flag honoured if present, otherwise last good channel values are held.".into()),
                description_zh: Some("每个 SBUS 通道 1:1 映射到 PWM 输出。若 RX 标志失效则尊重该标志,否则保持最近一帧的通道值。".into()),
                depends_on: vec!["sbus_uart_init".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "sbus_passthrough".into(),
            label: "SBUS frame-driven passthrough".into(),
            decisions: vec![
                "Frame-driven: PWM updates ride on SBUS frame arrival".into(),
                "No firmware watchdog — failsafe behaviour is whatever the SBUS receiver was configured to do".into(),
                "⚠️ Not recommended for new builds; offered for compatibility with existing SBUS gear".into(),
            ],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "output_channels".into(),
                label: "Output Channels".into(),
                label_zh: Some("输出通道数".into()),
                required: true,
                secret: false,
                description: "Number of SBUS channels to decode and fan out to PWM".into(),
                description_zh: Some("要解码并分发到 PWM 的 SBUS 通道数。".into()),
                default_value: Some(serde_json::Value::String("4".into())),
                enum_values: Some(vec![
                    EnumOption { value: "2".into(), label: "2 ch".into(), description: Some("Throttle + steering.".into()) },
                    EnumOption { value: "4".into(), label: "4 ch".into(), description: Some("Standard surface set.".into()) },
                    EnumOption { value: "6".into(), label: "6 ch".into(), description: Some("Surfaces + 2 aux.".into()) },
                    EnumOption { value: "8".into(), label: "8 ch".into(), description: Some("Full SBUS bank.".into()) },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "output_type".into(),
                label: "Output Type".into(),
                label_zh: Some("输出类型".into()),
                required: true,
                secret: false,
                description: "Signal type for the PWM channels".into(),
                description_zh: Some("PWM 通道的信号类型。".into()),
                default_value: Some(serde_json::Value::String("servo_pwm".into())),
                enum_values: Some(vec![
                    EnumOption { value: "servo_pwm".into(), label: "Servo PWM (1-2 ms, 50 Hz)".into(), description: Some("Standard hobby servo.".into()) },
                    EnumOption { value: "esc_pwm".into(), label: "ESC PWM (1-2 ms, 50–500 Hz)".into(), description: Some("Brushless ESC throttle signal.".into()) },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "sbus_invert".into(),
                label: "Invert SBUS Line".into(),
                label_zh: Some("反相 SBUS 信号".into()),
                required: false,
                secret: false,
                description: "Whether the SBUS line needs hardware inversion (most receivers do)".into(),
                description_zh: Some("SBUS 信号是否需要硬件反相(大多数接收机需要)。".into()),
                default_value: Some(serde_json::json!(true)),
                enum_values: None,
                depends_on: None,
            },
        ],
        feedback_paths: vec![SignalPath {
            id: "sbus_to_pwm".into(),
            name: "SBUS frame to PWM channel fan-out".into(),
            source: InputSurface::UartRx,
            transforms: vec![
                SignalPathStep {
                    order: 1,
                    node: TransformNode::UartProtocolParse,
                    label: Some("SBUS frame decode".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: TransformNode::OneToMany,
                    label: Some("Channel fan-out (no PID, no mixing)".into()),
                    description: None,
                },
            ],
            sink: OutputSurface::ServoDrive,
            feedback: vec![SignalPathStep {
                order: 1,
                node: FeedbackSurface::PhysicalMotion,
                label: None,
                description: None,
            }],
            expected_user_result: "RX channels drive PWM directly; failsafe is whatever the RX is configured to emit".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle { required: vec![], optional: vec!["ota".into()] },
        runtime_binding: RuntimeBinding::default(),
        external_contracts: vec!["SBUS (Futaba) RC protocol".into()],
        network_topology: NetworkTopology::PointToPoint,
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::ReceiverDirectDrive),
        communication_chains: None,
        pin_assignments: Some(receiver_direct_drive_pins()),
        family: Some(ImplementationFamily::Custom),
        form_factor_families: None,
        control_uplink: Some(ControlUplinkKind::Sbus),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::None),
        sensor_tier_min: None,
        actuator_family: None,
        power_rails: Some(PowerRailKind::TripleMotorServoLogic),
        failsafe: Some(FailsafeInfo {
            killswitch_source: vec![KillswitchSource::RxLoss],
            rx_loss_behavior: Some(RxLossBehavior::PassthroughLast),
            watchdog_ms: None,
            emergency_stop_wiring: EmergencyStopWiring::None,
        }),
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Caveat),
        ])),
    });

    // ── 12b. Quadrotor Stabilizer (CRSF / ELRS → DShot) ────────────────────
    //
    // Reference multirotor stabilizer per
    // `type-driven-ui/docs/vehicle-aircraft-control-dag.md` §L4 (family),
    // §L5 (chain), §L5.5 (failsafe), §L6 (sensor/actuator). S3-only because
    // FPU + vector ISA are required for Mahony/Madgwick at ≥1 kHz.

    r.register(SolutionDefinition {
        id: "quad_stabilizer_solution".into(),
        label: "Quadrotor Stabilizer".into(),
        label_zh: Some("多旋翼稳定器".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into(), "esp32s3_mini1".into()],
        fixed_inputs: vec![InputSurface::UartRx, InputSurface::I2cSensor],
        fixed_outputs: vec![OutputSurface::MotorDrive, OutputSurface::StatusLed],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "crsf_rx_init".into(),
                label: "Initialize CRSF UART receiver".into(),
                label_zh: Some("初始化 CRSF UART 接收机".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "imu_calibrate".into(),
                label: "Calibrate IMU (accel/gyro zero; mag if present)".into(),
                label_zh: Some("IMU 校准(加速度/陀螺零位;有磁力计则一并校准)".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "stabilizer_loop".into(),
                label: "Stabilizer loop @ control_rate_hz: sensor → AHRS → PID → mix → DShot".into(),
                label_zh: Some("稳定器循环(按 control_rate_hz):传感器 → AHRS → PID → 混控 → DShot".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["crsf_rx_init".into(), "imu_calibrate".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "stabilizer_1khz".into(),
            label: "1 kHz stabilizer loop".into(),
            decisions: vec![
                "Stabilizer runs on a dedicated core-pinned task; no blocking syscalls in the loop".into(),
                "CRSF channel-packet arrival triggers a setpoint update; the loop itself free-runs at control_rate_hz".into(),
                "Low-voltage or watchdog timeout cuts motors via DShot special command".into(),
            ],
        },
        user_parameters: vec![
            vehicle_type_param(),
            imu_axis_tier_param(),
            imu_chip_param(),
            control_rate_hz_param(),
            failsafe_timeout_ms_param(),
        ],
        feedback_paths: vec![SignalPath {
            id: "rc_cmd_to_motor".into(),
            name: "RC command to motor mix".into(),
            source: InputSurface::UartRx,
            transforms: vec![
                SignalPathStep {
                    order: 1,
                    node: TransformNode::CommandDispatch,
                    label: Some("CRSF channel decode".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: TransformNode::PidLoop,
                    label: Some("Stabilizer PID (roll/pitch/yaw)".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 3,
                    node: TransformNode::SafetyInterlock,
                    label: Some("Throttle lock + low-voltage cutoff".into()),
                    description: None,
                },
            ],
            sink: OutputSurface::MotorDrive,
            feedback: vec![SignalPathStep {
                order: 1,
                node: FeedbackSurface::PhysicalMotion,
                label: None,
                description: None,
            }],
            expected_user_result: "Stick input translates to stable attitude hold; kill-switch or signal loss cuts motors within watchdog_ms".into(),
        }],
        // ── Variant collapse (rshome-codegen-variants PRD Phase 4 T4.1) ──────
        //
        // Replaces the three sibling registrations (`quad_stabilizer_solution`,
        // `quad_stabilizer_dshot_solution`, `quad_stabilizer_bdshot_solution`)
        // that existed pre-2026-04-22. The base solution now represents the
        // CRSF→PID→ESC stabilizer shape; the three variants differ only in
        // the motor signaling protocol (PWM / DShot / BDShot) plus the
        // corresponding board assembly.
        variants: vec![
            SolutionVariantDefinition {
                id: "pwm".into(),
                label: "PWM (LEDC)".into(),
                label_zh: Some("PWM(LEDC)".into()),
                required_caps: vec![],
                parameter_defaults: BTreeMap::new(),
                add_components: vec![],
                remove_components: vec![],
                add_external_contracts: vec![],
                // PWM is the solution's default — no flag overlay, no
                // runtime_binding override needed.
                active_flag_add: vec![],
                active_flag_remove: vec![],
                user_parameter_overrides: vec![],
                runtime_binding_override: None,
            },
            SolutionVariantDefinition {
                id: "dshot".into(),
                label: "DShot600 over RMT".into(),
                label_zh: Some("DShot600 (RMT)".into()),
                required_caps: vec![],
                parameter_defaults: BTreeMap::new(),
                add_components: vec![],
                remove_components: vec![],
                add_external_contracts: vec![
                    "DShot600 ESC protocol over RMT".into(),
                ],
                active_flag_add: vec!["USE_DSHOT".into()],
                active_flag_remove: vec![],
                user_parameter_overrides: vec![],
                runtime_binding_override: Some(RuntimeBindingOverlay {
                    board_assembly: Some(
                        "esp32s3_va_multirotor_dshot_assembly".into(),
                    ),
                    ..Default::default()
                }),
            },
            SolutionVariantDefinition {
                id: "bdshot".into(),
                label: "Bidirectional DShot (eRPM RX)".into(),
                label_zh: Some("双向 DShot (eRPM 回读)".into()),
                required_caps: vec![],
                parameter_defaults: BTreeMap::new(),
                add_components: vec![],
                remove_components: vec![],
                add_external_contracts: vec![
                    "Bidirectional DShot600 over RMT (TX + eRPM RX; requires BLHeli_32/AM32 ESC firmware)".into(),
                ],
                active_flag_add: vec!["USE_BDSHOT".into()],
                active_flag_remove: vec![],
                user_parameter_overrides: vec![],
                runtime_binding_override: Some(RuntimeBindingOverlay {
                    board_assembly: Some(
                        "esp32s3_va_multirotor_bdshot_assembly".into(),
                    ),
                    ..Default::default()
                }),
            },
        ],
        component_bundle: ComponentBundle {
            required: vec!["wifi".into()],
            optional: vec!["ota".into()],
        },
        runtime_binding: {
            let mut b = brookesia_vehicle_binding(vec![
                "rshome_motor_control".into(),
                "rshome_imu".into(),
            ]);
            b.board_assembly = Some("esp32s3_va_multirotor_assembly".into());
            b
        },
        external_contracts: vec![
            "CRSF (ExpressLRS) RC protocol".into(),
            "DShot ESC protocol".into(),
        ],
        network_topology: NetworkTopology::default(),
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::ControlBoard),
        communication_chains: None,
        pin_assignments: Some(vehicle_control_pins()),
        family: Some(ImplementationFamily::Px4),
        form_factor_families: Some(vec![
            FormFactorKind::QuadcopterX,
            FormFactorKind::QuadcopterPlus,
            FormFactorKind::Tricopter,
            FormFactorKind::Hexacopter,
            FormFactorKind::OctocopterX,
            FormFactorKind::OctocopterCoax,
        ]),
        control_uplink: Some(ControlUplinkKind::Crsf),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::CrsfTelemetry),
        sensor_tier_min: Some(SensorTierKind::Standard9ax),
        actuator_family: Some(ActuatorFamily::QuadMix),
        power_rails: Some(PowerRailKind::DualLogicMotor),
        failsafe: Some(FailsafeInfo {
            killswitch_source: vec![
                KillswitchSource::RcSwitch,
                KillswitchSource::RxLoss,
                KillswitchSource::LowVoltage,
            ],
            rx_loss_behavior: Some(RxLossBehavior::MotorCutoff),
            watchdog_ms: Some(100),
            emergency_stop_wiring: EmergencyStopWiring::EscDshotCmd,
        }),
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Insufficient),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Insufficient),
        ])),
    });

    // ── 12b.i. Fixed-Wing Stabilizer (CRSF / ELRS → surfaces+throttle) ────
    //
    // ArduPlane-style stabilizer for fixed-wing airframes per
    // `type-driven-ui/docs/vehicle-aircraft-control-dag.md` §L1
    // (fixed-wing form factors), §L4 family table (line 398), §L5 chain
    // (line 482) and §L5.5 failsafe (line 527: glide_trim / 250 / relay_cutoff).
    // S3-only because attitude PID over 4+ surfaces wants FPU + vector ISA;
    // standard_9ax minimum to keep heading stable in a banked turn.

    r.register(SolutionDefinition {
        id: "fixedwing_stabilizer_solution".into(),
        label: "Fixed-Wing Stabilizer".into(),
        label_zh: Some("固定翼稳定器".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into(), "esp32s3_mini1".into()],
        fixed_inputs: vec![InputSurface::UartRx, InputSurface::I2cSensor],
        fixed_outputs: vec![
            OutputSurface::ServoDrive,
            OutputSurface::McpwmPwm,
            OutputSurface::StatusLed,
        ],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "crsf_rx_init".into(),
                label: "Initialize CRSF UART receiver".into(),
                label_zh: Some("初始化 CRSF UART 接收机".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "imu_calibrate".into(),
                label: "Calibrate IMU (accel/gyro/mag)".into(),
                label_zh: Some("IMU 校准(加速度/陀螺/磁力计)".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "stabilizer_loop".into(),
                label: "Stabilizer @ control_rate_hz: CRSF → AHRS → attitude PID → surface mix → PWM".into(),
                label_zh: Some("稳定器循环(按 control_rate_hz):CRSF → AHRS → 姿态 PID → 舵面混控 → PWM".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["crsf_rx_init".into(), "imu_calibrate".into()],
                ..Default::default()
            },
            OrchestrationStep {
                id: "rx_loss_glide".into(),
                label: "On RX loss: hold trim attitude (gentle bank), throttle low, until link returns".into(),
                label_zh: Some("接收丢失时:保持配平姿态(轻微滚转),油门收低,等待链路恢复".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["stabilizer_loop".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "stabilizer_500hz".into(),
            label: "500 Hz stabilizer loop".into(),
            decisions: vec![
                "Stabilizer runs on a dedicated core-pinned task; no blocking syscalls in the loop".into(),
                "CRSF channel-packet arrival triggers a setpoint update; the loop free-runs at control_rate_hz".into(),
                "RX loss → glide_trim (hold low-bank attitude + throttle low) until link returns or watchdog expires; then relay_cutoff".into(),
            ],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "airframe_variant".into(),
                label: "Airframe Variant".into(),
                label_zh: Some("机型变体".into()),
                required: true,
                secret: false,
                description: "Which fixed-wing surface mix to apply".into(),
                description_zh: Some("使用哪种固定翼舵面混控。".into()),
                default_value: Some(serde_json::Value::String("standard".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "standard".into(),
                        label: "Standard (aileron + elevator + rudder + throttle)".into(),
                        description: Some("Trainer / sport plane. 4 channels, separate tail.".into()),
                    },
                    EnumOption {
                        value: "vtail".into(),
                        label: "V-tail (ruddervator + aileron + throttle)".into(),
                        description: Some("Combined pitch+yaw on V-tail surfaces.".into()),
                    },
                    EnumOption {
                        value: "flying_wing".into(),
                        label: "Flying wing (elevons + throttle)".into(),
                        description: Some("No tail; combined pitch+roll on elevons.".into()),
                    },
                    EnumOption {
                        value: "glider".into(),
                        label: "Glider (aileron + elevator [+ rudder], no throttle)".into(),
                        description: Some("Unpowered; throttle channel ignored.".into()),
                    },
                ]),
                depends_on: None,
            },
            imu_axis_tier_param(),
            imu_chip_param(),
            control_rate_hz_param(),
            failsafe_timeout_ms_param(),
            UserParameterDefinition {
                id: "low_voltage_cutoff_v".into(),
                label: "Low-Voltage Cutoff (V)".into(),
                label_zh: Some("低压保护阈值(V)".into()),
                required: false,
                secret: false,
                description: "Battery voltage at which low_voltage killswitch activates".into(),
                description_zh: Some("触发低压关断保护的电池电压。".into()),
                default_value: Some(serde_json::Value::String("3.3".into())),
                enum_values: Some(vec![
                    EnumOption { value: "3.0".into(), label: "3.0 V/cell".into(), description: Some("Maximum range; risk of cell damage.".into()) },
                    EnumOption { value: "3.3".into(), label: "3.3 V/cell".into(), description: Some("Standard. Balanced.".into()) },
                    EnumOption { value: "3.5".into(), label: "3.5 V/cell".into(), description: Some("Conservative. Best for cell longevity.".into()) },
                ]),
                depends_on: None,
            },
        ],
        feedback_paths: vec![SignalPath {
            id: "rc_cmd_to_surfaces".into(),
            name: "RC command to surface deflection + throttle".into(),
            source: InputSurface::UartRx,
            transforms: vec![
                SignalPathStep {
                    order: 1,
                    node: TransformNode::CommandDispatch,
                    label: Some("CRSF channel decode".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: TransformNode::PidLoop,
                    label: Some("Attitude PID (roll/pitch/yaw)".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 3,
                    node: TransformNode::Mapping,
                    label: Some("Airframe-specific mix (standard/vtail/flying_wing/glider)".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 4,
                    node: TransformNode::SafetyInterlock,
                    label: Some("RX-loss watchdog → glide_trim; low-voltage → throttle cut".into()),
                    description: None,
                },
            ],
            sink: OutputSurface::ServoDrive,
            feedback: vec![SignalPathStep {
                order: 1,
                node: FeedbackSurface::PhysicalMotion,
                label: None,
                description: None,
            }],
            expected_user_result: "Stick input drives surfaces with attitude assist; signal loss → controlled glide rather than tumble".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec![],
            optional: vec!["ota".into()],
        },
        runtime_binding: brookesia_vehicle_binding(vec![
            "rshome_motor_control".into(),
            "rshome_imu".into(),
            "rshome_failsafe".into(),
        ]),
        external_contracts: vec![
            "CRSF (ExpressLRS) RC protocol".into(),
            "Servo PWM (1-2 ms)".into(),
        ],
        network_topology: NetworkTopology::default(),
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::ControlBoard),
        communication_chains: None,
        pin_assignments: Some(vehicle_control_pins()),
        family: Some(ImplementationFamily::Ardupilot),
        form_factor_families: Some(vec![
            FormFactorKind::FixedwingStandard,
            FormFactorKind::FixedwingVtail,
            FormFactorKind::FlyingWing,
            FormFactorKind::Glider,
        ]),
        control_uplink: Some(ControlUplinkKind::Crsf),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::CrsfTelemetry),
        sensor_tier_min: Some(SensorTierKind::Standard9ax),
        actuator_family: Some(ActuatorFamily::FixedwingSurfaces),
        power_rails: Some(PowerRailKind::TripleMotorServoLogic),
        failsafe: Some(FailsafeInfo {
            killswitch_source: vec![
                KillswitchSource::RcSwitch,
                KillswitchSource::RxLoss,
                KillswitchSource::LowVoltage,
            ],
            rx_loss_behavior: Some(RxLossBehavior::GlideTrim),
            // Fixed-wing exception to the "aircraft → 100 ms" rule. With
            // glide_trim, the airframe holds attitude and glides briefly on
            // RX loss; a 250 ms grace window lets the glide-trim routine
            // stabilise before relay_cutoff fires. Multirotors (quad/heli/
            // vtol) can't coast and use 100 ms. Formalized 2026-04-21 by
            // va-residuals Phase 1 T1.2.
            watchdog_ms: Some(250),
            emergency_stop_wiring: EmergencyStopWiring::RelayCutoff,
        }),
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Insufficient),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Insufficient),
        ])),
    });

    // ── 12c. ELRS CRSF quad (standard_fpv × control_board) ────────────────
    //
    // Four solutions that share CRSF UART plumbing with quad_stabilizer.
    // Values per `type-driven-ui/docs/vehicle-aircraft-control-dag.md`
    // §L4 family table (lines 380-412), §L5 chain table (lines 466-481),
    // §L5.5 failsafe table (lines 517-526), and §"Form Factor → Solution"
    // implications table (lines 218-240).

    // 12c.i — Brushed H-bridge (diff-drive)
    r.register(SolutionDefinition {
        id: "elrs_crsf_brushed_solution".into(),
        label: "ELRS CRSF → Brushed H-Bridge".into(),
        label_zh: Some("ELRS CRSF → 有刷 H 桥".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec![
            "esp32s3_wroom1".into(),
            "esp32s3_mini1".into(),
            "esp32c6_wroom1".into(),
            "esp32c6_mini1".into(),
        ],
        fixed_inputs: vec![InputSurface::UartRx, InputSurface::I2cSensor],
        fixed_outputs: vec![OutputSurface::MotorDrive, OutputSurface::StatusLed],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "crsf_rx_init".into(),
                label: "Initialize CRSF UART receiver at 420000 baud".into(),
                label_zh: Some("初始化 CRSF UART 接收机(420000 波特)".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "imu_calibrate".into(),
                label: "Calibrate IMU (skipped when imu_axis_tier = none)".into(),
                label_zh: Some("IMU 校准(imu_axis_tier = none 时跳过)".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "control_loop".into(),
                label: "CRSF decode → diff-drive mix → H-bridge PWM".into(),
                label_zh: Some("CRSF 解码 → 差速混控 → H 桥 PWM".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["crsf_rx_init".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "crsf_mixing".into(),
            label: "CRSF decode + mixing loop".into(),
            decisions: vec![
                "CRSF packet arrival event-drives the setpoint; mix runs on every packet".into(),
                "RC-switch kill cuts H-bridge enable line immediately (gpio_pulldown)".into(),
            ],
        },
        user_parameters: vec![
            vehicle_type_param(),
            imu_axis_tier_param(),
            imu_chip_param(),
            control_rate_hz_param(),
            failsafe_timeout_ms_param(),
        ],
        feedback_paths: vec![SignalPath {
            id: "crsf_to_motor".into(),
            name: "CRSF command to H-bridge".into(),
            source: InputSurface::UartRx,
            transforms: vec![
                SignalPathStep {
                    order: 1,
                    node: TransformNode::CommandDispatch,
                    label: Some("CRSF channel decode".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: TransformNode::SafetyInterlock,
                    label: Some("Rx-loss watchdog + kill switch".into()),
                    description: None,
                },
            ],
            sink: OutputSurface::MotorDrive,
            feedback: vec![SignalPathStep {
                order: 1,
                node: FeedbackSurface::PhysicalMotion,
                label: None,
                description: None,
            }],
            expected_user_result:
                "Stick input drives H-bridge PWM; signal loss stops motors within watchdog_ms"
                    .into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec![],
            optional: vec!["ota".into()],
        },
        runtime_binding: {
            let mut b =
                brookesia_vehicle_binding(vec!["rshome_motor_control".into(), "rshome_imu".into()]);
            b.board_assembly = Some("esp32s3_va_elrs_crsf_assembly".into());
            b
        },
        external_contracts: vec!["CRSF (ExpressLRS) RC protocol".into()],
        network_topology: NetworkTopology::default(),
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::ControlBoard),
        communication_chains: None,
        pin_assignments: Some(vehicle_control_pins()),
        family: Some(ImplementationFamily::Betaflight),
        form_factor_families: Some(vec![
            FormFactorKind::Wheeled2wdDiff,
            FormFactorKind::Wheeled4wdDiff,
            FormFactorKind::Wheeled6wd,
            FormFactorKind::TrackedSkidsteer,
        ]),
        control_uplink: Some(ControlUplinkKind::Crsf),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::CrsfTelemetry),
        sensor_tier_min: Some(SensorTierKind::Basic6ax),
        actuator_family: Some(ActuatorFamily::BrushedHbridge),
        power_rails: Some(PowerRailKind::DualLogicMotor),
        failsafe: Some(FailsafeInfo {
            killswitch_source: vec![KillswitchSource::RcSwitch, KillswitchSource::RxLoss],
            rx_loss_behavior: Some(RxLossBehavior::MotorCutoff),
            watchdog_ms: Some(250),
            emergency_stop_wiring: EmergencyStopWiring::GpioPulldown,
        }),
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Caveat),
        ])),
    });

    // 12c.ii — Brushless ESC PWM (Ackermann RC car)
    r.register(SolutionDefinition {
        id: "elrs_crsf_brushless_solution".into(),
        label: "ELRS CRSF → Brushless ESC".into(),
        label_zh: Some("ELRS CRSF → 无刷电调".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec![
            "esp32s3_wroom1".into(),
            "esp32s3_mini1".into(),
            "esp32c6_wroom1".into(),
            "esp32c6_mini1".into(),
        ],
        fixed_inputs: vec![InputSurface::UartRx, InputSurface::I2cSensor],
        fixed_outputs: vec![OutputSurface::MotorDrive, OutputSurface::StatusLed],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "crsf_rx_init".into(),
                label: "Initialize CRSF UART receiver at 420000 baud".into(),
                label_zh: Some("初始化 CRSF UART 接收机(420000 波特)".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "imu_calibrate".into(),
                label: "Calibrate IMU (skipped when imu_axis_tier = none)".into(),
                label_zh: Some("IMU 校准(imu_axis_tier = none 时跳过)".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "control_loop".into(),
                label: "CRSF decode → Ackermann mix → ESC PWM + steering servo".into(),
                label_zh: Some("CRSF 解码 → 阿克曼混控 → ESC PWM + 转向舵机".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["crsf_rx_init".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "crsf_mixing".into(),
            label: "CRSF decode + mixing loop".into(),
            decisions: vec![
                "ESC PWM updated every CRSF packet (~50 Hz typical)".into(),
                "Low-voltage kill cuts the ESC enable relay".into(),
            ],
        },
        user_parameters: vec![
            vehicle_type_param(),
            imu_axis_tier_param(),
            imu_chip_param(),
            control_rate_hz_param(),
            failsafe_timeout_ms_param(),
            // Phase 6 pilot extensions (va-residuals T6.2): CRSF UART pin,
            // ESC protocol variant, IMU I²C address. These replace what
            // users had to edit via raw JSON before.
            UserParameterDefinition {
                id: "crsf_uart_rx_gpio".into(),
                label: "CRSF UART RX GPIO".into(),
                label_zh: Some("CRSF UART 接收引脚".into()),
                required: true,
                secret: false,
                description: "GPIO pin the CRSF receiver's TX line connects to (ESP32 UART RX). Default matches vehicle_control_pins()."
                    .into(),
                description_zh: Some(
                    "CRSF 接收机 TX 连接到的 GPIO 引脚(ESP32 UART 端 RX),默认与 vehicle_control_pins() 一致。".into(),
                ),
                default_value: Some(serde_json::Value::String("5".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "5".into(),
                        label: "GPIO 5 (default)".into(),
                        description: Some(
                            "Matches the vehicle_control_pins() CRSF RX assignment.".into(),
                        ),
                    },
                    EnumOption {
                        value: "16".into(),
                        label: "GPIO 16".into(),
                        description: Some(
                            "Alternate on ESP32-S3 if GPIO 5 is already allocated.".into(),
                        ),
                    },
                    EnumOption {
                        value: "17".into(),
                        label: "GPIO 17".into(),
                        description: Some(
                            "Alternate on ESP32-S3 DevKitC boards with different flash pin layout.".into(),
                        ),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "esc_protocol".into(),
                label: "ESC Protocol".into(),
                label_zh: Some("电调协议".into()),
                required: true,
                secret: false,
                description: "Signal protocol to the brushless ESC. DShot variants require ESCs + RMT driver support."
                    .into(),
                description_zh: Some(
                    "发给无刷电调的信号协议。DShot 变体需要支持对应协议的电调 + RMT 驱动。".into(),
                ),
                default_value: Some(serde_json::Value::String("pwm".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "pwm".into(),
                        label: "Analog PWM (50-500 Hz)".into(),
                        description: Some(
                            "Classic servo-style ESC. Compatible with every BLHeli ESC; lowest rate.".into(),
                        ),
                    },
                    EnumOption {
                        value: "dshot300".into(),
                        label: "DShot300".into(),
                        description: Some(
                            "Digital. 300 kHz. Good default for racing/sport ESCs.".into(),
                        ),
                    },
                    EnumOption {
                        value: "dshot600".into(),
                        label: "DShot600".into(),
                        description: Some(
                            "Digital. 600 kHz. High-rate racing; requires BLHeli_32 or similar.".into(),
                        ),
                    },
                    EnumOption {
                        value: "bdshot".into(),
                        label: "Bidirectional DShot".into(),
                        description: Some(
                            "Digital + RPM telemetry. Enables RPM-based filters.".into(),
                        ),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "imu_i2c_addr".into(),
                label: "IMU I²C Address".into(),
                label_zh: Some("IMU I²C 地址".into()),
                required: false,
                secret: false,
                description: "I²C slave address of the IMU. Most modules use 0x68; 0x69 when AD0 pin is pulled high."
                    .into(),
                description_zh: Some(
                    "IMU 从机 I²C 地址。多数模块默认为 0x68;AD0 引脚拉高时为 0x69。".into(),
                ),
                default_value: Some(serde_json::Value::String("0x68".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "0x68".into(),
                        label: "0x68 (AD0=GND, default)".into(),
                        description: None,
                    },
                    EnumOption {
                        value: "0x69".into(),
                        label: "0x69 (AD0=VCC)".into(),
                        description: Some(
                            "Use when sharing I²C bus with another IMU or module that uses 0x68.".into(),
                        ),
                    },
                ]),
                depends_on: Some(ParameterDependency {
                    parameter_id: "imu_axis_tier".into(),
                    when_value: "6ax".into(), // unused when when_not_value is set
                    when_not_value: Some("none".into()),
                }),
            },
        ],
        feedback_paths: vec![SignalPath {
            id: "crsf_to_esc".into(),
            name: "CRSF command to ESC + servo".into(),
            source: InputSurface::UartRx,
            transforms: vec![
                SignalPathStep {
                    order: 1,
                    node: TransformNode::CommandDispatch,
                    label: Some("CRSF channel decode".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: TransformNode::SafetyInterlock,
                    label: Some("Rx-loss + low-voltage interlock".into()),
                    description: None,
                },
            ],
            sink: OutputSurface::MotorDrive,
            feedback: vec![SignalPathStep {
                order: 1,
                node: FeedbackSurface::PhysicalMotion,
                label: None,
                description: None,
            }],
            expected_user_result: "Throttle/steering commands drive ESC PWM + servo; relay cuts power on failsafe".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec![],
            optional: vec!["ota".into()],
        },
        runtime_binding: {
            let mut b = brookesia_vehicle_binding(vec![
                "rshome_motor_control".into(),
                "rshome_imu".into(),
            ]);
            b.board_assembly = Some("esp32s3_va_elrs_crsf_assembly".into());
            b
        },
        external_contracts: vec![
            "CRSF (ExpressLRS) RC protocol".into(),
            "ESC PWM".into(),
        ],
        network_topology: NetworkTopology::default(),
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::ControlBoard),
        communication_chains: None,
        pin_assignments: Some(vehicle_control_pins()),
        family: Some(ImplementationFamily::Betaflight),
        form_factor_families: Some(vec![
            FormFactorKind::Wheeled4wdAckermann,
            FormFactorKind::BigfootMonsterTruck,
            FormFactorKind::BigfootRockCrawler,
            FormFactorKind::AtvOffroad,
            FormFactorKind::DriftRallyRacer,
        ]),
        control_uplink: Some(ControlUplinkKind::Crsf),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::CrsfTelemetry),
        sensor_tier_min: Some(SensorTierKind::Basic6ax),
        actuator_family: Some(ActuatorFamily::BrushlessEscPwm),
        power_rails: Some(PowerRailKind::TripleMotorServoLogic),
        failsafe: Some(FailsafeInfo {
            killswitch_source: vec![
                KillswitchSource::RcSwitch,
                KillswitchSource::RxLoss,
                KillswitchSource::LowVoltage,
            ],
            rx_loss_behavior: Some(RxLossBehavior::MotorCutoff),
            watchdog_ms: Some(250),
            emergency_stop_wiring: EmergencyStopWiring::RelayCutoff,
        }),
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Caveat),
        ])),
    });

    // 12c.iii — DShot ESC (telemetry back via DShot bidirectional)
    r.register(SolutionDefinition {
        id: "elrs_crsf_dshot_solution".into(),
        label: "ELRS CRSF → DShot ESC".into(),
        label_zh: Some("ELRS CRSF → DShot 电调".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec![
            "esp32s3_wroom1".into(),
            "esp32s3_mini1".into(),
            "esp32c6_wroom1".into(),
            "esp32c6_mini1".into(),
        ],
        fixed_inputs: vec![InputSurface::UartRx, InputSurface::I2cSensor],
        fixed_outputs: vec![OutputSurface::MotorDrive, OutputSurface::StatusLed],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "crsf_rx_init".into(),
                label: "Initialize CRSF UART receiver at 420000 baud".into(),
                label_zh: Some("初始化 CRSF UART 接收机(420000 波特)".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "dshot_init".into(),
                label: "Initialize DShot600 output on RMT peripheral".into(),
                label_zh: Some("在 RMT 外设上初始化 DShot600 输出".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "control_loop".into(),
                label: "CRSF decode → DShot throttle commands + telemetry poll".into(),
                label_zh: Some("CRSF 解码 → DShot 油门指令 + 遥测轮询".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["crsf_rx_init".into(), "dshot_init".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "crsf_dshot".into(),
            label: "CRSF decode + DShot output loop".into(),
            decisions: vec![
                "DShot frame emitted every control period; telemetry polled between frames".into(),
                "Emergency stop uses DShot special command 21 (motor beep/stop) instead of GPIO"
                    .into(),
            ],
        },
        user_parameters: vec![
            vehicle_type_param(),
            imu_axis_tier_param(),
            imu_chip_param(),
            control_rate_hz_param(),
            failsafe_timeout_ms_param(),
        ],
        feedback_paths: vec![SignalPath {
            id: "crsf_to_dshot".into(),
            name: "CRSF command to DShot ESC".into(),
            source: InputSurface::UartRx,
            transforms: vec![
                SignalPathStep {
                    order: 1,
                    node: TransformNode::CommandDispatch,
                    label: Some("CRSF channel decode".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: TransformNode::SafetyInterlock,
                    label: Some("Rx-loss + low-voltage + DShot special-command stop".into()),
                    description: None,
                },
            ],
            sink: OutputSurface::MotorDrive,
            feedback: vec![SignalPathStep {
                order: 1,
                node: FeedbackSurface::PhysicalMotion,
                label: None,
                description: None,
            }],
            expected_user_result:
                "Throttle via DShot; telemetry (RPM/temp/voltage) returned from ESC via same line"
                    .into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec![],
            optional: vec!["ota".into()],
        },
        runtime_binding: {
            let mut b =
                brookesia_vehicle_binding(vec!["rshome_motor_control".into(), "rshome_imu".into()]);
            b.board_assembly = Some("esp32s3_va_elrs_crsf_assembly".into());
            b
        },
        external_contracts: vec![
            "CRSF (ExpressLRS) RC protocol".into(),
            "DShot ESC protocol".into(),
        ],
        network_topology: NetworkTopology::default(),
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::ControlBoard),
        communication_chains: None,
        pin_assignments: Some(vehicle_control_pins()),
        family: Some(ImplementationFamily::Betaflight),
        form_factor_families: Some(vec![
            FormFactorKind::Wheeled4wdAckermann,
            FormFactorKind::DriftRallyRacer,
        ]),
        control_uplink: Some(ControlUplinkKind::Crsf),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::DshotTelemetry),
        sensor_tier_min: Some(SensorTierKind::Basic6ax),
        actuator_family: Some(ActuatorFamily::BrushlessEscDshot),
        power_rails: Some(PowerRailKind::DualLogicMotor),
        failsafe: Some(FailsafeInfo {
            killswitch_source: vec![
                KillswitchSource::RcSwitch,
                KillswitchSource::RxLoss,
                KillswitchSource::LowVoltage,
            ],
            rx_loss_behavior: Some(RxLossBehavior::MotorCutoff),
            watchdog_ms: Some(250),
            emergency_stop_wiring: EmergencyStopWiring::EscDshotCmd,
        }),
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Caveat),
        ])),
    });

    // 12c.iv — MAVLink WiFi bridge (Ardurover-style)
    r.register(SolutionDefinition {
        id: "elrs_crsf_mavlink_solution".into(),
        label: "ELRS CRSF → MAVLink Bridge".into(),
        label_zh: Some("ELRS CRSF → MAVLink 桥接".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec![
            "esp32s3_wroom1".into(),
            "esp32s3_mini1".into(),
            "esp32c6_wroom1".into(),
            "esp32c6_mini1".into(),
        ],
        fixed_inputs: vec![
            InputSurface::UartRx,
            InputSurface::I2cSensor,
            InputSurface::WifiEvent,
        ],
        fixed_outputs: vec![OutputSurface::MotorDrive, OutputSurface::StatusLed],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "crsf_rx_init".into(),
                label: "Initialize CRSF UART receiver at 420000 baud".into(),
                label_zh: Some("初始化 CRSF UART 接收机(420000 波特)".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "imu_calibrate".into(),
                label: "Calibrate IMU".into(),
                label_zh: Some("IMU 校准".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "mavlink_wifi_start".into(),
                label: "Start MAVLink UDP listener on port 14550 (QGroundControl)".into(),
                label_zh: Some("在端口 14550 启动 MAVLink UDP 监听(QGroundControl)".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "control_loop".into(),
                label: "CRSF decode → Ackermann mix → ESC + servo; publish MAVLink state".into(),
                label_zh: Some("CRSF 解码 → 阿克曼混控 → ESC + 舵机;发布 MAVLink 状态".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["crsf_rx_init".into(), "mavlink_wifi_start".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "crsf_mavlink".into(),
            label: "CRSF mixing + MAVLink publish loop".into(),
            decisions: vec![
                "Control loop runs independently of MAVLink; telemetry publishes at 10 Hz".into(),
                "Rx-loss triggers RTH (ardurover-style) rather than motor cutoff".into(),
            ],
        },
        user_parameters: vec![
            vehicle_type_param(),
            imu_axis_tier_param(),
            imu_chip_param(),
            control_rate_hz_param(),
            failsafe_timeout_ms_param(),
        ],
        feedback_paths: vec![
            SignalPath {
                id: "crsf_to_esc".into(),
                name: "CRSF command to ESC + servo".into(),
                source: InputSurface::UartRx,
                transforms: vec![
                    SignalPathStep {
                        order: 1,
                        node: TransformNode::CommandDispatch,
                        label: Some("CRSF channel decode".into()),
                        description: None,
                    },
                    SignalPathStep {
                        order: 2,
                        node: TransformNode::SafetyInterlock,
                        label: Some("Rx-loss → RTH; low-voltage → relay cut".into()),
                        description: None,
                    },
                ],
                sink: OutputSurface::MotorDrive,
                feedback: vec![SignalPathStep {
                    order: 1,
                    node: FeedbackSurface::PhysicalMotion,
                    label: None,
                    description: None,
                }],
                expected_user_result:
                    "Stick input drives ESC + servo; RTH activates if radio link lost".into(),
            },
            SignalPath {
                id: "state_to_mavlink".into(),
                name: "Vehicle state to MAVLink (QGC)".into(),
                source: InputSurface::I2cSensor,
                transforms: vec![SignalPathStep {
                    order: 1,
                    node: TransformNode::CommandDispatch,
                    label: Some(
                        "MAVLink packet encode (HEARTBEAT, ATTITUDE, BATTERY_STATUS)".into(),
                    ),
                    description: None,
                }],
                sink: OutputSurface::NetworkApiState,
                feedback: vec![],
                expected_user_result: "QGroundControl sees vehicle state at 10 Hz over WiFi".into(),
            },
        ],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["wifi".into()],
            optional: vec!["ota".into()],
        },
        runtime_binding: {
            let mut b =
                brookesia_vehicle_binding(vec!["rshome_motor_control".into(), "rshome_imu".into()]);
            b.board_assembly = Some("esp32s3_va_elrs_crsf_assembly".into());
            b
        },
        external_contracts: vec![
            "CRSF (ExpressLRS) RC protocol".into(),
            "MAVLink 2 over UDP (QGroundControl)".into(),
        ],
        network_topology: NetworkTopology::Star,
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::ControlTelemetryBoard),
        communication_chains: None,
        pin_assignments: Some(vehicle_control_pins()),
        family: Some(ImplementationFamily::Ardupilot),
        form_factor_families: Some(vec![
            FormFactorKind::Wheeled4wdAckermann,
            FormFactorKind::AtvOffroad,
            FormFactorKind::BigfootMonsterTruck,
            FormFactorKind::AutonomousMower,
            FormFactorKind::SprayerSpot,
        ]),
        control_uplink: Some(ControlUplinkKind::Crsf),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::MavlinkWifi),
        // Bumped from basic_6ax: doc §L7 sensor-tier-floor table requires
        // standard_9ax + GPS for AutonomousMower/SprayerSpot families.
        // The wizard's `va_sensor_tier_floor` lint enforces this; the
        // MAVLink RTH failsafe also assumes a heading-stable IMU.
        sensor_tier_min: Some(SensorTierKind::Standard9ax),
        actuator_family: Some(ActuatorFamily::BrushlessEscPwm),
        power_rails: Some(PowerRailKind::TripleMotorServoLogic),
        failsafe: Some(FailsafeInfo {
            killswitch_source: vec![
                KillswitchSource::RcSwitch,
                KillswitchSource::RxLoss,
                KillswitchSource::LowVoltage,
            ],
            rx_loss_behavior: Some(RxLossBehavior::Rth),
            watchdog_ms: Some(250),
            emergency_stop_wiring: EmergencyStopWiring::RelayCutoff,
        }),
        topology_category: None,
        // elrs_crsf_mavlink's form_factor_families include autonomous_mower
        // and sprayer_spot alongside standard Ackermann cars. When a user
        // configures it for an agri variant, the Gps sensor becomes
        // required. Phase 9 polish.
        required_sensors: vec![SensorRequirement::Gps],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Caveat),
        ])),
    });

    // ── 12d. TX side trio (remote_control_tx + smartphone_gateway) ────────
    //
    // Closes the remote_control_tx + smartphone_gateway rows of the
    // §L4 grid now that the CRSF RX quadrant is done. None of these three
    // carry actuators, so `failsafe`, `actuator_family`, `sensor_tier_min`,
    // and `form_factor_families` stay `None`. See the doc's
    // §L5 chain table (lines 474-476, 494) and §L4 family table
    // (lines 389-391) for per-solution values.

    // 12d.i — ESP-NOW broadcast TX (handheld)
    r.register(SolutionDefinition {
        id: "esp_now_tx_solution".into(),
        label: "ESP-NOW TX (handheld)".into(),
        label_zh: Some("ESP-NOW 遥控器".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec![
            "esp32s3_wroom1".into(),
            "esp32s3_mini1".into(),
            "esp32c6_wroom1".into(),
            "esp32c6_mini1".into(),
        ],
        fixed_inputs: vec![InputSurface::AdcVoltage, InputSurface::ButtonGpio],
        fixed_outputs: vec![OutputSurface::EspNowData, OutputSurface::StatusLed],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "joystick_adc_init".into(),
                label: "Initialize joystick ADC channels + button GPIOs".into(),
                label_zh: Some("初始化摇杆 ADC 与按键 GPIO".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "esp_now_broadcast".into(),
                label: "Broadcast channel packet at control_rate_hz".into(),
                label_zh: Some("按 control_rate_hz 广播通道包".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["joystick_adc_init".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "tx_broadcast".into(),
            label: "Joystick sample + broadcast loop".into(),
            decisions: vec![
                "Sample ADC every control period; broadcast unconditionally".into(),
                "No retransmit; receivers handle their own failsafe on link loss".into(),
            ],
        },
        user_parameters: vec![control_rate_hz_param()],
        feedback_paths: vec![SignalPath {
            id: "stick_to_esp_now".into(),
            name: "Stick input to ESP-NOW broadcast".into(),
            source: InputSurface::AdcVoltage,
            transforms: vec![SignalPathStep {
                order: 1,
                node: TransformNode::CommandDispatch,
                label: Some("Pack channels → ESP-NOW frame".into()),
                description: None,
            }],
            sink: OutputSurface::EspNowData,
            feedback: vec![],
            expected_user_result:
                "Joystick and switches broadcast to the ESP-NOW mesh at control_rate_hz".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec![],
            optional: vec!["ota".into()],
        },
        runtime_binding: brookesia_vehicle_binding(vec![]),
        external_contracts: vec!["ESP-NOW broadcast protocol".into()],
        network_topology: NetworkTopology::PointToPoint,
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::RemoteControlTx),
        communication_chains: None,
        pin_assignments: Some(remote_control_tx_pins()),
        family: Some(ImplementationFamily::Custom),
        form_factor_families: None,
        control_uplink: Some(ControlUplinkKind::EspNow),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::None),
        sensor_tier_min: None,
        actuator_family: None,
        power_rails: Some(PowerRailKind::SingleLogicOnly),
        failsafe: None,
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Caveat),
        ])),
    });

    // 12d.ii — ELRS / CRSF TX firmware (handheld)
    r.register(SolutionDefinition {
        id: "elrs_tx_solution".into(),
        label: "ELRS CRSF TX (handheld)".into(),
        label_zh: Some("ELRS CRSF 遥控器".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec![
            "esp32s3_wroom1".into(),
            "esp32s3_mini1".into(),
            "esp32c6_wroom1".into(),
            "esp32c6_mini1".into(),
        ],
        fixed_inputs: vec![
            InputSurface::AdcVoltage,
            InputSurface::ButtonGpio,
            InputSurface::UartRx,
        ],
        fixed_outputs: vec![
            OutputSurface::UartTx,
            OutputSurface::StatusLed,
        ],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "joystick_adc_init".into(),
                label: "Initialize joystick ADC + switch GPIOs".into(),
                label_zh: Some("初始化摇杆 ADC 与开关 GPIO".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "crsf_tx_init".into(),
                label: "Initialize CRSF UART TX to radio module at 420000 baud".into(),
                label_zh: Some("向射频模块以 420000 波特启动 CRSF UART TX".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "crsf_frame_loop".into(),
                label: "Pack sticks + switches into CRSF frame; emit at control_rate_hz; decode RX telemetry".into(),
                label_zh: Some("将摇杆/开关打包成 CRSF 帧,按 control_rate_hz 发送;解析下行遥测".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["joystick_adc_init".into(), "crsf_tx_init".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "elrs_tx_loop".into(),
            label: "CRSF frame emission + telemetry decode".into(),
            decisions: vec![
                "Channel frame sent at fixed cadence (250–500 Hz typical)".into(),
                "RX telemetry (RSSI / LQ / battery) displayed on status UI".into(),
            ],
        },
        user_parameters: vec![control_rate_hz_param()],
        feedback_paths: vec![SignalPath {
            id: "stick_to_crsf".into(),
            name: "Stick input to CRSF radio".into(),
            source: InputSurface::AdcVoltage,
            transforms: vec![SignalPathStep {
                order: 1,
                node: TransformNode::CommandDispatch,
                label: Some("Pack channels → CRSF frame".into()),
                description: None,
            }],
            sink: OutputSurface::UartTx,
            feedback: vec![SignalPathStep {
                order: 1,
                node: FeedbackSurface::LedIndicator,
                label: Some("RSSI / LQ status LED".into()),
                description: None,
            }],
            expected_user_result: "Stick motion drives CRSF frames out of the UART to the RF module at up to 500 Hz".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec![],
            optional: vec!["ota".into()],
        },
        runtime_binding: brookesia_vehicle_binding(vec![]),
        external_contracts: vec![
            "CRSF (ExpressLRS) RC protocol".into(),
            "ExpressLRS TX firmware lineage".into(),
        ],
        network_topology: NetworkTopology::None,
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::RemoteControlTx),
        communication_chains: None,
        pin_assignments: Some(remote_control_tx_pins()),
        family: Some(ImplementationFamily::Betaflight),
        form_factor_families: None,
        control_uplink: Some(ControlUplinkKind::Crsf),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::CrsfTelemetry),
        sensor_tier_min: None,
        actuator_family: None,
        power_rails: Some(PowerRailKind::SingleLogicOnly),
        failsafe: None,
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Caveat),
        ])),
    });

    // 12d.iii — Phone ↔ Vehicle bridge (smartphone_gateway)
    //
    // Parameterized over `phone_side_link` (ble_gatt | usb_cdc) and
    // `vehicle_side_protocol` (esp_now | wifi_mesh | wifi_80211lr | ble_mesh).
    // `control_uplink` is set to `BleGatt` as the default surface value; the
    // wizard's `phone_side_link` parameter toggles between BLE and USB at
    // runtime without changing this annotation. See doc lines 319-320.
    r.register(SolutionDefinition {
        id: "phone_bridge_solution".into(),
        label: "Phone ↔ Vehicle Bridge".into(),
        label_zh: Some("手机 — 车辆中继".into()),
        kind: SolutionKind::ConnectivityBridge,
        supported_modules: vec![
            "esp32s3_wroom1".into(),
            "esp32s3_mini1".into(),
            "esp32c6_wroom1".into(),
            "esp32c6_mini1".into(),
        ],
        fixed_inputs: vec![
            InputSurface::BleEvent,
            InputSurface::UsbCdcCommand,
            InputSurface::EspNowData,
            InputSurface::WifiEvent,
        ],
        fixed_outputs: vec![
            OutputSurface::EspNowData,
            OutputSurface::WifiPacket,
            OutputSurface::BlePacket,
            OutputSurface::StatusLed,
        ],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "phone_link_init".into(),
                label: "Initialize the phone-side link (BLE GATT service or USB CDC)".into(),
                label_zh: Some("初始化手机侧链路(BLE GATT 服务或 USB CDC)".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "vehicle_link_init".into(),
                label: "Initialize vehicle-side protocol (ESP-NOW / Wi-Fi Mesh / 802.11 LR / BLE Mesh)".into(),
                label_zh: Some("初始化车辆侧协议(ESP-NOW / Wi-Fi Mesh / 802.11 LR / BLE Mesh)".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "bidirectional_relay".into(),
                label: "Forward phone → vehicle commands and vehicle → phone telemetry".into(),
                label_zh: Some("双向转发:手机→车辆指令和车辆→手机遥测".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["phone_link_init".into(), "vehicle_link_init".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "relay_loop".into(),
            label: "Bidirectional relay between phone and vehicle".into(),
            decisions: vec![
                "No state buffering; frames forwarded as they arrive".into(),
                "Gateway is stateless — if phone-side or vehicle-side link drops, the other side times out via its own failsafe".into(),
            ],
        },
        user_parameters: vec![
            phone_side_link_param(),
            vehicle_side_protocol_param(),
            control_rate_hz_param(),
        ],
        feedback_paths: vec![
            SignalPath {
                id: "phone_to_vehicle".into(),
                name: "Phone command to vehicle protocol".into(),
                source: InputSurface::BleEvent,
                transforms: vec![SignalPathStep {
                    order: 1,
                    node: TransformNode::CommandDispatch,
                    label: Some("Parse phone command → emit on vehicle-side protocol".into()),
                    description: None,
                }],
                sink: OutputSurface::EspNowData,
                feedback: vec![],
                expected_user_result: "Phone commands reach the vehicle fleet with <30 ms added latency".into(),
            },
            SignalPath {
                id: "vehicle_to_phone".into(),
                name: "Vehicle telemetry to phone".into(),
                source: InputSurface::EspNowData,
                transforms: vec![SignalPathStep {
                    order: 1,
                    node: TransformNode::CommandDispatch,
                    label: Some("Parse vehicle telemetry → push on phone link".into()),
                    description: None,
                }],
                sink: OutputSurface::BlePacket,
                feedback: vec![],
                expected_user_result: "Vehicle state surfaces in the phone app over the active phone-side link".into(),
            },
        ],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["wifi".into()],
            optional: vec!["ota".into()],
        },
        runtime_binding: brookesia_vehicle_binding(vec![]),
        external_contracts: vec![
            "BLE GATT service (phone app)".into(),
            "USB CDC (tethered phone)".into(),
            "ESP-NOW / Wi-Fi Mesh / 802.11 LR / BLE Mesh (vehicle side)".into(),
        ],
        network_topology: NetworkTopology::Mesh,
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::SmartphoneGateway),
        communication_chains: None,
        pin_assignments: Some(remote_control_tx_pins()),
        family: Some(ImplementationFamily::Custom),
        form_factor_families: None,
        control_uplink: Some(ControlUplinkKind::BleGatt),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::CustomUart),
        sensor_tier_min: None,
        actuator_family: None,
        power_rails: Some(PowerRailKind::SingleLogicOnly),
        failsafe: None,
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Caveat),
        ])),
    });

    // ── 12d.v. Phone ↔ CRSF Bridge (standard_fpv × smartphone_gateway) ────
    //
    // Fills the single remaining ⚠ cell in the topology×role matrix per
    // va-residuals ADR-03. Unlike `phone_bridge_solution` (which relays to
    // ESP-NOW / Wi-Fi Mesh / 802.11 LR / BLE Mesh), this variant speaks
    // CRSF on the vehicle side — typical pattern: phone → BLE → ESP32
    // → UART → ELRS-compatible RF module → vehicle's CRSF RX. Gives a
    // user with a phone app (but no dedicated RC TX) an ExpressLRS-class
    // control link, which is the whole point of the standard_fpv topology.
    // Gateway is stateless; vehicle-side failsafe owned by the CRSF RX
    // and the downstream control_board's own policy.
    r.register(SolutionDefinition {
        id: "phone_bridge_crsf_solution".into(),
        label: "Phone ↔ CRSF Bridge (ELRS)".into(),
        label_zh: Some("手机 — CRSF 桥(ELRS)".into()),
        kind: SolutionKind::ConnectivityBridge,
        supported_modules: vec![
            "esp32s3_wroom1".into(),
            "esp32s3_mini1".into(),
            "esp32c6_wroom1".into(),
            "esp32c6_mini1".into(),
        ],
        fixed_inputs: vec![
            InputSurface::BleEvent,
            InputSurface::UsbCdcCommand,
            InputSurface::UartRx,
        ],
        fixed_outputs: vec![
            OutputSurface::UartTx,
            OutputSurface::BlePacket,
            OutputSurface::StatusLed,
        ],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "phone_link_init".into(),
                label: "Initialize phone-side link (BLE GATT or USB CDC)".into(),
                label_zh: Some("初始化手机侧链路(BLE GATT 或 USB CDC)".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "crsf_uart_init".into(),
                label: "Initialize CRSF UART TX at 420000 baud to the ELRS module".into(),
                label_zh: Some("初始化 CRSF UART TX(420000 波特)到 ELRS 模块".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "phone_to_crsf_relay".into(),
                label: "Phone stick input → CRSF channel packet every packet_rate_ms".into(),
                label_zh: Some("手机摇杆输入 → 按 packet_rate_ms 发送 CRSF 通道帧".into()),
                description: Some(
                    "Phone-app input (BLE) maps to 16-channel CRSF frames; ESP32 transmits on the \
                     UART TX line to a plugged-in ExpressLRS module, which drives the RF link. \
                     Throttle goes to failsafe_throttle on phone-link loss."
                        .into(),
                ),
                description_zh: Some(
                    "手机应用输入(BLE)映射为 16 通道 CRSF 帧;ESP32 通过 UART TX 发送给外接 ExpressLRS 模块,由模块负责射频。手机链路丢失时油门回落到 failsafe_throttle。".into(),
                ),
                depends_on: vec!["phone_link_init".into(), "crsf_uart_init".into()],
                ..Default::default()
            },
            OrchestrationStep {
                id: "crsf_telemetry_relay".into(),
                label: "CRSF telemetry (battery / RSSI / LQ) → phone app".into(),
                label_zh: Some("CRSF 遥测(电池 / RSSI / LQ)→ 手机应用".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["crsf_uart_init".into(), "phone_link_init".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "crsf_relay_loop".into(),
            label: "Phone ↔ CRSF packet-rate relay".into(),
            decisions: vec![
                "CRSF channel packets emitted at packet_rate_hz (default 500 Hz; ELRS-compatible)".into(),
                "Phone-link loss → gateway stops emitting; ELRS module drives its own failsafe".into(),
                "CRSF RX telemetry (battery, RSSI, LQ) forwarded to phone opportunistically".into(),
            ],
        },
        user_parameters: vec![
            phone_side_link_param(),
            UserParameterDefinition {
                id: "crsf_uart_tx_gpio".into(),
                label: "CRSF UART TX GPIO".into(),
                label_zh: Some("CRSF UART 发送引脚".into()),
                required: true,
                secret: false,
                description: "ESP32 GPIO wired to the ELRS module's CRSF input".into(),
                description_zh: Some("ESP32 输出到 ELRS 模块 CRSF 输入的 GPIO。".into()),
                default_value: Some(serde_json::Value::String("17".into())),
                enum_values: Some(vec![
                    EnumOption { value: "17".into(), label: "GPIO 17 (default)".into(), description: Some("ESP32-S3 strapping-safe default.".into()) },
                    EnumOption { value: "43".into(), label: "GPIO 43".into(), description: Some("ESP32-S3 U0TXD alternate (frees GPIO 17).".into()) },
                    EnumOption { value: "6".into(), label: "GPIO 6".into(), description: Some("ESP32-C6 default.".into()) },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "packet_rate_hz".into(),
                label: "CRSF Packet Rate (Hz)".into(),
                label_zh: Some("CRSF 包速率 (Hz)".into()),
                required: true,
                secret: false,
                description: "ELRS-compatible CRSF frame rate. 250/500 Hz for regular use; 1000 Hz for race-grade radios only."
                    .into(),
                description_zh: Some(
                    "ELRS 兼容的 CRSF 帧率。常规用 250/500 Hz;1000 Hz 仅适合竞速级遥控。".into(),
                ),
                default_value: Some(serde_json::Value::String("500".into())),
                enum_values: Some(vec![
                    EnumOption { value: "50".into(), label: "50 Hz".into(), description: Some("Low-rate / long-range; fits most cars + planes.".into()) },
                    EnumOption { value: "250".into(), label: "250 Hz".into(), description: Some("Balanced racing-drone default.".into()) },
                    EnumOption { value: "500".into(), label: "500 Hz (default)".into(), description: Some("ExpressLRS default for multirotors.".into()) },
                    EnumOption { value: "1000".into(), label: "1000 Hz".into(), description: Some("Race-grade; requires supported ELRS hardware.".into()) },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "failsafe_throttle".into(),
                label: "Failsafe Throttle (µs)".into(),
                label_zh: Some("失控保护油门 (µs)".into()),
                required: false,
                secret: false,
                description: "CRSF channel value emitted on phone-link loss while the gateway stays up. 988 = throttle cut; 1500 = center."
                    .into(),
                description_zh: Some(
                    "手机链路丢失但网关仍在运行时使用的 CRSF 通道值。988 = 关油门;1500 = 中位。".into(),
                ),
                default_value: Some(serde_json::Value::String("988".into())),
                enum_values: Some(vec![
                    EnumOption { value: "988".into(), label: "988 µs — throttle cut".into(), description: Some("Default. Safest for aircraft and ground vehicles.".into()) },
                    EnumOption { value: "1500".into(), label: "1500 µs — center".into(), description: Some("Hold last steering, cruise throttle. Use for boats only.".into()) },
                ]),
                depends_on: None,
            },
        ],
        feedback_paths: vec![SignalPath {
            id: "phone_to_crsf".into(),
            name: "Phone input to CRSF RF".into(),
            source: InputSurface::BleEvent,
            transforms: vec![
                SignalPathStep {
                    order: 1,
                    node: TransformNode::CommandDispatch,
                    label: Some("BLE command parse → CRSF channel map".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: TransformNode::Mapping,
                    label: Some("16-channel CRSF frame emit @ packet_rate_hz".into()),
                    description: None,
                },
            ],
            sink: OutputSurface::UartTx,
            feedback: vec![SignalPathStep {
                order: 1,
                node: FeedbackSurface::PhysicalMotion,
                label: Some("Vehicle RX drives actuators → motion observable".into()),
                description: None,
            }],
            expected_user_result:
                "Phone app stick inputs reach the ELRS-linked vehicle with <10 ms added latency over the RF link; CRSF telemetry surfaces back in the app."
                    .into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec![],
            optional: vec!["ota".into()],
        },
        runtime_binding: brookesia_vehicle_binding(vec![]),
        external_contracts: vec![
            "BLE GATT service (phone app)".into(),
            "USB CDC (tethered phone)".into(),
            "CRSF (ExpressLRS) UART — external ELRS TX module".into(),
        ],
        network_topology: NetworkTopology::PointToPoint,
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::SmartphoneGateway),
        communication_chains: None,
        pin_assignments: Some(remote_control_tx_pins()),
        family: Some(ImplementationFamily::Betaflight),
        form_factor_families: None,
        control_uplink: Some(ControlUplinkKind::Crsf),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::CrsfTelemetry),
        sensor_tier_min: None,
        actuator_family: None,
        power_rails: Some(PowerRailKind::SingleLogicOnly),
        failsafe: None,
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Caveat),
        ])),
    });

    // ── 12e. Research / Hybrid SBC quad ───────────────────────────────────
    //
    // Closes the research_hybrid column of the §L4 grid (doc lines 355-360).
    // All four pair an ESP32 with a companion SBC (Raspberry Pi / Jetson) —
    // the ESP32 side is either a thin WiFi bridge or a MAVLink passthrough.
    // `mcu_sbc_bridge` is the only actuator-bearing solution in this set
    // (ESP32 drives motors while SBC runs the planner); the others are pure
    // relays.

    // 12e.i — MCU ↔ SBC bridge (safety MCU, planner on SBC)
    r.register(SolutionDefinition {
        id: "mcu_sbc_bridge_solution".into(),
        label: "MCU ↔ SBC bridge".into(),
        label_zh: Some("MCU ↔ SBC 桥接".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec![
            "esp32s3_wroom1".into(),
            "esp32s3_mini1".into(),
            "esp32c6_wroom1".into(),
            "esp32c6_mini1".into(),
        ],
        fixed_inputs: vec![InputSurface::UartRx, InputSurface::I2cSensor],
        fixed_outputs: vec![OutputSurface::MotorDrive, OutputSurface::StatusLed],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "crsf_rx_init".into(),
                label: "Initialize CRSF UART RX at 420000 baud".into(),
                label_zh: Some("初始化 CRSF UART RX(420000 波特)".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "sbc_uart_bridge".into(),
                label: "Forward MAVLink between SBC (Pi/Jetson) and motor driver".into(),
                label_zh: Some("在 SBC 与电机驱动器之间转发 MAVLink".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "safety_loop".into(),
                label: "MCU-retained safety: rx_loss + sbc_heartbeat_loss → motor cutoff".into(),
                label_zh: Some("MCU 侧安全层:RX 丢失 + SBC 心跳超时 → 切断电机".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["crsf_rx_init".into(), "sbc_uart_bridge".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "sbc_bridge_loop".into(),
            label: "Safety-gated SBC bridge".into(),
            decisions: vec![
                "SBC heartbeat arrives every 100ms; missing >500ms → motor cutoff".into(),
                "Control commands pass through SBC; MCU retains final kill authority".into(),
            ],
        },
        user_parameters: vec![
            vehicle_type_param(),
            imu_axis_tier_param(),
            imu_chip_param(),
            control_rate_hz_param(),
            failsafe_timeout_ms_param(),
        ],
        feedback_paths: vec![SignalPath {
            id: "sbc_cmd_to_motor".into(),
            name: "SBC command to motor".into(),
            source: InputSurface::UartRx,
            transforms: vec![
                SignalPathStep {
                    order: 1,
                    node: TransformNode::CommandDispatch,
                    label: Some("MAVLink parse".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: TransformNode::SafetyInterlock,
                    label: Some("SBC heartbeat watchdog".into()),
                    description: None,
                },
            ],
            sink: OutputSurface::MotorDrive,
            feedback: vec![SignalPathStep {
                order: 1,
                node: FeedbackSurface::PhysicalMotion,
                label: None,
                description: None,
            }],
            expected_user_result:
                "SBC drives motors through the MCU; SBC failure cuts motors in 500 ms".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec![],
            optional: vec!["ota".into()],
        },
        runtime_binding: {
            let mut b =
                brookesia_vehicle_binding(vec!["rshome_motor_control".into(), "rshome_imu".into()]);
            b.board_assembly = Some("esp32s3_va_sbc_bridge_assembly".into());
            b
        },
        external_contracts: vec![
            "CRSF (ExpressLRS) RC protocol".into(),
            "MAVLink 2 over UART (SBC)".into(),
        ],
        network_topology: NetworkTopology::None,
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::ControlBoard),
        communication_chains: None,
        pin_assignments: Some(vehicle_control_pins()),
        family: Some(ImplementationFamily::Custom),
        form_factor_families: None,
        control_uplink: Some(ControlUplinkKind::Crsf),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::MavlinkUart),
        sensor_tier_min: Some(SensorTierKind::Basic6ax),
        actuator_family: Some(ActuatorFamily::BrushlessEscPwm),
        power_rails: Some(PowerRailKind::TripleMotorServoLogic),
        failsafe: Some(FailsafeInfo {
            killswitch_source: vec![KillswitchSource::RxLoss, KillswitchSource::SbcHeartbeatLoss],
            rx_loss_behavior: Some(RxLossBehavior::MotorCutoff),
            watchdog_ms: Some(500),
            emergency_stop_wiring: EmergencyStopWiring::GpioPulldown,
        }),
        topology_category: None,
        required_sensors: vec![],
        companion_link: Some(CompanionLinkKind::Uart),
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Caveat),
        ])),
    });

    // 12e.ii — Video board with SBC companion (WebRTC out)
    r.register(SolutionDefinition {
        id: "video_board_sbc_companion_solution".into(),
        label: "Video board + SBC (WebRTC)".into(),
        label_zh: Some("视频板 + SBC(WebRTC)".into()),
        kind: SolutionKind::ConnectivityBridge,
        supported_modules: vec!["esp32s3_wroom1".into(), "esp32s3_mini1".into()],
        fixed_inputs: vec![InputSurface::CameraFrame, InputSurface::UartRx],
        fixed_outputs: vec![OutputSurface::UartTx, OutputSurface::StatusLed],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "camera_init".into(),
                label: "Initialize camera (LCD_CAM / DVP interface)".into(),
                label_zh: Some("初始化摄像头(LCD_CAM / DVP 接口)".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "sbc_uart_stream".into(),
                label: "Stream raw frames to SBC for WebRTC encode".into(),
                label_zh: Some("将原始帧流送到 SBC 做 WebRTC 编码".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["camera_init".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "camera_to_sbc".into(),
            label: "Frame capture + UART DMA to SBC".into(),
            decisions: vec![
                "Frames streamed raw; H.264/H.265 encode runs on the SBC".into(),
                "No MCU-side buffer — back-pressure handled by SBC".into(),
            ],
        },
        // PRD Task 1.1 closes master design §10.1.
        user_parameters: vec![
            wifi_ssid_param(),
            wifi_password_param(),
            mavlink_udp_port_param(),
            telemetry_rate_hz_param(),
        ],
        feedback_paths: vec![SignalPath {
            id: "camera_to_webrtc".into(),
            name: "Camera frame to WebRTC".into(),
            source: InputSurface::CameraFrame,
            transforms: vec![SignalPathStep {
                order: 1,
                node: TransformNode::CommandDispatch,
                label: Some("UART DMA to SBC encoder".into()),
                description: None,
            }],
            sink: OutputSurface::UartTx,
            feedback: vec![],
            expected_user_result:
                "SBC publishes WebRTC stream of the camera feed at <200 ms latency".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec![],
            optional: vec!["ota".into()],
        },
        runtime_binding: brookesia_vehicle_binding(vec!["rshome_camera".into()]),
        external_contracts: vec![
            "UART stream to SBC encoder".into(),
            "WebRTC (served from SBC)".into(),
        ],
        network_topology: NetworkTopology::None,
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::VideoBoard),
        communication_chains: None,
        pin_assignments: Some(video_board_pins()),
        family: Some(ImplementationFamily::Custom),
        form_factor_families: None,
        control_uplink: Some(ControlUplinkKind::None),
        video_downlink: Some(VideoDownlinkKind::WebrtcSbc),
        telemetry: Some(TelemetryKind::MavlinkUart),
        sensor_tier_min: None,
        actuator_family: None,
        power_rails: Some(PowerRailKind::SingleLogicOnly),
        failsafe: None,
        topology_category: None,
        required_sensors: vec![],
        companion_link: Some(CompanionLinkKind::Uart),
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Insufficient),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Insufficient),
        ])),
    });

    // 12e.iii — MAVLink groundstation (QGroundControl compatible)
    r.register(SolutionDefinition {
        id: "mavlink_groundstation_solution".into(),
        label: "MAVLink Groundstation (QGC)".into(),
        label_zh: Some("MAVLink 地面站 (QGC)".into()),
        kind: SolutionKind::ConnectivityBridge,
        supported_modules: vec![
            "esp32s3_wroom1".into(),
            "esp32s3_mini1".into(),
            "esp32c6_wroom1".into(),
            "esp32c6_mini1".into(),
        ],
        fixed_inputs: vec![InputSurface::WifiEvent, InputSurface::UartRx],
        fixed_outputs: vec![OutputSurface::WifiPacket, OutputSurface::StatusLed],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "wifi_connect".into(),
                label: "Connect to home network".into(),
                label_zh: Some("连接家庭 Wi-Fi".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "mavlink_udp_relay".into(),
                label: "UDP ↔ UART relay on port 14550 (QGroundControl)".into(),
                label_zh: Some("在端口 14550 双向 UDP ↔ UART 转发 (QGroundControl)".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["wifi_connect".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "mavlink_relay".into(),
            label: "Bidirectional MAVLink relay".into(),
            decisions: vec![
                "Frames forwarded with no filtering; QGC sees the fleet directly".into(),
            ],
        },
        // PRD Task 1.1 closes master design §10.1.
        user_parameters: vec![
            wifi_ssid_param(),
            wifi_password_param(),
            mavlink_udp_port_param(),
            telemetry_rate_hz_param(),
        ],
        feedback_paths: vec![SignalPath {
            id: "qgc_to_vehicle".into(),
            name: "QGC command to vehicle".into(),
            source: InputSurface::WifiEvent,
            transforms: vec![SignalPathStep {
                order: 1,
                node: TransformNode::CommandDispatch,
                label: Some("UDP 14550 → UART".into()),
                description: None,
            }],
            sink: OutputSurface::WifiPacket,
            feedback: vec![],
            expected_user_result:
                "QGroundControl running on a laptop controls the vehicle via the ESP32 WiFi bridge"
                    .into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["wifi".into()],
            optional: vec!["ota".into()],
        },
        runtime_binding: brookesia_vehicle_binding(vec![]),
        external_contracts: vec!["MAVLink 2 over UDP (QGroundControl)".into()],
        network_topology: NetworkTopology::Star,
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::SmartphoneGateway),
        communication_chains: None,
        pin_assignments: None,
        family: Some(ImplementationFamily::Ardupilot),
        form_factor_families: None,
        control_uplink: Some(ControlUplinkKind::WifiMavlink),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::MavlinkWifi),
        sensor_tier_min: None,
        actuator_family: None,
        power_rails: Some(PowerRailKind::SingleLogicOnly),
        failsafe: None,
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Caveat),
        ])),
    });

    // 12e.iv — Web UI groundstation (browser WebRTC via SBC)
    r.register(SolutionDefinition {
        id: "web_ui_groundstation_solution".into(),
        label: "Web UI Groundstation (WebRTC)".into(),
        label_zh: Some("Web UI 地面站 (WebRTC)".into()),
        kind: SolutionKind::ConnectivityBridge,
        supported_modules: vec![
            "esp32s3_wroom1".into(),
            "esp32s3_mini1".into(),
            "esp32c6_wroom1".into(),
            "esp32c6_mini1".into(),
        ],
        fixed_inputs: vec![InputSurface::WifiEvent, InputSurface::UartRx],
        fixed_outputs: vec![
            OutputSurface::WifiPacket,
            OutputSurface::NetworkApiState,
            OutputSurface::StatusLed,
        ],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "wifi_connect".into(),
                label: "Connect to home network".into(),
                label_zh: Some("连接家庭 Wi-Fi".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "signaling_relay".into(),
                label: "WebRTC signaling relay to SBC-hosted UI".into(),
                label_zh: Some("WebRTC 信令转发到 SBC 承载的 UI".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["wifi_connect".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "webrtc_signaling".into(),
            label: "WebRTC signaling + MAVLink side-channel".into(),
            decisions: vec!["Media stream served from SBC; ESP32 only handles signaling + MAVLink".into()],
        },
        // PRD Task 1.1 closes master design §10.1. Web UI adds the
        // webrtc_signaling_url since it must reach the SBC's signaling
        // endpoint (the other two GCS solutions don't need it).
        user_parameters: vec![
            wifi_ssid_param(),
            wifi_password_param(),
            mavlink_udp_port_param(),
            telemetry_rate_hz_param(),
            webrtc_signaling_url_param(),
        ],
        feedback_paths: vec![SignalPath {
            id: "browser_to_vehicle".into(),
            name: "Browser command to vehicle".into(),
            source: InputSurface::WifiEvent,
            transforms: vec![SignalPathStep {
                order: 1,
                node: TransformNode::CommandDispatch,
                label: Some("WebRTC data channel → MAVLink over UART".into()),
                description: None,
            }],
            sink: OutputSurface::NetworkApiState,
            feedback: vec![],
            expected_user_result: "Browser pilots the vehicle with a live video preview, signaling via ESP32 and media via SBC".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["wifi".into()],
            optional: vec!["ota".into()],
        },
        runtime_binding: brookesia_vehicle_binding(vec![]),
        external_contracts: vec![
            "WebRTC signaling (browser ↔ SBC)".into(),
            "MAVLink 2 over UART (to vehicle)".into(),
        ],
        network_topology: NetworkTopology::Star,
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::SmartphoneGateway),
        communication_chains: None,
        pin_assignments: None,
        family: Some(ImplementationFamily::Custom),
        form_factor_families: None,
        control_uplink: Some(ControlUplinkKind::WifiMavlink),
        video_downlink: Some(VideoDownlinkKind::WebrtcSbc),
        telemetry: Some(TelemetryKind::MavlinkWifi),
        sensor_tier_min: None,
        actuator_family: None,
        power_rails: Some(PowerRailKind::SingleLogicOnly),
        failsafe: None,
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Caveat),
        ])),
    });

    // ── 12f. Mecanum / Omni Holonomic Mix ─────────────────────────────────
    //
    // Per `type-driven-ui/docs/vehicle-aircraft-control-dag.md` §L1
    // (mecanum/omni form factors, standard_9ax floor for yaw lock),
    // §L4 family (line 402, custom — no upstream lineage),
    // §L5 chain (line 486: esp_now / none / custom_uart),
    // and the §"Form Factor → Solution" table line 223 (mecanum/omni mix).

    r.register(SolutionDefinition {
        id: "mecanum_control_solution".into(),
        label: "Mecanum / Omni Holonomic Drive".into(),
        label_zh: Some("麦克纳姆 / 全向轮控制器".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into(), "esp32s3_mini1".into()],
        fixed_inputs: vec![InputSurface::EspNowData, InputSurface::I2cSensor],
        fixed_outputs: vec![OutputSurface::MotorDrive, OutputSurface::StatusLed],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "esp_now_init".into(),
                label: "Initialize ESP-NOW receiver".into(),
                label_zh: Some("初始化 ESP-NOW 接收".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "imu_calibrate".into(),
                label: "Calibrate IMU (yaw lock for holonomic motion)".into(),
                label_zh: Some("IMU 校准(全向运动的航向锁定)".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "control_loop".into(),
                label: "Decode (vx, vy, ω) command → mecanum/omni allocation → 3-4 wheel PWMs".into(),
                label_zh: Some("解码 (vx, vy, ω) 命令 → 麦克纳姆/全向轮分配 → 3-4 路 PWM".into()),
                description: Some("Mecanum 4-wheel: 4-equation linear allocation. Omni 3-wheel: 120° basis. Omni 4-wheel: 90° basis. IMU yaw feeds back as setpoint correction.".into()),
                description_zh: Some("麦克纳姆 4 轮:4 方程线性分配。3 轮全向:120° 基。4 轮全向:90° 基。IMU 偏航作为设定值修正反馈。".into()),
                depends_on: vec!["esp_now_init".into(), "imu_calibrate".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "holonomic_loop".into(),
            label: "200 Hz holonomic allocation loop".into(),
            decisions: vec![
                "Allocation matrix recomputed only when wheel layout config changes (mecanum vs omni3 vs omni4)".into(),
                "IMU yaw runs at 200 Hz; wheel velocity setpoints update on every ESP-NOW packet".into(),
            ],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "wheel_layout".into(),
                label: "Wheel Layout".into(),
                label_zh: Some("车轮布局".into()),
                required: true,
                secret: false,
                description: "Holonomic platform geometry".into(),
                description_zh: Some("全向平台的几何配置。".into()),
                default_value: Some(serde_json::Value::String("mecanum_4wheel".into())),
                enum_values: Some(vec![
                    EnumOption { value: "mecanum_4wheel".into(), label: "Mecanum 4-wheel (90°)".into(), description: Some("Standard mecanum: 4 wheels at 90°, 45° rollers.".into()) },
                    EnumOption { value: "omniwheel_3wheel".into(), label: "Omni 3-wheel (120°)".into(), description: Some("Triangle platform with 3 omni wheels.".into()) },
                    EnumOption { value: "omniwheel_4wheel".into(), label: "Omni 4-wheel (90°)".into(), description: Some("Cross-layout omni; less common than mecanum.".into()) },
                ]),
                depends_on: None,
            },
            imu_axis_tier_param(),
            imu_chip_param(),
            control_rate_hz_param(),
            failsafe_timeout_ms_param(),
        ],
        feedback_paths: vec![SignalPath {
            id: "esp_now_to_wheels".into(),
            name: "ESP-NOW (vx,vy,ω) command to wheel allocation".into(),
            source: InputSurface::EspNowData,
            transforms: vec![
                SignalPathStep { order: 1, node: TransformNode::CommandDispatch, label: Some("ESP-NOW packet → (vx,vy,ω)".into()), description: None },
                SignalPathStep { order: 2, node: TransformNode::Mapping, label: Some("Holonomic allocation per wheel_layout".into()), description: None },
                SignalPathStep { order: 3, node: TransformNode::SafetyInterlock, label: Some("Watchdog cuts wheels on packet loss".into()), description: None },
            ],
            sink: OutputSurface::MotorDrive,
            feedback: vec![SignalPathStep { order: 1, node: FeedbackSurface::PhysicalMotion, label: None, description: None }],
            expected_user_result: "Robot translates in any direction without rotating; IMU keeps heading locked".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle { required: vec![], optional: vec!["ota".into()] },
        runtime_binding: brookesia_vehicle_binding(vec![
            "rshome_motor_control".into(),
            "rshome_imu".into(),
            "rshome_failsafe".into(),
        ]),
        external_contracts: vec!["ESP-NOW (vx,vy,ω) command frame".into()],
        network_topology: NetworkTopology::PointToPoint,
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::ControlBoard),
        communication_chains: None,
        pin_assignments: Some(vehicle_control_pins()),
        family: Some(ImplementationFamily::Custom),
        form_factor_families: Some(vec![
            FormFactorKind::Mecanum4wheel,
            FormFactorKind::Omniwheel3wheel,
            FormFactorKind::Omniwheel4wheel,
        ]),
        control_uplink: Some(ControlUplinkKind::EspNow),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::CustomUart),
        sensor_tier_min: Some(SensorTierKind::Standard9ax),
        actuator_family: Some(ActuatorFamily::MixedDiffDrive),
        power_rails: Some(PowerRailKind::DualLogicMotor),
        failsafe: Some(FailsafeInfo {
            killswitch_source: vec![KillswitchSource::RxLoss, KillswitchSource::TimeoutNoPacket],
            rx_loss_behavior: Some(RxLossBehavior::MotorCutoff),
            watchdog_ms: Some(500),
            emergency_stop_wiring: EmergencyStopWiring::GpioPulldown,
        }),
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Caveat),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Insufficient),
        ])),
    });

    // ── 12g. Marine Surface (boat / hovercraft / hydrofoil / sailboat) ─────
    //
    // Per doc §L4 (line 403, ardupilot ArduBoat), §L5 (line 487:
    // crsf / none / mavlink_wifi), §L5.5 (line 532: rth / 1000 ms /
    // relay_cutoff — marine has long failsafe windows because the boat
    // doesn't fall down when it loses signal).

    r.register(SolutionDefinition {
        id: "marine_surface_solution".into(),
        label: "Marine Surface (Boat / Hovercraft / Sailboat)".into(),
        label_zh: Some("水面载具(船 / 气垫船 / 帆船)".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec![
            "esp32s3_wroom1".into(),
            "esp32s3_mini1".into(),
            "esp32c6_wroom1".into(),
        ],
        fixed_inputs: vec![InputSurface::UartRx, InputSurface::I2cSensor, InputSurface::WifiEvent],
        fixed_outputs: vec![OutputSurface::ServoDrive, OutputSurface::MotorDrive, OutputSurface::WifiPacket],
        fixed_orchestration: vec![
            OrchestrationStep { id: "crsf_rx_init".into(), label: "Initialize CRSF UART receiver".into(), label_zh: Some("初始化 CRSF UART 接收机".into()), description: None, description_zh: None, depends_on: vec![] , ..Default::default() },
            OrchestrationStep { id: "mavlink_init".into(), label: "Initialize MAVLink WiFi telemetry to ground station".into(), label_zh: Some("初始化 MAVLink Wi-Fi 遥测到地面站".into()), description: None, description_zh: None, depends_on: vec![] , ..Default::default() },
            OrchestrationStep { id: "imu_gps_init".into(), label: "Initialize IMU + GPS for RTH path".into(), label_zh: Some("初始化 IMU + GPS(支持自动返航)".into()), description: None, description_zh: None, depends_on: vec![] , ..Default::default() },
            OrchestrationStep { id: "control_loop".into(), label: "CRSF → marine mix (single rudder | twin-prop diff | sail+rudder) → actuators".into(), label_zh: Some("CRSF → 水面混控(单舵 | 双桨差速 | 帆+舵)→ 执行器".into()), description: None, description_zh: None, depends_on: vec!["crsf_rx_init".into(), "imu_gps_init".into()] , ..Default::default() },
        ],
        scheduling: SchedulingPolicy {
            id: "marine_loop".into(),
            label: "50 Hz marine control loop".into(),
            decisions: vec![
                "Slow loop (50 Hz) — boats/hovercraft don't need fast reaction".into(),
                "RX loss → return-to-home (RTH) using GPS heading; relay_cutoff at 1000 ms watchdog".into(),
            ],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "hull_type".into(),
                label: "Hull Type".into(),
                label_zh: Some("船体类型".into()),
                required: true,
                secret: false,
                description: "Marine platform geometry — drives mixing".into(),
                description_zh: Some("水面平台几何 — 决定混控方式。".into()),
                default_value: Some(serde_json::Value::String("single_rudder".into())),
                enum_values: Some(vec![
                    EnumOption { value: "single_rudder".into(), label: "Single Rudder + Prop".into(), description: Some("Traditional boat: one prop + servo rudder.".into()) },
                    EnumOption { value: "twin_prop_diff".into(), label: "Twin Prop Differential".into(), description: Some("Two props, skid-steer.".into()) },
                    EnumOption { value: "hovercraft".into(), label: "Hovercraft".into(), description: Some("Lift fan + thrust fan + rudder.".into()) },
                    EnumOption { value: "hydrofoil".into(), label: "Hydrofoil".into(), description: Some("Prop + foil-tilt servos.".into()) },
                    EnumOption { value: "sailboat".into(), label: "Sailboat".into(), description: Some("Sail servo + rudder servo.".into()) },
                ]),
                depends_on: None,
            },
            imu_axis_tier_param(),
            imu_chip_param(),
            failsafe_timeout_ms_param(),
        ],
        feedback_paths: vec![SignalPath {
            id: "crsf_to_actuators".into(),
            name: "CRSF command to marine actuators".into(),
            source: InputSurface::UartRx,
            transforms: vec![
                SignalPathStep { order: 1, node: TransformNode::CommandDispatch, label: Some("CRSF channel decode".into()), description: None },
                SignalPathStep { order: 2, node: TransformNode::Mapping, label: Some("Hull-specific mix".into()), description: None },
                SignalPathStep { order: 3, node: TransformNode::SafetyInterlock, label: Some("RTH on RX loss; relay_cutoff at watchdog".into()), description: None },
            ],
            sink: OutputSurface::ServoDrive,
            feedback: vec![SignalPathStep { order: 1, node: FeedbackSurface::PhysicalMotion, label: None, description: None }],
            expected_user_result: "Stick input drives boat with ground-station telemetry; signal loss triggers GPS return-to-home".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle { required: vec!["wifi".into()], optional: vec!["ota".into()] },
        runtime_binding: {
            let mut b = brookesia_vehicle_binding(vec![
                "rshome_motor_control".into(),
                "rshome_imu".into(),
                "rshome_failsafe".into(),
            ]);
            b.board_assembly = Some("esp32s3_va_marine_assembly".into());
            b
        },
        external_contracts: vec!["CRSF (ExpressLRS)".into(), "MAVLink v2 over WiFi".into()],
        network_topology: NetworkTopology::Star,
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::ControlBoard),
        communication_chains: None,
        pin_assignments: Some(vehicle_car_pins()),
        family: Some(ImplementationFamily::Ardupilot),
        form_factor_families: Some(vec![
            FormFactorKind::BoatSingleRudder,
            FormFactorKind::BoatTwinPropDiff,
            FormFactorKind::Hovercraft,
            FormFactorKind::Hydrofoil,
            FormFactorKind::Sailboat,
        ]),
        control_uplink: Some(ControlUplinkKind::Crsf),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::MavlinkWifi),
        sensor_tier_min: Some(SensorTierKind::Basic6ax),
        actuator_family: Some(ActuatorFamily::MixedAckermann),
        power_rails: Some(PowerRailKind::TripleMotorServoLogic),
        failsafe: Some(FailsafeInfo {
            killswitch_source: vec![KillswitchSource::RcSwitch, KillswitchSource::RxLoss, KillswitchSource::LowVoltage],
            rx_loss_behavior: Some(RxLossBehavior::Rth),
            watchdog_ms: Some(1000),
            emergency_stop_wiring: EmergencyStopWiring::RelayCutoff,
        }),
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Caveat),
        ])),
    });

    // ── 12h. LTA Slow Actuator (blimp / airship) ─────────────────────────
    //
    // Per doc §L4 (line 407, custom), §L5 (line 491:
    // crsf / mjpeg_http / mavlink_wifi), §L5.5 (line 536: glide_trim /
    // 1000 ms / none — LTA platforms drift, they don't fall).

    r.register(SolutionDefinition {
        id: "lta_slow_actuator_solution".into(),
        label: "LTA Blimp / Airship Controller".into(),
        label_zh: Some("飞艇 / 空艇控制器".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into()],
        fixed_inputs: vec![InputSurface::UartRx, InputSurface::CameraFrame],
        fixed_outputs: vec![
            OutputSurface::MotorDrive,
            OutputSurface::ServoDrive,
            OutputSurface::HttpMjpegStream,
        ],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "crsf_rx_init".into(),
                label: "Initialize CRSF UART receiver".into(),
                label_zh: Some("初始化 CRSF UART 接收机".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "camera_init".into(),
                label: "Initialize camera + MJPEG HTTP server".into(),
                label_zh: Some("初始化摄像头 + MJPEG HTTP 服务".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "control_loop".into(),
                label: "Slow loop (10 Hz): CRSF → 2 thrusters + rudder servo".into(),
                label_zh: Some("慢速循环(10 Hz):CRSF → 2 个推进器 + 舵机".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["crsf_rx_init".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "lta_slow".into(),
            label: "10 Hz slow control loop".into(),
            decisions: vec![
                "Blimp dynamics are slow — 10 Hz is plenty".into(),
                "On RX loss: trim attitude (slow drift) for 1 s, then cut thrusters".into(),
            ],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "envelope_type".into(),
                label: "Envelope Type".into(),
                label_zh: Some("艇体类型".into()),
                required: true,
                secret: false,
                description: "Blimp / airship envelope geometry".into(),
                description_zh: Some("飞艇/空艇艇体几何。".into()),
                default_value: Some(serde_json::Value::String("blimp".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "blimp".into(),
                        label: "Blimp (non-rigid)".into(),
                        description: Some("Pressurized envelope; simplest LTA.".into()),
                    },
                    EnumOption {
                        value: "airship".into(),
                        label: "Airship (semi-rigid)".into(),
                        description: Some("Internal frame; control surfaces.".into()),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "thruster_count".into(),
                label: "Thruster Count".into(),
                label_zh: Some("推进器数量".into()),
                required: false,
                secret: false,
                description: "Number of thrust motors (left/right pair = 2)".into(),
                description_zh: Some("推力电机数量(左右对 = 2)。".into()),
                default_value: Some(serde_json::Value::String("2".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "1".into(),
                        label: "1 (single)".into(),
                        description: Some("Mono-thrust + rudder steering.".into()),
                    },
                    EnumOption {
                        value: "2".into(),
                        label: "2 (twin)".into(),
                        description: Some("Differential thrust + rudder.".into()),
                    },
                    EnumOption {
                        value: "3".into(),
                        label: "3 (twin + vertical)".into(),
                        description: Some("Adds altitude thrust.".into()),
                    },
                ]),
                depends_on: None,
            },
            failsafe_timeout_ms_param(),
        ],
        feedback_paths: vec![SignalPath {
            id: "crsf_to_thrusters".into(),
            name: "CRSF command to LTA thrusters".into(),
            source: InputSurface::UartRx,
            transforms: vec![
                SignalPathStep {
                    order: 1,
                    node: TransformNode::CommandDispatch,
                    label: Some("CRSF decode".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: TransformNode::Mapping,
                    label: Some("Thrust + rudder mix".into()),
                    description: None,
                },
            ],
            sink: OutputSurface::MotorDrive,
            feedback: vec![SignalPathStep {
                order: 1,
                node: FeedbackSurface::PhysicalMotion,
                label: None,
                description: None,
            }],
            expected_user_result:
                "Stick input drives slow thrusters; LTA drifts gracefully on signal loss".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["wifi".into()],
            optional: vec!["ota".into()],
        },
        runtime_binding: brookesia_vehicle_binding(vec!["rshome_motor_control".into()]),
        external_contracts: vec!["CRSF (ExpressLRS)".into()],
        network_topology: NetworkTopology::Star,
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::ControlBoard),
        communication_chains: None,
        pin_assignments: Some(vehicle_control_pins()),
        family: Some(ImplementationFamily::Custom),
        form_factor_families: Some(vec![FormFactorKind::LtaBlimp, FormFactorKind::LtaAirship]),
        control_uplink: Some(ControlUplinkKind::Crsf),
        video_downlink: Some(VideoDownlinkKind::MjpegHttp),
        telemetry: Some(TelemetryKind::MavlinkWifi),
        sensor_tier_min: Some(SensorTierKind::Basic6ax),
        actuator_family: Some(ActuatorFamily::MixedDiffDrive),
        power_rails: Some(PowerRailKind::DualLogicMotor),
        failsafe: Some(FailsafeInfo {
            killswitch_source: vec![KillswitchSource::RcSwitch, KillswitchSource::RxLoss],
            rx_loss_behavior: Some(RxLossBehavior::GlideTrim),
            watchdog_ms: Some(1000),
            emergency_stop_wiring: EmergencyStopWiring::None,
        }),
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Insufficient),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Insufficient),
        ])),
    });

    // ── 12i. Agricultural Tool Dispatch (mower / sprayer / tractor) ────────
    //
    // Per doc §L1 (agri form factors with standard_9ax + GPS floor),
    // §"Form Factor → Solution" (line 234: agricultural mix dispatches to
    // PTO/boom on top of wheeled control). Standard_fpv class — uses CRSF
    // + MAVLink for ground-station coordination.

    r.register(SolutionDefinition {
        id: "agri_tool_dispatch_solution".into(),
        label: "Agricultural Tool Dispatch (Mower / Sprayer / Tractor)".into(),
        label_zh: Some("农业工具控制器(割草机 / 喷药机 / 拖拉机)".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into(), "esp32s3_mini1".into()],
        fixed_inputs: vec![InputSurface::UartRx, InputSurface::I2cSensor, InputSurface::ButtonGpio],
        fixed_outputs: vec![OutputSurface::MotorDrive, OutputSurface::ServoDrive, OutputSurface::RelayDrive],
        fixed_orchestration: vec![
            OrchestrationStep { id: "crsf_rx_init".into(), label: "Initialize CRSF UART".into(), label_zh: Some("初始化 CRSF UART".into()), description: None, description_zh: None, depends_on: vec![] , ..Default::default() },
            OrchestrationStep { id: "tool_safety_init".into(), label: "Bind tool e-stop GPIO + bumper interrupt".into(), label_zh: Some("绑定工具急停 GPIO + 防撞中断".into()), description: Some("Bumper + e-stop disable mower blade or sprayer pump immediately, regardless of RC state.".into()), description_zh: Some("防撞与急停立即停掉割草刀片或喷药泵,优先于遥控状态。".into()), depends_on: vec![] , ..Default::default() },
            OrchestrationStep { id: "control_loop".into(), label: "CRSF → wheeled mix + tool actuator (mower blade / pump / PTO)".into(), label_zh: Some("CRSF → 轮式混控 + 工具执行器(刀片 / 泵 / 动力输出)".into()), description: None, description_zh: None, depends_on: vec!["crsf_rx_init".into(), "tool_safety_init".into()] , ..Default::default() },
        ],
        scheduling: SchedulingPolicy {
            id: "agri_loop".into(),
            label: "100 Hz wheeled control + event-driven tool dispatch".into(),
            decisions: vec![
                "Wheel control runs at 100 Hz; tool actuator updates only on operator command".into(),
                "Bumper/e-stop interrupt cuts BOTH wheels and tool actuator".into(),
                "GPS-conditioned RTH on RX loss".into(),
            ],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "tool_kind".into(),
                label: "Agricultural Tool".into(),
                label_zh: Some("农业工具类型".into()),
                required: true,
                secret: false,
                description: "Which tool actuator type is wired".into(),
                description_zh: Some("挂载的工具执行器类型。".into()),
                default_value: Some(serde_json::Value::String("mower_blade".into())),
                enum_values: Some(vec![
                    EnumOption { value: "mower_blade".into(), label: "Mower Blade (brushless)".into(), description: Some("ESC-driven cutting blade. Bumper + e-stop required.".into()) },
                    EnumOption { value: "sprayer_pump".into(), label: "Sprayer Pump + Boom Servos".into(), description: Some("Spot/section spray. Optional vision-guided variant uses SBC.".into()) },
                    EnumOption { value: "pto_three_point".into(), label: "PTO + 3-Point Hitch".into(), description: Some("Tractor PTO output + hitch lift. Research-grade only.".into()) },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "drive_type".into(),
                label: "Drive Type".into(),
                label_zh: Some("驱动方式".into()),
                required: true,
                secret: false,
                description: "Wheeled platform mix".into(),
                description_zh: Some("轮式平台的混控方式。".into()),
                default_value: Some(serde_json::Value::String("ackermann".into())),
                enum_values: Some(vec![
                    EnumOption { value: "diff".into(), label: "Differential (mower)".into(), description: Some("Skid-steer, common on robot mowers.".into()) },
                    EnumOption { value: "ackermann".into(), label: "Ackermann (sprayer)".into(), description: Some("ESC + servo, common on sprayers.".into()) },
                    EnumOption { value: "tractor".into(), label: "Heavy 4WD (tractor)".into(), description: Some("Full-size; regulatory/safety-heavy.".into()) },
                ]),
                depends_on: None,
            },
            imu_axis_tier_param(),
            imu_chip_param(),
            failsafe_timeout_ms_param(),
        ],
        feedback_paths: vec![SignalPath {
            id: "crsf_to_tool".into(),
            name: "CRSF command to wheels + tool".into(),
            source: InputSurface::UartRx,
            transforms: vec![
                SignalPathStep { order: 1, node: TransformNode::CommandDispatch, label: Some("CRSF decode".into()), description: None },
                SignalPathStep { order: 2, node: TransformNode::Mapping, label: Some("Wheeled mix + tool dispatch".into()), description: None },
                SignalPathStep { order: 3, node: TransformNode::SafetyInterlock, label: Some("Bumper/e-stop kills BOTH wheels and tool".into()), description: None },
            ],
            sink: OutputSurface::MotorDrive,
            feedback: vec![SignalPathStep { order: 1, node: FeedbackSurface::PhysicalMotion, label: None, description: None }],
            expected_user_result: "Operator drives platform; tool engages on command; bumper or e-stop halts everything".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle { required: vec!["wifi".into()], optional: vec!["ota".into()] },
        runtime_binding: brookesia_vehicle_binding(vec![
            "rshome_motor_control".into(),
            "rshome_imu".into(),
            "rshome_failsafe".into(),
        ]),
        external_contracts: vec!["CRSF (ExpressLRS)".into(), "MAVLink v2 (optional ground station)".into()],
        network_topology: NetworkTopology::Star,
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::ControlTelemetryBoard),
        communication_chains: None,
        pin_assignments: Some(vehicle_car_pins()),
        family: Some(ImplementationFamily::Ardupilot),
        form_factor_families: Some(vec![
            FormFactorKind::AutonomousMower,
            FormFactorKind::SprayerSpot,
            FormFactorKind::TractorTowedImplement,
        ]),
        control_uplink: Some(ControlUplinkKind::Crsf),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::MavlinkWifi),
        // Research tier required for tractors; standard_9ax is the floor for
        // mower/sprayer per the doc's per-family table.
        sensor_tier_min: Some(SensorTierKind::Research),
        actuator_family: Some(ActuatorFamily::MixedAckermann),
        power_rails: Some(PowerRailKind::TripleMotorServoLogic),
        failsafe: Some(FailsafeInfo {
            killswitch_source: vec![
                KillswitchSource::RcSwitch,
                KillswitchSource::RxLoss,
                KillswitchSource::EmergencyButton,
                KillswitchSource::LowVoltage,
            ],
            rx_loss_behavior: Some(RxLossBehavior::Rth),
            watchdog_ms: Some(500),
            emergency_stop_wiring: EmergencyStopWiring::RelayCutoff,
        }),
        topology_category: None,
        required_sensors: vec![SensorRequirement::Gps],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Caveat),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Insufficient),
        ])),
    });

    // ── 12j. Helicopter Stabilizer (single / coax / tandem) ──────────────
    //
    // Per doc §L4 (line 399, ardupilot ArduCopter-Heli), §L5 (line 483:
    // crsf / none / crsf_telemetry), §L5.5 (line 528: motor_cutoff /
    // 100 ms / esc_dshot_cmd — heli must cut on link loss, no glide).

    r.register(SolutionDefinition {
        id: "heli_stabilizer_solution".into(),
        label: "Helicopter Stabilizer".into(),
        label_zh: Some("直升机稳定器".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into(), "esp32s3_mini1".into()],
        fixed_inputs: vec![InputSurface::UartRx, InputSurface::I2cSensor],
        fixed_outputs: vec![OutputSurface::ServoDrive, OutputSurface::MotorDrive, OutputSurface::StatusLed],
        fixed_orchestration: vec![
            OrchestrationStep { id: "crsf_rx_init".into(), label: "Initialize CRSF UART receiver".into(), label_zh: Some("初始化 CRSF UART 接收机".into()), description: None, description_zh: None, depends_on: vec![] , ..Default::default() },
            OrchestrationStep { id: "imu_calibrate".into(), label: "Calibrate 9-axis IMU".into(), label_zh: Some("9 轴 IMU 校准".into()), description: None, description_zh: None, depends_on: vec![] , ..Default::default() },
            OrchestrationStep { id: "stabilizer_loop".into(), label: "1 kHz stabilizer: CRSF → AHRS → attitude PID → swashplate mix → servo PWM + ESC".into(), label_zh: Some("1 kHz 稳定器循环:CRSF → AHRS → 姿态 PID → 自动倾斜器混控 → 舵机 + 电调".into()), description: None, description_zh: None, depends_on: vec!["crsf_rx_init".into(), "imu_calibrate".into()] , ..Default::default() },
        ],
        scheduling: SchedulingPolicy {
            id: "heli_1khz".into(),
            label: "1 kHz stabilizer + swashplate mix".into(),
            decisions: vec![
                "Swashplate mix runs at 1 kHz (3-servo CCPM standard)".into(),
                "Tail rotor: separate ESC for single-rotor; coax has no tail; tandem syncs both heads".into(),
                "RX loss cuts main + tail ESC via DShot kill command within 100 ms".into(),
            ],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "rotor_config".into(),
                label: "Rotor Configuration".into(),
                label_zh: Some("旋翼配置".into()),
                required: true,
                secret: false,
                description: "Helicopter rotor + tail layout".into(),
                description_zh: Some("直升机旋翼与尾桨布局。".into()),
                default_value: Some(serde_json::Value::String("single_rotor".into())),
                enum_values: Some(vec![
                    EnumOption { value: "single_rotor".into(), label: "Single Rotor + Tail".into(), description: Some("Standard heli: main + tail. Swashplate + tail rotor.".into()) },
                    EnumOption { value: "coaxial".into(), label: "Coaxial".into(), description: Some("Two counter-rotating rotors. No tail.".into()) },
                    EnumOption { value: "tandem".into(), label: "Tandem (Chinook)".into(), description: Some("Fore + aft rotors. Two swashplates synced.".into()) },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "swashplate_type".into(),
                label: "Swashplate Type".into(),
                label_zh: Some("自动倾斜器类型".into()),
                required: true,
                secret: false,
                description: "Servo arrangement under the swashplate".into(),
                description_zh: Some("自动倾斜器下的舵机排布。".into()),
                default_value: Some(serde_json::Value::String("ccpm_3servo_120".into())),
                enum_values: Some(vec![
                    EnumOption { value: "ccpm_3servo_120".into(), label: "3-Servo CCPM (120°)".into(), description: Some("Standard hobby-grade swashplate.".into()) },
                    EnumOption { value: "ccpm_3servo_140".into(), label: "3-Servo CCPM (140°)".into(), description: Some("Wider servo span.".into()) },
                    EnumOption { value: "h1_mechanical".into(), label: "H-1 Mechanical Mix".into(), description: Some("Servos pre-mixed by linkages.".into()) },
                ]),
                depends_on: Some(ParameterDependency {
                    parameter_id: "rotor_config".into(),
                    when_value: "single_rotor".into(),
                    when_not_value: None,
                }),
            },
            imu_axis_tier_param(),
            imu_chip_param(),
            control_rate_hz_param(),
            failsafe_timeout_ms_param(),
        ],
        feedback_paths: vec![SignalPath {
            id: "crsf_to_swashplate".into(),
            name: "CRSF command to swashplate + tail rotor".into(),
            source: InputSurface::UartRx,
            transforms: vec![
                SignalPathStep { order: 1, node: TransformNode::CommandDispatch, label: Some("CRSF channel decode".into()), description: None },
                SignalPathStep { order: 2, node: TransformNode::PidLoop, label: Some("Attitude PID (roll/pitch/yaw)".into()), description: None },
                SignalPathStep { order: 3, node: TransformNode::Mapping, label: Some("Swashplate CCPM mix + tail rotor".into()), description: None },
                SignalPathStep { order: 4, node: TransformNode::SafetyInterlock, label: Some("DShot kill on RX loss / low voltage".into()), description: None },
            ],
            sink: OutputSurface::ServoDrive,
            feedback: vec![SignalPathStep { order: 1, node: FeedbackSurface::PhysicalMotion, label: None, description: None }],
            expected_user_result: "Cyclic + collective + tail input produces stable hover; signal loss kills both main and tail within 100 ms".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle { required: vec![], optional: vec!["ota".into()] },
        runtime_binding: {
            let mut b = brookesia_vehicle_binding(vec![
                "rshome_motor_control".into(),
                "rshome_imu".into(),
                "rshome_failsafe".into(),
            ]);
            b.board_assembly = Some("esp32s3_va_multirotor_assembly".into());
            b
        },
        external_contracts: vec!["CRSF (ExpressLRS)".into(), "DShot ESC".into()],
        network_topology: NetworkTopology::default(),
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::ControlBoard),
        communication_chains: None,
        pin_assignments: Some(vehicle_control_pins()),
        family: Some(ImplementationFamily::Ardupilot),
        form_factor_families: Some(vec![
            FormFactorKind::HeliSingleRotor,
            FormFactorKind::HeliCoaxial,
            FormFactorKind::HeliTandem,
        ]),
        control_uplink: Some(ControlUplinkKind::Crsf),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::CrsfTelemetry),
        sensor_tier_min: Some(SensorTierKind::Standard9ax),
        actuator_family: Some(ActuatorFamily::HeliSwashplate),
        power_rails: Some(PowerRailKind::TripleMotorServoLogic),
        failsafe: Some(FailsafeInfo {
            killswitch_source: vec![KillswitchSource::RcSwitch, KillswitchSource::RxLoss, KillswitchSource::LowVoltage],
            rx_loss_behavior: Some(RxLossBehavior::MotorCutoff),
            watchdog_ms: Some(100),
            emergency_stop_wiring: EmergencyStopWiring::EscDshotCmd,
        }),
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Insufficient),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Insufficient),
        ])),
    });

    // ── 12k. Modular Cubelets (educational, runtime topology) ──────────────
    //
    // Per doc §L1 (educational_modular family — runtime topology
    // discovered via I²C daisy chain) and §"Form Factor → Solution"
    // (line 239: modular_dynamic_solution discovers topology at boot).

    r.register(SolutionDefinition {
        id: "modular_dynamic_solution".into(),
        label: "Modular Cubelets (Runtime Topology)".into(),
        label_zh: Some("模块化积木(运行时拓扑发现)".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec![
            "esp32c6_wroom1".into(),
            "esp32c6_mini1".into(),
            "esp32s3_wroom1".into(),
        ],
        fixed_inputs: vec![InputSurface::I2cSensor, InputSurface::EspNowData, InputSurface::ServiceCall],
        fixed_outputs: vec![OutputSurface::I2cMasterWrite, OutputSurface::MotorDrive],
        fixed_orchestration: vec![
            OrchestrationStep { id: "i2c_scan".into(), label: "Scan I²C bus for connected cubelets (addr discovery)".into(), label_zh: Some("扫描 I²C 总线发现已连接的积木(地址发现)".into()), description: Some("Each cubelet exposes a type byte at known address; firmware builds a topology map at boot and on hot-plug events.".into()), description_zh: Some("每个积木在已知地址暴露类型字节;固件在启动和热插拔事件时构建拓扑图。".into()), depends_on: vec![] , ..Default::default() },
            OrchestrationStep { id: "esp_now_init".into(), label: "Initialize ESP-NOW receiver (control commands)".into(), label_zh: Some("初始化 ESP-NOW 接收(控制命令)".into()), description: None, description_zh: None, depends_on: vec![] , ..Default::default() },
            OrchestrationStep { id: "dispatch_loop".into(), label: "Dispatch commands to discovered cubelets via I²C; handle hot-plug periodically".into(), label_zh: Some("通过 I²C 向已发现的积木分发命令;周期性处理热插拔".into()), description: None, description_zh: None, depends_on: vec!["i2c_scan".into(), "esp_now_init".into()] , ..Default::default() },
        ],
        scheduling: SchedulingPolicy {
            id: "modular_dispatch".into(),
            label: "Topology-driven dispatch (rescan on disconnect)".into(),
            decisions: vec![
                "Initial scan on boot; periodic re-scan every 1 s; immediate re-scan on I²C NAK".into(),
                "Each cubelet handles its own actuator; this MCU is a router, not a controller".into(),
            ],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "rescan_interval_ms".into(),
                label: "Topology Re-scan Interval (ms)".into(),
                label_zh: Some("拓扑重扫间隔(毫秒)".into()),
                required: false,
                secret: false,
                description: "How often to re-discover the cubelet chain".into(),
                description_zh: Some("重新发现积木链的间隔。".into()),
                default_value: Some(serde_json::Value::String("1000".into())),
                enum_values: Some(vec![
                    EnumOption { value: "500".into(), label: "500 ms".into(), description: Some("Aggressive — for hot-plug-heavy demos.".into()) },
                    EnumOption { value: "1000".into(), label: "1 s".into(), description: Some("Standard.".into()) },
                    EnumOption { value: "5000".into(), label: "5 s".into(), description: Some("Relaxed — for stable topologies.".into()) },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "i2c_baud_hz".into(),
                label: "I²C Bus Clock (Hz)".into(),
                label_zh: Some("I²C 时钟频率(Hz)".into()),
                required: false,
                secret: false,
                description: "I²C clock used for the cubelet daisy chain".into(),
                description_zh: Some("积木菊花链使用的 I²C 时钟频率。".into()),
                default_value: Some(serde_json::Value::String("100000".into())),
                enum_values: Some(vec![
                    EnumOption { value: "100000".into(), label: "100 kHz (standard)".into(), description: Some("Universal compatibility.".into()) },
                    EnumOption { value: "400000".into(), label: "400 kHz (fast)".into(), description: Some("Faster, may need stronger pull-ups for long chains.".into()) },
                ]),
                depends_on: None,
            },
            failsafe_timeout_ms_param(),
        ],
        feedback_paths: vec![SignalPath {
            id: "esp_now_to_cubelets".into(),
            name: "ESP-NOW command to discovered cubelet chain".into(),
            source: InputSurface::EspNowData,
            transforms: vec![
                SignalPathStep { order: 1, node: TransformNode::CommandDispatch, label: Some("ESP-NOW packet decode".into()), description: None },
                SignalPathStep { order: 2, node: TransformNode::OneToMany, label: Some("Fan-out to cubelets via I²C".into()), description: None },
            ],
            sink: OutputSurface::I2cMasterWrite,
            feedback: vec![SignalPathStep { order: 1, node: FeedbackSurface::LedIndicator, label: Some("Per-cubelet status LED".into()), description: None }],
            expected_user_result: "Plug in or pull out cubelets at runtime; control commands dispatch to whichever cubelets are present".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle { required: vec![], optional: vec!["ota".into()] },
        runtime_binding: brookesia_vehicle_binding(vec![
            "rshome_failsafe".into(),
        ]),
        external_contracts: vec!["I²C cubelet handshake protocol (1 type byte at addr)".into()],
        network_topology: NetworkTopology::PointToPoint,
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::ControlBoard),
        communication_chains: None,
        pin_assignments: Some(vehicle_control_pins()),
        family: Some(ImplementationFamily::Custom),
        form_factor_families: Some(vec![FormFactorKind::ModularCubelets]),
        control_uplink: Some(ControlUplinkKind::EspNow),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::CustomUart),
        sensor_tier_min: Some(SensorTierKind::Basic6ax),
        actuator_family: Some(ActuatorFamily::MixedDiffDrive),
        power_rails: Some(PowerRailKind::DualLogicMotor),
        failsafe: Some(FailsafeInfo {
            killswitch_source: vec![KillswitchSource::RxLoss, KillswitchSource::TimeoutNoPacket],
            rx_loss_behavior: Some(RxLossBehavior::MotorCutoff),
            watchdog_ms: Some(500),
            emergency_stop_wiring: EmergencyStopWiring::GpioPulldown,
        }),
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Caveat),
        ])),
    });

    // ── 12l. Hopping / Jumping Robot (ballistic phase) ─────────────────────
    //
    // Per doc §L1 (jumping_hopping family with advanced_10ax floor — needs
    // altitude+gyro fused for landing recovery) and §"Form Factor →
    // Solution" line 240 (hopping_ballistic_solution: ballistic-phase
    // attitude hold).

    r.register(SolutionDefinition {
        id: "hopping_ballistic_solution".into(),
        label: "Hopping / Jumping Robot (Ballistic Recovery)".into(),
        label_zh: Some("跳跃机器人(弹道恢复)".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into(), "esp32s3_mini1".into()],
        fixed_inputs: vec![InputSurface::I2cSensor, InputSurface::EspNowData],
        fixed_outputs: vec![OutputSurface::ServoDrive, OutputSurface::MotorDrive, OutputSurface::StatusLed],
        fixed_orchestration: vec![
            OrchestrationStep { id: "imu_baro_calibrate".into(), label: "Calibrate IMU + barometer (advanced_10ax)".into(), label_zh: Some("校准 IMU + 气压计(10 轴)".into()), description: None, description_zh: None, depends_on: vec![] , ..Default::default() },
            OrchestrationStep { id: "spring_arm".into(), label: "Wind torsion spring; latch in armed state".into(), label_zh: Some("绕紧扭力弹簧;锁定为待发射状态".into()), description: None, description_zh: None, depends_on: vec![] , ..Default::default() },
            OrchestrationStep { id: "release_loop".into(), label: "On command: release latch → ballistic phase → mid-air orientation hold → landing recovery".into(), label_zh: Some("收到命令时:释放锁扣 → 弹道阶段 → 空中姿态保持 → 落地恢复".into()), description: Some("State machine: ARMED → BALLISTIC → LANDING → RECOVERED. IMU drives mid-air control; baro detects apex and impending landing.".into()), description_zh: Some("状态机:ARMED → BALLISTIC → LANDING → RECOVERED。IMU 驱动空中控制;气压计检测最高点和即将落地。".into()), depends_on: vec!["imu_baro_calibrate".into(), "spring_arm".into()] , ..Default::default() },
        ],
        scheduling: SchedulingPolicy {
            id: "hopping_state_machine".into(),
            label: "Hopping state machine (1 kHz IMU during ballistic)".into(),
            decisions: vec![
                "Pre-jump: idle, wait for command".into(),
                "Ballistic: 1 kHz IMU + tail/leg attitude correction".into(),
                "Landing: detect via baro + accel impulse; cut motors briefly to absorb".into(),
                "Recovered: re-arm spring on operator request".into(),
            ],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "launch_mechanism".into(),
                label: "Launch Mechanism".into(),
                label_zh: Some("发射机构".into()),
                required: true,
                secret: false,
                description: "How the jump is stored and released".into(),
                description_zh: Some("跳跃如何储能与释放。".into()),
                default_value: Some(serde_json::Value::String("torsion_spring".into())),
                enum_values: Some(vec![
                    EnumOption { value: "compressed_spring".into(), label: "Compressed Spring + Latch".into(), description: Some("Single-shot release.".into()) },
                    EnumOption { value: "torsion_spring".into(), label: "Motor-Wound Torsion Spring".into(), description: Some("Repeated hops; motor re-winds between jumps.".into()) },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "imu_loop_hz".into(),
                label: "Ballistic IMU Loop Rate (Hz)".into(),
                label_zh: Some("弹道阶段 IMU 循环频率(Hz)".into()),
                required: false,
                secret: false,
                description: "IMU sampling during the in-air phase; needs ≥500 Hz for orientation recovery".into(),
                description_zh: Some("空中阶段 IMU 采样频率;姿态恢复至少 500 Hz。".into()),
                default_value: Some(serde_json::Value::String("1000".into())),
                enum_values: Some(vec![
                    EnumOption { value: "500".into(), label: "500 Hz".into(), description: Some("Minimum for landing recovery.".into()) },
                    EnumOption { value: "1000".into(), label: "1 kHz".into(), description: Some("Standard.".into()) },
                ]),
                depends_on: None,
            },
            failsafe_timeout_ms_param(),
        ],
        feedback_paths: vec![SignalPath {
            id: "imu_to_recovery".into(),
            name: "IMU + baro → landing recovery actuators".into(),
            source: InputSurface::I2cSensor,
            transforms: vec![
                SignalPathStep { order: 1, node: TransformNode::Filter, label: Some("AHRS (Madgwick) + baro fusion".into()), description: None },
                SignalPathStep { order: 2, node: TransformNode::StateMachine, label: Some("Ballistic phase state machine".into()), description: None },
                SignalPathStep { order: 3, node: TransformNode::PidLoop, label: Some("Mid-air attitude hold".into()), description: None },
            ],
            sink: OutputSurface::ServoDrive,
            feedback: vec![SignalPathStep { order: 1, node: FeedbackSurface::PhysicalMotion, label: None, description: None }],
            expected_user_result: "Robot launches, holds level attitude in flight, lands gracefully without tip-over".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle { required: vec![], optional: vec!["ota".into()] },
        runtime_binding: {
            let mut b = brookesia_vehicle_binding(vec![
                "rshome_motor_control".into(),
                "rshome_imu".into(),
                "rshome_failsafe".into(),
            ]);
            b.board_assembly = Some("esp32s3_va_multirotor_assembly".into());
            b
        },
        external_contracts: vec!["ESP-NOW launch command".into()],
        network_topology: NetworkTopology::PointToPoint,
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::ControlBoard),
        communication_chains: None,
        pin_assignments: Some(vehicle_control_pins()),
        family: Some(ImplementationFamily::Custom),
        form_factor_families: Some(vec![FormFactorKind::JumpingRobot, FormFactorKind::Grasshopper]),
        control_uplink: Some(ControlUplinkKind::EspNow),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::CustomUart),
        sensor_tier_min: Some(SensorTierKind::Advanced10ax),
        actuator_family: Some(ActuatorFamily::SteeringServo),
        power_rails: Some(PowerRailKind::DualLogicMotor),
        failsafe: Some(FailsafeInfo {
            killswitch_source: vec![KillswitchSource::RxLoss, KillswitchSource::TimeoutNoPacket, KillswitchSource::EmergencyButton],
            rx_loss_behavior: Some(RxLossBehavior::MotorCutoff),
            watchdog_ms: Some(250),
            emergency_stop_wiring: EmergencyStopWiring::GpioPulldown,
        }),
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Caveat),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Insufficient),
        ])),
    });

    // ── 12m. Amphibious Transition (wheels + props, dual-mode) ──────────────
    //
    // Per doc §L1 (amphibious_wheels_plus_prop with standard_9ax + water-
    // contact sensor floor) and §"Form Factor → Solution" line 237
    // (amphibious_transition_solution reuses wheeled + marine mixes).

    r.register(SolutionDefinition {
        id: "amphibious_transition_solution".into(),
        label: "Amphibious (Wheels + Props, Dual-Mode)".into(),
        label_zh: Some("水陆两栖(轮 + 桨,双模式)".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into(), "esp32s3_mini1".into()],
        fixed_inputs: vec![InputSurface::UartRx, InputSurface::I2cSensor, InputSurface::AdcVoltage],
        fixed_outputs: vec![OutputSurface::MotorDrive, OutputSurface::ServoDrive, OutputSurface::StatusLed],
        fixed_orchestration: vec![
            OrchestrationStep { id: "crsf_rx_init".into(), label: "Initialize CRSF UART receiver".into(), label_zh: Some("初始化 CRSF UART 接收机".into()), description: None, description_zh: None, depends_on: vec![] , ..Default::default() },
            OrchestrationStep { id: "water_sensor_init".into(), label: "Initialize water-contact sensor (resistive or capacitive)".into(), label_zh: Some("初始化水接触传感器(电阻式或电容式)".into()), description: None, description_zh: None, depends_on: vec![] , ..Default::default() },
            OrchestrationStep { id: "control_loop".into(), label: "Auto-switch on water detection: wheeled (Ackermann/diff) ↔ marine (twin-prop) mix".into(), label_zh: Some("水检测自动切换:轮式(阿克曼/差速)↔ 水面(双桨)混控".into()), description: None, description_zh: None, depends_on: vec!["crsf_rx_init".into(), "water_sensor_init".into()] , ..Default::default() },
        ],
        scheduling: SchedulingPolicy {
            id: "amphibious_dual_mode".into(),
            label: "Dual-mode auto-switching".into(),
            decisions: vec![
                "Land mode: wheels active, props off".into(),
                "Water mode: props active, wheels off".into(),
                "Transition: 500 ms hysteresis on water sensor to avoid mode chatter".into(),
            ],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "land_drive_type".into(),
                label: "Land Drive Type".into(),
                label_zh: Some("陆地驱动方式".into()),
                required: true,
                secret: false,
                description: "Wheeled mix on land".into(),
                description_zh: Some("陆地上的轮式混控。".into()),
                default_value: Some(serde_json::Value::String("ackermann".into())),
                enum_values: Some(vec![
                    EnumOption { value: "diff".into(), label: "4WD Differential".into(), description: Some("Skid-steer.".into()) },
                    EnumOption { value: "ackermann".into(), label: "Ackermann".into(), description: Some("ESC + servo.".into()) },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "water_sensor_threshold".into(),
                label: "Water Sensor Threshold".into(),
                label_zh: Some("水检测阈值".into()),
                required: false,
                secret: false,
                description: "ADC threshold for water detection (lower = more sensitive)".into(),
                description_zh: Some("水检测的 ADC 阈值(越低越灵敏)。".into()),
                default_value: Some(serde_json::Value::String("2048".into())),
                enum_values: None,
                depends_on: None,
            },
            imu_axis_tier_param(),
            imu_chip_param(),
            failsafe_timeout_ms_param(),
        ],
        feedback_paths: vec![SignalPath {
            id: "crsf_dual_mode".into(),
            name: "CRSF command to wheels OR props (water-sensor gated)".into(),
            source: InputSurface::UartRx,
            transforms: vec![
                SignalPathStep { order: 1, node: TransformNode::CommandDispatch, label: Some("CRSF decode".into()), description: None },
                SignalPathStep { order: 2, node: TransformNode::StateMachine, label: Some("Land/water mode arbiter".into()), description: None },
                SignalPathStep { order: 3, node: TransformNode::Mapping, label: Some("Wheeled mix OR marine mix".into()), description: None },
                SignalPathStep { order: 4, node: TransformNode::SafetyInterlock, label: Some("Hysteresis on water-sensor edges".into()), description: None },
            ],
            sink: OutputSurface::MotorDrive,
            feedback: vec![SignalPathStep { order: 1, node: FeedbackSurface::PhysicalMotion, label: None, description: None }],
            expected_user_result: "Vehicle drives on land, transitions to water without operator input, swims via aft props".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle { required: vec![], optional: vec!["ota".into()] },
        runtime_binding: {
            let mut b = brookesia_vehicle_binding(vec![
                "rshome_motor_control".into(),
                "rshome_imu".into(),
                "rshome_failsafe".into(),
            ]);
            b.board_assembly = Some("esp32s3_va_marine_assembly".into());
            b
        },
        external_contracts: vec!["CRSF (ExpressLRS)".into()],
        network_topology: NetworkTopology::default(),
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::ControlBoard),
        communication_chains: None,
        pin_assignments: Some(vehicle_car_pins()),
        family: Some(ImplementationFamily::Ardupilot),
        form_factor_families: Some(vec![FormFactorKind::AmphibiousWheelsPlusProp]),
        control_uplink: Some(ControlUplinkKind::Crsf),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::MavlinkWifi),
        sensor_tier_min: Some(SensorTierKind::Standard9ax),
        actuator_family: Some(ActuatorFamily::MixedAckermann),
        power_rails: Some(PowerRailKind::TripleMotorServoLogic),
        failsafe: Some(FailsafeInfo {
            killswitch_source: vec![KillswitchSource::RcSwitch, KillswitchSource::RxLoss, KillswitchSource::LowVoltage],
            rx_loss_behavior: Some(RxLossBehavior::MotorCutoff),
            watchdog_ms: Some(500),
            emergency_stop_wiring: EmergencyStopWiring::RelayCutoff,
        }),
        topology_category: None,
        required_sensors: vec![SensorRequirement::WaterContact],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Caveat),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Insufficient),
        ])),
    });

    // ── 12n. VTOL Transition (tail-sitter / tilt-rotor / quadplane / bicopter)
    //
    // Per doc §L4 (line 400, px4 PX4 VTOL transition logic), §L5 (line
    // 484: crsf / none / mavlink_wifi), §L5.5 (line 529: hover_hold /
    // 100 ms / esc_dshot_cmd), §"Form Factor → Solution" line 231
    // (dual-mode mix + transition state machine). Advanced/specialty.

    r.register(SolutionDefinition {
        id: "vtol_transition_solution".into(),
        label: "VTOL Transition (Tail-sitter / Tilt-rotor / Quadplane / Bicopter)".into(),
        label_zh: Some("VTOL 过渡控制(尾座 / 倾转旋翼 / 四旋翼+固定翼 / 双旋翼)".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into(), "esp32s3_mini1".into()],
        fixed_inputs: vec![InputSurface::UartRx, InputSurface::I2cSensor, InputSurface::WifiEvent],
        fixed_outputs: vec![OutputSurface::ServoDrive, OutputSurface::MotorDrive, OutputSurface::WifiPacket],
        fixed_orchestration: vec![
            OrchestrationStep { id: "crsf_rx_init".into(), label: "Initialize CRSF UART receiver".into(), label_zh: Some("初始化 CRSF UART 接收机".into()), description: None, description_zh: None, depends_on: vec![] , ..Default::default() },
            OrchestrationStep { id: "imu_baro_calibrate".into(), label: "Calibrate 10-axis IMU + barometer (transition phase needs altitude)".into(), label_zh: Some("校准 10 轴 IMU + 气压计(过渡阶段需要高度)".into()), description: None, description_zh: None, depends_on: vec![] , ..Default::default() },
            OrchestrationStep { id: "transition_sm".into(), label: "Transition state machine: HOVER ↔ TRANSITION ↔ FORWARD; mix dispatched per state".into(), label_zh: Some("过渡状态机:HOVER ↔ TRANSITION ↔ FORWARD;按状态分发混控".into()), description: Some("HOVER uses multirotor mix. FORWARD uses fixed-wing mix. TRANSITION blends both per airspeed/tilt and holds attitude through the corner.".into()), description_zh: Some("HOVER 使用多旋翼混控。FORWARD 使用固定翼混控。TRANSITION 按空速/倾角混合两者并维持姿态平滑过渡。".into()), depends_on: vec!["crsf_rx_init".into(), "imu_baro_calibrate".into()] , ..Default::default() },
        ],
        scheduling: SchedulingPolicy {
            id: "vtol_1khz".into(),
            label: "1 kHz VTOL transition mix".into(),
            decisions: vec![
                "Stabilizer + transition mix at 1 kHz on core 1; MAVLink telemetry on core 0".into(),
                "RX loss in HOVER → motor_cutoff. RX loss in FORWARD → glide_trim until link returns or watchdog".into(),
                "Transition direction inferred from airspeed + commanded tilt".into(),
            ],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "vtol_type".into(),
                label: "VTOL Type".into(),
                label_zh: Some("VTOL 类型".into()),
                required: true,
                secret: false,
                description: "VTOL airframe + transition mechanism".into(),
                description_zh: Some("VTOL 机型与过渡机构。".into()),
                default_value: Some(serde_json::Value::String("tailsitter".into())),
                enum_values: Some(vec![
                    EnumOption { value: "tailsitter".into(), label: "Tail-sitter".into(), description: Some("Pitches whole airframe from hover to forward.".into()) },
                    EnumOption { value: "tiltrotor".into(), label: "Tilt-rotor".into(), description: Some("Rotor pylons tilt vertical → horizontal.".into()) },
                    EnumOption { value: "quadplane".into(), label: "Quadplane".into(), description: Some("Quadcopter for hover + tractor/pusher prop for cruise.".into()) },
                    EnumOption { value: "bicopter".into(), label: "Bicopter".into(), description: Some("2 tilting rotors only.".into()) },
                ]),
                depends_on: None,
            },
            imu_axis_tier_param(),
            imu_chip_param(),
            control_rate_hz_param(),
            failsafe_timeout_ms_param(),
        ],
        feedback_paths: vec![SignalPath {
            id: "crsf_to_vtol".into(),
            name: "CRSF + transition state machine to actuators".into(),
            source: InputSurface::UartRx,
            transforms: vec![
                SignalPathStep { order: 1, node: TransformNode::CommandDispatch, label: Some("CRSF channel decode".into()), description: None },
                SignalPathStep { order: 2, node: TransformNode::StateMachine, label: Some("HOVER / TRANSITION / FORWARD".into()), description: None },
                SignalPathStep { order: 3, node: TransformNode::PidLoop, label: Some("Per-state attitude PID".into()), description: None },
                SignalPathStep { order: 4, node: TransformNode::Mapping, label: Some("VTOL type-specific mix (tailsitter / tiltrotor / quadplane / bicopter)".into()), description: None },
                SignalPathStep { order: 5, node: TransformNode::SafetyInterlock, label: Some("State-aware failsafe (hover_hold vs glide_trim)".into()), description: None },
            ],
            sink: OutputSurface::ServoDrive,
            feedback: vec![SignalPathStep { order: 1, node: FeedbackSurface::PhysicalMotion, label: None, description: None }],
            expected_user_result: "Hover, transition smoothly to forward flight, and back; signal loss handled per current flight state".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle { required: vec!["wifi".into()], optional: vec!["ota".into()] },
        runtime_binding: {
            let mut b = brookesia_vehicle_binding(vec![
                "rshome_motor_control".into(),
                "rshome_imu".into(),
                "rshome_failsafe".into(),
            ]);
            b.board_assembly = Some("esp32s3_va_multirotor_assembly".into());
            b
        },
        external_contracts: vec!["CRSF (ExpressLRS)".into(), "MAVLink v2 over WiFi".into(), "DShot ESC".into()],
        network_topology: NetworkTopology::Star,
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::ControlBoard),
        communication_chains: None,
        pin_assignments: Some(vehicle_control_pins()),
        family: Some(ImplementationFamily::Px4),
        form_factor_families: Some(vec![
            FormFactorKind::VtolTailsitter,
            FormFactorKind::VtolTiltrotor,
            FormFactorKind::VtolQuadplane,
            FormFactorKind::VtolBicopter,
        ]),
        control_uplink: Some(ControlUplinkKind::Crsf),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::MavlinkWifi),
        sensor_tier_min: Some(SensorTierKind::Advanced10ax),
        actuator_family: Some(ActuatorFamily::VtolTransition),
        power_rails: Some(PowerRailKind::TripleMotorServoLogic),
        failsafe: Some(FailsafeInfo {
            killswitch_source: vec![KillswitchSource::RcSwitch, KillswitchSource::RxLoss, KillswitchSource::LowVoltage],
            rx_loss_behavior: Some(RxLossBehavior::HoverHold),
            watchdog_ms: Some(100),
            emergency_stop_wiring: EmergencyStopWiring::EscDshotCmd,
        }),
        topology_category: None,
        required_sensors: vec![],
        companion_link: None,
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Insufficient),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Insufficient),
        ])),
    });

    // ── 12o. ROV Thruster Allocation (4/6 thruster + AUV) ─────────────────
    //
    // Per doc §L4 (line 404, ardupilot ArduSub), §L5 (line 488:
    // wifi_mavlink / webrtc_sbc / mavlink_uart), research_hybrid topology
    // ONLY (SBC handles vision, MCU does thruster allocation + failsafe).

    r.register(SolutionDefinition {
        id: "rov_thruster_allocation_solution".into(),
        label: "ROV Thruster Allocation (Submerged 4/6 thruster)".into(),
        label_zh: Some("ROV 推进器分配(水下 4/6 推进器)".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into(), "esp32s3_mini1".into()],
        fixed_inputs: vec![InputSurface::WifiEvent, InputSurface::I2cSensor, InputSurface::UartRx],
        fixed_outputs: vec![OutputSurface::MotorDrive, OutputSurface::UartTx, OutputSurface::WifiPacket],
        fixed_orchestration: vec![
            OrchestrationStep { id: "wifi_mavlink_init".into(), label: "Initialize tethered WiFi + MAVLink server".into(), label_zh: Some("初始化有缆 WiFi + MAVLink 服务器".into()), description: None, description_zh: None, depends_on: vec![] , ..Default::default() },
            OrchestrationStep { id: "imu_depth_init".into(), label: "Initialize IMU + depth/pressure sensor".into(), label_zh: Some("初始化 IMU + 深度/压力传感器".into()), description: None, description_zh: None, depends_on: vec![] , ..Default::default() },
            OrchestrationStep { id: "sbc_uart_init".into(), label: "Initialize UART link to SBC companion (MAVLink uart for video pipeline)".into(), label_zh: Some("初始化与 SBC 伴侣的 UART 链路(用于视频管线的 MAVLink uart)".into()), description: None, description_zh: None, depends_on: vec![] , ..Default::default() },
            OrchestrationStep { id: "thruster_loop".into(), label: "MAVLink → 6-DOF setpoint → thruster allocation matrix → ESCs".into(), label_zh: Some("MAVLink → 6 自由度设定值 → 推进器分配矩阵 → 电调".into()), description: None, description_zh: None, depends_on: vec!["wifi_mavlink_init".into(), "imu_depth_init".into()] , ..Default::default() },
        ],
        scheduling: SchedulingPolicy {
            id: "rov_holonomic".into(),
            label: "200 Hz 6-DOF allocation loop".into(),
            decisions: vec![
                "Allocation matrix recomputed only when thruster_count config changes".into(),
                "SBC heartbeat loss → unpowered (drift) — surface team retrieves via tether".into(),
                "Low voltage cuts thrusters but not telemetry tether".into(),
            ],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "thruster_layout".into(),
                label: "Thruster Layout".into(),
                label_zh: Some("推进器布局".into()),
                required: true,
                secret: false,
                description: "Number and orientation of thrusters".into(),
                description_zh: Some("推进器数量与方向。".into()),
                default_value: Some(serde_json::Value::String("rov_4thruster".into())),
                enum_values: Some(vec![
                    EnumOption { value: "rov_4thruster".into(), label: "ROV 4-thruster (forward + vertical pair)".into(), description: Some("3-DOF: surge, heave, yaw. Tethered.".into()) },
                    EnumOption { value: "rov_6thruster".into(), label: "ROV 6-thruster (vectored)".into(), description: Some("Holonomic 6-DOF underwater.".into()) },
                    EnumOption { value: "auv_torpedo".into(), label: "AUV Torpedo (1 prop + control fins)".into(), description: Some("Autonomous; no tether.".into()) },
                ]),
                depends_on: None,
            },
            imu_axis_tier_param(),
            imu_chip_param(),
            failsafe_timeout_ms_param(),
        ],
        feedback_paths: vec![SignalPath {
            id: "mavlink_to_thrusters".into(),
            name: "MAVLink 6-DOF setpoint to thruster ESCs".into(),
            source: InputSurface::WifiEvent,
            transforms: vec![
                SignalPathStep { order: 1, node: TransformNode::CommandDispatch, label: Some("MAVLink decode".into()), description: None },
                SignalPathStep { order: 2, node: TransformNode::Mapping, label: Some("Thruster allocation matrix (per layout)".into()), description: None },
                SignalPathStep { order: 3, node: TransformNode::SafetyInterlock, label: Some("SBC heartbeat watchdog → unpowered drift".into()), description: None },
            ],
            sink: OutputSurface::MotorDrive,
            feedback: vec![SignalPathStep { order: 1, node: FeedbackSurface::PhysicalMotion, label: None, description: None }],
            expected_user_result: "Surface MAVLink commands drive 4/6 thrusters via tether; SBC handles vision/RTSP separately".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle { required: vec!["wifi".into()], optional: vec!["ota".into()] },
        runtime_binding: {
            let mut b = brookesia_vehicle_binding(vec![
                "rshome_motor_control".into(),
                "rshome_imu".into(),
                "rshome_failsafe".into(),
                "rshome_interboard".into(),
            ]);
            b.board_assembly = Some("esp32s3_va_sbc_bridge_assembly".into());
            b
        },
        external_contracts: vec!["MAVLink v2 over WiFi (tether)".into(), "MAVLink uart to SBC".into(), "WebRTC video from SBC (out-of-band)".into()],
        network_topology: NetworkTopology::Star,
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::ControlBoard),
        communication_chains: None,
        pin_assignments: Some(dual_mcu_control_board_pins()),
        family: Some(ImplementationFamily::Ardupilot),
        form_factor_families: Some(vec![
            FormFactorKind::Rov4thruster,
            FormFactorKind::Rov6thruster,
            FormFactorKind::AuvTorpedo,
        ]),
        control_uplink: Some(ControlUplinkKind::WifiMavlink),
        video_downlink: Some(VideoDownlinkKind::WebrtcSbc),
        telemetry: Some(TelemetryKind::MavlinkUart),
        sensor_tier_min: Some(SensorTierKind::Standard9ax),
        actuator_family: Some(ActuatorFamily::ThrusterVector),
        power_rails: Some(PowerRailKind::TripleMotorServoLogic),
        failsafe: Some(FailsafeInfo {
            killswitch_source: vec![KillswitchSource::SbcHeartbeatLoss, KillswitchSource::TimeoutNoPacket, KillswitchSource::LowVoltage],
            rx_loss_behavior: Some(RxLossBehavior::Unpowered),
            watchdog_ms: Some(500),
            emergency_stop_wiring: EmergencyStopWiring::RelayCutoff,
        }),
        topology_category: None,
        required_sensors: vec![SensorRequirement::Depth],
        companion_link: Some(CompanionLinkKind::Uart),
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Insufficient),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Insufficient),
        ])),
    });

    // ── 12p. Legged Controller (biped / quadruped / hexapod / octopod) ─────
    //
    // Per doc §L1 line 100 (legged ⚠️ research_hybrid topology ONLY; SBC
    // companion REQUIRED) and §L4 (line 405, custom — SBC handles IK; MCU
    // stub only). MCU does servo timing + failsafe; SBC does IK/gait
    // planning over UART.

    r.register(SolutionDefinition {
        id: "legged_controller_solution".into(),
        label: "Legged Robot Controller (MCU servo bus + SBC IK)".into(),
        label_zh: Some("足式机器人控制器(MCU 舵机总线 + SBC 逆运动学)".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into(), "esp32s3_mini1".into()],
        fixed_inputs: vec![InputSurface::UartRx, InputSurface::I2cSensor, InputSurface::ButtonGpio],
        fixed_outputs: vec![OutputSurface::ServoDrive, OutputSurface::UartTx],
        fixed_orchestration: vec![
            OrchestrationStep { id: "sbc_uart_init".into(), label: "Initialize UART link to SBC (joint targets in, sensor states out)".into(), label_zh: Some("初始化与 SBC 的 UART 链路(关节目标输入,传感器状态输出)".into()), description: None, description_zh: None, depends_on: vec![] , ..Default::default() },
            OrchestrationStep { id: "servo_bus_init".into(), label: "Initialize servo bus (Dynamixel / Robotis / custom serial)".into(), label_zh: Some("初始化舵机总线(Dynamixel / Robotis / 自定义串行)".into()), description: None, description_zh: None, depends_on: vec![] , ..Default::default() },
            OrchestrationStep { id: "imu_calibrate".into(), label: "Calibrate body-frame IMU".into(), label_zh: Some("校准机体坐标 IMU".into()), description: None, description_zh: None, depends_on: vec![] , ..Default::default() },
            OrchestrationStep { id: "servo_loop".into(), label: "200 Hz servo loop: SBC sends joint targets via UART → MCU drives servos with deadline + failsafe".into(), label_zh: Some("200 Hz 舵机循环:SBC 通过 UART 下发关节目标 → MCU 按截止时间驱动舵机并执行失效保护".into()), description: Some("MCU does NOT compute IK or gait — SBC handles those. MCU enforces servo update deadline; if SBC heartbeat lost → unpowered (servos limp).".into()), description_zh: Some("MCU 不执行 IK 或步态计算 — 这些由 SBC 完成。MCU 强制舵机更新截止时间;若 SBC 心跳丢失 → 舵机卸力。".into()), depends_on: vec!["sbc_uart_init".into(), "servo_bus_init".into(), "imu_calibrate".into()] , ..Default::default() },
        ],
        scheduling: SchedulingPolicy {
            id: "legged_servo_bus".into(),
            label: "200 Hz servo bus loop, SBC at higher-level cadence".into(),
            decisions: vec![
                "MCU runs only the servo bus + failsafe; no kinematics".into(),
                "SBC heartbeat loss within 250 ms → cut servo power (unpowered)".into(),
                "Emergency button is wired to the same GPIO that releases servo power".into(),
            ],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "leg_count".into(),
                label: "Leg Count".into(),
                label_zh: Some("腿部数量".into()),
                required: true,
                secret: false,
                description: "Robot leg topology".into(),
                description_zh: Some("机器人腿部拓扑。".into()),
                default_value: Some(serde_json::Value::String("quadruped".into())),
                enum_values: Some(vec![
                    EnumOption { value: "biped".into(), label: "Biped (2)".into(), description: Some("≥12 servos. SBC mandatory.".into()) },
                    EnumOption { value: "quadruped".into(), label: "Quadruped (4)".into(), description: Some("≥12 servos. Spot-style.".into()) },
                    EnumOption { value: "hexapod".into(), label: "Hexapod (6)".into(), description: Some("≥18 servos. Stable gait.".into()) },
                    EnumOption { value: "octopod".into(), label: "Octopod (8)".into(), description: Some("≥24 servos. Specialty.".into()) },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "servo_bus".into(),
                label: "Servo Bus".into(),
                label_zh: Some("舵机总线".into()),
                required: true,
                secret: false,
                description: "Servo communication protocol".into(),
                description_zh: Some("舵机通信协议。".into()),
                default_value: Some(serde_json::Value::String("dynamixel".into())),
                enum_values: Some(vec![
                    EnumOption { value: "dynamixel".into(), label: "Dynamixel TTL".into(), description: Some("Robotis half-duplex serial.".into()) },
                    EnumOption { value: "lx_lewansoul".into(), label: "LX (LewanSoul/Hiwonder)".into(), description: Some("Hobby smart servo.".into()) },
                    EnumOption { value: "pwm_bank".into(), label: "PWM bank (12-24 ch)".into(), description: Some("Plain PWM via PCA9685.".into()) },
                ]),
                depends_on: None,
            },
            imu_axis_tier_param(),
            imu_chip_param(),
            failsafe_timeout_ms_param(),
        ],
        feedback_paths: vec![SignalPath {
            id: "sbc_to_servos".into(),
            name: "SBC joint targets to servo bus".into(),
            source: InputSurface::UartRx,
            transforms: vec![
                SignalPathStep { order: 1, node: TransformNode::UartProtocolParse, label: Some("SBC joint-target frame decode".into()), description: None },
                SignalPathStep { order: 2, node: TransformNode::OneToMany, label: Some("Fan-out to per-leg servos".into()), description: None },
                SignalPathStep { order: 3, node: TransformNode::SafetyInterlock, label: Some("SBC heartbeat watchdog → servo power cut".into()), description: None },
            ],
            sink: OutputSurface::ServoDrive,
            feedback: vec![SignalPathStep { order: 1, node: FeedbackSurface::PhysicalMotion, label: None, description: None }],
            expected_user_result: "SBC commands joint targets; MCU drives servos at deadline; loss of SBC link cuts servo power".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle { required: vec![], optional: vec!["ota".into()] },
        runtime_binding: {
            let mut b = brookesia_vehicle_binding(vec![
                "rshome_motor_control".into(),
                "rshome_imu".into(),
                "rshome_failsafe".into(),
                "rshome_interboard".into(),
            ]);
            b.board_assembly = Some("esp32s3_va_sbc_bridge_assembly".into());
            b
        },
        external_contracts: vec!["UART joint-target protocol (SBC → MCU, 16-bit positions per joint)".into(), "Dynamixel/LX/PWM servo bus".into()],
        network_topology: NetworkTopology::PointToPoint,
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::ControlBoard),
        communication_chains: None,
        pin_assignments: Some(dual_mcu_control_board_pins()),
        family: Some(ImplementationFamily::Custom),
        form_factor_families: Some(vec![
            FormFactorKind::BipedHumanoid,
            FormFactorKind::Quadruped,
            FormFactorKind::Hexapod,
            FormFactorKind::Octopod,
        ]),
        control_uplink: Some(ControlUplinkKind::WifiMavlink),
        video_downlink: Some(VideoDownlinkKind::WebrtcSbc),
        telemetry: Some(TelemetryKind::MavlinkUart),
        sensor_tier_min: Some(SensorTierKind::Standard9ax),
        actuator_family: Some(ActuatorFamily::SteeringServo),
        power_rails: Some(PowerRailKind::TripleMotorServoLogic),
        failsafe: Some(FailsafeInfo {
            killswitch_source: vec![KillswitchSource::SbcHeartbeatLoss, KillswitchSource::EmergencyButton],
            rx_loss_behavior: Some(RxLossBehavior::Unpowered),
            watchdog_ms: Some(250),
            emergency_stop_wiring: EmergencyStopWiring::RelayCutoff,
        }),
        topology_category: None,
        required_sensors: vec![SensorRequirement::JointEncoder],
        companion_link: Some(CompanionLinkKind::Uart),
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Caveat),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Insufficient),
        ])),
    });

    // ── 12q. Articulated Sequencer (snake / crane / excavator / continuum)
    //
    // Per doc §L4 (line 406, custom), §L5 (line 490: wifi_mavlink / none /
    // custom_uart), §"Form Factor → Solution" (lines 235, 238: serial
    // segment coordination + per-joint servo/hydraulic + continuum). Used
    // by snake, crane, excavator, soft gripper, tentacle arm.

    r.register(SolutionDefinition {
        id: "articulated_sequencer_solution".into(),
        label: "Articulated Sequencer (Snake / Crane / Excavator / Continuum)".into(),
        label_zh: Some("关节序列控制器(蛇 / 起重机 / 挖掘机 / 连续体)".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into(), "esp32s3_mini1".into()],
        fixed_inputs: vec![InputSurface::WifiEvent, InputSurface::I2cSensor, InputSurface::ButtonGpio],
        fixed_outputs: vec![OutputSurface::ServoDrive, OutputSurface::RelayDrive, OutputSurface::WifiPacket],
        fixed_orchestration: vec![
            OrchestrationStep { id: "wifi_mavlink_init".into(), label: "Initialize WiFi + MAVLink for ground-station coordination".into(), label_zh: Some("初始化 WiFi + MAVLink 以与地面站协调".into()), description: None, description_zh: None, depends_on: vec![] , ..Default::default() },
            OrchestrationStep { id: "joint_bus_init".into(), label: "Initialize per-joint actuator bus (servo / hydraulic valve / pneumatic chamber)".into(), label_zh: Some("初始化各关节执行器总线(舵机 / 液压阀 / 气动腔)".into()), description: None, description_zh: None, depends_on: vec![] , ..Default::default() },
            OrchestrationStep { id: "sequencer_loop".into(), label: "Sequenced joint update at 100 Hz with phase-locked timing per segment".into(), label_zh: Some("100 Hz 顺序关节更新,各段相位锁定时序".into()), description: None, description_zh: None, depends_on: vec!["wifi_mavlink_init".into(), "joint_bus_init".into()] , ..Default::default() },
        ],
        scheduling: SchedulingPolicy {
            id: "articulated_sequencer".into(),
            label: "100 Hz sequenced joint update".into(),
            decisions: vec![
                "Joints update in fixed phase order (head→tail for snake; base→tip for crane)".into(),
                "Emergency button cuts ALL joints simultaneously regardless of phase".into(),
                "No MCU-side IK — operator (or SBC) computes joint targets".into(),
            ],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "articulation_kind".into(),
                label: "Articulated Platform".into(),
                label_zh: Some("关节平台".into()),
                required: true,
                secret: false,
                description: "Which articulated form factor".into(),
                description_zh: Some("具体关节平台类型。".into()),
                default_value: Some(serde_json::Value::String("snake".into())),
                enum_values: Some(vec![
                    EnumOption { value: "snake".into(), label: "Snake (modular segments)".into(), description: Some("Slither gait via segment yaw + pitch.".into()) },
                    EnumOption { value: "worm".into(), label: "Worm (peristaltic)".into(), description: Some("Sequenced segment expansion/contraction.".into()) },
                    EnumOption { value: "rolling_ball".into(), label: "Rolling Ball (mass-shift)".into(), description: Some("Internal pendulum or mass-shift.".into()) },
                    EnumOption { value: "excavator".into(), label: "Excavator Arm (4+ joints)".into(), description: Some("Boom / stick / bucket / swing.".into()) },
                    EnumOption { value: "crane".into(), label: "Crane (slewing + luff + hoist)".into(), description: Some("3 joints + winch.".into()) },
                    EnumOption { value: "loader".into(), label: "Skid-Steer Loader (bucket tilt/lift)".into(), description: Some("Front-end loader.".into()) },
                    EnumOption { value: "soft_gripper".into(), label: "Soft Gripper (pneumatic)".into(), description: Some("Variable-stiffness chamber control.".into()) },
                    EnumOption { value: "tentacle".into(), label: "Tentacle Arm (cable-driven)".into(), description: Some("Continuum cable control. IK on SBC.".into()) },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "joint_count".into(),
                label: "Joint Count".into(),
                label_zh: Some("关节数量".into()),
                required: true,
                secret: false,
                description: "Number of independently-driven joints".into(),
                description_zh: Some("独立驱动的关节数量。".into()),
                default_value: Some(serde_json::Value::String("8".into())),
                enum_values: Some(vec![
                    EnumOption { value: "3".into(), label: "3 (crane base)".into(), description: None },
                    EnumOption { value: "4".into(), label: "4 (excavator base)".into(), description: None },
                    EnumOption { value: "8".into(), label: "8 (snake / worm)".into(), description: None },
                    EnumOption { value: "16".into(), label: "16 (long snake / continuum)".into(), description: None },
                ]),
                depends_on: None,
            },
            failsafe_timeout_ms_param(),
        ],
        feedback_paths: vec![SignalPath {
            id: "mavlink_to_joints".into(),
            name: "MAVLink joint-target stream to actuator bus".into(),
            source: InputSurface::WifiEvent,
            transforms: vec![
                SignalPathStep { order: 1, node: TransformNode::CommandDispatch, label: Some("MAVLink decode".into()), description: None },
                SignalPathStep { order: 2, node: TransformNode::OneToMany, label: Some("Phase-locked joint update".into()), description: None },
                SignalPathStep { order: 3, node: TransformNode::SafetyInterlock, label: Some("E-stop cuts all joints".into()), description: None },
            ],
            sink: OutputSurface::ServoDrive,
            feedback: vec![SignalPathStep { order: 1, node: FeedbackSurface::PhysicalMotion, label: None, description: None }],
            expected_user_result: "Operator commands joint targets via ground station; MCU sequences updates; e-stop halts all joints".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle { required: vec!["wifi".into()], optional: vec!["ota".into()] },
        runtime_binding: {
            let mut b = brookesia_vehicle_binding(vec![
                "rshome_motor_control".into(),
                "rshome_failsafe".into(),
            ]);
            b.board_assembly = Some("esp32s3_va_sbc_bridge_assembly".into());
            b
        },
        external_contracts: vec!["MAVLink v2 over WiFi".into(), "Per-segment servo / hydraulic valve protocol".into()],
        network_topology: NetworkTopology::Star,
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::ControlTelemetryBoard),
        communication_chains: None,
        pin_assignments: Some(vehicle_control_pins()),
        family: Some(ImplementationFamily::Custom),
        form_factor_families: Some(vec![
            FormFactorKind::SnakeSerpentine,
            FormFactorKind::WormModular,
            FormFactorKind::RollingBall,
            // ModularReconfigurable retired — see platform.rs comment.
            FormFactorKind::ExcavatorArm,
            FormFactorKind::CraneBoom,
            FormFactorKind::SkidSteerLoader,
            FormFactorKind::SoftGripper,
            FormFactorKind::TentacleArm,
        ]),
        control_uplink: Some(ControlUplinkKind::WifiMavlink),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::CustomUart),
        sensor_tier_min: Some(SensorTierKind::Standard9ax),
        actuator_family: Some(ActuatorFamily::HydraulicJoint),
        power_rails: Some(PowerRailKind::TripleMotorServoLogic),
        failsafe: Some(FailsafeInfo {
            killswitch_source: vec![KillswitchSource::TimeoutNoPacket, KillswitchSource::EmergencyButton],
            rx_loss_behavior: Some(RxLossBehavior::Unpowered),
            watchdog_ms: Some(500),
            emergency_stop_wiring: EmergencyStopWiring::GpioPulldown,
        }),
        topology_category: None,
        required_sensors: vec![
            SensorRequirement::JointEncoder,
            SensorRequirement::PressureStrain,
        ],
        companion_link: Some(CompanionLinkKind::Can),
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Caveat),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Insufficient),
        ])),
    });

    // ── 12r. Climbing Controller (suction / cable / magnetic) ─────────────
    //
    // Per doc §L1 line 188-190 (climbing form factors with adhesion-state
    // sensor floor) and §"Form Factor → Solution" line 236 (climbing_
    // controller_solution: wheeled/legged drive + adhesion control loop;
    // adhesion is safety-critical — lose suction → immediate failsafe
    // detach).

    r.register(SolutionDefinition {
        id: "climbing_controller_solution".into(),
        label: "Climbing Robot (Suction / Cable / Magnetic)".into(),
        label_zh: Some("攀爬机器人(吸附 / 缆绳 / 磁吸)".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into(), "esp32s3_mini1".into()],
        fixed_inputs: vec![InputSurface::WifiEvent, InputSurface::I2cSensor, InputSurface::AdcVoltage, InputSurface::ButtonGpio],
        fixed_outputs: vec![OutputSurface::MotorDrive, OutputSurface::RelayDrive, OutputSurface::WifiPacket],
        fixed_orchestration: vec![
            OrchestrationStep { id: "wifi_mavlink_init".into(), label: "Initialize WiFi + MAVLink".into(), label_zh: Some("初始化 WiFi + MAVLink".into()), description: None, description_zh: None, depends_on: vec![] , ..Default::default() },
            OrchestrationStep { id: "adhesion_init".into(), label: "Initialize adhesion sensor (suction pressure / magnet current / cable tension)".into(), label_zh: Some("初始化附着传感器(吸盘压力 / 磁铁电流 / 缆绳张力)".into()), description: Some("Adhesion is safety-critical — sensor must produce a valid reading before drive enable.".into()), description_zh: Some("附着是安全关键 — 传感器必须产生有效读数后才允许驱动使能。".into()), depends_on: vec![] , ..Default::default() },
            OrchestrationStep { id: "drive_loop".into(), label: "Drive + adhesion control loop @ 100 Hz; lose adhesion → stop drive + alarm".into(), label_zh: Some("100 Hz 驱动 + 附着控制循环;失去附着 → 停止驱动 + 告警".into()), description: None, description_zh: None, depends_on: vec!["adhesion_init".into()] , ..Default::default() },
        ],
        scheduling: SchedulingPolicy {
            id: "climbing_safety_first".into(),
            label: "100 Hz drive loop with adhesion-priority interlock".into(),
            decisions: vec![
                "Adhesion sensor read at 200 Hz; out-of-range for >50 ms triggers full stop".into(),
                "Drive output gated by adhesion-OK signal — no command can override".into(),
                "Operator e-stop cuts both drive AND adhesion (controlled detach)".into(),
            ],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "adhesion_kind".into(),
                label: "Adhesion Mechanism".into(),
                label_zh: Some("附着机构".into()),
                required: true,
                secret: false,
                description: "How the robot stays attached to the surface".into(),
                description_zh: Some("机器人附着于表面的方式。".into()),
                default_value: Some(serde_json::Value::String("suction".into())),
                enum_values: Some(vec![
                    EnumOption { value: "suction".into(), label: "Vacuum Suction".into(), description: Some("Pad feet or continuous fan. Pressure sensor required.".into()) },
                    EnumOption { value: "cable".into(), label: "Cable Climbing".into(), description: Some("Drive rollers clamp cable. Tension sensor.".into()) },
                    EnumOption { value: "magnetic".into(), label: "Switchable Electromagnets".into(), description: Some("Ferrous structures only. Magnet current sensed.".into()) },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "adhesion_threshold_pct".into(),
                label: "Adhesion Threshold (%)".into(),
                label_zh: Some("附着阈值(%)".into()),
                required: false,
                secret: false,
                description: "Minimum acceptable adhesion strength as percent of nominal".into(),
                description_zh: Some("最低可接受的附着强度(标称值的百分比)。".into()),
                default_value: Some(serde_json::Value::String("70".into())),
                enum_values: Some(vec![
                    EnumOption { value: "50".into(), label: "50% (permissive)".into(), description: Some("Risk of fall.".into()) },
                    EnumOption { value: "70".into(), label: "70% (standard)".into(), description: None },
                    EnumOption { value: "85".into(), label: "85% (strict)".into(), description: Some("More false alarms but safer.".into()) },
                ]),
                depends_on: None,
            },
            imu_axis_tier_param(),
            imu_chip_param(),
            failsafe_timeout_ms_param(),
        ],
        feedback_paths: vec![SignalPath {
            id: "adhesion_to_drive".into(),
            name: "Adhesion sensor + MAVLink command → safety-gated drive".into(),
            source: InputSurface::AdcVoltage,
            transforms: vec![
                SignalPathStep { order: 1, node: TransformNode::Filter, label: Some("Adhesion sensor low-pass".into()), description: None },
                SignalPathStep { order: 2, node: TransformNode::Threshold, label: Some("Adhesion-OK threshold".into()), description: None },
                SignalPathStep { order: 3, node: TransformNode::SafetyInterlock, label: Some("Drive enable gated by adhesion-OK".into()), description: None },
                SignalPathStep { order: 4, node: TransformNode::CommandDispatch, label: Some("MAVLink drive command (only when gate is open)".into()), description: None },
            ],
            sink: OutputSurface::MotorDrive,
            feedback: vec![
                SignalPathStep { order: 1, node: FeedbackSurface::LedIndicator, label: Some("Adhesion-OK indicator".into()), description: None },
                SignalPathStep { order: 2, node: FeedbackSurface::SoundFeedback, label: Some("Beeper on adhesion alarm".into()), description: None },
            ],
            expected_user_result: "Robot drives only when adhesion is good; loss of adhesion immediately halts drive and alerts operator".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle { required: vec!["wifi".into()], optional: vec!["ota".into()] },
        runtime_binding: brookesia_vehicle_binding(vec![
            "rshome_motor_control".into(),
            "rshome_imu".into(),
            "rshome_failsafe".into(),
        ]),
        external_contracts: vec!["MAVLink v2 over WiFi".into(), "Adhesion sensor (analog)".into()],
        network_topology: NetworkTopology::Star,
        domain: Some(DomainKind::VehicleAircraftControl),
        architecture_tier: Some(McuRole::ControlBoard),
        communication_chains: None,
        pin_assignments: Some(vehicle_car_pins()),
        family: Some(ImplementationFamily::Custom),
        form_factor_families: Some(vec![
            FormFactorKind::WallClimbingSuction,
            FormFactorKind::CableClimbing,
            FormFactorKind::MagneticClimber,
        ]),
        control_uplink: Some(ControlUplinkKind::WifiMavlink),
        video_downlink: Some(VideoDownlinkKind::None),
        telemetry: Some(TelemetryKind::MavlinkWifi),
        sensor_tier_min: Some(SensorTierKind::Standard9ax),
        actuator_family: Some(ActuatorFamily::MixedDiffDrive),
        power_rails: Some(PowerRailKind::TripleMotorServoLogic),
        failsafe: Some(FailsafeInfo {
            killswitch_source: vec![
                KillswitchSource::RxLoss,
                KillswitchSource::TimeoutNoPacket,
                KillswitchSource::EmergencyButton,
                KillswitchSource::LowVoltage,
            ],
            rx_loss_behavior: Some(RxLossBehavior::MotorCutoff),
            watchdog_ms: Some(250),
            emergency_stop_wiring: EmergencyStopWiring::RelayCutoff,
        }),
        topology_category: None,
        required_sensors: vec![SensorRequirement::Adhesion],
        // Climbing robots routinely offload vision-based adhesion feedback
        // (surface imaging, magnet-current profiling) to a companion SBC;
        // the MCU handles real-time safety / adhesion-interlock, the SBC
        // handles heavier per-frame analysis. Phase 3 T3.2 initially missed
        // this row; caught by `va_companion_link_presence.rs` heuristic.
        companion_link: Some(CompanionLinkKind::Uart),
        chip_coverage: Some(BTreeMap::from([
            (ChipFamilyKind::Esp32S3, ChipCoverageStatus::Preferred),
            (ChipFamilyKind::Esp32C6, ChipCoverageStatus::Caveat),
            (ChipFamilyKind::Esp32D0wd, ChipCoverageStatus::Insufficient),
        ])),
    });

    // ── 13. ESP-NOW Sensor (P2P, battery-powered) ──────────────────────────

    r.register(SolutionDefinition {
        id: "esp_now_sensor_solution".into(),
        label: "ESP-NOW Sensor (Point-to-Point)".into(),
        label_zh: Some("ESP-NOW 传感器(点对点)".into()),
        kind: SolutionKind::ConnectivityBridge,
        supported_modules: vec!["esp32c6_wroom1".into(), "esp32s3_wroom1".into()],
        fixed_inputs: vec![InputSurface::EspNowData, InputSurface::ServiceCall],
        fixed_outputs: vec![OutputSurface::EspNowData],
        fixed_orchestration: vec![],
        scheduling: SchedulingPolicy {
            id: "periodic_wake".into(),
            label: "Periodic deep-sleep wake + transmit".into(),
            decisions: vec!["Wake → read sensor → ESP-NOW send → sleep".into()],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "peer_mac".into(),
                label: "Peer MAC Address".into(),
                label_zh: Some("对端 MAC 地址".into()),
                required: true,
                secret: false,
                description: "MAC address of the receiving device".into(),
                description_zh: Some("接收设备的 MAC 地址。".into()),
                default_value: None,
                enum_values: None,
                depends_on: None,
            },
            UserParameterDefinition {
                id: "sleep_interval_s".into(),
                label: "Sleep Interval (s)".into(),
                label_zh: Some("休眠间隔(秒)".into()),
                required: false,
                secret: false,
                description: "Deep sleep interval in seconds".into(),
                description_zh: Some("深度睡眠的间隔,单位秒。".into()),
                default_value: Some(serde_json::Value::String("60".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "10".into(),
                        label: "10 s".into(),
                        description: Some("Fast updates.".into()),
                    },
                    EnumOption {
                        value: "30".into(),
                        label: "30 s".into(),
                        description: Some("Moderate.".into()),
                    },
                    EnumOption {
                        value: "60".into(),
                        label: "60 s".into(),
                        description: Some("Standard.".into()),
                    },
                    EnumOption {
                        value: "300".into(),
                        label: "5 min".into(),
                        description: Some("Battery saving.".into()),
                    },
                ]),
                depends_on: None,
            },
        ],
        feedback_paths: vec![SignalPath {
            id: "sensor_to_espnow".into(),
            name: "Sensor reading via ESP-NOW".into(),
            source: InputSurface::EspNowData,
            transforms: vec![],
            sink: OutputSurface::EspNowData,
            feedback: vec![SignalPathStep {
                order: 1,
                node: FeedbackSurface::LedIndicator,
                label: None,
                description: None,
            }],
            expected_user_result: "Sensor data sent to peer via ESP-NOW".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec![],
            optional: vec![],
        },
        runtime_binding: RuntimeBinding {
            family: Some("brookesia_service".into()),
            managed_components: vec![ManagedComponentDep {
                name: "esp-now".into(),
                version: Some("*".into()),
                git: None,
                namespace: Some("espressif".into()),
            }],
            codegen_path: CodegenPath::BrookesiaManaged,
            ..Default::default()
        },
        external_contracts: vec!["ESP-NOW protocol".into()],
        network_topology: NetworkTopology::PointToPoint,
        domain: Some(DomainKind::IotDeviceTooling),
        architecture_tier: None,
        communication_chains: None,
        pin_assignments: None,
        family: Some(ImplementationFamily::BrookesiaService),
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
    });

    // ── 14. MQTT Sensor Gateway (IoT) ────────────────────────────────────

    r.register(SolutionDefinition {
        id: "mqtt_gateway_bridge".into(),
        label: "MQTT Sensor Gateway".into(),
        label_zh: Some("MQTT 传感器网关".into()),
        kind: SolutionKind::ConnectivityBridge,
        supported_modules: vec!["esp32s3_wroom1".into(), "esp32c6_wroom1".into()],
        fixed_inputs: vec![
            InputSurface::I2cSensor,
            InputSurface::AdcVoltage,
            InputSurface::EspNowData,
            InputSurface::MqttMessage,
        ],
        fixed_outputs: vec![],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "wifi_connect".into(),
                label: "Connect to WiFi (STA mode)".into(),
                label_zh: Some("连接 WiFi（STA 模式）".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "mqtt_connect".into(),
                label: "Establish MQTT connection to broker".into(),
                label_zh: Some("建立到 MQTT Broker 的连接".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["wifi_connect".into()],
                ..Default::default()
            },
            OrchestrationStep {
                id: "poll_or_receive".into(),
                label: "Poll local sensors or receive ESP-NOW frames".into(),
                label_zh: Some("轮询本地传感器或接收 ESP-NOW 帧".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "mqtt_publish".into(),
                label: "Publish JSON payload to MQTT topic".into(),
                label_zh: Some("向 MQTT 主题发布 JSON 数据".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["mqtt_connect".into(), "poll_or_receive".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "periodic_publish".into(),
            label: "Periodic sensor poll + MQTT publish".into(),
            decisions: vec!["Poll interval per sensor, publish on change or interval".into()],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "broker_port".into(),
                label: "Broker Port".into(),
                label_zh: Some("Broker 端口".into()),
                required: false,
                secret: false,
                description: "MQTT broker port".into(),
                description_zh: Some("MQTT Broker 端口。".into()),
                default_value: Some(serde_json::Value::String("1883".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "1883".into(),
                        label: "1883 (Standard)".into(),
                        description: Some("Unencrypted MQTT.".into()),
                    },
                    EnumOption {
                        value: "8883".into(),
                        label: "8883 (TLS)".into(),
                        description: Some("TLS-encrypted MQTT.".into()),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "mqtt_transport".into(),
                label: "Transport".into(),
                label_zh: Some("传输方式".into()),
                required: false,
                secret: false,
                description: "MQTT transport protocol".into(),
                description_zh: Some("MQTT 传输协议。".into()),
                default_value: Some(serde_json::Value::String("tcp".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "tcp".into(),
                        label: "TCP".into(),
                        description: Some("Plain TCP. Suitable for LAN.".into()),
                    },
                    EnumOption {
                        value: "tls".into(),
                        label: "TLS".into(),
                        description: Some("TLS encryption. Required for WAN.".into()),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "data_source".into(),
                label: "Data Source".into(),
                label_zh: Some("数据来源".into()),
                required: true,
                secret: false,
                description: "Where sensor data comes from".into(),
                description_zh: Some("传感器数据的来源。".into()),
                default_value: Some(serde_json::Value::String("local_sensors".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "local_sensors".into(),
                        label: "Local Sensors".into(),
                        description: Some("I2C/SPI/ADC sensors wired to this device.".into()),
                    },
                    EnumOption {
                        value: "espnow_relay".into(),
                        label: "ESP-NOW Relay".into(),
                        description: Some("Receive from battery ESP-NOW sensor nodes.".into()),
                    },
                    EnumOption {
                        value: "both".into(),
                        label: "Both".into(),
                        description: Some("Local sensors + ESP-NOW relay.".into()),
                    },
                ]),
                depends_on: None,
            },
            {
                let mut p = iot_poll_interval_param();
                p.depends_on = Some(ParameterDependency {
                    parameter_id: "data_source".into(),
                    when_value: String::new(),
                    when_not_value: Some("espnow_relay".into()),
                });
                p
            },
        ],
        feedback_paths: vec![SignalPath {
            id: "sensor_to_mqtt".into(),
            name: "Sensor readings to MQTT broker".into(),
            source: InputSurface::I2cSensor,
            transforms: vec![
                SignalPathStep {
                    order: 1,
                    node: TransformNode::SensorFrameDecode,
                    label: None,
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: TransformNode::Filter,
                    label: None,
                    description: None,
                },
            ],
            sink: OutputSurface::WifiPacket,
            feedback: vec![SignalPathStep {
                order: 1,
                node: FeedbackSurface::MqttPublish,
                label: None,
                description: None,
            }],
            expected_user_result: "Sensor readings published to MQTT topics".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["wifi".into()],
            optional: vec!["sensor".into(), "i2c".into(), "spi".into(), "ota".into()],
        },
        runtime_binding: RuntimeBinding {
            family: Some("brookesia_service".into()),
            managed_components: vec![ManagedComponentDep {
                name: "esp_mqtt".into(),
                version: Some("*".into()),
                git: None,
                namespace: Some("espressif".into()),
            }],
            codegen_path: CodegenPath::BrookesiaManaged,
            ..Default::default()
        },
        external_contracts: vec!["MQTT 3.1.1 / 5.0 broker".into()],
        network_topology: NetworkTopology::default(),
        domain: Some(DomainKind::IotDeviceTooling),
        architecture_tier: None,
        communication_chains: None,
        pin_assignments: None,
        family: Some(ImplementationFamily::BrookesiaService),
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
    });

    // ── 15. BLE Scanner Proxy (IoT) ───────────────────────────────────────

    r.register(SolutionDefinition {
        id: "ble_scanner_proxy".into(),
        label: "BLE Scanner Proxy".into(),
        label_zh: Some("BLE 扫描代理".into()),
        kind: SolutionKind::ConnectivityBridge,
        supported_modules: vec!["esp32s3_wroom1".into(), "esp32c6_wroom1".into()],
        fixed_inputs: vec![InputSurface::BleEvent, InputSurface::ServiceCall],
        fixed_outputs: vec![OutputSurface::NetworkApiState],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "ble_init".into(),
                label: "Initialize BLE scanner".into(),
                label_zh: Some("初始化 BLE 扫描器".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "scan_advertisements".into(),
                label: "Scan for BLE advertisements".into(),
                label_zh: Some("扫描 BLE 广播".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["ble_init".into()],
                ..Default::default()
            },
            OrchestrationStep {
                id: "parse_payload".into(),
                label: "Decode manufacturer-specific data".into(),
                label_zh: Some("解码厂商特定数据".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["scan_advertisements".into()],
                ..Default::default()
            },
            OrchestrationStep {
                id: "publish_state".into(),
                label: "Relay parsed readings to rshome-ha or MQTT".into(),
                label_zh: Some("将解析后的数据上报到 rshome-ha 或 MQTT".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["parse_payload".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "continuous_scan".into(),
            label: "Continuous BLE scan + uplink publish".into(),
            decisions: vec!["Scan window/interval, filter by device type".into()],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "scan_mode".into(),
                label: "Scan Mode".into(),
                label_zh: Some("扫描模式".into()),
                required: false,
                secret: false,
                description: "BLE scan type".into(),
                description_zh: Some("BLE 扫描类型。".into()),
                default_value: Some(serde_json::Value::String("passive".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "passive".into(),
                        label: "Passive".into(),
                        description: Some("Listen only. Lower power, no scan request sent.".into()),
                    },
                    EnumOption {
                        value: "active".into(),
                        label: "Active".into(),
                        description: Some("Sends scan request. Gets scan response data.".into()),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "scan_interval_ms".into(),
                label: "Scan Interval (ms)".into(),
                label_zh: Some("扫描间隔(毫秒)".into()),
                required: false,
                secret: false,
                description: "BLE scan interval in milliseconds".into(),
                description_zh: Some("BLE 扫描间隔,单位毫秒。".into()),
                default_value: Some(serde_json::Value::String("320".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "320".into(),
                        label: "320 ms".into(),
                        description: Some("Fast. Best detection rate.".into()),
                    },
                    EnumOption {
                        value: "640".into(),
                        label: "640 ms".into(),
                        description: Some("Standard.".into()),
                    },
                    EnumOption {
                        value: "1280".into(),
                        label: "1280 ms".into(),
                        description: Some("Moderate. Balanced power.".into()),
                    },
                    EnumOption {
                        value: "5000".into(),
                        label: "5000 ms".into(),
                        description: Some("Low power. May miss infrequent adverts.".into()),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "device_filter".into(),
                label: "Device Filter".into(),
                label_zh: Some("设备过滤器".into()),
                required: false,
                secret: false,
                description: "Filter BLE advertisements by device type".into(),
                description_zh: Some("按设备类型过滤 BLE 广播。".into()),
                default_value: Some(serde_json::Value::String("all".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "all".into(),
                        label: "All Devices".into(),
                        description: Some("No filter. Report all visible BLE devices.".into()),
                    },
                    EnumOption {
                        value: "xiaomi_miflora".into(),
                        label: "Xiaomi MiFlora".into(),
                        description: Some(
                            "Plant sensor (soil moisture, light, temperature).".into(),
                        ),
                    },
                    EnumOption {
                        value: "switchbot".into(),
                        label: "SwitchBot".into(),
                        description: Some("SwitchBot curtain, plug, meter, etc.".into()),
                    },
                    EnumOption {
                        value: "ruuvi".into(),
                        label: "Ruuvi".into(),
                        description: Some("RuuviTag environmental sensor.".into()),
                    },
                    EnumOption {
                        value: "ibeacon".into(),
                        label: "iBeacon".into(),
                        description: Some("Apple iBeacon proximity beacons.".into()),
                    },
                    EnumOption {
                        value: "custom_mac".into(),
                        label: "Custom MAC".into(),
                        description: Some("Filter by specific MAC address.".into()),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "target_mac".into(),
                label: "Target MAC Address".into(),
                label_zh: Some("目标 MAC 地址".into()),
                required: false,
                secret: false,
                description: "MAC address to filter (e.g. AA:BB:CC:DD:EE:FF)".into(),
                description_zh: Some("要过滤的 MAC 地址（如 AA:BB:CC:DD:EE:FF）。".into()),
                default_value: None,
                enum_values: None,
                depends_on: Some(ParameterDependency {
                    parameter_id: "device_filter".into(),
                    when_value: "custom_mac".into(),
                    when_not_value: None,
                }),
            },
            uplink_protocol_param(),
        ],
        feedback_paths: vec![SignalPath {
            id: "ble_to_uplink".into(),
            name: "BLE advertisement to HA/MQTT".into(),
            source: InputSurface::BleEvent,
            transforms: vec![
                SignalPathStep {
                    order: 1,
                    node: TransformNode::SensorFrameDecode,
                    label: None,
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: TransformNode::Filter,
                    label: None,
                    description: None,
                },
            ],
            sink: OutputSurface::NetworkApiState,
            feedback: vec![SignalPathStep {
                order: 1,
                node: FeedbackSurface::ApiState,
                label: None,
                description: None,
            }],
            expected_user_result: "BLE sensor readings appear in rshome-ha or MQTT topics".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["wifi".into()],
            optional: vec![
                "sensor".into(),
                "binary_sensor".into(),
                "api".into(),
                "ota".into(),
            ],
        },
        runtime_binding: {
            let mut rb = RuntimeBinding {
                family: Some("brookesia_service".into()),
                managed_components: vec![ManagedComponentDep {
                    name: "esp_ble".into(),
                    version: Some("*".into()),
                    git: None,
                    namespace: Some("espressif".into()),
                }],
                codegen_path: CodegenPath::BrookesiaManaged,
                ..Default::default()
            };
            rb.ha_entities = vec![
                crate::ha_export::HaEntityExportDefinition::sensor_entity(
                    "ble_temperature",
                    "BLE Temperature",
                    "temperature",
                    "°C",
                    crate::ha_export::StateBinding {
                        source_event: "ble_scanner_0.reading".into(),
                        field_map: BTreeMap::from([("temperature".into(), "value".into())]),
                    },
                ),
                crate::ha_export::HaEntityExportDefinition::sensor_entity(
                    "ble_humidity",
                    "BLE Humidity",
                    "humidity",
                    "%",
                    crate::ha_export::StateBinding {
                        source_event: "ble_scanner_0.reading".into(),
                        field_map: BTreeMap::from([("humidity".into(), "value".into())]),
                    },
                ),
                crate::ha_export::HaEntityExportDefinition::sensor_entity(
                    "ble_battery",
                    "BLE Battery",
                    "battery",
                    "%",
                    crate::ha_export::StateBinding {
                        source_event: "ble_scanner_0.reading".into(),
                        field_map: BTreeMap::from([("battery".into(), "value".into())]),
                    },
                ),
                crate::ha_export::HaEntityExportDefinition::sensor_entity(
                    "ble_rssi",
                    "BLE Signal Strength",
                    "signal_strength",
                    "dBm",
                    crate::ha_export::StateBinding {
                        source_event: "ble_scanner_0.reading".into(),
                        field_map: BTreeMap::from([("rssi".into(), "value".into())]),
                    },
                ),
            ];
            rb
        },
        external_contracts: vec!["rshome-ha Native API".into()],
        network_topology: NetworkTopology::default(),
        domain: Some(DomainKind::IotDeviceTooling),
        architecture_tier: None,
        communication_chains: None,
        pin_assignments: None,
        family: Some(ImplementationFamily::BrookesiaService),
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
    });

    // ── 16. Environment Data Logger (IoT — Sensing & Acquisition) ────────

    r.register(SolutionDefinition {
        id: "env_data_logger".into(),
        label: "Environment Data Logger".into(),
        label_zh: Some("环境数据记录器".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into(), "esp32c6_wroom1".into()],
        fixed_inputs: vec![
            InputSurface::I2cSensor,
            InputSurface::SpiSensor,
            InputSurface::AdcVoltage,
            InputSurface::TimerTick,
        ],
        fixed_outputs: vec![
            OutputSurface::SpiMasterWrite,
            OutputSurface::UsbTx,
            OutputSurface::NetworkApiState,
        ],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "init_storage".into(),
                label: "Initialize storage backend (SD/flash)".into(),
                label_zh: Some("初始化存储后端（SD/Flash）".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "init_sensors".into(),
                label: "Initialize I2C/SPI/ADC sensors".into(),
                label_zh: Some("初始化 I2C/SPI/ADC 传感器".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "start_logging".into(),
                label: "Start periodic logging loop".into(),
                label_zh: Some("启动周期性记录循环".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["init_storage".into(), "init_sensors".into()],
                ..Default::default()
            },
            OrchestrationStep {
                id: "batch_upload".into(),
                label: "Batch upload via WiFi (optional)".into(),
                label_zh: Some("通过 WiFi 批量上传（可选）".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["start_logging".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "periodic_log".into(),
            label: "Periodic sensor poll + log to storage".into(),
            decisions: vec!["Log interval per sensor, upload on schedule or manual".into()],
        },
        user_parameters: vec![
            {
                let mut p = iot_poll_interval_param();
                p.id = "log_interval_ms".into();
                p.label = "Log Interval (ms)".into();
                p.label_zh = Some("记录间隔(毫秒)".into());
                p.description = "How often to log sensor readings".into();
                p.description_zh = Some("传感器记录间隔,单位毫秒。".into());
                p
            },
            storage_backend_param(),
            upload_mode_param(),
            UserParameterDefinition {
                id: "max_records".into(),
                label: "Max Records".into(),
                label_zh: Some("最大记录数".into()),
                required: false,
                secret: false,
                description: "Maximum number of records before overwriting oldest".into(),
                description_zh: Some("超过此数量后覆盖最旧的记录。".into()),
                default_value: Some(serde_json::Value::String("10000".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "1000".into(),
                        label: "1,000".into(),
                        description: Some("Small buffer.".into()),
                    },
                    EnumOption {
                        value: "10000".into(),
                        label: "10,000".into(),
                        description: Some("Standard.".into()),
                    },
                    EnumOption {
                        value: "100000".into(),
                        label: "100,000".into(),
                        description: Some("Large buffer (SD only).".into()),
                    },
                ]),
                depends_on: None,
            },
        ],
        feedback_paths: vec![SignalPath {
            id: "sensor_to_storage".into(),
            name: "Sensor reading to local storage".into(),
            source: InputSurface::I2cSensor,
            transforms: vec![
                SignalPathStep {
                    order: 1,
                    node: TransformNode::SensorFrameDecode,
                    label: None,
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: TransformNode::Filter,
                    label: None,
                    description: None,
                },
            ],
            sink: OutputSurface::SpiMasterWrite,
            feedback: vec![
                SignalPathStep {
                    order: 1,
                    node: FeedbackSurface::LedIndicator,
                    label: None,
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: FeedbackSurface::SerialLog,
                    label: None,
                    description: None,
                },
            ],
            expected_user_result: "Sensor readings logged to SD card or flash with timestamps"
                .into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["spi".into(), "sensor".into()],
            optional: vec!["wifi".into(), "i2c".into(), "ota".into()],
        },
        runtime_binding: RuntimeBinding {
            family: Some("brookesia_service".into()),
            managed_components: vec![ManagedComponentDep {
                name: "esp_vfs_fat".into(),
                version: Some("*".into()),
                git: None,
                namespace: Some("espressif".into()),
            }],
            codegen_path: CodegenPath::BrookesiaManaged,
            ..Default::default()
        },
        external_contracts: vec![],
        network_topology: NetworkTopology::Star,
        domain: Some(DomainKind::IotDeviceTooling),
        architecture_tier: None,
        communication_chains: None,
        pin_assignments: Some(iot_sensor_pins()),
        family: Some(ImplementationFamily::BrookesiaService),
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
    });

    // ── 17. Thread/Zigbee Sensor (IoT — Sensing & Acquisition) ────────────

    r.register(SolutionDefinition {
        id: "thread_zigbee_sensor".into(),
        label: "Thread/Zigbee Sensor".into(),
        label_zh: Some("Thread/Zigbee 传感器".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32c6_wroom1".into(), "esp32c6_mini1".into()],
        fixed_inputs: vec![
            InputSurface::I2cSensor,
            InputSurface::AdcVoltage,
            InputSurface::ButtonGpio,
        ],
        fixed_outputs: vec![OutputSurface::NetworkApiState],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "thread_join".into(),
                label: "Join Thread/Zigbee network".into(),
                label_zh: Some("加入 Thread/Zigbee 网络".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "sensor_init".into(),
                label: "Initialize sensors".into(),
                label_zh: Some("初始化传感器".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "start_reporting".into(),
                label: "Start periodic reporting".into(),
                label_zh: Some("启动周期性上报".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["thread_join".into(), "sensor_init".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "periodic_report".into(),
            label: "Periodic sensor report over Thread/Zigbee".into(),
            decisions: vec!["Report interval, sleep between reports".into()],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "protocol".into(),
                label: "Protocol".into(),
                label_zh: Some("协议".into()),
                required: true,
                secret: false,
                description: "Network protocol for this sensor".into(),
                description_zh: Some("传感器使用的网络协议。".into()),
                default_value: Some(serde_json::Value::String("thread".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "thread".into(),
                        label: "Thread 1.3".into(),
                        description: Some("IPv6 mesh. Matter-compatible.".into()),
                    },
                    EnumOption {
                        value: "zigbee".into(),
                        label: "Zigbee 3.0".into(),
                        description: Some("IEEE 802.15.4 star topology.".into()),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "sensor_type".into(),
                label: "Sensor Type".into(),
                label_zh: Some("传感器类型".into()),
                required: true,
                secret: false,
                description: "Primary sensor function".into(),
                description_zh: Some("主要传感器功能。".into()),
                default_value: Some(serde_json::Value::String("temperature".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "temperature".into(),
                        label: "Temperature/Humidity".into(),
                        description: Some("BME280/SHT3x environmental sensor.".into()),
                    },
                    EnumOption {
                        value: "motion".into(),
                        label: "Motion (PIR)".into(),
                        description: Some("PIR motion detector.".into()),
                    },
                    EnumOption {
                        value: "door_window".into(),
                        label: "Door/Window".into(),
                        description: Some("Reed switch contact sensor.".into()),
                    },
                    EnumOption {
                        value: "light".into(),
                        label: "Light Level".into(),
                        description: Some("Ambient light sensor (BH1750/TSL2561).".into()),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "report_interval_s".into(),
                label: "Report Interval (s)".into(),
                label_zh: Some("上报间隔(秒)".into()),
                required: false,
                secret: false,
                description: "How often to report sensor readings".into(),
                description_zh: Some("传感器数据上报间隔,单位秒。".into()),
                default_value: Some(serde_json::Value::String("60".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "10".into(),
                        label: "10 s".into(),
                        description: Some("Fast updates.".into()),
                    },
                    EnumOption {
                        value: "30".into(),
                        label: "30 s".into(),
                        description: Some("Moderate.".into()),
                    },
                    EnumOption {
                        value: "60".into(),
                        label: "60 s".into(),
                        description: Some("Standard.".into()),
                    },
                    EnumOption {
                        value: "300".into(),
                        label: "5 min".into(),
                        description: Some("Battery saving.".into()),
                    },
                ]),
                depends_on: None,
            },
        ],
        feedback_paths: vec![SignalPath {
            id: "sensor_to_thread".into(),
            name: "Sensor reading via Thread/Zigbee".into(),
            source: InputSurface::I2cSensor,
            transforms: vec![
                SignalPathStep {
                    order: 1,
                    node: TransformNode::SensorFrameDecode,
                    label: None,
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: TransformNode::Calibration,
                    label: None,
                    description: None,
                },
            ],
            sink: OutputSurface::NetworkApiState,
            feedback: vec![
                SignalPathStep {
                    order: 1,
                    node: FeedbackSurface::LedIndicator,
                    label: None,
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: FeedbackSurface::ApiState,
                    label: None,
                    description: None,
                },
            ],
            expected_user_result:
                "Sensor readings delivered to Thread Border Router or Zigbee coordinator".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec![],
            optional: vec!["sensor".into(), "binary_sensor".into(), "i2c".into()],
        },
        runtime_binding: RuntimeBinding {
            family: Some("brookesia_service".into()),
            managed_components: vec![
                ManagedComponentDep {
                    name: "esp_openthread".into(),
                    version: Some("*".into()),
                    git: None,
                    namespace: Some("espressif".into()),
                },
                ManagedComponentDep {
                    name: "esp_zigbee_lib".into(),
                    version: Some("*".into()),
                    git: None,
                    namespace: Some("espressif".into()),
                },
            ],
            codegen_path: CodegenPath::BrookesiaManaged,
            ..Default::default()
        },
        external_contracts: vec!["Thread 1.3".into(), "Zigbee 3.0".into()],
        network_topology: NetworkTopology::Mesh,
        domain: Some(DomainKind::IotDeviceTooling),
        architecture_tier: None,
        communication_chains: None,
        pin_assignments: None,
        family: Some(ImplementationFamily::BrookesiaService),
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
    });

    // ── 18. Zigbee/Thread Gateway (IoT — Connectivity & Bridging) ─────────

    r.register(SolutionDefinition {
        id: "zigbee_thread_gateway".into(),
        label: "Thread Border Router / Zigbee Gateway".into(),
        label_zh: Some("Thread 边界路由 / Zigbee 网关".into()),
        kind: SolutionKind::ConnectivityBridge,
        supported_modules: vec!["esp32c6_wroom1".into()],
        fixed_inputs: vec![InputSurface::WifiEvent, InputSurface::BleEvent],
        fixed_outputs: vec![OutputSurface::NetworkApiState, OutputSurface::WifiPacket],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "wifi_init".into(),
                label: "Connect to WiFi (uplink)".into(),
                label_zh: Some("连接 WiFi（上行链路）".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "thread_init".into(),
                label: "Initialize Thread/Zigbee radio".into(),
                label_zh: Some("初始化 Thread/Zigbee 射频".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "start_border_router".into(),
                label: "Start border router / coordinator".into(),
                label_zh: Some("启动边界路由 / 协调器".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["wifi_init".into(), "thread_init".into()],
                ..Default::default()
            },
            OrchestrationStep {
                id: "serve_api".into(),
                label: "Serve device list to rshome-ha / MQTT".into(),
                label_zh: Some("向 rshome-ha / MQTT 提供设备列表".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["start_border_router".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "event_driven_relay".into(),
            label: "Event-driven relay from Thread/Zigbee to WiFi".into(),
            decisions: vec!["Forward on change, periodic keep-alive".into()],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "gateway_mode".into(),
                label: "Gateway Mode".into(),
                label_zh: Some("网关模式".into()),
                required: true,
                secret: false,
                description: "Protocol to bridge".into(),
                description_zh: Some("要桥接的协议。".into()),
                default_value: Some(serde_json::Value::String("thread_border_router".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "thread_border_router".into(),
                        label: "Thread Border Router".into(),
                        description: Some("IPv6 mesh ↔ WiFi. Matter-compatible.".into()),
                    },
                    EnumOption {
                        value: "zigbee_coordinator".into(),
                        label: "Zigbee Coordinator".into(),
                        description: Some("Zigbee ↔ WiFi bridge.".into()),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "network_name".into(),
                label: "Network Name".into(),
                label_zh: Some("网络名称".into()),
                required: false,
                secret: false,
                description: "Thread/Zigbee network name".into(),
                description_zh: Some("Thread/Zigbee 网络名称。".into()),
                default_value: Some(serde_json::Value::String("rshome-mesh".into())),
                enum_values: None,
                depends_on: None,
            },
            UserParameterDefinition {
                id: "channel".into(),
                label: "Radio Channel".into(),
                label_zh: Some("射频信道".into()),
                required: false,
                secret: false,
                description: "802.15.4 radio channel (11-26)".into(),
                description_zh: Some("802.15.4 射频信道（11-26）。".into()),
                default_value: Some(serde_json::Value::String("15".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "15".into(),
                        label: "Ch 15 (default)".into(),
                        description: Some("Least WiFi overlap.".into()),
                    },
                    EnumOption {
                        value: "20".into(),
                        label: "Ch 20".into(),
                        description: Some("Good WiFi coexistence.".into()),
                    },
                    EnumOption {
                        value: "25".into(),
                        label: "Ch 25".into(),
                        description: Some("Minimal WiFi overlap.".into()),
                    },
                ]),
                depends_on: None,
            },
            uplink_protocol_param(),
        ],
        feedback_paths: vec![SignalPath {
            id: "thread_to_wifi".into(),
            name: "Thread/Zigbee device data to WiFi uplink".into(),
            source: InputSurface::WifiEvent,
            transforms: vec![
                SignalPathStep {
                    order: 1,
                    node: TransformNode::CommandDispatch,
                    label: None,
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: TransformNode::OneToMany,
                    label: None,
                    description: None,
                },
            ],
            sink: OutputSurface::NetworkApiState,
            feedback: vec![
                SignalPathStep {
                    order: 1,
                    node: FeedbackSurface::ApiState,
                    label: None,
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: FeedbackSurface::WebStatus,
                    label: None,
                    description: None,
                },
            ],
            expected_user_result: "Thread/Zigbee sensors visible in rshome-ha or MQTT".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["wifi".into()],
            optional: vec!["api".into(), "ota".into()],
        },
        runtime_binding: {
            let mut rb = RuntimeBinding {
                family: Some("brookesia_service".into()),
                managed_components: vec![
                    ManagedComponentDep {
                        name: "esp_openthread".into(),
                        version: Some("*".into()),
                        git: None,
                        namespace: Some("espressif".into()),
                    },
                    ManagedComponentDep {
                        name: "esp_zigbee_lib".into(),
                        version: Some("*".into()),
                        git: None,
                        namespace: Some("espressif".into()),
                    },
                ],
                codegen_path: CodegenPath::BrookesiaManaged,
                ..Default::default()
            };
            rb.ha_entities = vec![
                crate::ha_export::HaEntityExportDefinition::sensor_entity(
                    "child_device_count",
                    "Connected Devices",
                    "None",
                    "devices",
                    crate::ha_export::StateBinding {
                        source_event: "zigbee_gw_0.topology".into(),
                        field_map: BTreeMap::from([("child_count".into(), "value".into())]),
                    },
                ),
                crate::ha_export::HaEntityExportDefinition::sensor_entity(
                    "network_channel",
                    "Radio Channel",
                    "None",
                    "",
                    crate::ha_export::StateBinding {
                        source_event: "zigbee_gw_0.topology".into(),
                        field_map: BTreeMap::from([("channel".into(), "value".into())]),
                    },
                ),
                crate::ha_export::HaEntityExportDefinition {
                    kind: crate::ha_export::HaEntityKind::BinarySensor,
                    object_id: "network_formed".into(),
                    unique_id: None,
                    name: "Network Formed".into(),
                    device_class: Some("connectivity".into()),
                    unit_of_measurement: None,
                    entity_category: Some("diagnostic".into()),
                    icon: Some("mdi:lan-connect".into()),
                    command_bindings: vec![],
                    state_binding: Some(crate::ha_export::StateBinding {
                        source_event: "zigbee_gw_0.topology".into(),
                        field_map: BTreeMap::from([("formed".into(), "state".into())]),
                    }),
                    availability_event: None,
                },
            ];
            rb
        },
        external_contracts: vec![
            "Thread Border Router".into(),
            "Zigbee 3.0".into(),
            "rshome-ha Native API".into(),
        ],
        network_topology: NetworkTopology::Mesh,
        domain: Some(DomainKind::IotDeviceTooling),
        architecture_tier: None,
        communication_chains: None,
        pin_assignments: None,
        family: Some(ImplementationFamily::BrookesiaService),
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
    });

    // ── 19. UART Debug Probe (IoT — Tooling & Debugging) ──────────────────

    r.register(SolutionDefinition {
        id: "uart_debug_probe".into(),
        label: "UART Debug Probe".into(),
        label_zh: Some("UART 调试探针".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into()],
        fixed_inputs: vec![
            InputSurface::UartRx,
            InputSurface::UsbCdcCommand,
            InputSurface::TimerTick,
        ],
        fixed_outputs: vec![
            OutputSurface::UsbTx,
            OutputSurface::WifiPacket,
            OutputSurface::StatusLed,
        ],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "usb_init".into(),
                label: "Initialize USB CDC".into(),
                label_zh: Some("初始化 USB CDC".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "uart_init".into(),
                label: "Initialize target UART".into(),
                label_zh: Some("初始化目标 UART".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "start_capture".into(),
                label: "Start UART capture".into(),
                label_zh: Some("启动 UART 抓取".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["usb_init".into(), "uart_init".into()],
                ..Default::default()
            },
            OrchestrationStep {
                id: "wifi_stream".into(),
                label: "Stream to WiFi (optional)".into(),
                label_zh: Some("WiFi 推流（可选）".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["start_capture".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "continuous_capture".into(),
            label: "Continuous UART capture + forwarding".into(),
            decisions: vec!["Capture mode: passthrough, log, or WiFi stream".into()],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "baud_rate".into(),
                label: "Baud Rate".into(),
                label_zh: Some("波特率".into()),
                required: true,
                secret: false,
                description: "Target device UART baud rate".into(),
                description_zh: Some("目标设备 UART 波特率。".into()),
                default_value: Some(serde_json::Value::String("115200".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "9600".into(),
                        label: "9600".into(),
                        description: Some("Legacy devices.".into()),
                    },
                    EnumOption {
                        value: "115200".into(),
                        label: "115200".into(),
                        description: Some("Standard. ESP32 default.".into()),
                    },
                    EnumOption {
                        value: "921600".into(),
                        label: "921600".into(),
                        description: Some("Fast. ESP-IDF monitor default.".into()),
                    },
                    EnumOption {
                        value: "2000000".into(),
                        label: "2000000".into(),
                        description: Some("Maximum. Short cables only.".into()),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "capture_mode".into(),
                label: "Capture Mode".into(),
                label_zh: Some("抓取模式".into()),
                required: true,
                secret: false,
                description: "How captured data is handled".into(),
                description_zh: Some("抓取数据的处理方式。".into()),
                default_value: Some(serde_json::Value::String("passthrough".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "passthrough".into(),
                        label: "USB Passthrough".into(),
                        description: Some("Forward UART ↔ USB CDC directly.".into()),
                    },
                    EnumOption {
                        value: "log_to_flash".into(),
                        label: "Log to Flash".into(),
                        description: Some("Store UART output in flash. Download later.".into()),
                    },
                    EnumOption {
                        value: "wifi_stream".into(),
                        label: "WiFi Stream".into(),
                        description: Some("Stream UART output via WebSocket over WiFi.".into()),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "timestamp_format".into(),
                label: "Timestamp Format".into(),
                label_zh: Some("时间戳格式".into()),
                required: false,
                secret: false,
                description: "Timestamp prepended to each line".into(),
                description_zh: Some("每行前附加的时间戳格式。".into()),
                default_value: Some(serde_json::Value::String("millis".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "none".into(),
                        label: "None".into(),
                        description: Some("No timestamps.".into()),
                    },
                    EnumOption {
                        value: "millis".into(),
                        label: "Milliseconds".into(),
                        description: Some("Relative ms since boot.".into()),
                    },
                    EnumOption {
                        value: "iso8601".into(),
                        label: "ISO 8601".into(),
                        description: Some("Absolute time (requires NTP).".into()),
                    },
                ]),
                depends_on: None,
            },
        ],
        feedback_paths: vec![SignalPath {
            id: "uart_to_usb".into(),
            name: "UART capture to USB/WiFi".into(),
            source: InputSurface::UartRx,
            transforms: vec![
                SignalPathStep {
                    order: 1,
                    node: TransformNode::UartProtocolParse,
                    label: None,
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: TransformNode::Filter,
                    label: None,
                    description: None,
                },
            ],
            sink: OutputSurface::UsbTx,
            feedback: vec![
                SignalPathStep {
                    order: 1,
                    node: FeedbackSurface::SerialLog,
                    label: None,
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: FeedbackSurface::WebStatus,
                    label: None,
                    description: None,
                },
            ],
            expected_user_result:
                "Target device UART output visible in USB terminal or web console".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["uart".into()],
            optional: vec!["wifi".into(), "ota".into()],
        },
        runtime_binding: RuntimeBinding {
            family: Some("brookesia_service".into()),
            managed_components: vec![],
            codegen_path: CodegenPath::BrookesiaManaged,
            ..Default::default()
        },
        external_contracts: vec![],
        network_topology: NetworkTopology::Local,
        domain: Some(DomainKind::IotDeviceTooling),
        architecture_tier: None,
        communication_chains: None,
        pin_assignments: Some(iot_debug_probe_pins()),
        family: Some(ImplementationFamily::BrookesiaService),
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
    });

    // ── 20. I2C Bus Analyzer (IoT — Tooling & Debugging) ──────────────────

    r.register(SolutionDefinition {
        id: "i2c_bus_analyzer".into(),
        label: "I2C Bus Analyzer".into(),
        label_zh: Some("I2C 总线分析仪".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into(), "esp32c6_wroom1".into()],
        fixed_inputs: vec![
            InputSurface::I2cSensor,
            InputSurface::UsbCdcCommand,
            InputSurface::ServiceCall,
        ],
        fixed_outputs: vec![
            OutputSurface::I2cMasterWrite,
            OutputSurface::NetworkApiState,
            OutputSurface::UsbTx,
        ],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "i2c_init".into(),
                label: "Initialize I2C bus".into(),
                label_zh: Some("初始化 I2C 总线".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "bus_scan".into(),
                label: "Scan for I2C devices (0x00-0x7F)".into(),
                label_zh: Some("扫描 I2C 设备（0x00-0x7F）".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["i2c_init".into()],
                ..Default::default()
            },
            OrchestrationStep {
                id: "start_monitor".into(),
                label: "Start traffic monitor".into(),
                label_zh: Some("启动流量监控".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["bus_scan".into()],
                ..Default::default()
            },
            OrchestrationStep {
                id: "serve_web_ui".into(),
                label: "Serve web UI (AP mode)".into(),
                label_zh: Some("提供 Web 界面（AP 模式）".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["start_monitor".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "continuous_scan".into(),
            label: "Continuous I2C bus scan + monitor".into(),
            decisions: vec!["Scan on boot, monitor on demand".into()],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "i2c_speed".into(),
                label: "I2C Speed".into(),
                label_zh: Some("I2C 速率".into()),
                required: false,
                secret: false,
                description: "I2C bus clock frequency".into(),
                description_zh: Some("I2C 总线时钟频率。".into()),
                default_value: Some(serde_json::Value::String("400000".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "100000".into(),
                        label: "100 kHz (Standard)".into(),
                        description: Some("Maximum compatibility.".into()),
                    },
                    EnumOption {
                        value: "400000".into(),
                        label: "400 kHz (Fast)".into(),
                        description: Some("Standard for most sensors.".into()),
                    },
                    EnumOption {
                        value: "1000000".into(),
                        label: "1 MHz (Fast+)".into(),
                        description: Some("Fast mode plus. Check device support.".into()),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "scan_on_boot".into(),
                label: "Scan on Boot".into(),
                label_zh: Some("启动时扫描".into()),
                required: false,
                secret: false,
                description: "Automatically scan I2C bus on power-up".into(),
                description_zh: Some("上电时自动扫描 I2C 总线。".into()),
                default_value: Some(serde_json::Value::String("true".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "true".into(),
                        label: "Yes".into(),
                        description: Some("Scan immediately on boot.".into()),
                    },
                    EnumOption {
                        value: "false".into(),
                        label: "No".into(),
                        description: Some("Wait for manual trigger.".into()),
                    },
                ]),
                depends_on: None,
            },
        ],
        feedback_paths: vec![SignalPath {
            id: "i2c_to_web".into(),
            name: "I2C bus data to web UI".into(),
            source: InputSurface::I2cSensor,
            transforms: vec![
                SignalPathStep {
                    order: 1,
                    node: TransformNode::SensorFrameDecode,
                    label: None,
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: TransformNode::Filter,
                    label: None,
                    description: None,
                },
            ],
            sink: OutputSurface::NetworkApiState,
            feedback: vec![
                SignalPathStep {
                    order: 1,
                    node: FeedbackSurface::WebStatus,
                    label: None,
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: FeedbackSurface::SerialLog,
                    label: None,
                    description: None,
                },
            ],
            expected_user_result: "I2C device addresses and register data visible in web UI".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["wifi".into(), "i2c".into()],
            optional: vec!["ota".into()],
        },
        runtime_binding: RuntimeBinding {
            family: Some("brookesia_service".into()),
            managed_components: vec![ManagedComponentDep {
                name: "brookesia_service_wifi".into(),
                version: Some("~0.7".into()),
                git: None,
                namespace: None,
            }],
            codegen_path: CodegenPath::BrookesiaManaged,
            ..Default::default()
        },
        external_contracts: vec![],
        network_topology: NetworkTopology::Local,
        domain: Some(DomainKind::IotDeviceTooling),
        architecture_tier: None,
        communication_chains: None,
        pin_assignments: Some(iot_i2c_analyzer_pins()),
        family: Some(ImplementationFamily::BrookesiaService),
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
    });

    // ── 20b. sigrok Logic Analyzer (IoT — Tooling & Debugging) ─────────────

    r.register(SolutionDefinition {
        id: "sigrok_logic_analyzer".into(),
        label: "sigrok Logic Analyzer (SUMP/OLS)".into(),
        label_zh: Some("sigrok 逻辑分析仪（SUMP/OLS）".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec![
            "esp32s3_wroom1".into(),
        ],
        fixed_inputs: vec![
            InputSurface::CaptureSignal,
            InputSurface::UsbCdcCommand,
        ],
        fixed_outputs: vec![
            OutputSurface::UsbTx,
            OutputSurface::StatusLed,
        ],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "usb_init".into(),
                label: "Initialize USB Serial/JTAG transport".into(),
                label_zh: Some("初始化 USB Serial/JTAG 传输".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "capture_configure".into(),
                label: "Configure LCD_CAM parallel capture + DMA".into(),
                label_zh: Some("配置 LCD_CAM 并行采集 + DMA".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "sump_listen".into(),
                label: "Listen for SUMP commands from host".into(),
                label_zh: Some("监听主机 SUMP 命令".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["usb_init".into(), "capture_configure".into()],
                ..Default::default()
            },
            OrchestrationStep {
                id: "trigger_capture".into(),
                label: "Arm trigger → capture → upload samples".into(),
                label_zh: Some("触发器就绪 → 采集 → 上传样本".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["sump_listen".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "host_driven".into(),
            label: "Host-driven capture cycle (SUMP protocol)".into(),
            decisions: vec!["Capture starts on SUMP arm command from host".into()],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "channel_count".into(),
                label: "Channel Count".into(),
                label_zh: Some("通道数".into()),
                required: true,
                secret: false,
                description: "Number of parallel capture channels".into(),
                description_zh: Some("并行采集通道数。".into()),
                default_value: Some(serde_json::Value::String("8".into())),
                enum_values: Some(vec![
                    EnumOption { value: "4".into(), label: "4 channels".into(), description: Some("Minimal: I2C + SPI CS.".into()) },
                    EnumOption { value: "8".into(), label: "8 channels".into(), description: Some("Standard: covers most protocols.".into()) },
                    EnumOption { value: "16".into(), label: "16 channels".into(), description: Some("Wide: parallel bus analysis.".into()) },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "sample_rate".into(),
                label: "Sample Rate".into(),
                label_zh: Some("采样率".into()),
                required: true,
                secret: false,
                description: "Maximum capture sample rate".into(),
                description_zh: Some("最大采集采样率。".into()),
                default_value: Some(serde_json::Value::String("10000000".into())),
                enum_values: Some(vec![
                    EnumOption { value: "1000000".into(), label: "1 MHz".into(), description: Some("I2C standard/fast mode, UART.".into()) },
                    EnumOption { value: "4000000".into(), label: "4 MHz".into(), description: Some("I2C high-speed, slow SPI.".into()) },
                    EnumOption { value: "10000000".into(), label: "10 MHz".into(), description: Some("SPI, WS2812, most protocols.".into()) },
                    EnumOption { value: "20000000".into(), label: "20 MHz".into(), description: Some("Fast SPI. LCD_CAM limit on 8ch.".into()) },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "capture_depth".into(),
                label: "Capture Depth".into(),
                label_zh: Some("采集深度".into()),
                required: false,
                secret: false,
                description: "Sample buffer size (SRAM or PSRAM)".into(),
                description_zh: Some("采样缓冲区大小（SRAM 或 PSRAM）。".into()),
                default_value: Some(serde_json::Value::String("100000".into())),
                enum_values: Some(vec![
                    EnumOption { value: "50000".into(), label: "50K samples".into(), description: Some("Low memory. SRAM only.".into()) },
                    EnumOption { value: "100000".into(), label: "100K samples".into(), description: Some("Standard. SRAM only.".into()) },
                    EnumOption { value: "500000".into(), label: "500K samples".into(), description: Some("Deep. Requires PSRAM.".into()) },
                    EnumOption { value: "1000000".into(), label: "1M samples".into(), description: Some("Maximum. Requires PSRAM.".into()) },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "trigger_mode".into(),
                label: "Trigger Mode".into(),
                label_zh: Some("触发模式".into()),
                required: false,
                secret: false,
                description: "When to start capturing".into(),
                description_zh: Some("何时开始采集。".into()),
                default_value: Some(serde_json::Value::String("immediate".into())),
                enum_values: Some(vec![
                    EnumOption { value: "immediate".into(), label: "Immediate".into(), description: Some("Start capture on arm. Free-running.".into()) },
                    EnumOption { value: "edge_rising".into(), label: "Rising Edge".into(), description: Some("Trigger on rising edge of CH0.".into()) },
                    EnumOption { value: "edge_falling".into(), label: "Falling Edge".into(), description: Some("Trigger on falling edge of CH0.".into()) },
                    EnumOption { value: "pattern".into(), label: "Pattern Match".into(), description: Some("Trigger on bit pattern across channels.".into()) },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "transport".into(),
                label: "USB Transport".into(),
                label_zh: Some("USB 传输方式".into()),
                required: false,
                secret: false,
                description: "USB transport for SUMP protocol".into(),
                description_zh: Some("SUMP 协议的 USB 传输方式。".into()),
                default_value: Some(serde_json::Value::String("usb_serial_jtag".into())),
                enum_values: Some(vec![
                    EnumOption { value: "usb_serial_jtag".into(), label: "USB Serial/JTAG".into(), description: Some("Zero-config. Always present on S3.".into()) },
                    EnumOption { value: "usb_otg_cdc".into(), label: "USB-OTG CDC (TinyUSB)".into(), description: Some("Cleaner descriptors. Disables JTAG debug.".into()) },
                ]),
                depends_on: None,
            },
        ],
        feedback_paths: vec![SignalPath {
            id: "gpio_to_host".into(),
            name: "GPIO capture to host via SUMP".into(),
            source: InputSurface::CaptureSignal,
            transforms: vec![
                SignalPathStep {
                    order: 1,
                    node: TransformNode::CaptureCompare,
                    label: Some("LCD_CAM DMA parallel capture".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: TransformNode::Filter,
                    label: Some("Trigger FSM (pre/post buffer)".into()),
                    description: None,
                },
            ],
            sink: OutputSurface::UsbTx,
            feedback: vec![SignalPathStep {
                order: 1,
                node: FeedbackSurface::SerialLog,
                label: Some("SUMP sample upload".into()),
                description: None,
            }],
            expected_user_result: "Captured signals decode correctly in PulseView (I2C/UART/SPI/1-Wire/WS2812)".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["sigrok".into()],
            optional: vec![
                "wifi".into(),
                "ota".into(),
            ],
        },
        runtime_binding: RuntimeBinding {
            family: Some("brookesia_service".into()),
            managed_components: vec![],
            codegen_path: CodegenPath::BrookesiaManaged,
            ..Default::default()
        },
        external_contracts: vec![],
        network_topology: NetworkTopology::Local,
        domain: Some(DomainKind::IotDeviceTooling),
        architecture_tier: None,
        communication_chains: None,
        pin_assignments: Some(iot_sigrok_la_pins()),
        family: Some(ImplementationFamily::BrookesiaService),
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
    });

    // ── 21. Dashboard Display (IoT — Monitoring & Display) ────────────────

    r.register(SolutionDefinition {
        id: "dashboard_display".into(),
        label: "LCD Dashboard Display".into(),
        label_zh: Some("LCD 仪表盘".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into()],
        fixed_inputs: vec![
            InputSurface::I2cSensor,
            InputSurface::MqttMessage,
            InputSurface::RotaryEncoder,
            InputSurface::ButtonGpio,
        ],
        fixed_outputs: vec![
            OutputSurface::LcdFrame,
            OutputSurface::StatusLed,
            OutputSurface::NetworkApiState,
        ],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "display_init".into(),
                label: "Initialize display driver".into(),
                label_zh: Some("初始化显示驱动".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "sensor_init".into(),
                label: "Initialize data sources".into(),
                label_zh: Some("初始化数据源".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "start_render_loop".into(),
                label: "Start render loop".into(),
                label_zh: Some("启动渲染循环".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["display_init".into(), "sensor_init".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "render_loop".into(),
            label: "Display render loop + sensor poll".into(),
            decisions: vec!["Refresh rate, page switching via encoder".into()],
        },
        user_parameters: vec![
            display_driver_param(),
            UserParameterDefinition {
                id: "display_resolution".into(),
                label: "Resolution".into(),
                label_zh: Some("分辨率".into()),
                required: false,
                secret: false,
                description: "Display resolution".into(),
                description_zh: Some("显示分辨率。".into()),
                default_value: Some(serde_json::Value::String("240x240".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "128x64".into(),
                        label: "128x64 (OLED)".into(),
                        description: Some("Monochrome OLED.".into()),
                    },
                    EnumOption {
                        value: "240x240".into(),
                        label: "240x240 (TFT)".into(),
                        description: Some("Small square TFT.".into()),
                    },
                    EnumOption {
                        value: "320x240".into(),
                        label: "320x240 (TFT)".into(),
                        description: Some("Standard TFT.".into()),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "data_source".into(),
                label: "Data Source".into(),
                label_zh: Some("数据来源".into()),
                required: true,
                secret: false,
                description: "Where displayed data comes from".into(),
                description_zh: Some("显示数据的来源。".into()),
                default_value: Some(serde_json::Value::String("local_sensors".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "local_sensors".into(),
                        label: "Local Sensors".into(),
                        description: Some("I2C/SPI/ADC sensors wired to this device.".into()),
                    },
                    EnumOption {
                        value: "mqtt_subscribe".into(),
                        label: "MQTT Subscribe".into(),
                        description: Some("Display data from MQTT topics.".into()),
                    },
                    EnumOption {
                        value: "both".into(),
                        label: "Both".into(),
                        description: Some("Local sensors + MQTT data.".into()),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "rotation".into(),
                label: "Display Rotation".into(),
                label_zh: Some("显示旋转".into()),
                required: false,
                secret: false,
                description: "Screen rotation angle".into(),
                description_zh: Some("屏幕旋转角度。".into()),
                default_value: Some(serde_json::Value::String("0".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "0".into(),
                        label: "0°".into(),
                        description: None,
                    },
                    EnumOption {
                        value: "90".into(),
                        label: "90°".into(),
                        description: None,
                    },
                    EnumOption {
                        value: "180".into(),
                        label: "180°".into(),
                        description: None,
                    },
                    EnumOption {
                        value: "270".into(),
                        label: "270°".into(),
                        description: None,
                    },
                ]),
                depends_on: None,
            },
        ],
        feedback_paths: vec![
            SignalPath {
                id: "sensor_to_display".into(),
                name: "Sensor data to LCD".into(),
                source: InputSurface::I2cSensor,
                transforms: vec![
                    SignalPathStep {
                        order: 1,
                        node: TransformNode::SensorFrameDecode,
                        label: None,
                        description: None,
                    },
                    SignalPathStep {
                        order: 2,
                        node: TransformNode::Mapping,
                        label: None,
                        description: None,
                    },
                ],
                sink: OutputSurface::LcdFrame,
                feedback: vec![
                    SignalPathStep {
                        order: 1,
                        node: FeedbackSurface::DisplayText,
                        label: None,
                        description: None,
                    },
                    SignalPathStep {
                        order: 2,
                        node: FeedbackSurface::LedIndicator,
                        label: None,
                        description: None,
                    },
                ],
                expected_user_result: "Sensor readings displayed on LCD with real-time updates"
                    .into(),
            },
            SignalPath {
                id: "encoder_to_display".into(),
                name: "Rotary encoder navigation".into(),
                source: InputSurface::RotaryEncoder,
                transforms: vec![
                    SignalPathStep {
                        order: 1,
                        node: TransformNode::Debounce,
                        label: None,
                        description: None,
                    },
                    SignalPathStep {
                        order: 2,
                        node: TransformNode::StateMachine,
                        label: None,
                        description: None,
                    },
                ],
                sink: OutputSurface::LcdFrame,
                feedback: vec![],
                expected_user_result: "Rotary encoder navigates between dashboard pages".into(),
            },
        ],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["wifi".into(), "spi".into()],
            optional: vec!["i2c".into(), "sensor".into(), "ota".into()],
        },
        runtime_binding: RuntimeBinding {
            family: Some("brookesia_service".into()),
            managed_components: vec![ManagedComponentDep {
                name: "esp_lcd".into(),
                version: Some("*".into()),
                git: None,
                namespace: Some("espressif".into()),
            }],
            codegen_path: CodegenPath::BrookesiaManaged,
            ..Default::default()
        },
        external_contracts: vec![],
        network_topology: NetworkTopology::Star,
        domain: Some(DomainKind::IotDeviceTooling),
        architecture_tier: None,
        communication_chains: None,
        pin_assignments: Some(iot_display_pins()),
        family: Some(ImplementationFamily::BrookesiaService),
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
    });

    // ── 22. Status LED Notifier (IoT — Monitoring & Display) ──────────────

    r.register(SolutionDefinition {
        id: "status_led_notifier".into(),
        label: "Addressable LED Status Notifier".into(),
        label_zh: Some("可寻址 LED 状态指示器".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into(), "esp32c6_wroom1".into()],
        fixed_inputs: vec![
            InputSurface::MqttMessage,
            InputSurface::I2cSensor,
            InputSurface::ServiceCall,
            InputSurface::ApiCommand,
        ],
        fixed_outputs: vec![
            OutputSurface::RmtWaveform,
            OutputSurface::StatusLed,
            OutputSurface::NetworkApiState,
        ],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "led_init".into(),
                label: "Initialize LED strip (RMT)".into(),
                label_zh: Some("初始化 LED 灯带（RMT）".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "connect_data_source".into(),
                label: "Connect to data source (MQTT/sensor/API)".into(),
                label_zh: Some("连接数据源（MQTT/传感器/API）".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "start_animation_loop".into(),
                label: "Start animation loop".into(),
                label_zh: Some("启动动画循环".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["led_init".into(), "connect_data_source".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "animation_loop".into(),
            label: "LED animation loop driven by data events".into(),
            decisions: vec!["Map data ranges to colors/patterns, animate transitions".into()],
        },
        user_parameters: vec![
            led_type_param(),
            UserParameterDefinition {
                id: "led_count".into(),
                label: "LED Count".into(),
                label_zh: Some("LED 数量".into()),
                required: true,
                secret: false,
                description: "Number of addressable LEDs".into(),
                description_zh: Some("可寻址 LED 的数量。".into()),
                default_value: Some(serde_json::Value::String("16".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "1".into(),
                        label: "1 (single)".into(),
                        description: Some("Single status LED.".into()),
                    },
                    EnumOption {
                        value: "8".into(),
                        label: "8 (small ring)".into(),
                        description: Some("Small NeoPixel ring.".into()),
                    },
                    EnumOption {
                        value: "16".into(),
                        label: "16 (ring)".into(),
                        description: Some("Standard NeoPixel ring.".into()),
                    },
                    EnumOption {
                        value: "30".into(),
                        label: "30 (strip 0.5m)".into(),
                        description: Some("Half-meter strip.".into()),
                    },
                    EnumOption {
                        value: "60".into(),
                        label: "60 (strip 1m)".into(),
                        description: Some("One-meter strip.".into()),
                    },
                    EnumOption {
                        value: "144".into(),
                        label: "144 (strip 1m HD)".into(),
                        description: Some("High-density one-meter strip.".into()),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "data_source".into(),
                label: "Data Source".into(),
                label_zh: Some("数据来源".into()),
                required: true,
                secret: false,
                description: "What drives LED state".into(),
                description_zh: Some("驱动 LED 状态的数据来源。".into()),
                default_value: Some(serde_json::Value::String("mqtt".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "mqtt".into(),
                        label: "MQTT Topic".into(),
                        description: Some(
                            "Subscribe to MQTT topics for color/pattern commands.".into(),
                        ),
                    },
                    EnumOption {
                        value: "sensor".into(),
                        label: "Local Sensor".into(),
                        description: Some(
                            "Map sensor values to colors (e.g., AQI → green/yellow/red).".into(),
                        ),
                    },
                    EnumOption {
                        value: "api_command".into(),
                        label: "rshome-ha API Command".into(),
                        description: Some("Control via rshome-ha light entity.".into()),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "default_pattern".into(),
                label: "Default Pattern".into(),
                label_zh: Some("默认灯效".into()),
                required: false,
                secret: false,
                description: "LED pattern shown on boot or when idle".into(),
                description_zh: Some("启动或空闲时显示的灯效。".into()),
                default_value: Some(serde_json::Value::String("solid".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "solid".into(),
                        label: "Solid Color".into(),
                        description: Some("Static single color.".into()),
                    },
                    EnumOption {
                        value: "breathing".into(),
                        label: "Breathing".into(),
                        description: Some("Smooth fade in/out.".into()),
                    },
                    EnumOption {
                        value: "chase".into(),
                        label: "Chase".into(),
                        description: Some("Pixel chase animation.".into()),
                    },
                    EnumOption {
                        value: "fire".into(),
                        label: "Fire".into(),
                        description: Some("Fire simulation effect.".into()),
                    },
                ]),
                depends_on: None,
            },
        ],
        feedback_paths: vec![
            SignalPath {
                id: "mqtt_to_led".into(),
                name: "MQTT command to LED pattern".into(),
                source: InputSurface::MqttMessage,
                transforms: vec![
                    SignalPathStep {
                        order: 1,
                        node: TransformNode::CommandDispatch,
                        label: None,
                        description: None,
                    },
                    SignalPathStep {
                        order: 2,
                        node: TransformNode::Mapping,
                        label: None,
                        description: None,
                    },
                ],
                sink: OutputSurface::RmtWaveform,
                feedback: vec![
                    SignalPathStep {
                        order: 1,
                        node: FeedbackSurface::LedIndicator,
                        label: None,
                        description: None,
                    },
                    SignalPathStep {
                        order: 2,
                        node: FeedbackSurface::ApiState,
                        label: None,
                        description: None,
                    },
                ],
                expected_user_result:
                    "LED strip responds to MQTT commands with color/pattern changes".into(),
            },
            SignalPath {
                id: "sensor_to_led".into(),
                name: "Sensor value to LED color".into(),
                source: InputSurface::I2cSensor,
                transforms: vec![
                    SignalPathStep {
                        order: 1,
                        node: TransformNode::SensorFrameDecode,
                        label: None,
                        description: None,
                    },
                    SignalPathStep {
                        order: 2,
                        node: TransformNode::Threshold,
                        label: None,
                        description: None,
                    },
                ],
                sink: OutputSurface::RmtWaveform,
                feedback: vec![],
                expected_user_result:
                    "LED color reflects sensor value (e.g., AQI green→yellow→red)".into(),
            },
        ],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["wifi".into()],
            optional: vec![
                "light".into(),
                "i2c".into(),
                "sensor".into(),
                "api".into(),
                "ota".into(),
            ],
        },
        runtime_binding: {
            let mut rb = RuntimeBinding {
                family: Some("brookesia_service".into()),
                managed_components: vec![ManagedComponentDep {
                    name: "led_strip".into(),
                    version: Some("*".into()),
                    git: None,
                    namespace: Some("espressif".into()),
                }],
                codegen_path: CodegenPath::BrookesiaManaged,
                ..Default::default()
            };
            rb.ha_entities = vec![crate::ha_export::HaEntityExportDefinition {
                kind: crate::ha_export::HaEntityKind::Light,
                object_id: "led_strip".into(),
                unique_id: None,
                name: "LED Strip".into(),
                device_class: None,
                unit_of_measurement: None,
                entity_category: None,
                icon: Some("mdi:led-strip-variant".into()),
                command_bindings: vec![
                    crate::ha_export::CommandBinding::new(
                        "turn_on",
                        "led_strip_0.on",
                        BTreeMap::new(),
                    ),
                    crate::ha_export::CommandBinding::new(
                        "turn_off",
                        "led_strip_0.off",
                        BTreeMap::new(),
                    ),
                    crate::ha_export::CommandBinding::new(
                        "set_rgb",
                        "led_strip_0.set_color",
                        BTreeMap::new(),
                    ),
                    crate::ha_export::CommandBinding::new(
                        "set_effect",
                        "led_strip_0.set_effect",
                        BTreeMap::new(),
                    ),
                ],
                state_binding: Some(crate::ha_export::StateBinding {
                    source_event: "led_strip_0.state".into(),
                    field_map: BTreeMap::from([
                        ("on".into(), "state".into()),
                        ("brightness".into(), "brightness".into()),
                        ("r".into(), "color_r".into()),
                        ("g".into(), "color_g".into()),
                        ("b".into(), "color_b".into()),
                        ("effect".into(), "effect".into()),
                    ]),
                }),
                availability_event: None,
            }];
            rb
        },
        external_contracts: vec![
            "MQTT 3.1.1 / 5.0 broker".into(),
            "rshome-ha Native API".into(),
        ],
        network_topology: NetworkTopology::Star,
        domain: Some(DomainKind::IotDeviceTooling),
        architecture_tier: None,
        communication_chains: None,
        pin_assignments: Some(iot_led_strip_pins()),
        family: Some(ImplementationFamily::BrookesiaService),
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
    });

    // ── 23. ESP-Mesh-Lite Network (multi-hop mesh) ─────────────────────────

    r.register(SolutionDefinition {
        id: "mesh_lite_network_solution".into(),
        label: "ESP-Mesh-Lite Network (Multi-Hop Mesh)".into(),
        label_zh: Some("ESP-Mesh-Lite 网络(多跳网状)".into()),
        kind: SolutionKind::ConnectivityBridge,
        supported_modules: vec!["esp32s3_wroom1".into(), "esp32c6_wroom1".into()],
        fixed_inputs: vec![InputSurface::WifiEvent, InputSurface::ServiceCall],
        fixed_outputs: vec![OutputSurface::WifiPacket, OutputSurface::NetworkApiState],
        fixed_orchestration: vec![],
        scheduling: SchedulingPolicy {
            id: "mesh_event".into(),
            label: "Mesh event-driven (topology changes + data)".into(),
            decisions: vec![
                "Root node: STA to router + mesh AP".into(),
                "Non-root: mesh-only, auto-routing".into(),
            ],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "mesh_id".into(),
                label: "Mesh Network ID".into(),
                label_zh: Some("网状网络 ID".into()),
                required: true,
                secret: false,
                description: "Shared mesh network identifier".into(),
                description_zh: Some("共享的 mesh 网络标识。".into()),
                default_value: Some(serde_json::Value::String("rshome-mesh".into())),
                enum_values: None,
                depends_on: None,
            },
            UserParameterDefinition {
                id: "is_root".into(),
                label: "Root Node".into(),
                label_zh: Some("根节点".into()),
                required: false,
                secret: false,
                description: "Whether this device is the mesh root (connected to router)".into(),
                description_zh: Some("本设备是否是 mesh 根节点(连接到路由器)。".into()),
                default_value: Some(serde_json::json!(false)),
                enum_values: None,
                depends_on: None,
            },
        ],
        feedback_paths: vec![SignalPath {
            id: "mesh_data_flow".into(),
            name: "Mesh-Lite data flow".into(),
            source: InputSurface::WifiEvent,
            transforms: vec![SignalPathStep {
                order: 1,
                node: TransformNode::StateMachine,
                label: Some("Mesh topology manager".into()),
                description: None,
            }],
            sink: OutputSurface::WifiPacket,
            feedback: vec![SignalPathStep {
                order: 1,
                node: FeedbackSurface::RuntimeMetrics,
                label: Some("Mesh topology metrics".into()),
                description: None,
            }],
            expected_user_result: "Sensor data routed through mesh to root node".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["wifi".into()],
            optional: vec![],
        },
        runtime_binding: RuntimeBinding {
            family: Some("brookesia_service".into()),
            managed_components: vec![
                ManagedComponentDep {
                    name: "mesh_lite".into(),
                    version: Some("*".into()),
                    git: None,
                    namespace: Some("espressif".into()),
                },
                ManagedComponentDep {
                    name: "brookesia_service_wifi".into(),
                    version: Some("~0.7".into()),
                    git: None,
                    namespace: None,
                },
            ],
            codegen_path: CodegenPath::BrookesiaManaged,
            ..Default::default()
        },
        external_contracts: vec!["ESP-Mesh-Lite protocol".into()],
        network_topology: NetworkTopology::Mesh,
        domain: None,
        architecture_tier: None,
        communication_chains: None,
        pin_assignments: None,
        family: Some(ImplementationFamily::BrookesiaService),
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
    });

    // ── CAN/RS485 Bus Sampler (IoT Device Tooling) ────────────────────────────

    r.register(SolutionDefinition {
        id: "bus_sampler_solution".into(),
        label: "CAN/RS485 Bus Sampler".into(),
        label_zh: Some("CAN/RS485 总线采样器".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32_d0wd_v3".into()],
        fixed_inputs: vec![
            InputSurface::CanBusFrame,
            InputSurface::Rs485Data,
            InputSurface::AdcVoltage,
            InputSurface::ButtonGpio,
        ],
        fixed_outputs: vec![
            OutputSurface::SdCardWrite,
            OutputSurface::NetworkApiState,
            OutputSurface::StatusLed,
            OutputSurface::CanBusTx,
            OutputSurface::Rs485Tx,
        ],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "bus_init".into(),
                label: "Initialize CAN (TWAI) and RS485 (UART) drivers".into(),
                label_zh: Some("初始化 CAN (TWAI) 和 RS485 (UART) 驱动".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "sd_mount".into(),
                label: "Mount SD card and open log files".into(),
                label_zh: Some("挂载 SD 卡并打开日志文件".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["bus_init".into()],
                ..Default::default()
            },
            OrchestrationStep {
                id: "wifi_telemetry".into(),
                label: "Start WiFi STA and HTTP telemetry server".into(),
                label_zh: Some("启动 WiFi STA 和 HTTP 遥测服务".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "dual_core_bus".into(),
            label: "Dual-core bus sampling".into(),
            decisions: vec![
                "CAN RX task pinned to core 1".into(),
                "RS485 RX + WiFi on core 0".into(),
                "SD flush on 5s timer".into(),
            ],
        },
        user_parameters: vec![],
        feedback_paths: vec![
            SignalPath {
                id: "can_to_sd".into(),
                name: "CAN bus capture to SD card".into(),
                source: InputSurface::CanBusFrame,
                transforms: vec![SignalPathStep {
                    order: 1,
                    node: TransformNode::CanFrameDecode,
                    label: Some("Decode CAN frame".into()),
                    description: None,
                }],
                sink: OutputSurface::SdCardWrite,
                feedback: vec![SignalPathStep {
                    order: 1,
                    node: FeedbackSurface::BusErrorAlert,
                    label: None,
                    description: None,
                }],
                expected_user_result: "CAN frames logged as CSV to SD card with timestamps".into(),
            },
            SignalPath {
                id: "rs485_to_sd".into(),
                name: "RS485 data capture to SD card".into(),
                source: InputSurface::Rs485Data,
                transforms: vec![SignalPathStep {
                    order: 1,
                    node: TransformNode::Rs485ProtocolDecode,
                    label: Some("Decode RS485 data".into()),
                    description: None,
                }],
                sink: OutputSurface::SdCardWrite,
                feedback: vec![SignalPathStep {
                    order: 1,
                    node: FeedbackSurface::BusErrorAlert,
                    label: None,
                    description: None,
                }],
                expected_user_result: "RS485 data logged as hex CSV to SD card with timestamps"
                    .into(),
            },
            SignalPath {
                id: "bus_to_http".into(),
                name: "Bus status to HTTP telemetry".into(),
                source: InputSurface::CanBusFrame,
                transforms: vec![SignalPathStep {
                    order: 1,
                    node: TransformNode::OneToMany,
                    label: Some("Aggregate bus statistics".into()),
                    description: None,
                }],
                sink: OutputSurface::NetworkApiState,
                feedback: vec![SignalPathStep {
                    order: 1,
                    node: FeedbackSurface::WebStatus,
                    label: None,
                    description: None,
                }],
                expected_user_result: "GET /status returns JSON with frame counts and bus state"
                    .into(),
            },
        ],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["can_bus".into(), "rs485_bus".into(), "uart".into()],
            optional: vec!["wifi".into(), "ota".into()],
        },
        runtime_binding: {
            let mut binding = brookesia_wifi_binding();
            binding.board_assembly = Some("esp32_can485_devboard".into());
            binding.managed_components.push(ManagedComponentDep {
                name: "led_strip".into(),
                version: Some("^2".into()),
                git: None,
                namespace: Some("espressif".into()),
            });
            binding
        },
        external_contracts: vec![],
        network_topology: NetworkTopology::Star,
        domain: Some(DomainKind::IotDeviceTooling),
        architecture_tier: None,
        communication_chains: None,
        pin_assignments: Some(vec![
            PinAssignment {
                function: "CAN RX".into(),
                default_gpio: 26,
                alternatives: vec![],
                capability: "CanBus".into(),
            },
            PinAssignment {
                function: "CAN TX".into(),
                default_gpio: 27,
                alternatives: vec![],
                capability: "CanBus".into(),
            },
            PinAssignment {
                function: "RS485 RO".into(),
                default_gpio: 21,
                alternatives: vec![],
                capability: "Rs485".into(),
            },
            PinAssignment {
                function: "RS485 DI".into(),
                default_gpio: 22,
                alternatives: vec![],
                capability: "Rs485".into(),
            },
            PinAssignment {
                function: "RS485 DE".into(),
                default_gpio: 17,
                alternatives: vec![],
                capability: "Rs485".into(),
            },
            PinAssignment {
                function: "SD CS".into(),
                default_gpio: 13,
                alternatives: vec![],
                capability: "Spi".into(),
            },
            PinAssignment {
                function: "VIN ADC".into(),
                default_gpio: 36,
                alternatives: vec![],
                capability: "Adc".into(),
            },
            PinAssignment {
                function: "WS2812".into(),
                default_gpio: 4,
                alternatives: vec![],
                capability: "Rmt".into(),
            },
            PinAssignment {
                function: "Boot Key".into(),
                default_gpio: 0,
                alternatives: vec![],
                capability: "Gpio".into(),
            },
        ]),
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
    });

    // ── IoT Phase 2 solutions (2026-04-16) ──────────────────────────────────

    // 28. LD2410 mmWave Presence Sensor
    r.register(SolutionDefinition {
        id: "ld2410_presence_solution".into(),
        label: "LD2410 mmWave Presence Sensor".into(),
        label_zh: Some("LD2410 毫米波人体存在传感器".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into(), "esp32c6_wroom1".into()],
        fixed_inputs: vec![InputSurface::UartSensor, InputSurface::TimerTick],
        fixed_outputs: vec![OutputSurface::NetworkApiState, OutputSurface::StatusLed],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "init_uart".into(),
                label: "Initialize UART2 for LD2410 at 256000 baud".into(),
                label_zh: Some("初始化 LD2410 UART2（256000 波特）".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "configure_ld2410".into(),
                label: "Send config frames: sensitivity, max distance, timeout".into(),
                label_zh: Some("发送配置帧：灵敏度、最大距离、超时".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["init_uart".into()],
                ..Default::default()
            },
            OrchestrationStep {
                id: "detect_presence_loop".into(),
                label: "Parse data frames → presence + distance events".into(),
                label_zh: Some("解析数据帧 → 存在 + 距离事件".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["configure_ld2410".into()],
                ..Default::default()
            },
            OrchestrationStep {
                id: "publish_events".into(),
                label: "Publish presence + distance to HA / MQTT".into(),
                label_zh: Some("将存在 + 距离发布到 HA / MQTT".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["detect_presence_loop".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "event_driven_presence".into(),
            label: "Event-driven presence detection".into(),
            decisions: vec![
                "LD2410 UART frames at ~10 Hz; debounce with presence_timeout_s".into(),
                "Only publish on state change or periodic heartbeat".into(),
            ],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "sensitivity_level".into(),
                label: "Sensitivity".into(),
                label_zh: Some("灵敏度".into()),
                required: false,
                secret: false,
                description:
                    "LD2410 detection sensitivity (higher = more distance, more false positives)"
                        .into(),
                description_zh: Some("LD2410 检测灵敏度（更高 = 距离更远，但误检也更多）。".into()),
                default_value: Some(serde_json::Value::String("medium".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "low".into(),
                        label: "Low".into(),
                        description: Some("Conservative; fewer false positives.".into()),
                    },
                    EnumOption {
                        value: "medium".into(),
                        label: "Medium".into(),
                        description: Some("Balanced default.".into()),
                    },
                    EnumOption {
                        value: "high".into(),
                        label: "High".into(),
                        description: Some("Sensitive; more range but more false positives.".into()),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "max_detection_cm".into(),
                label: "Max Detection Range (cm)".into(),
                label_zh: Some("最大检测距离（厘米）".into()),
                required: false,
                secret: false,
                description:
                    "Maximum detection range in centimeters (LD2410 supports up to 600 cm)".into(),
                description_zh: Some("最大检测距离，单位厘米（LD2410 最大支持 600 cm）。".into()),
                default_value: Some(serde_json::Value::String("600".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "200".into(),
                        label: "2 m (desk-scale)".into(),
                        description: None,
                    },
                    EnumOption {
                        value: "400".into(),
                        label: "4 m (room-scale)".into(),
                        description: None,
                    },
                    EnumOption {
                        value: "600".into(),
                        label: "6 m (full range)".into(),
                        description: None,
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "presence_timeout_s".into(),
                label: "Clear Timeout (s)".into(),
                label_zh: Some("清除超时（秒）".into()),
                required: false,
                secret: false,
                description: "Seconds of no detection before clearing presence state".into(),
                description_zh: Some("无检测多少秒后清除存在状态。".into()),
                default_value: Some(serde_json::Value::String("5".into())),
                enum_values: None,
                depends_on: None,
            },
            uplink_protocol_param(),
        ],
        feedback_paths: vec![SignalPath {
            id: "ld2410_to_api".into(),
            name: "mmWave frame to HA state".into(),
            source: InputSurface::UartSensor,
            transforms: vec![
                SignalPathStep {
                    order: 1,
                    node: TransformNode::UartProtocolParse,
                    label: Some("LD2410 frame decode".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: TransformNode::Threshold,
                    label: Some("Sensitivity threshold".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 3,
                    node: TransformNode::Debounce,
                    label: Some("presence_timeout_s debounce".into()),
                    description: None,
                },
            ],
            sink: OutputSurface::NetworkApiState,
            feedback: vec![
                SignalPathStep {
                    order: 1,
                    node: FeedbackSurface::LedIndicator,
                    label: None,
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: FeedbackSurface::ApiState,
                    label: None,
                    description: None,
                },
            ],
            expected_user_result:
                "HA sees a binary_sensor that toggles on presence, plus a distance sensor".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["uart".into(), "sensor".into(), "wifi".into()],
            optional: vec!["ota".into(), "api".into(), "mqtt".into()],
        },
        runtime_binding: brookesia_wifi_binding(),
        external_contracts: vec![],
        network_topology: NetworkTopology::Star,
        domain: Some(DomainKind::IotDeviceTooling),
        architecture_tier: None,
        communication_chains: None,
        pin_assignments: Some(iot_ld2410_pins()),
        family: Some(ImplementationFamily::BrookesiaService),
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
    });

    // 29. CT-Clamp Power Monitor
    r.register(SolutionDefinition {
        id: "ct_clamp_power_monitor_solution".into(),
        label: "CT-Clamp Power Monitor".into(),
        label_zh: Some("CT 钳形功率监测器".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into(), "esp32c6_wroom1".into()],
        fixed_inputs: vec![InputSurface::AdcVoltage, InputSurface::TimerTick],
        fixed_outputs: vec![OutputSurface::NetworkApiState],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "init_adc".into(),
                label: "Initialize ADC (ADC1 continuous mode, 12-bit)".into(),
                label_zh: Some("初始化 ADC（ADC1 连续采样，12 位）".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "calibrate_offset".into(),
                label: "Calibrate DC offset from 1-second idle sample".into(),
                label_zh: Some("通过 1 秒空闲采样校准 DC 偏置".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["init_adc".into()],
                ..Default::default()
            },
            OrchestrationStep {
                id: "measure_rms_loop".into(),
                label: "Sample at sample_rate_hz, compute true-RMS per phase".into(),
                label_zh: Some("按 sample_rate_hz 采样，逐相计算真有效值（RMS）".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["calibrate_offset".into()],
                ..Default::default()
            },
            OrchestrationStep {
                id: "publish_power".into(),
                label: "Publish current / voltage / apparent power / energy counters".into(),
                label_zh: Some("发布电流 / 电压 / 视在功率 / 累计能量".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["measure_rms_loop".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "periodic_rms".into(),
            label: "Periodic RMS sampling + report".into(),
            decisions: vec![
                "ADC at 50–500 Hz per phase; RMS window 20 ms (50 Hz) or 16.6 ms (60 Hz)".into(),
                "Report at report_interval_s; aggregate min/max/avg between reports".into(),
            ],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "ct_ratio_a_to_ma".into(),
                label: "CT Ratio (primary A : secondary mA)".into(),
                label_zh: Some("CT 变比（初级 A : 次级 mA）".into()),
                required: true,
                secret: false,
                description: "CT clamp ratio, e.g. 100:50 for a 100A clamp with 50mA output".into(),
                description_zh: Some("CT 钳变比，例如 100A 量程 50mA 输出 → 100:50。".into()),
                default_value: Some(serde_json::Value::String("100:50".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "20:25".into(),
                        label: "20A:25mA (low range)".into(),
                        description: None,
                    },
                    EnumOption {
                        value: "100:50".into(),
                        label: "100A:50mA (SCT-013-000)".into(),
                        description: None,
                    },
                    EnumOption {
                        value: "200:100".into(),
                        label: "200A:100mA".into(),
                        description: None,
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "burden_resistor_ohm".into(),
                label: "Burden Resistor (Ω)".into(),
                label_zh: Some("负载电阻（Ω）".into()),
                required: false,
                secret: false,
                description: "Burden resistor across the CT secondary (sets voltage for ADC)"
                    .into(),
                description_zh: Some("CT 次级两端的负载电阻，决定 ADC 读取电压。".into()),
                default_value: Some(serde_json::Value::String("22".into())),
                enum_values: None,
                depends_on: None,
            },
            UserParameterDefinition {
                id: "sample_rate_hz".into(),
                label: "ADC Sample Rate (Hz)".into(),
                label_zh: Some("ADC 采样率（Hz）".into()),
                required: false,
                secret: false,
                description: "Per-phase ADC samples per second for RMS calculation".into(),
                description_zh: Some("逐相 ADC 每秒采样次数（用于 RMS 计算）。".into()),
                default_value: Some(serde_json::Value::String("50".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "50".into(),
                        label: "50 Hz (coarse)".into(),
                        description: None,
                    },
                    EnumOption {
                        value: "200".into(),
                        label: "200 Hz".into(),
                        description: None,
                    },
                    EnumOption {
                        value: "500".into(),
                        label: "500 Hz (fine)".into(),
                        description: None,
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "phases".into(),
                label: "Phase Count".into(),
                label_zh: Some("相数".into()),
                required: false,
                secret: false,
                description: "Single-phase or three-phase system".into(),
                description_zh: Some("单相或三相系统。".into()),
                default_value: Some(serde_json::Value::String("1".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "1".into(),
                        label: "Single-phase".into(),
                        description: None,
                    },
                    EnumOption {
                        value: "3".into(),
                        label: "Three-phase".into(),
                        description: None,
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "report_interval_s".into(),
                label: "Report Interval (s)".into(),
                label_zh: Some("上报间隔（秒）".into()),
                required: false,
                secret: false,
                description: "Seconds between aggregated power reports".into(),
                description_zh: Some("两次聚合上报之间的秒数。".into()),
                default_value: Some(serde_json::Value::String("10".into())),
                enum_values: None,
                depends_on: None,
            },
            uplink_protocol_param(),
        ],
        feedback_paths: vec![SignalPath {
            id: "adc_to_power".into(),
            name: "CT-clamp ADC to HA power state".into(),
            source: InputSurface::AdcVoltage,
            transforms: vec![
                SignalPathStep {
                    order: 1,
                    node: TransformNode::Calibration,
                    label: Some("DC-offset removal".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: TransformNode::Filter,
                    label: Some("RMS window".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 3,
                    node: TransformNode::Normalization,
                    label: Some("CT-ratio × burden-R scaling".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 4,
                    node: TransformNode::PeriodicTask,
                    label: Some("Aggregate + publish".into()),
                    description: None,
                },
            ],
            sink: OutputSurface::NetworkApiState,
            feedback: vec![
                SignalPathStep {
                    order: 1,
                    node: FeedbackSurface::ApiState,
                    label: None,
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: FeedbackSurface::RuntimeMetrics,
                    label: None,
                    description: None,
                },
            ],
            expected_user_result:
                "HA sees current (A), apparent power (VA), and energy (Wh) per phase".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["adc".into(), "sensor".into(), "wifi".into()],
            optional: vec!["ota".into(), "api".into(), "mqtt".into()],
        },
        runtime_binding: {
            let mut binding = brookesia_wifi_binding();
            binding.codegen_path = CodegenPath::SelfHosted;
            binding
        },
        external_contracts: vec![],
        network_topology: NetworkTopology::Star,
        domain: Some(DomainKind::IotDeviceTooling),
        architecture_tier: None,
        communication_chains: None,
        pin_assignments: Some(iot_ct_clamp_pins()),
        family: Some(ImplementationFamily::Custom),
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
    });

    // 30. Air-Quality Station (PMS5003 + SCD40)
    r.register(SolutionDefinition {
        id: "air_quality_pm_station_solution".into(),
        label: "Air Quality Station (PM + CO₂)".into(),
        label_zh: Some("空气质量站（PM + CO₂）".into()),
        kind: SolutionKind::FirmwareAppliance,
        supported_modules: vec!["esp32s3_wroom1".into(), "esp32c6_wroom1".into()],
        fixed_inputs: vec![
            InputSurface::UartSensor,
            InputSurface::I2cSensor,
            InputSurface::TimerTick,
        ],
        fixed_outputs: vec![OutputSurface::NetworkApiState],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "init_pms".into(),
                label: "Initialize PMS5003 UART + SET pin".into(),
                label_zh: Some("初始化 PMS5003 UART + SET 引脚".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "init_scd40".into(),
                label: "Initialize SCD40 I²C + set pressure/altitude compensation".into(),
                label_zh: Some("初始化 SCD40 I²C + 设置气压/海拔补偿".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "sample_combined_loop".into(),
                label: "Read PM + CO₂/T/RH on sample_interval_s cadence".into(),
                label_zh: Some("按 sample_interval_s 周期读取 PM + CO₂/温湿度".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["init_pms".into(), "init_scd40".into()],
                ..Default::default()
            },
            OrchestrationStep {
                id: "publish_air_quality".into(),
                label: "Publish readings (raw or AQI-derived) to HA / MQTT".into(),
                label_zh: Some("将读数（原始或 AQI）发布到 HA / MQTT".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["sample_combined_loop".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "air_quality_poll".into(),
            label: "Combined PM + CO₂ polling + publish".into(),
            decisions: vec![
                "PMS5003 wake_cycle_s = 0 → continuous fan; >0 → duty-cycled".into(),
                "SCD40 samples at 5-second cadence (sensor limit)".into(),
            ],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "pms_wake_cycle_s".into(),
                label: "PMS Wake Cycle (s)".into(),
                label_zh: Some("PMS 唤醒周期（秒）".into()),
                required: false,
                secret: false,
                description: "0 = continuous fan; >0 = sleep between samples to save lifetime"
                    .into(),
                description_zh: Some("0 = 风扇持续运行；>0 = 采样之间休眠以延长寿命。".into()),
                default_value: Some(serde_json::Value::String("60".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "0".into(),
                        label: "Continuous".into(),
                        description: Some("Fan always on; best responsiveness.".into()),
                    },
                    EnumOption {
                        value: "60".into(),
                        label: "60 s".into(),
                        description: None,
                    },
                    EnumOption {
                        value: "300".into(),
                        label: "5 min".into(),
                        description: Some("Low duty cycle; long sensor life.".into()),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "scd_altitude_m".into(),
                label: "Installation Altitude (m)".into(),
                label_zh: Some("安装海拔（米）".into()),
                required: false,
                secret: false,
                description: "SCD40 altitude compensation for CO₂ accuracy".into(),
                description_zh: Some("SCD40 CO₂ 精度需要的海拔补偿。".into()),
                default_value: Some(serde_json::Value::String("0".into())),
                enum_values: None,
                depends_on: None,
            },
            UserParameterDefinition {
                id: "ambient_pressure_mbar".into(),
                label: "Ambient Pressure (mbar)".into(),
                label_zh: Some("环境气压（毫巴）".into()),
                required: false,
                secret: false,
                description: "Ambient pressure override for SCD40 (default 1013 mbar)".into(),
                description_zh: Some("SCD40 环境气压（默认 1013 mbar）。".into()),
                default_value: Some(serde_json::Value::String("1013".into())),
                enum_values: None,
                depends_on: None,
            },
            UserParameterDefinition {
                id: "sample_interval_s".into(),
                label: "Sample Interval (s)".into(),
                label_zh: Some("采样间隔（秒）".into()),
                required: false,
                secret: false,
                description: "Seconds between combined PM + CO₂ samples".into(),
                description_zh: Some("两次 PM + CO₂ 联合采样之间的秒数。".into()),
                default_value: Some(serde_json::Value::String("30".into())),
                enum_values: None,
                depends_on: None,
            },
            UserParameterDefinition {
                id: "publish_format".into(),
                label: "Publish Format".into(),
                label_zh: Some("上报格式".into()),
                required: false,
                secret: false,
                description: "Raw concentrations or derived AQI".into(),
                description_zh: Some("上报原始浓度或派生 AQI。".into()),
                default_value: Some(serde_json::Value::String("raw".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "raw".into(),
                        label: "Raw (µg/m³, ppm)".into(),
                        description: None,
                    },
                    EnumOption {
                        value: "aqi_derived".into(),
                        label: "Raw + AQI".into(),
                        description: Some("Adds US EPA AQI index.".into()),
                    },
                ]),
                depends_on: None,
            },
            uplink_protocol_param(),
        ],
        feedback_paths: vec![SignalPath {
            id: "air_quality_to_api".into(),
            name: "PM + CO₂ sensors to HA state".into(),
            source: InputSurface::I2cSensor,
            transforms: vec![
                SignalPathStep {
                    order: 1,
                    node: TransformNode::SensorFrameDecode,
                    label: Some("PMS frame + SCD40 I²C decode".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: TransformNode::Filter,
                    label: Some("Median filter".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 3,
                    node: TransformNode::PeriodicTask,
                    label: Some("Publish on cadence".into()),
                    description: None,
                },
            ],
            sink: OutputSurface::NetworkApiState,
            feedback: vec![
                SignalPathStep {
                    order: 1,
                    node: FeedbackSurface::LedIndicator,
                    label: None,
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: FeedbackSurface::ApiState,
                    label: None,
                    description: None,
                },
            ],
            expected_user_result:
                "HA sees PM1.0/2.5/10, CO₂, temperature, humidity, and optional AQI".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["uart".into(), "i2c".into(), "sensor".into(), "wifi".into()],
            optional: vec!["ota".into(), "api".into(), "mqtt".into()],
        },
        runtime_binding: brookesia_wifi_binding(),
        external_contracts: vec![],
        network_topology: NetworkTopology::Star,
        domain: Some(DomainKind::IotDeviceTooling),
        architecture_tier: None,
        communication_chains: None,
        pin_assignments: Some(iot_air_quality_pins()),
        family: Some(ImplementationFamily::BrookesiaService),
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
    });

    // 31. LoRaWAN Gateway (SX1302)
    r.register(SolutionDefinition {
        id: "lorawan_gateway_solution".into(),
        label: "LoRaWAN Gateway (SX1302)".into(),
        label_zh: Some("LoRaWAN 网关（SX1302）".into()),
        kind: SolutionKind::ConnectivityBridge,
        supported_modules: vec!["esp32s3_wroom1".into()],
        fixed_inputs: vec![InputSurface::SpiSensor, InputSurface::TimerTick],
        fixed_outputs: vec![
            OutputSurface::NetworkApiState,
            OutputSurface::SpiMasterWrite,
        ],
        fixed_orchestration: vec![
            OrchestrationStep {
                id: "init_sx1302".into(),
                label: "Initialize SX1302 SPI, load firmware, reset sequence".into(),
                label_zh: Some("初始化 SX1302 SPI、加载固件、复位序列".into()),
                description: None,
                description_zh: None,
                depends_on: vec![],
                ..Default::default()
            },
            OrchestrationStep {
                id: "load_channel_plan".into(),
                label: "Load regional channel plan (8 channels)".into(),
                label_zh: Some("加载区域信道计划（8 信道）".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["init_sx1302".into()],
                ..Default::default()
            },
            OrchestrationStep {
                id: "rx_loop".into(),
                label: "Continuous 8-channel RX + packet deduplication".into(),
                label_zh: Some("8 信道持续接收 + 数据包去重".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["load_channel_plan".into()],
                ..Default::default()
            },
            OrchestrationStep {
                id: "uplink_to_ns".into(),
                label: "Forward uplinks to Network Server via Semtech UDP protocol".into(),
                label_zh: Some("通过 Semtech UDP 协议转发上行到网络服务器".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["rx_loop".into()],
                ..Default::default()
            },
            OrchestrationStep {
                id: "downlink_schedule".into(),
                label: "Schedule downlinks from NS on class-A RX windows".into(),
                label_zh: Some("在 class-A RX 窗口内调度来自网络服务器的下行".into()),
                description: None,
                description_zh: None,
                depends_on: vec!["uplink_to_ns".into()],
                ..Default::default()
            },
        ],
        scheduling: SchedulingPolicy {
            id: "packet_forwarder".into(),
            label: "Continuous packet forwarder".into(),
            decisions: vec![
                "Semtech UDP protocol (eu868/us915/cn470/as923/au915 selectable)".into(),
                "Class-A devices: downlink within 1 s / 2 s RX1 / RX2 windows".into(),
                "Keepalive to NS at keepalive_interval_s".into(),
            ],
        },
        user_parameters: vec![
            UserParameterDefinition {
                id: "region".into(),
                label: "LoRaWAN Region".into(),
                label_zh: Some("LoRaWAN 区域".into()),
                required: true,
                secret: false,
                description: "Regional channel plan / frequency band".into(),
                description_zh: Some("区域信道计划 / 频段。".into()),
                default_value: Some(serde_json::Value::String("eu868".into())),
                enum_values: Some(vec![
                    EnumOption {
                        value: "eu868".into(),
                        label: "EU868".into(),
                        description: Some("Europe 868 MHz.".into()),
                    },
                    EnumOption {
                        value: "us915".into(),
                        label: "US915".into(),
                        description: Some("North America 915 MHz.".into()),
                    },
                    EnumOption {
                        value: "cn470".into(),
                        label: "CN470".into(),
                        description: Some("China 470 MHz.".into()),
                    },
                    EnumOption {
                        value: "as923".into(),
                        label: "AS923".into(),
                        description: Some("Asia 923 MHz.".into()),
                    },
                    EnumOption {
                        value: "au915".into(),
                        label: "AU915".into(),
                        description: Some("Australia 915 MHz.".into()),
                    },
                ]),
                depends_on: None,
            },
            UserParameterDefinition {
                id: "network_server_url".into(),
                label: "Network Server URL".into(),
                label_zh: Some("网络服务器 URL".into()),
                required: true,
                secret: true,
                description:
                    "Semtech UDP packet-forwarder endpoint (e.g. udp://ns.example.com:1700)".into(),
                description_zh: Some(
                    "Semtech UDP 包转发器端点（例如 udp://ns.example.com:1700）。".into(),
                ),
                default_value: None,
                enum_values: None,
                depends_on: None,
            },
            UserParameterDefinition {
                id: "gateway_eui".into(),
                label: "Gateway EUI".into(),
                label_zh: Some("网关 EUI".into()),
                required: false,
                secret: true,
                description: "64-bit gateway identifier. Leave blank to auto-derive from MAC"
                    .into(),
                description_zh: Some("64 位网关标识。留空则由 MAC 自动派生。".into()),
                default_value: None,
                enum_values: None,
                depends_on: None,
            },
            UserParameterDefinition {
                id: "keepalive_interval_s".into(),
                label: "Keepalive Interval (s)".into(),
                label_zh: Some("保活间隔（秒）".into()),
                required: false,
                secret: false,
                description: "Seconds between PULL_DATA keepalives to the NS".into(),
                description_zh: Some("向网络服务器发送 PULL_DATA 保活的间隔秒数。".into()),
                default_value: Some(serde_json::Value::String("10".into())),
                enum_values: None,
                depends_on: None,
            },
        ],
        feedback_paths: vec![SignalPath {
            id: "lorawan_rx_forward".into(),
            name: "SX1302 RX to Network Server".into(),
            source: InputSurface::SpiSensor,
            transforms: vec![
                SignalPathStep {
                    order: 1,
                    node: TransformNode::SensorFrameDecode,
                    label: Some("SX1302 RX frame".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: TransformNode::Filter,
                    label: Some("Deduplication".into()),
                    description: None,
                },
                SignalPathStep {
                    order: 3,
                    node: TransformNode::ProtobufEncode,
                    label: Some("Semtech UDP encode".into()),
                    description: None,
                },
            ],
            sink: OutputSurface::NetworkApiState,
            feedback: vec![
                SignalPathStep {
                    order: 1,
                    node: FeedbackSurface::RuntimeMetrics,
                    label: None,
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: FeedbackSurface::LedIndicator,
                    label: None,
                    description: None,
                },
            ],
            expected_user_result:
                "LoRaWAN end-devices reach the NS via this gateway on the chosen band".into(),
        }],
        variants: vec![],
        component_bundle: ComponentBundle {
            required: vec!["spi".into(), "wifi".into()],
            optional: vec!["ota".into(), "api".into()],
        },
        runtime_binding: {
            let mut binding = brookesia_wifi_binding();
            binding.codegen_path = CodegenPath::SelfHosted;
            binding
        },
        external_contracts: vec!["Semtech UDP Packet Forwarder Protocol".into()],
        network_topology: NetworkTopology::Star,
        domain: Some(DomainKind::IotDeviceTooling),
        architecture_tier: None,
        communication_chains: None,
        pin_assignments: Some(iot_sx1302_pins()),
        family: Some(ImplementationFamily::Custom),
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
    });

    // Auto-populate topology_category for V&A solutions (ADR-01 / F1.3).
    r.populate_topology_category();

    r
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solution_definition_serde_roundtrip() {
        let def = SolutionDefinition {
            id: "test_sol".into(),
            label: "Test Solution".into(),
            label_zh: None,
            kind: SolutionKind::FirmwareAppliance,
            supported_modules: vec!["mod_a".into()],
            fixed_inputs: vec![InputSurface::ButtonGpio],
            fixed_outputs: vec![OutputSurface::GpioLevel],
            fixed_orchestration: vec![],
            scheduling: SchedulingPolicy {
                id: "simple".into(),
                label: "Simple".into(),
                decisions: vec![],
            },
            user_parameters: vec![],
            feedback_paths: vec![],
            variants: vec![],
            component_bundle: ComponentBundle {
                required: vec!["wifi".into()],
                optional: vec![],
            },
            runtime_binding: RuntimeBinding::default(),
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
        };
        let json = serde_json::to_string_pretty(&def).unwrap();
        let back: SolutionDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(def, back);
    }

    #[test]
    fn registry_crud() {
        let mut reg = SolutionRegistry::new();
        assert!(reg.get("foo").is_none());
        assert_eq!(reg.all().count(), 0);

        reg.register(SolutionDefinition {
            id: "foo".into(),
            label: "Foo".into(),
            label_zh: None,
            kind: SolutionKind::FirmwareAppliance,
            supported_modules: vec!["mod_a".into()],
            fixed_inputs: vec![],
            fixed_outputs: vec![],
            fixed_orchestration: vec![],
            scheduling: SchedulingPolicy {
                id: "s".into(),
                label: "S".into(),
                decisions: vec![],
            },
            user_parameters: vec![],
            feedback_paths: vec![],
            variants: vec![],
            component_bundle: ComponentBundle::default(),
            runtime_binding: RuntimeBinding::default(),
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
        });

        assert!(reg.get("foo").is_some());
        assert_eq!(reg.all().count(), 1);
    }

    #[test]
    fn for_module_filters_correctly() {
        let reg = default_solution_registry();
        let eye = reg.for_module("esp32s3_wroom1");
        // camera_stream, phone_browser_video, phone_rtsp_av, direct_control_video
        let ids: Vec<_> = eye.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"camera_stream"));
        assert!(ids.contains(&"phone_browser_video_solution"));
        assert!(ids.contains(&"direct_control_video_solution"));
    }

    #[test]
    fn for_kind_filters_correctly() {
        let reg = default_solution_registry();
        let firmware = reg.for_kind(SolutionKind::FirmwareAppliance);
        // 12 original FirmwareAppliance + 6 IoT FirmwareAppliance + bus_sampler = 19, +1 bus_sampler = 20,
        // + Phase 2 IoT (ld2410, ct_clamp, air_quality_pm) = 23,
        // + V&A pilot quad_stabilizer_solution = 24,
        // + V&A Phase 2 elrs_crsf_{brushed,brushless,dshot,mavlink} = 28,
        // + V&A Phase 3 TX pair (esp_now_tx, elrs_tx) = 30,
        // + V&A Phase 4 mcu_sbc_bridge = 31
        // + V&A Phase 5 priority-5 (direct_control_telemetry, balance_stabilizer,
        //   analog_vtx_passthrough, sbus_passthrough, fixedwing_stabilizer) = 36
        // + V&A Phase 6 form-factor pack (mecanum, marine, lta, agri,
        //   heli, modular, hopping, amphibious, vtol, rov, legged,
        //   articulated, climbing) = 49
        // (Phase 5d quad_stabilizer_dshot_solution + Phase 5d.2
        //  quad_stabilizer_bdshot_solution collapsed into variants of
        //  quad_stabilizer_solution by rshome-codegen-variants Phase 4.
        //  Net firmware count unchanged from the pre-Phase-5d baseline.)
        assert_eq!(firmware.len(), 49);
        let bridge = reg.for_kind(SolutionKind::ConnectivityBridge);
        // 6 original + zigbee_thread_gateway + lorawan_gateway (Phase 2) = 8,
        // + V&A Phase 3 phone_bridge = 9,
        // + V&A Phase 4 (video_board_sbc_companion, mavlink_groundstation, web_ui_groundstation) = 12
        // + post-va-residuals phone_bridge_crsf_solution (ADR-03 filler) = 13
        assert_eq!(bridge.len(), 13);
    }

    #[test]
    fn default_registry_solution_count() {
        let reg = default_solution_registry();
        // 44 baseline + 5 V&A priority-5 + 13 V&A form-factor pack
        // (mecanum, marine, lta, agri, heli, modular, hopping, amphibious,
        // vtol, rov, legged, articulated, climbing) = 62
        // + post-va-residuals phone_bridge_crsf_solution (ADR-03 filler) = 63
        // (Phase 5d quad_stabilizer_dshot_solution + Phase 5d.2
        //  quad_stabilizer_bdshot_solution collapsed into variants by
        //  rshome-codegen-variants Phase 4 T4.1. Total back to 63.)
        assert_eq!(reg.all().count(), 63);
    }

    /// Phase-4 variant-collapse contract test (rshome-codegen-variants
    /// PRD Phase 4 exit criterion). `quad_stabilizer_*_solution` ids must
    /// now exist only as variants on `quad_stabilizer_solution`.
    #[test]
    fn quad_stabilizer_variants_collapsed() {
        let reg = default_solution_registry();
        let sol = reg.get("quad_stabilizer_solution").expect("base present");
        assert_eq!(sol.variants.len(), 3);
        let ids: Vec<&str> = sol.variants.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, vec!["pwm", "dshot", "bdshot"]);
        assert!(
            reg.get("quad_stabilizer_dshot_solution").is_none(),
            "quad_stabilizer_dshot_solution must no longer be a top-level id",
        );
        assert!(
            reg.get("quad_stabilizer_bdshot_solution").is_none(),
            "quad_stabilizer_bdshot_solution must no longer be a top-level id",
        );

        // Variants carry the active-flag deltas the codegen dispatcher reads.
        let dshot = sol.variants.iter().find(|v| v.id == "dshot").unwrap();
        assert_eq!(dshot.active_flag_add, vec!["USE_DSHOT".to_string()]);
        let bdshot = sol.variants.iter().find(|v| v.id == "bdshot").unwrap();
        assert_eq!(bdshot.active_flag_add, vec!["USE_BDSHOT".to_string()]);

        // Variants override `runtime_binding.board_assembly` so Brookesia
        // Path A still resolves to the right physical board layout once
        // its variant-awareness follow-on lands.
        let dshot_binding = dshot
            .runtime_binding_override
            .as_ref()
            .expect("dshot runtime_binding_override");
        assert_eq!(
            dshot_binding.board_assembly.as_deref(),
            Some("esp32s3_va_multirotor_dshot_assembly"),
        );
        let bdshot_binding = bdshot
            .runtime_binding_override
            .as_ref()
            .expect("bdshot runtime_binding_override");
        assert_eq!(
            bdshot_binding.board_assembly.as_deref(),
            Some("esp32s3_va_multirotor_bdshot_assembly"),
        );
    }

    // ── Vehicle & Aircraft Control lints ────────────────────────────────────
    //
    // These enforce the design doc's §Verification rules (lines 671-680 of
    // `type-driven-ui/docs/vehicle-aircraft-control-dag.md`). They lock in
    // the per-solution chain + failsafe annotations so future additions to
    // the V&A domain can't forget them.

    #[test]
    fn vehicle_aircraft_solutions_declare_chain_enums() {
        let reg = default_solution_registry();
        for sol in reg.all() {
            if sol.domain != Some(DomainKind::VehicleAircraftControl) {
                continue;
            }
            assert!(
                sol.control_uplink.is_some(),
                "V&A solution '{}' missing control_uplink",
                sol.id
            );
            assert!(
                sol.video_downlink.is_some(),
                "V&A solution '{}' missing video_downlink",
                sol.id
            );
            assert!(
                sol.telemetry.is_some(),
                "V&A solution '{}' missing telemetry",
                sol.id
            );
        }
    }

    #[test]
    fn vehicle_actuator_solutions_declare_failsafe() {
        let reg = default_solution_registry();
        // Solutions without actuators (TX-only, video-only, relay) are exempt.
        let no_actuator = [
            "remote_control_tx_solution",
            "video_board_solution",
            "analog_vtx_passthrough_solution",
            "esp_now_tx_solution",
            "elrs_tx_solution",
            "phone_bridge_solution",
            "phone_bridge_crsf_solution",
            "video_board_sbc_companion_solution",
            "mavlink_groundstation_solution",
            "web_ui_groundstation_solution",
        ];
        for sol in reg.all() {
            if sol.domain != Some(DomainKind::VehicleAircraftControl) {
                continue;
            }
            if no_actuator.contains(&sol.id.as_str()) {
                continue;
            }
            let fs = sol
                .failsafe
                .as_ref()
                .unwrap_or_else(|| panic!("V&A actuator solution '{}' missing failsafe", sol.id));
            assert!(
                fs.rx_loss_behavior.is_some(),
                "V&A actuator solution '{}' missing rx_loss_behavior",
                sol.id
            );
            // Receiver Direct-Drive and SBUS Passthrough are the legacy
            // exceptions documented in the doc's §L5.5 table as
            // `watchdog_ms = null` (failsafe deferred to the RX itself).
            // Both labels are required to carry a `⚠️` marker — see
            // `vehicle_legacy_passthroughs_carry_warning_marker` below.
            let watchdog_exempt = matches!(
                sol.id.as_str(),
                "receiver_direct_drive_solution" | "sbus_passthrough_solution"
            );
            if !watchdog_exempt {
                assert!(
                    fs.watchdog_ms.is_some(),
                    "V&A actuator solution '{}' missing watchdog_ms",
                    sol.id
                );
            }
        }
    }

    #[test]
    fn quad_stabilizer_is_s3_only() {
        use crate::platform::{ChipCoverageStatus, ChipFamilyKind};
        let reg = default_solution_registry();
        let qs = reg
            .get("quad_stabilizer_solution")
            .expect("quad_stabilizer_solution must be registered");
        let cov = qs
            .chip_coverage
            .as_ref()
            .expect("quad_stabilizer_solution must declare chip_coverage");
        assert_eq!(
            cov.get(&ChipFamilyKind::Esp32S3),
            Some(&ChipCoverageStatus::Preferred),
            "quad_stabilizer should prefer S3"
        );
        assert_eq!(
            cov.get(&ChipFamilyKind::Esp32C6),
            Some(&ChipCoverageStatus::Insufficient),
            "quad_stabilizer should mark C6 as insufficient"
        );
    }

    #[test]
    fn default_solutions_have_feedback_paths() {
        let reg = default_solution_registry();
        // New TX/video/receiver solutions may not have feedback paths yet
        let skip = [
            "remote_control_tx_solution",
            "video_board_solution",
            "receiver_direct_drive_solution",
        ];
        for sol in reg.all() {
            if skip.contains(&sol.id.as_str()) {
                continue;
            }
            assert!(
                !sol.feedback_paths.is_empty(),
                "solution '{}' has no feedback paths",
                sol.id
            );
        }
    }

    #[test]
    fn composite_device_has_two_variants() {
        let reg = default_solution_registry();
        let sol = reg.get("composite_device_firmware").unwrap();
        assert_eq!(sol.variants.len(), 2);
        assert_eq!(sol.variants[0].id, "profile_a");
        assert_eq!(sol.variants[1].id, "profile_b");
    }

    /// Locks the JSON shape of `SolutionVariantDefinition` so the
    /// rshome-codegen-variants Phase 0 field additions round-trip
    /// cleanly for the only real variant consumer shipped today. If a
    /// future edit renames or retypes a variant field, this test fires
    /// before any `registry-data.json` re-export silently regresses.
    #[test]
    fn composite_device_variants_serde_roundtrip() {
        let reg = default_solution_registry();
        let sol = reg
            .get("composite_device_firmware")
            .expect("composite_device_firmware must be in the registry");
        for v in &sol.variants {
            let json = serde_json::to_string(v).expect("serialize variant");
            let back: SolutionVariantDefinition =
                serde_json::from_str(&json).expect("deserialize variant");
            assert_eq!(v, &back, "round-trip identity for variant '{}'", v.id);
            // The new Phase 0 fields default to empty; confirm that the
            // JSON does not carry them (skip_serializing_if) so old
            // registry-data.json payloads without them still deserialize.
            assert!(
                !json.contains("active_flag_add"),
                "unused `active_flag_add` leaked into JSON for '{}': {json}",
                v.id,
            );
            assert!(
                !json.contains("user_parameter_overrides"),
                "unused `user_parameter_overrides` leaked into JSON for '{}'",
                v.id,
            );
            assert!(
                !json.contains("runtime_binding_override"),
                "unused `runtime_binding_override` leaked into JSON for '{}'",
                v.id,
            );
        }
    }

    /// Confirm the new overlay types serialize to the expected shape so
    /// TS consumers can mirror their types deterministically.
    #[test]
    fn overlay_types_have_expected_json_shape() {
        let op = UserParameterOverride {
            op: UserParameterOverrideOp::Add,
            id: "dshot_rate_khz".into(),
            parameter: Some(UserParameterDefinition {
                id: "dshot_rate_khz".into(),
                label: "DShot Rate (kHz)".into(),
                label_zh: None,
                required: true,
                secret: false,
                description: "DShot bit rate".into(),
                description_zh: None,
                default_value: Some(serde_json::json!(600)),
                enum_values: None,
                depends_on: None,
            }),
        };
        let json = serde_json::to_value(&op).expect("serialize");
        assert_eq!(json["op"], "add");
        assert_eq!(json["id"], "dshot_rate_khz");
        assert!(json["parameter"].is_object());

        let rm = UserParameterOverride {
            op: UserParameterOverrideOp::Remove,
            id: "pwm_polarity".into(),
            parameter: None,
        };
        let json = serde_json::to_value(&rm).expect("serialize");
        assert_eq!(json["op"], "remove");
        assert_eq!(json["id"], "pwm_polarity");
        assert!(
            json.get("parameter").is_none(),
            "skip_serializing_if should elide None parameter"
        );

        let overlay = RuntimeBindingOverlay {
            add_custom_components: vec!["rshome_dshot".into()],
            remove_custom_components: vec!["rshome_pwm".into()],
            ..Default::default()
        };
        let json = serde_json::to_value(&overlay).expect("serialize");
        assert_eq!(json["add_custom_components"][0], "rshome_dshot");
        assert_eq!(json["remove_custom_components"][0], "rshome_pwm");
        assert!(json.get("board_assembly").is_none(), "None elided");
        assert!(json.get("add_managed_components").is_none(), "empty elided");
    }

    #[test]
    fn cross_reference_modules_exist() {
        let mod_reg = crate::module::default_module_registry();
        let sol_reg = default_solution_registry();
        for sol in sol_reg.all() {
            for mod_id in &sol.supported_modules {
                assert!(
                    mod_reg.get(mod_id).is_some(),
                    "solution '{}' references non-existent module '{}'",
                    sol.id,
                    mod_id
                );
            }
        }
    }

    #[test]
    fn cross_reference_required_components_exist() {
        let comp_reg = crate::registry::ComponentRegistry::default_registry();
        let sol_reg = default_solution_registry();
        for sol in sol_reg.all() {
            for comp_id in &sol.component_bundle.required {
                assert!(
                    comp_reg.get(comp_id).is_some(),
                    "solution '{}' requires non-existent component '{}'",
                    sol.id,
                    comp_id
                );
            }
        }
    }

    #[test]
    fn sensor_hub_supports_two_modules() {
        let reg = default_solution_registry();
        let sol = reg.get("sensor_hub").unwrap();
        assert_eq!(sol.supported_modules.len(), 2);
    }

    #[test]
    fn sensor_hub_has_primary_sensor_param() {
        let reg = default_solution_registry();
        let sol = reg.get("sensor_hub").unwrap();
        let param = sol
            .user_parameters
            .iter()
            .find(|p| p.id == "primary_sensor")
            .unwrap();
        assert!(param.required);
        assert_eq!(param.enum_values.as_ref().unwrap().len(), 8);
        // Default is BME280
        assert_eq!(param.default_value.as_ref().unwrap(), "bme280_i2c");
    }

    #[test]
    fn sensor_hub_has_secondary_sensor_param() {
        let reg = default_solution_registry();
        let sol = reg.get("sensor_hub").unwrap();
        let param = sol
            .user_parameters
            .iter()
            .find(|p| p.id == "secondary_sensor")
            .unwrap();
        assert!(!param.required);
        assert_eq!(param.default_value.as_ref().unwrap(), "none");
    }

    #[test]
    fn sensor_hub_has_pin_assignments() {
        let reg = default_solution_registry();
        let sol = reg.get("sensor_hub").unwrap();
        assert!(sol.pin_assignments.is_some());
    }

    #[test]
    fn for_module_sensor_hub_from_c6() {
        let reg = default_solution_registry();
        let sols = reg.for_module("esp32c6_wroom1");
        assert!(sols.iter().any(|s| s.id == "sensor_hub"));
    }

    #[test]
    fn ble_scanner_proxy_has_ha_entities() {
        let reg = default_solution_registry();
        let sol = reg.get("ble_scanner_proxy").unwrap();
        assert!(!sol.runtime_binding.ha_entities.is_empty());
        assert_eq!(sol.runtime_binding.ha_entities.len(), 4);
    }

    #[test]
    fn zigbee_thread_gateway_has_ha_entities() {
        let reg = default_solution_registry();
        let sol = reg.get("zigbee_thread_gateway").unwrap();
        assert!(!sol.runtime_binding.ha_entities.is_empty());
        assert_eq!(sol.runtime_binding.ha_entities.len(), 3);
    }

    #[test]
    fn status_led_notifier_has_ha_entities() {
        let reg = default_solution_registry();
        let sol = reg.get("status_led_notifier").unwrap();
        assert!(!sol.runtime_binding.ha_entities.is_empty());
        assert_eq!(sol.runtime_binding.ha_entities.len(), 1);
        assert_eq!(
            sol.runtime_binding.ha_entities[0].kind,
            crate::ha_export::HaEntityKind::Light
        );
    }

    #[test]
    fn for_module_custom_board() {
        let reg = default_solution_registry();
        let sols = reg.for_module("esp32s3_wroom1");
        // Custom board supports vehicle solutions + dual_mcu_car
        assert!(
            sols.len() >= 4,
            "custom board should support vehicle solutions, got {}",
            sols.len()
        );
        let ids: Vec<&str> = sols.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"direct_control_solution"));
    }

    #[test]
    fn for_module_eye_board() {
        let reg = default_solution_registry();
        let sols = reg.for_module("esp32s3_wroom1");
        let ids: Vec<&str> = sols.iter().map(|s| s.id.as_str()).collect();
        assert!(
            ids.contains(&"camera_stream"),
            "EYE should support camera_stream"
        );
        assert!(
            ids.contains(&"direct_control_video_solution"),
            "EYE should support video control"
        );
    }

    #[test]
    fn runtime_binding_serde_roundtrip() {
        let binding = RuntimeBinding {
            family: Some("brookesia_service".into()),
            managed_components: vec![ManagedComponentDep {
                name: "brookesia_service_wifi".into(),
                version: Some("~0.7".into()),
                git: None,
                namespace: None,
            }],
            custom_components: vec!["rshome_failsafe".into()],
            parameter_projection: BTreeMap::from([("key".into(), "comp.field".into())]),
            board_assembly: Some("esp32s3_gpio_relay_assembly".into()),
            ha_entities: vec![],
            codegen_path: CodegenPath::BrookesiaManaged,
        };
        let json = serde_json::to_string_pretty(&binding).unwrap();
        let back: RuntimeBinding = serde_json::from_str(&json).unwrap();
        assert_eq!(binding, back);
    }

    #[test]
    fn codegen_path_default_is_brookesia() {
        assert_eq!(CodegenPath::default(), CodegenPath::BrookesiaManaged);
    }

    #[test]
    fn codegen_path_serde() {
        let json = serde_json::to_string(&CodegenPath::BrookesiaManaged).unwrap();
        assert_eq!(json, "\"brookesia_managed\"");
        let json2 = serde_json::to_string(&CodegenPath::SelfHosted).unwrap();
        assert_eq!(json2, "\"self_hosted\"");
        let back: CodegenPath = serde_json::from_str(&json).unwrap();
        assert_eq!(back, CodegenPath::BrookesiaManaged);
    }

    #[test]
    fn runtime_binding_default_has_brookesia_path() {
        let rb = RuntimeBinding::default();
        assert_eq!(rb.codegen_path, CodegenPath::BrookesiaManaged);
        assert!(rb.board_assembly.is_none());
        assert!(rb.ha_entities.is_empty());
    }

    #[test]
    fn runtime_binding_with_board_assembly_roundtrip() {
        use crate::ha_export;
        let binding = RuntimeBinding {
            family: Some("brookesia_service".into()),
            managed_components: vec![],
            custom_components: vec![],
            parameter_projection: BTreeMap::new(),
            board_assembly: Some("esp32s3_gpio_relay_assembly".into()),
            ha_entities: vec![ha_export::HaEntityExportDefinition::switch_entity(
                "relay_1",
                "Relay 1",
                vec![ha_export::CommandBinding {
                    ha_command: "turn_on".into(),
                    service_function: "gpio.set".into(),
                    parameter_map: BTreeMap::from([("value".into(), serde_json::json!(true))]),
                }],
                ha_export::StateBinding {
                    source_event: "gpio.state".into(),
                    field_map: BTreeMap::from([("is_on".into(), "value".into())]),
                },
            )],
            codegen_path: CodegenPath::BrookesiaManaged,
        };
        let json = serde_json::to_string_pretty(&binding).unwrap();
        let back: RuntimeBinding = serde_json::from_str(&json).unwrap();
        assert_eq!(binding, back);
        assert_eq!(back.ha_entities.len(), 1);
        assert_eq!(back.ha_entities[0].object_id, "relay_1");
    }

    #[test]
    fn vehicle_solutions_have_custom_components() {
        let reg = default_solution_registry();
        for id in &["direct_control_solution", "direct_control_video_solution"] {
            let sol = reg.get(id).unwrap();
            assert!(
                !sol.runtime_binding.custom_components.is_empty(),
                "vehicle solution '{}' should have custom components (motor/imu/failsafe)",
                id
            );
        }
    }

    #[test]
    fn firmware_solutions_have_managed_components() {
        let reg = default_solution_registry();
        // New vehicle role solutions use default runtime binding (no managed components yet)
        let skip = [
            "remote_control_tx_solution",
            "video_board_solution",
            "receiver_direct_drive_solution",
        ];
        for sol in reg.all() {
            if skip.contains(&sol.id.as_str()) {
                continue;
            }
            if sol.component_bundle.required.contains(&"wifi".into()) {
                assert!(
                    !sol.runtime_binding.managed_components.is_empty(),
                    "solution '{}' requires wifi but has no managed components",
                    sol.id
                );
            }
        }
    }

    // Retired 2026-04-21 alongside the `implementation_family` struct
    // field (va-residuals Q11 full resolution). The typed `family` field
    // serves the same intent; the `direct_control_*` solutions now have
    // `family: Some(ImplementationFamily::EspDrone)` and the two phone_*
    // solutions have their own lineage variants. The presence guarantee
    // is covered by `va_chain_presence.rs` at a higher-value layer.

    // ── Phase 6: Flix integration tests ──────────────────────────────────

    #[test]
    fn vehicle_solutions_tagged_with_domain() {
        use crate::platform::DomainKind;
        let reg = default_solution_registry();
        for id in [
            "direct_control_solution",
            "direct_control_video_solution",
            "dual_mcu_car_solution",
        ] {
            let s = reg.get(id).unwrap();
            assert_eq!(
                s.domain,
                Some(DomainKind::VehicleAircraftControl),
                "solution '{id}' should be VehicleAircraftControl"
            );
        }
    }

    #[test]
    fn vehicle_solutions_have_enum_parameters() {
        let reg = default_solution_registry();
        for id in [
            "direct_control_solution",
            "direct_control_video_solution",
            "dual_mcu_car_solution",
        ] {
            let s = reg.get(id).unwrap();
            let has_enum = s.user_parameters.iter().any(|p| p.enum_values.is_some());
            assert!(has_enum, "solution '{id}' should have enum parameters");
        }
    }

    #[test]
    fn vehicle_solutions_have_pin_assignments() {
        let reg = default_solution_registry();
        for id in [
            "direct_control_solution",
            "direct_control_video_solution",
            "dual_mcu_car_solution",
        ] {
            let s = reg.get(id).unwrap();
            assert!(s.pin_assignments.is_some(), "solution '{id}' needs pins");
            assert!(!s.pin_assignments.as_ref().unwrap().is_empty());
        }
    }

    #[test]
    fn vehicle_pin_defaults_have_no_conflicts() {
        let reg = default_solution_registry();
        for id in [
            "direct_control_solution",
            "direct_control_video_solution",
            "dual_mcu_car_solution",
        ] {
            let s = reg.get(id).unwrap();
            if let Some(pins) = &s.pin_assignments {
                let mut seen = std::collections::HashMap::new();
                for pin in pins {
                    seen.entry(pin.default_gpio)
                        .or_insert_with(Vec::new)
                        .push(&pin.function);
                }
                for (gpio, fns) in &seen {
                    assert!(
                        fns.len() <= 1,
                        "solution '{id}': GPIO {gpio} conflict: {fns:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn dual_mcu_solution_has_architecture_tier() {
        use crate::platform::ArchitectureTier;
        let reg = default_solution_registry();
        let s = reg.get("dual_mcu_car_solution").unwrap();
        assert_eq!(s.architecture_tier, Some(McuRole::ControlBoard));
    }

    #[test]
    fn dual_mcu_solution_modules_exist() {
        let mod_reg = crate::module::default_module_registry();
        let sol_reg = default_solution_registry();
        let s = sol_reg.get("dual_mcu_car_solution").unwrap();
        for m in &s.supported_modules {
            assert!(
                mod_reg.get(m).is_some(),
                "dual_mcu_car_solution references non-existent module '{m}'"
            );
        }
    }

    #[test]
    fn all_depends_on_references_valid_parameter() {
        let reg = default_solution_registry();
        for s in reg.all() {
            let ids: Vec<&str> = s.user_parameters.iter().map(|p| p.id.as_str()).collect();
            for p in &s.user_parameters {
                if let Some(dep) = &p.depends_on {
                    assert!(
                        ids.contains(&dep.parameter_id.as_str()),
                        "solution '{}' param '{}' depends_on '{}' not found",
                        s.id,
                        p.id,
                        dep.parameter_id
                    );
                }
            }
        }
    }

    // ── IoT domain tests ────────────────────────────────────────────────

    #[test]
    fn iot_domain_has_ten_solutions() {
        use crate::platform::DomainKind;
        let reg = default_solution_registry();
        let iot: Vec<_> = reg
            .all()
            .filter(|s| s.domain == Some(DomainKind::IotDeviceTooling))
            .collect();
        assert_eq!(
            iot.len(),
            17,
            "IoT domain should have 17 solutions, got: {:?}",
            iot.iter().map(|s| &s.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn mqtt_gateway_supports_two_modules() {
        let reg = default_solution_registry();
        let sol = reg.get("mqtt_gateway_bridge").unwrap();
        assert_eq!(sol.supported_modules.len(), 2);
        assert!(sol.supported_modules.contains(&"esp32s3_wroom1".into()));
        assert!(sol.supported_modules.contains(&"esp32c6_wroom1".into()));
    }

    #[test]
    fn ble_scanner_cascading_target_mac() {
        let reg = default_solution_registry();
        let sol = reg.get("ble_scanner_proxy").unwrap();
        let target_mac = sol
            .user_parameters
            .iter()
            .find(|p| p.id == "target_mac")
            .unwrap();
        let dep = target_mac.depends_on.as_ref().unwrap();
        assert_eq!(dep.parameter_id, "device_filter");
        assert_eq!(dep.when_value, "custom_mac");
    }

    #[test]
    fn mqtt_gateway_poll_interval_hidden_for_espnow_relay() {
        let reg = default_solution_registry();
        let sol = reg.get("mqtt_gateway_bridge").unwrap();
        let poll = sol
            .user_parameters
            .iter()
            .find(|p| p.id == "poll_interval_ms")
            .unwrap();
        let dep = poll.depends_on.as_ref().unwrap();
        assert_eq!(dep.parameter_id, "data_source");
        assert_eq!(dep.when_not_value.as_deref(), Some("espnow_relay"));
    }

    // ── New IoT solution tests ──────────────────────────────────────────

    #[test]
    fn env_data_logger_exists_with_pin_assignments() {
        let reg = default_solution_registry();
        let sol = reg.get("env_data_logger").unwrap();
        assert_eq!(sol.kind, SolutionKind::FirmwareAppliance);
        assert!(sol.pin_assignments.is_some());
        assert_eq!(sol.supported_modules.len(), 2);
    }

    #[test]
    fn thread_zigbee_sensor_c6_exclusive() {
        let reg = default_solution_registry();
        let sol = reg.get("thread_zigbee_sensor").unwrap();
        assert_eq!(sol.supported_modules.len(), 2);
        assert!(sol
            .supported_modules
            .iter()
            .all(|m| m.starts_with("esp32c6")));
    }

    #[test]
    fn zigbee_thread_gateway_c6_exclusive() {
        let reg = default_solution_registry();
        let sol = reg.get("zigbee_thread_gateway").unwrap();
        assert_eq!(sol.supported_modules.len(), 1);
        assert_eq!(sol.supported_modules[0], "esp32c6_wroom1");
    }

    #[test]
    fn uart_debug_probe_s3_exclusive() {
        let reg = default_solution_registry();
        let sol = reg.get("uart_debug_probe").unwrap();
        assert_eq!(sol.supported_modules.len(), 1);
        assert_eq!(sol.supported_modules[0], "esp32s3_wroom1");
        assert!(sol.pin_assignments.is_some());
    }

    #[test]
    fn sigrok_la_s3_exclusive() {
        let reg = default_solution_registry();
        let sol = reg.get("sigrok_logic_analyzer").unwrap();
        assert_eq!(sol.supported_modules.len(), 1);
        assert_eq!(sol.supported_modules[0], "esp32s3_wroom1");
        assert_eq!(sol.kind, SolutionKind::FirmwareAppliance);
        assert!(sol.pin_assignments.is_some());
    }

    #[test]
    fn sigrok_la_has_five_parameters() {
        let reg = default_solution_registry();
        let sol = reg.get("sigrok_logic_analyzer").unwrap();
        assert_eq!(sol.user_parameters.len(), 5);
        let ids: Vec<&str> = sol.user_parameters.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&"channel_count"));
        assert!(ids.contains(&"sample_rate"));
        assert!(ids.contains(&"capture_depth"));
        assert!(ids.contains(&"trigger_mode"));
        assert!(ids.contains(&"transport"));
    }

    #[test]
    fn sigrok_la_requires_sigrok_component() {
        let reg = default_solution_registry();
        let sol = reg.get("sigrok_logic_analyzer").unwrap();
        assert!(sol.component_bundle.required.contains(&"sigrok".into()));
    }

    #[test]
    fn i2c_bus_analyzer_supports_two_modules() {
        let reg = default_solution_registry();
        let sol = reg.get("i2c_bus_analyzer").unwrap();
        assert_eq!(sol.supported_modules.len(), 2);
        assert!(sol.supported_modules.contains(&"esp32s3_wroom1".into()));
        assert!(sol.supported_modules.contains(&"esp32c6_wroom1".into()));
    }

    #[test]
    fn dashboard_display_s3_exclusive() {
        let reg = default_solution_registry();
        let sol = reg.get("dashboard_display").unwrap();
        assert_eq!(sol.supported_modules.len(), 1);
        assert_eq!(sol.supported_modules[0], "esp32s3_wroom1");
        assert_eq!(sol.feedback_paths.len(), 2);
    }

    #[test]
    fn status_led_notifier_has_dual_signal_paths() {
        let reg = default_solution_registry();
        let sol = reg.get("status_led_notifier").unwrap();
        assert_eq!(sol.feedback_paths.len(), 2);
        let ids: Vec<&str> = sol.feedback_paths.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&"mqtt_to_led"));
        assert!(ids.contains(&"sensor_to_led"));
    }

    #[test]
    fn all_new_iot_solutions_have_no_architecture_tier() {
        use crate::platform::DomainKind;
        let reg = default_solution_registry();
        let new_ids = [
            "env_data_logger",
            "thread_zigbee_sensor",
            "zigbee_thread_gateway",
            "uart_debug_probe",
            "i2c_bus_analyzer",
            "dashboard_display",
            "status_led_notifier",
        ];
        for id in new_ids {
            let sol = reg.get(id).unwrap();
            assert_eq!(
                sol.domain,
                Some(DomainKind::IotDeviceTooling),
                "{id} should be IoT domain"
            );
            assert_eq!(
                sol.architecture_tier, None,
                "{id} should have no architecture_tier"
            );
        }
    }
}
