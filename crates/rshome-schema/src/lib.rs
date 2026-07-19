//! `rshome-schema` — Component registry, entity types, platform definitions.
//!
//! This crate is the Phase 0 foundation of Yet-Another-ESPHome (YAE).  It defines
//! the type system consumed by all subsequent phases: config validation, code
//! generation, and the browser wizard.
//!
//! # Modules
//!
//! - [`entity`] — Rust types for all 16+ ESPHome entity categories.
//! - [`registry`] — Component/platform hierarchy and dependency resolution.
//! - [`feature_flags`] — Maps component selections to `USE_*` C defines.
//! - [`pin`] — GPIO and peripheral resource allocation / conflict detection.
//! - [`export`] — JSON Schema export (and future GraphQL via `--features graphql`).
//!
//! # Wasm compilation
//!
//! This crate compiles for `wasm32-unknown-unknown` without modification.  Enable
//! the `wasm` feature to expose `wasm-bindgen` exports:
//!
//! ```bash
//! wasm-pack build crates/rshome-schema --target web -- --features wasm
//! ```

pub mod assembly;
pub mod entity;
pub mod export;
pub mod feature_flags;
#[cfg(feature = "dag")]
pub mod graph;
pub mod ha_export;
pub mod module;
pub mod orchestration;
pub mod pin;
pub mod platform;
pub mod profile;
pub mod registry;
pub mod solution;
pub mod solution_legacy;

// ── Re-exports ────────────────────────────────────────────────────────────────

pub use assembly::{
    AssemblyId, AssemblyRegistry, DeviceDeclaration, HardwareAssemblyDefinition,
    PeripheralDeclaration,
};
pub use entity::{
    AlarmCodeFormat, AlarmControlPanelFeature, AlarmControlPanelSchema, BinaryFilterConfig,
    BinarySensorDeviceClass, BinarySensorSchema, ButtonDeviceClass, ButtonSchema, ClimateMode,
    ClimatePreset, ClimateSchema, ClimateTraitsConfig, ClimateVisualConfig, CoverDeviceClass,
    CoverSchema, EntityCategory, EntityCommon, EntitySchema, EntityType, EventDeviceClass,
    EventSchema, FanSchema, FilterConfig, LightRestoreMode, LightSchema, LightType, LockSchema,
    MediaPlayerFeature, MediaPlayerSchema, NumberMode, NumberSchema, RestoreMode, SelectSchema,
    SensorDeviceClass, SensorSchema, StateClass, SwitchDeviceClass, SwitchSchema, TextFilterConfig,
    TextMode, TextSchema, TextSensorDeviceClass, TextSensorSchema,
};
pub use feature_flags::{FeatureCategory, FeatureFlag, FeatureFlagSet};
#[cfg(feature = "dag")]
pub use graph::{
    BuildPipelineDag, BuildStageKind, BuildStageNode, ComponentDag, ComponentNode, CycleError,
    DepEdge, OrchestrationDag, OrchestrationNode, SignalNode, SignalPathDag,
};
pub use ha_export::{CommandBinding, HaEntityExportDefinition, HaEntityKind, StateBinding};
pub use module::{ModuleDefinition, ModuleId, ModuleRegistry};
pub use pin::{PinAllocation, PinMode, PullMode, ResourceTracker};
pub use platform::{
    ArchitectureTier, Capability, CapabilityProfile, ChipTarget, CommunicationChainKind,
    ComponentDomain, ComponentInteraction, ComponentPlatformBinding, DomainKind, FeedbackSurface,
    InputSurface, McuRole, OutputSurface, PlatformCatalog, PlatformDefinition, PlatformKind,
    PlatformTargetDefinition, PlatformTree, SignalPath, SignalPathStep, SignalPathTemplate,
    TransformNode,
};
pub use profile::{ManagerFeature, ProfileDefinition, ProfileRegistry, ResourceBindings};
pub use registry::{
    ComponentDefinition, ComponentId, ComponentRegistry, ConfigMode, InstancePolicy, PlatformId,
};
pub use solution::{
    CodegenPath, ComponentBundle, ManagedComponentDep, NetworkTopology, OrchestrationStep,
    RuntimeBinding, SchedulingPolicy, SolutionDefinition, SolutionId, SolutionKind,
    SolutionRegistry, SolutionVariantDefinition, UserParameterDefinition, VariantId,
};

// ── Wasm bindings (browser export surface) ────────────────────────────────────

#[cfg(feature = "wasm")]
pub mod wasm_bindings;
