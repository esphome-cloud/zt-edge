//! `rshome-wasm-host` — WASM integration host for rshome-ha.
//!
//! # Overview
//!
//! Custom Home Automation integrations are compiled to WASM Component Model
//! modules that implement the `rshome-integration` WIT world
//! (`wit/rshome-integration.wit`).  This crate provides:
//!
//! * [`WasmHostActor`] — loads/unloads integration modules, exposes them as
//!   child actors.
//! * [`IntegrationActor`] — per-integration lifecycle + message dispatch.
//! * Capability model ([`Handle`], [`Rights`], [`ObjectRegistry`]) for secure
//!   cross-boundary references.
//! * Bridge functions (`entity_bridge`, `service_bridge`, `discovery`, etc.)
//!   that implement the host-side of the WIT imports.
//! * [`GuestFunctions`] trait + [`MockGuest`] for test isolation.

#![allow(clippy::module_name_repetitions)]

pub mod capability;
pub mod config_flow;
pub mod context;
pub mod coordinator;
pub mod diagnostics;
pub mod discovery;
pub mod domain_lowering;
pub mod entity_bridge;
pub mod guest;
pub mod host_actor;
pub mod integration_actor;
pub mod repairs;
pub mod sdk_gen;
pub mod service_bridge;

// ── Public re-exports ─────────────────────────────────────────────────────────

pub use capability::{CapError, Handle, ObjectRegistry, Rights};
pub use context::{CapabilityContext, SharedCtx};
pub use guest::GuestFunctions;
pub use host_actor::{
    GuestFactory, IntegrationId, IntegrationInfo, WasmHostActor, WasmHostError, WasmHostMsg,
};
pub use integration_actor::{IntegrationActor, IntegrationMsg};

#[cfg(test)]
pub use guest::MockGuest;
