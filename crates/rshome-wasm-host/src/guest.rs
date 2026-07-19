//! GuestFunctions trait + MockGuest + WasmGuest.
//!
//! All bridges and actors are written against `GuestFunctions`, making them
//! testable without compiling real WASM.  `MockGuest` records every call for
//! assertion in unit tests.  `WasmGuest` wraps a real wasmtime component
//! (execution deferred to Phase 6; Phase 3 validates structure only).

/// All callable exports a WASM integration module must implement.
///
/// Methods are synchronous so they can be called from `spawn_blocking`.
pub trait GuestFunctions: Send + Sync + 'static {
    /// Initialize the integration with key-value config pairs.
    fn setup(&self, config: Vec<(String, String)>) -> Result<(), String>;

    /// Tear down the integration (called on unload / actor stop).
    fn teardown(&self);

    /// Drive one step of the config-flow UI.
    fn config_flow_step(
        &self,
        step_id: &str,
        user_input: Option<Vec<(String, String)>>,
    ) -> Result<Vec<u8>, String>;

    /// Periodic coordinator refresh — returns updated data as JSON bytes.
    fn coordinator_update(&self, coordinator_handle: u64) -> Result<Vec<u8>, String>;

    /// Return diagnostic key-value pairs for this integration.
    fn get_diagnostics(&self) -> Vec<(String, String)>;

    /// Drive one step of a repair flow.
    fn repair_step(
        &self,
        repair_id: &str,
        step_id: &str,
        user_input: Option<Vec<(String, String)>>,
    ) -> Result<Vec<u8>, String>;
}

// ── Call recording ─────────────────────────────────────────────────────────────

/// A record of a single call made to a `MockGuest`.
#[derive(Debug, Clone, PartialEq)]
pub enum GuestCall {
    Setup(Vec<(String, String)>),
    Teardown,
    ConfigFlowStep {
        step_id: String,
        user_input: Option<Vec<(String, String)>>,
    },
    CoordinatorUpdate {
        coordinator_handle: u64,
    },
    GetDiagnostics,
    RepairStep {
        repair_id: String,
        step_id: String,
        user_input: Option<Vec<(String, String)>>,
    },
}

// ── MockGuest ─────────────────────────────────────────────────────────────────

/// Test double that records calls and returns configurable responses.
pub struct MockGuest {
    /// All recorded calls in order.
    pub calls: parking_lot::Mutex<Vec<GuestCall>>,

    // Configurable outcomes (None = use defaults below)
    pub setup_error: Option<String>,
    pub config_flow_bytes: Vec<u8>,
    pub config_flow_error: Option<String>,
    pub coordinator_bytes: Vec<u8>,
    pub coordinator_error: Option<String>,
    pub diagnostics: Vec<(String, String)>,
    pub repair_bytes: Vec<u8>,
    pub repair_error: Option<String>,
}

impl Default for MockGuest {
    fn default() -> Self {
        Self {
            calls: parking_lot::Mutex::new(Vec::new()),
            setup_error: None,
            config_flow_bytes: br#"{"type":"form"}"#.to_vec(),
            config_flow_error: None,
            coordinator_bytes: br#"{"temperature":22.5}"#.to_vec(),
            coordinator_error: None,
            diagnostics: vec![("status".into(), "ok".into())],
            repair_bytes: br#"{"complete":true}"#.to_vec(),
            repair_error: None,
        }
    }
}

