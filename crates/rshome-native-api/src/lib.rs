#![allow(clippy::module_name_repetitions)]

pub mod codec;
pub mod command_dispatch;
pub mod connection;
pub mod entity_list;
pub mod error;
pub mod fnv;
pub mod mdns;
pub mod msg_types;
pub mod proto_gen;
pub mod server;
pub mod state_push;

// ── Stable protocol surface ──────────────────────────────────────────────────
// Used by `rshome-device-link` and any future HA-side clients.

/// TCP port on which ESPHome-compatible firmware exposes the Native API.
pub const NATIVE_API_PORT: u16 = 6053;

/// Ping interval for the HA-side Native API client session.
/// Governs future `rshome-device-link` client heartbeat behavior.
pub const CLIENT_PING_INTERVAL_SECS: u64 = 15;

/// Hard inactivity timeout for the HA-side client session.
pub const CLIENT_INACTIVITY_TIMEOUT_SECS: u64 = 30;

pub use codec::EspHomeCodec;
pub use error::ApiError;
pub use fnv::{entity_key, fnv1a_32};
pub use state_push::{climate_mode_to_int, state_to_frame};

// ── Transitional server modules ──────────────────────────────────────────────
// transitional — do not use for new HA-side work

pub use command_dispatch::CommandDispatcher;
pub use connection::{ConnectionActor, ConnectionMsg};
pub use server::{NativeApiMsg, NativeApiServerActor};
