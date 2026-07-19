//! `rshome-wizard` — Wasm-compiled config engine for the browser wizard.
//!
//! This crate wraps `rshome-schema` and `rshome-config` behind a thin
//! `wasm-bindgen` surface that JavaScript can call from the browser.
//!
//! # Build
//!
//! ```bash
//! wasm-pack build crates/rshome-wizard --target web -- --features wasm
//! ```
//!
//! # Functions
//!
//! All functions accept and return JSON strings for zero-copy interoperability.
//!
//! | Function | Input | Output |
//! |---|---|---|
//! | `validate_config` | `RawConfig` JSON | `{valid, errors, active_flags, chip_target}` |
//! | `validate_partial` | `PartialInput` JSON | `ValidationError[]` |
//! | `list_components` | `{target, category}` JSON | `ComponentInfo[]` |
//! | `get_pin_map` | `"esp32"` \| `"esp32s3"` \| `"esp32c6"` | `PinInfo[]` |
//! | `compute_feature_flags` | component-ID array JSON | `{flags, c_defines, cargo_features}` |
//! | `get_component_schema` | component ID string | `ComponentDefinition` JSON |

pub mod bindings;
pub mod export;
pub mod pin_conflict;
pub mod pin_map;
pub mod types;
pub mod workspace;

#[cfg(feature = "wasm")]
pub mod wasm_bindings;
