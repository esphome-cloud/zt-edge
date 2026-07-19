//! `rshome-config` — 10-stage config validation pipeline for rshome.
//!
//! Sits on top of `rshome-schema` and transforms raw user-provided config into
//! a [`ValidatedConfig`] through a series of progressive validation stages.
//!
//! # Pipeline stages
//!
//! 1. **Package merge** — Load referenced packages and merge component lists.
//! 2. **Substitutions** — Replace `${var}` / `${var:=default}` placeholders.
//! 3. **Extend/Remove** — Process `!extend` and `!remove` directives.
//! 4. **External components** — Validate dependency source trust model.
//! 5. **Preload** — Parse `esphome:` block to determine platform/board/framework.
//! 6. **Component loading** — Resolve AUTO_LOAD chains via registry.
//! 7. **Schema validation** — Type-check all config values.
//! 8. **ID resolution** — Assign entity IDs, detect duplicates, check cross-refs.
//! 9. **Final validation** — Cross-component invariants (e.g. api requires wifi).
//! 10. **Pin conflicts** — Walk GPIO assignments, detect hardware conflicts.
//!
//! # Wasm compilation
//!
//! This crate compiles for `wasm32-unknown-unknown`.  Enable the `wasm` feature
//! to expose `wasm-bindgen` exports:
//!
//! ```bash
//! wasm-pack build crates/rshome-config --target web -- --features wasm
//! ```

pub mod error;
pub mod export;
pub mod pipeline;
pub mod raw;
pub mod sigrok;
pub mod stages;
pub mod validated;

pub use error::{Severity, ValidationError, ValidationStage};
pub use export::{export_config, import_esphome_yaml, ExportFormat, ImportError};
pub use pipeline::{PartialConfig, ValidationPipeline, ValidationResult};
pub use raw::{
    ComponentConfig, EsphomeBlock, FrameworkConfig, IdfComponentRef, PackageRef, PackageStore,
    ProjectConfig, RawConfig,
};
pub use validated::{
    DependencyGraph, FrameworkType, ValidatedComponent, ValidatedConfig, ValidatedEsphomeBlock,
};

#[cfg(feature = "wasm")]
pub mod wasm_bindings;