impl GuestFunctions for MockGuest {
    fn setup(&self, config: Vec<(String, String)>) -> Result<(), String> {
        self.calls.lock().push(GuestCall::Setup(config));
        match &self.setup_error {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }

    fn teardown(&self) {
        self.calls.lock().push(GuestCall::Teardown);
    }

    fn config_flow_step(
        &self,
        step_id: &str,
        user_input: Option<Vec<(String, String)>>,
    ) -> Result<Vec<u8>, String> {
        self.calls.lock().push(GuestCall::ConfigFlowStep {
            step_id: step_id.to_owned(),
            user_input: user_input.clone(),
        });
        match &self.config_flow_error {
            Some(e) => Err(e.clone()),
            None => Ok(self.config_flow_bytes.clone()),
        }
    }

    fn coordinator_update(&self, coordinator_handle: u64) -> Result<Vec<u8>, String> {
        self.calls
            .lock()
            .push(GuestCall::CoordinatorUpdate { coordinator_handle });
        match &self.coordinator_error {
            Some(e) => Err(e.clone()),
            None => Ok(self.coordinator_bytes.clone()),
        }
    }

    fn get_diagnostics(&self) -> Vec<(String, String)> {
        self.calls.lock().push(GuestCall::GetDiagnostics);
        self.diagnostics.clone()
    }

    fn repair_step(
        &self,
        repair_id: &str,
        step_id: &str,
        user_input: Option<Vec<(String, String)>>,
    ) -> Result<Vec<u8>, String> {
        self.calls.lock().push(GuestCall::RepairStep {
            repair_id: repair_id.to_owned(),
            step_id: step_id.to_owned(),
            user_input: user_input.clone(),
        });
        match &self.repair_error {
            Some(e) => Err(e.clone()),
            None => Ok(self.repair_bytes.clone()),
        }
    }
}

// ── WasmGuest ─────────────────────────────────────────────────────────────────

/// Production guest backed by a real wasmtime component.
///
/// Phase 3: validates WASM magic and stores bytes.
/// Phase 6: full component instantiation with host function linking.
pub struct WasmGuest {
    wasm_bytes: Vec<u8>,
}

impl WasmGuest {
    /// Create a `WasmGuest` from raw WASM component bytes.
    ///
    /// Validates the WASM magic header.  Full component-model validation
    /// (exports present, types match, fuel budget) is deferred to Phase 6.
    pub fn new(wasm_bytes: Vec<u8>) -> Result<Self, String> {
        if wasm_bytes.len() < 8 {
            return Err("WASM bytes too short".into());
        }
        // WASM magic: \0asm
        if &wasm_bytes[0..4] != b"\0asm" {
            return Err("invalid WASM magic (expected \\0asm)".into());
        }
        Ok(Self { wasm_bytes })
    }

    /// Lazily initialized process-wide wasmtime Engine (component model enabled).
    fn engine() -> &'static wasmtime::Engine {
        use std::sync::OnceLock;
        static ENGINE: OnceLock<wasmtime::Engine> = OnceLock::new();
        ENGINE.get_or_init(|| {
            let mut config = wasmtime::Config::new();
            config.wasm_component_model(true);
            wasmtime::Engine::new(&config).expect("wasmtime Engine init failed")
        })
    }

    /// Return the raw WASM bytes (useful for diagnostics / validation tools).
    pub fn wasm_bytes(&self) -> &[u8] {
        &self.wasm_bytes
    }

    /// Compile the component without instantiating it.
    /// Used for static analysis / size validation.
    pub fn try_compile(&self) -> Result<(), String> {
        wasmtime::component::Component::from_binary(Self::engine(), &self.wasm_bytes)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

impl GuestFunctions for WasmGuest {
    fn setup(&self, _config: Vec<(String, String)>) -> Result<(), String> {
        // Phase 6: instantiate component with host function linker and call export
        tracing::warn!(
            bytes = self.wasm_bytes.len(),
            "WasmGuest::setup — component execution deferred to Phase 6"
        );
        Ok(())
    }

    fn teardown(&self) {}

    fn config_flow_step(
        &self,
        _step_id: &str,
        _user_input: Option<Vec<(String, String)>>,
    ) -> Result<Vec<u8>, String> {
        Err("WasmGuest component execution not yet implemented (Phase 6)".into())
    }

    fn coordinator_update(&self, _coordinator_handle: u64) -> Result<Vec<u8>, String> {
        Err("WasmGuest component execution not yet implemented (Phase 6)".into())
    }

    fn get_diagnostics(&self) -> Vec<(String, String)> {
        vec![("status".into(), "wasm_execution_deferred".into())]
    }

    fn repair_step(
        &self,
        _repair_id: &str,
        _step_id: &str,
        _user_input: Option<Vec<(String, String)>>,
    ) -> Result<Vec<u8>, String> {
        Err("WasmGuest component execution not yet implemented (Phase 6)".into())
    }
}
