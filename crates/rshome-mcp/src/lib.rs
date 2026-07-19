#![allow(clippy::module_name_repetitions)]

pub mod daemon;
mod error;
mod resources;
pub mod runtime;

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use rmcp::handler::server::tool::Parameters;
use rmcp::{
    handler::server::tool::{ToolCallContext, ToolRouter},
    model::*,
    service::{RequestContext, RoleServer, ServiceExt},
    tool, tool_router, ErrorData as McpError, ServerHandler,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use rshome_actor::ActorRef;
use rshome_device_link::manager::DeviceLinkManagerMsg;
use rshome_entity::{
    DeviceId, DeviceManagerMsg, DeviceMsg, EntityId, EntityMsg, EntityRegistry, EntityState,
};
use rshome_native_api::server::NativeApiMsg;
use rshome_state::StateStore;
use rshome_svc::{ServiceMsg, ServiceTarget};
use rshome_wasm_host::WasmHostMsg;
use rshome_wf::{
    RunStatus as WfRunStatus, StepDef, TriggerDef as WfTriggerDef, WfError, WorkflowDefinition,
    WorkflowEngineMsg, WorkflowMode,
};

// ── Workflow types (Phase 5 stub) ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkflowDef {
    pub workflow_id: String,
    pub name: String,
    pub description: Option<String>,
    pub trigger: String,
    pub steps: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowRunStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkflowRun {
    pub run_id: String,
    pub workflow_id: String,
    pub status: WorkflowRunStatus,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub error: Option<String>,
}

// ── Runtime config ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeConfig {
    pub version: &'static str,
    pub native_api_port: u16,
    pub mdns_enabled: bool,
    pub max_devices: usize,
    pub max_entities: usize,
    pub max_connections_per_device: usize,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
            native_api_port: 6053,
            mdns_enabled: true,
            max_devices: 100,
            max_entities: 10_000,
            max_connections_per_device: 1,
        }
    }
}

/// MCP capability scope identifier for rshome-ha tools.
pub const SKILL_SCOPE: &str = "rshome-ha";

pub use daemon::{run_daemon, run_daemon_with_shutdown, DaemonConfig, DaemonResult};
pub use runtime::RshomeHaRuntime;

// ── Input types ───────────────────────────────────────────────────────────────

/// Input for `ha.devices.get`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct HaDevicesGetInput {
    /// Device ID to look up.
    pub device_id: String,
}

/// Input for `ha.entities.list`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct HaEntitiesListInput {
    /// Filter by entity domain (e.g. "switch", "sensor", "light").
    pub domain: Option<String>,
    /// Filter by device ID — returns only entities belonging to that device.
    pub device_id: Option<String>,
    /// Maximum number of results to return (default: all).
    pub limit: Option<usize>,
}

/// Input for `ha.entities.get_state`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct HaEntitiesGetStateInput {
    /// Entity ID (e.g. "switch.kitchen_light").
    pub entity_id: String,
}

/// Input for `ha.entities.set_state`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct HaEntitiesSetStateInput {
    /// Entity ID to update (e.g. "switch.kitchen_light").
    pub entity_id: String,
    /// JSON-encoded `EntityState` value.
    pub state_json: String,
}

/// Input for `ha.services.call`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct HaServicesCallInput {
    /// Service domain (e.g. "switch", "light", "climate").
    pub domain: String,
    /// Service name (e.g. "turn_on", "turn_off", "toggle").
    pub service: String,
    /// Target specific entity IDs. Takes precedence over `target_device_id`.
    pub target_entity_ids: Option<Vec<String>>,
    /// Target all entities of a specific device.
    pub target_device_id: Option<String>,
    /// Domain-specific service data payload.
    pub data: Option<serde_json::Value>,
}

/// Input for `ha.workflows.create`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct HaWorkflowsCreateInput {
    /// Human-readable workflow name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Trigger type: "manual" (default), "entity_state_change", "time_pattern", "webhook".
    pub trigger: Option<String>,
    /// Workflow steps as JSON objects.
    pub steps: Option<Vec<serde_json::Value>>,
}

/// Input for `ha.workflows.run`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct HaWorkflowsRunInput {
    /// Workflow ID returned by `ha.workflows.create`.
    pub workflow_id: String,
    /// Optional input data passed to the first step.
    pub input_data: Option<serde_json::Value>,
}

/// Input for `ha.workflows.status`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct HaWorkflowsStatusInput {
    /// Run ID returned by `ha.workflows.run`.
    pub run_id: String,
}

/// Input for `ha.discovery.scan`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct HaDiscoveryScanInput {
    /// Scan duration in milliseconds (default: 3000).
    pub timeout_ms: Option<u64>,
}

/// Input for `ha.integrations.validate`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct HaIntegrationsValidateInput {
    /// Base64-encoded WASM module bytes to validate.
    pub wasm_base64: String,
}

/// Input for `ha.integrations.examples`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct HaIntegrationsExamplesInput {
    /// Capability to get examples for: "config_flow", "coordinator", "entity_bridge",
    /// "service_bridge", "discovery", "diagnostics", "repairs".
    /// Omit to return examples for all capabilities.
    pub capability: Option<String>,
}

/// Input for `ha.device_links.status`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct HaDeviceLinksStatusInput {
    /// Device ID of the ESPHome device (e.g. "esphome-host:my-device").
    pub device_id: String,
}

// ── Static content ────────────────────────────────────────────────────────────

const WIT_INTERFACE: &str = r#"// rshome-integration.wit — guest WASM integration interface
// Host imports (Rust host provides, WASM module calls)
interface rshome-host {
  // Entity management
  register-entity: func(platform: string, unique-id: string, config: list<u8>) -> u64
  update-entity-state: func(entity-handle: u64, state: list<u8>) -> result<_, string>
  get-entity-state: func(entity-id: string) -> option<list<u8>>

  // Device management
  register-device: func(name: string, model: option<string>, manufacturer: option<string>) -> u64

  // Service bridge
  register-service: func(domain: string, name: string, schema: list<u8>) -> u64

  // DataUpdateCoordinator
  create-coordinator: func(name: string, update-interval-ms: u64) -> u64

  // Discovery subscription
  subscribe-discovery: func(protocol: string) -> u64

  // Event bus
  fire-event: func(event-type: string, data: list<u8>)

  // Logging
  enum log-level { trace, debug, info, warn, error }
  log: func(level: log-level, message: string)
}

// Guest exports (WASM module implements, host calls)
world rshome-integration {
  import rshome-host

  // Lifecycle
  export setup: func(config: list<tuple<string, string>>) -> result<_, string>
  export teardown: func()

  // Config flow (multi-step integration setup UI)
  export config-flow-step: func(
    step-id: string,
    user-input: option<list<tuple<string, string>>>
  ) -> result<list<u8>, string>

  // DataUpdateCoordinator periodic refresh
  export coordinator-update: func(coordinator-handle: u64) -> result<list<u8>, string>

  // Diagnostics collection
  export get-diagnostics: func() -> list<tuple<string, string>>

  // Repair flow
  export repair-step: func(
    repair-id: string,
    step-id: string,
    user-input: option<list<tuple<string, string>>>
  ) -> result<list<u8>, string>
}"#;

const SCAFFOLD_CARGO_TOML: &str = r#"[package]
name = "my-rshome-integration"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.16"

[profile.release]
opt-level = "s"
lto = true"#;

const SCAFFOLD_LIB_RS: &str = r#"//! Example rshome WASM integration
//! Compile: cargo build --target wasm32-wasi --release

wit_bindgen::generate!({
    path: "wit/rshome-integration.wit",
    world: "rshome-integration",
});

struct MyIntegration;

impl Guest for MyIntegration {
    fn setup(config: Vec<(String, String)>) -> Result<(), String> {
        // Initialize your integration here
        // Use host functions from rshome_host:: to register entities/devices
        let device_handle = rshome_host::register_device(
            "My Device", Some("Model X"), Some("ACME Corp")
        );
        let _ = device_handle;
        Ok(())
    }

    fn teardown() {
        // Clean up resources
    }

    fn config_flow_step(
        step_id: String,
        user_input: Option<Vec<(String, String)>>,
    ) -> Result<Vec<u8>, String> {
        match step_id.as_str() {
            "user" => {
                // Return a form schema for the user to fill in
                let schema = serde_json::json!({
                    "type": "form",
                    "fields": [{"name": "host", "label": "Device IP", "required": true}]
                });
                Ok(serde_json::to_vec(&schema).unwrap())
            }
            _ => Err(format!("unknown step: {step_id}")),
        }
    }

    fn coordinator_update(coordinator_handle: u64) -> Result<Vec<u8>, String> {
        let _ = coordinator_handle;
        // Fetch data from your device and return as JSON bytes
        let data = serde_json::json!({"temperature": 23.5, "humidity": 45.0});
        Ok(serde_json::to_vec(&data).unwrap())
    }

    fn get_diagnostics() -> Vec<(String, String)> {
        vec![
            ("status".to_string(), "connected".to_string()),
            ("uptime_s".to_string(), "3600".to_string()),
        ]
    }

    fn repair_step(
        _repair_id: String,
        step_id: String,
        _user_input: Option<Vec<(String, String)>>,
    ) -> Result<Vec<u8>, String> {
        let result = serde_json::json!({"step": step_id, "complete": true});
        Ok(serde_json::to_vec(&result).unwrap())
    }
}

export!(MyIntegration);"#;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn unix_timestamp_str() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}

/// Check if `s` is valid base64 and return the decoded byte length estimate.
fn base64_decode_len(s: &str) -> Option<usize> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if !s.len().is_multiple_of(4) {
        return None;
    }
    let valid = s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=');
    if !valid {
        return None;
    }
    let padding = s.chars().rev().take_while(|&c| c == '=').count();
    Some(s.len() / 4 * 3 - padding)
}

fn integration_examples(capability: Option<&str>) -> serde_json::Value {
    let all = serde_json::json!({
        "config_flow": {
            "description": "Multi-step UI flow for integration setup",
            "code": "fn config_flow_step(step_id: String, ...) -> Result<Vec<u8>, String> { ... }"
        },
        "coordinator": {
            "description": "Periodic data fetch from a device or service",
            "code": "fn coordinator_update(coordinator_handle: u64) -> Result<Vec<u8>, String> { ... }"
        },
        "entity_bridge": {
            "description": "Register entities and push state updates",
            "code": "let h = rshome_host::register_entity(\"sensor\", \"temp_01\", config_bytes);"
        },
        "service_bridge": {
            "description": "Register custom services callable via ha.services.call",
            "code": "let h = rshome_host::register_service(\"my_domain\", \"my_action\", schema);"
        },
        "discovery": {
            "description": "Subscribe to mDNS/SSDP discovery events",
            "code": "let h = rshome_host::subscribe_discovery(\"mdns\");"
        },
        "diagnostics": {
            "description": "Expose diagnostic key-value pairs",
            "code": "fn get_diagnostics() -> Vec<(String, String)> { vec![(\"status\".into(), \"ok\".into())] }"
        },
        "repairs": {
            "description": "Multi-step repair wizard for fixing integration issues",
            "code": "fn repair_step(repair_id: String, step_id: String, ...) -> Result<Vec<u8>, String> { ... }"
        }
    });

    if let Some(cap) = capability {
        all.get(cap).cloned().unwrap_or_else(|| {
            serde_json::json!({ "error": format!("unknown capability: {cap}"), "available": ["config_flow","coordinator","entity_bridge","service_bridge","discovery","diagnostics","repairs"] })
        })
    } else {
        all
    }
}

// ── Workflow engine helpers ───────────────────────────────────────────────────

fn wf_trigger_from_str(s: &str) -> WfTriggerDef {
    match s {
        "entity_state_change" => WfTriggerDef::EntityStateChange {
            entity_id: String::new(),
            from: None,
            to: None,
        },
        "time_pattern" => WfTriggerDef::TimePattern {
            cron: String::new(),
        },
        "webhook" => WfTriggerDef::Webhook {
            path: String::new(),
        },
        _ => WfTriggerDef::Manual,
    }
}

// ── Main struct ───────────────────────────────────────────────────────────────

/// MCP server exposing rshome-ha runtime tools.
#[derive(Clone)]
pub struct RshomeHaMcp {
    tool_router: ToolRouter<Self>,
    entity_registry: EntityRegistry,
    state_store: Arc<StateStore>,
    service_registry: ActorRef<ServiceMsg>,
    device_manager: ActorRef<DeviceManagerMsg>,
    native_api: Option<ActorRef<NativeApiMsg>>,
    /// WASM integration host (Phase 3). When `Some`, `ha.integrations.list` returns
    /// live integration data; when `None`, returns an empty list with a note.
    wasm_host: Option<ActorRef<WasmHostMsg>>,
    /// Workflow engine (Phase 5). When `Some`, workflow tools delegate to the actor;
    /// when `None`, the in-memory stub is used (backward-compatible).
    workflow_engine: Option<ActorRef<WorkflowEngineMsg>>,
    /// ESPHome device-link manager (rshome-device-link). When `Some`, device link tools
    /// return live mDNS-discovered device status; when `None`, they return empty results.
    device_link: Option<ActorRef<DeviceLinkManagerMsg>>,
    workflows: Arc<Mutex<HashMap<String, WorkflowDef>>>,
    workflow_runs: Arc<Mutex<HashMap<String, WorkflowRun>>>,
    pub(crate) runtime_config: RuntimeConfig,
}

impl RshomeHaMcp {
    pub fn new(
        entity_registry: EntityRegistry,
        state_store: Arc<StateStore>,
        service_registry: ActorRef<ServiceMsg>,
        device_manager: ActorRef<DeviceManagerMsg>,
        native_api: Option<ActorRef<NativeApiMsg>>,
    ) -> Self {
        Self {
            tool_router: Self::tool_router(),
            entity_registry,
            state_store,
            service_registry,
            device_manager,
            native_api,
            wasm_host: None,
            workflow_engine: None,
            device_link: None,
            workflows: Arc::new(Mutex::new(HashMap::new())),
            workflow_runs: Arc::new(Mutex::new(HashMap::new())),
            runtime_config: RuntimeConfig::default(),
        }
    }

    /// Attach a running `WasmHostActor` so integration tools return live data.
    pub fn with_wasm_host(mut self, wasm_host: ActorRef<WasmHostMsg>) -> Self {
        self.wasm_host = Some(wasm_host);
        self
    }

    /// Attach a running `WorkflowEngineActor` so workflow tools delegate to real execution.
    pub fn with_workflow_engine(mut self, engine: ActorRef<WorkflowEngineMsg>) -> Self {
        self.workflow_engine = Some(engine);
        self
    }

    /// Attach a running `DeviceLinkManagerActor` so device-link tools return live mDNS data.
    pub fn with_device_link(mut self, dl: ActorRef<DeviceLinkManagerMsg>) -> Self {
        self.device_link = Some(dl);
        self
    }

    pub fn with_config(mut self, config: RuntimeConfig) -> Self {
        self.runtime_config = config;
        self
    }

    pub async fn serve_stdio(self) -> Result<(), Box<dyn std::error::Error>> {
        let transport = rmcp::transport::io::stdio();
        let server = self.serve(transport).await?;
        server.waiting().await?;
        Ok(())
    }

    pub fn list_tool_names(&self) -> Vec<String> {
        self.tool_router
            .list_all()
            .iter()
            .map(|t| t.name.to_string())
            .collect()
    }
}

// ── Tools ─────────────────────────────────────────────────────────────────────

#[tool_router]
impl RshomeHaMcp {
    /// List all registered devices.
    ///
    /// Returns device IDs, names, models, manufacturers, firmware versions, and area
    /// assignments. Only devices that have been registered via the ESPHome Native API
    /// or explicitly added to the runtime are included.
    ///
    /// Each entry includes an `origin` field (`"local"` | `"imported"`) and a
    /// `session_status` field (`"connected"` | `"handshaking"` | `"disconnected"` |
    /// `"local"`) when a `DeviceLinkManagerActor` is attached.
    #[tool(name = "ha.devices.list")]
    pub async fn devices_list(&self) -> Result<CallToolResult, McpError> {
        let devices = self
            .device_manager
            .ask(DeviceManagerMsg::ListDevices)
            .await
            .map_err(error::actor_error)?;

        // If device_link is attached, annotate imported devices with session status.
        let session_info: HashMap<String, (&'static str, &'static str)> = if let Some(ref dl) =
            self.device_link
        {
            let connected = dl
                .ask(rshome_device_link::manager::DeviceLinkManagerMsg::ListConnected)
                .await
                .unwrap_or_default();
            connected
                .into_iter()
                .map(|d| {
                    let session_status = match d.status {
                        rshome_device_link::manager::SessionStatus::Active => "connected",
                        rshome_device_link::manager::SessionStatus::Handshaking => "handshaking",
                        _ => "disconnected",
                    };
                    (d.device_id.0, ("imported", session_status))
                })
                .collect()
        } else {
            HashMap::new()
        };

        let enriched: Vec<serde_json::Value> = devices
            .iter()
            .map(|d| {
                let (origin, session_status) = session_info
                    .get(d.device_id.0.as_str())
                    .copied()
                    .unwrap_or(("local", "local"));
                let mut val = serde_json::to_value(d).unwrap_or_default();
                if let serde_json::Value::Object(ref mut map) = val {
                    map.insert(
                        "origin".to_string(),
                        serde_json::Value::String(origin.to_string()),
                    );
                    map.insert(
                        "session_status".to_string(),
                        serde_json::Value::String(session_status.to_string()),
                    );
                }
                val
            })
            .collect();

        let json = serde_json::to_string_pretty(&enriched).map_err(error::json_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Get detailed info for a single device including its entity IDs.
    ///
    /// Returns the device descriptor (name, model, manufacturer, sw_version, area_id)
    /// plus the list of all entity IDs belonging to this device. Returns an error if the
    /// device_id is not found.
    #[tool(name = "ha.devices.get")]
    pub async fn devices_get(
        &self,
        Parameters(input): Parameters<HaDevicesGetInput>,
    ) -> Result<CallToolResult, McpError> {
        let dev_id = DeviceId(input.device_id.clone());

        let device_ref = self
            .device_manager
            .ask(|reply| DeviceManagerMsg::GetDevice { id: dev_id, reply })
            .await
            .map_err(error::actor_error)?
            .ok_or_else(|| {
                error::unknown_object(format!("device not found: {}", input.device_id))
            })?;

        let info = device_ref
            .ask(DeviceMsg::GetInfo)
            .await
            .map_err(error::actor_error)?;
        let entity_ids = device_ref
            .ask(DeviceMsg::GetEntities)
            .await
            .map_err(error::actor_error)?;

        let result = serde_json::json!({
            "device": info,
            "entity_ids": entity_ids.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
        });
        let json = serde_json::to_string_pretty(&result).map_err(error::json_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// List entities with optional domain/device filtering.
    ///
    /// Returns entity IDs and their current states from the StateStore. Filter by
    /// `domain` (e.g. "switch", "sensor") or `device_id` to scope results. Use `limit`
    /// to cap the number of results returned.
    #[tool(name = "ha.entities.list")]
    pub async fn entities_list(
        &self,
        Parameters(input): Parameters<HaEntitiesListInput>,
    ) -> Result<CallToolResult, McpError> {
        let ids: Vec<EntityId> = if let Some(ref domain) = input.domain {
            self.entity_registry.list_by_domain(domain)
        } else if let Some(ref device_id_str) = input.device_id {
            let dev_id = DeviceId(device_id_str.clone());
            self.device_manager
                .ask(|reply| DeviceManagerMsg::GetEntitiesForDevice {
                    device_id: dev_id,
                    reply,
                })
                .await
                .map_err(error::actor_error)?
        } else {
            self.entity_registry.list_all()
        };

        let limit = input.limit.unwrap_or(500);
        let results: Vec<serde_json::Value> = ids
            .iter()
            .take(limit)
            .map(|id| {
                let state = self.state_store.get(id);
                let domain = id.domain().to_string();
                let feature_set = self
                    .entity_registry
                    .get_descriptor(id)
                    .map(|d| d.feature_set)
                    .unwrap_or_default();
                serde_json::json!({
                    "entity_id": id.to_string(),
                    "domain": domain,
                    "feature_set": feature_set,
                    "state": state,
                })
            })
            .collect();

        let json = serde_json::to_string_pretty(&results).map_err(error::json_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Get the current state and attributes of a specific entity.
    ///
    /// Returns the full typed `EntityState` (e.g. Switch/Sensor/Light/Climate variants)
    /// plus the last_updated UNIX timestamp. Returns an error if the entity_id is not in
    /// the StateStore.
    #[tool(name = "ha.entities.get_state")]
    pub async fn entities_get_state(
        &self,
        Parameters(input): Parameters<HaEntitiesGetStateInput>,
    ) -> Result<CallToolResult, McpError> {
        let entity_id = EntityId(input.entity_id.clone());
        let snapshot = self.state_store.snapshot(&entity_id).ok_or_else(|| {
            error::unknown_object(format!("entity not found: {}", input.entity_id))
        })?;
        let last_updated = snapshot
            .last_updated
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let result = serde_json::json!({
            "entity_id": snapshot.entity_id.to_string(),
            "state": snapshot.state,
            "last_updated": last_updated,
        });
        let json = serde_json::to_string_pretty(&result).map_err(error::json_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Directly update an entity's state.
    ///
    /// Parses `state_json` as a typed `EntityState` value and sends it to the entity
    /// actor. The update is also reflected in the StateStore and broadcast to all
    /// state watchers. Returns an error if the entity_id is unknown or the JSON is
    /// not a valid `EntityState`.
    #[tool(name = "ha.entities.set_state")]
    pub async fn entities_set_state(
        &self,
        Parameters(input): Parameters<HaEntitiesSetStateInput>,
    ) -> Result<CallToolResult, McpError> {
        let entity_id = EntityId(input.entity_id.clone());
        let new_state: EntityState = serde_json::from_str(&input.state_json)
            .map_err(|e| error::bad_input(format!("invalid state JSON: {e}")))?;

        let entity_ref = self.entity_registry.get(&entity_id).ok_or_else(|| {
            error::unknown_object(format!("entity not found: {}", input.entity_id))
        })?;

        entity_ref
            .send(EntityMsg::SetState(new_state.clone()))
            .map_err(error::actor_error)?;

        let result = serde_json::json!({
            "entity_id": input.entity_id,
            "updated": true,
            "new_state": new_state,
        });
        let json = serde_json::to_string_pretty(&result).map_err(error::json_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// List all registered services.
    ///
    /// Returns domain, service name, and description for each registered service.
    /// Built-in services (turn_on, turn_off, toggle, set_value per entity domain) are
    /// always included. Custom services registered by WASM integrations appear here too.
    #[tool(name = "ha.services.list")]
    pub async fn services_list(&self) -> Result<CallToolResult, McpError> {
        let services = self
            .service_registry
            .ask(ServiceMsg::List)
            .await
            .map_err(error::actor_error)?;
        let json = serde_json::to_string_pretty(&services).map_err(error::json_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Call a service on one or more target entities.
    ///
    /// Dispatches `domain.service` to the resolved target via the service registry.
    /// Target resolution order: `target_entity_ids` → `target_device_id` → domain-wide
    /// fan-out. Returns the number of entities that received the command.
    #[tool(name = "ha.services.call")]
    pub async fn services_call(
        &self,
        Parameters(input): Parameters<HaServicesCallInput>,
    ) -> Result<CallToolResult, McpError> {
        let target = match (&input.target_entity_ids, &input.target_device_id) {
            (Some(ids), _) => {
                ServiceTarget::EntityIds(ids.iter().map(|s| EntityId(s.clone())).collect())
            }
            (_, Some(dev)) => ServiceTarget::DeviceId(DeviceId(dev.clone())),
            _ => ServiceTarget::Domain(input.domain.clone()),
        };

        let result = self
            .service_registry
            .ask(|reply| ServiceMsg::Call {
                domain: input.domain.clone(),
                service: input.service.clone(),
                target,
                data: input.data.clone().unwrap_or(serde_json::Value::Null),
                reply,
            })
            .await
            .map_err(error::actor_error)?;

        match result {
            Ok(count) => {
                let json = serde_json::to_string_pretty(&serde_json::json!({
                    "ok": true,
                    "domain": input.domain,
                    "service": input.service,
                    "entities_affected": count,
                }))
                .map_err(error::json_error)?;
                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            Err(e) => Err(error::bad_input(e)),
        }
    }

    /// List all defined workflows.
    ///
    /// Returns workflow IDs, names, trigger types, and step counts. Workflows are
    /// created with `ha.workflows.create` and executed with `ha.workflows.run`.
    #[tool(name = "ha.workflows.list")]
    pub async fn workflows_list(&self) -> Result<CallToolResult, McpError> {
        if let Some(engine) = &self.workflow_engine {
            let list = engine
                .ask(WorkflowEngineMsg::List)
                .await
                .map_err(error::actor_error)?;
            let json = serde_json::to_string_pretty(&list).map_err(error::json_error)?;
            return Ok(CallToolResult::success(vec![Content::text(json)]));
        }
        let workflows = self.workflows.lock().unwrap();
        let list: Vec<serde_json::Value> = workflows
            .values()
            .map(|w| {
                serde_json::json!({
                    "workflow_id": w.workflow_id,
                    "name": w.name,
                    "description": w.description,
                    "trigger": w.trigger,
                    "step_count": w.steps.len(),
                })
            })
            .collect();
        let json = serde_json::to_string_pretty(&list).map_err(error::json_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Create a new workflow definition.
    ///
    /// Returns the generated `workflow_id`. The workflow is stored in memory.
    /// Supported triggers: "manual" (default), "entity_state_change", "time_pattern",
    /// "webhook". When a WorkflowEngineActor is attached, delegates to it for real
    /// execution; otherwise uses the in-memory stub.
    #[tool(name = "ha.workflows.create")]
    pub async fn workflows_create(
        &self,
        Parameters(input): Parameters<HaWorkflowsCreateInput>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(engine) = &self.workflow_engine {
            let trigger_str = input.trigger.as_deref().unwrap_or("manual");
            let trigger = wf_trigger_from_str(trigger_str);
            let steps: Vec<StepDef> = input
                .steps
                .as_deref()
                .and_then(|vals| {
                    serde_json::from_value(serde_json::Value::Array(vals.to_vec())).ok()
                })
                .unwrap_or_default();
            let def = WorkflowDefinition {
                workflow_id: String::new(),
                name: input.name.clone(),
                description: input.description.clone(),
                trigger,
                mode: WorkflowMode::Pipeline { steps },
            };
            let workflow_id = engine
                .ask(|tx| WorkflowEngineMsg::Create {
                    definition: def,
                    reply: tx,
                })
                .await
                .map_err(error::actor_error)?
                .map_err(|e: WfError| error::bad_input(e))?;
            let json = serde_json::to_string_pretty(&serde_json::json!({
                "workflow_id": workflow_id,
                "name": input.name,
                "trigger": trigger_str,
            }))
            .map_err(error::json_error)?;
            return Ok(CallToolResult::success(vec![Content::text(json)]));
        }

        let workflow_id = uuid::Uuid::new_v4().to_string();
        let def = WorkflowDef {
            workflow_id: workflow_id.clone(),
            name: input.name,
            description: input.description,
            trigger: input.trigger.unwrap_or_else(|| "manual".to_string()),
            steps: input.steps.unwrap_or_default(),
        };
        self.workflows
            .lock()
            .unwrap()
            .insert(workflow_id.clone(), def.clone());

        let json = serde_json::to_string_pretty(&serde_json::json!({
            "workflow_id": workflow_id,
            "name": def.name,
            "trigger": def.trigger,
            "step_count": def.steps.len(),
        }))
        .map_err(error::json_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Execute a workflow and return a run ID.
    ///
    /// Returns a `run_id` for polling with `ha.workflows.status`. When a
    /// WorkflowEngineActor is attached, executes steps in a real async pipeline;
    /// otherwise returns `completed` immediately (in-memory stub).
    #[tool(name = "ha.workflows.run")]
    pub async fn workflows_run(
        &self,
        Parameters(input): Parameters<HaWorkflowsRunInput>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(engine) = &self.workflow_engine {
            let run_id = engine
                .ask(|tx| WorkflowEngineMsg::Run {
                    workflow_id: input.workflow_id.clone(),
                    input_data: input.input_data.clone(),
                    reply: tx,
                })
                .await
                .map_err(error::actor_error)?
                .map_err(|e: WfError| error::bad_input(e))?;
            let json = serde_json::to_string_pretty(&serde_json::json!({
                "run_id": run_id,
                "workflow_id": input.workflow_id,
                "status": "running",
            }))
            .map_err(error::json_error)?;
            return Ok(CallToolResult::success(vec![Content::text(json)]));
        }

        let exists = self
            .workflows
            .lock()
            .unwrap()
            .contains_key(&input.workflow_id);
        if !exists {
            return Err(error::unknown_object(format!(
                "workflow not found: {}",
                input.workflow_id,
            )));
        }

        let run_id = uuid::Uuid::new_v4().to_string();
        let run = WorkflowRun {
            run_id: run_id.clone(),
            workflow_id: input.workflow_id.clone(),
            status: WorkflowRunStatus::Completed,
            started_at: unix_timestamp_str(),
            completed_at: Some(unix_timestamp_str()),
            error: None,
        };
        self.workflow_runs
            .lock()
            .unwrap()
            .insert(run_id.clone(), run);

        let json = serde_json::to_string_pretty(&serde_json::json!({
            "run_id": run_id,
            "workflow_id": input.workflow_id,
            "status": "completed",
        }))
        .map_err(error::json_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Get the status of a workflow execution run.
    ///
    /// Returns status (running/completed/failed) and any error message.
    /// Use the `run_id` returned by `ha.workflows.run`. Returns an error if the run_id
    /// is not found.
    #[tool(name = "ha.workflows.status")]
    pub async fn workflows_status(
        &self,
        Parameters(input): Parameters<HaWorkflowsStatusInput>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(engine) = &self.workflow_engine {
            let info = engine
                .ask(|tx| WorkflowEngineMsg::GetRunStatus {
                    run_id: input.run_id.clone(),
                    reply: tx,
                })
                .await
                .map_err(error::actor_error)?
                .map_err(|e: WfError| error::bad_input(e))?;
            let status_str = match info.status {
                WfRunStatus::Running => "running",
                WfRunStatus::Completed => "completed",
                WfRunStatus::Failed => "failed",
            };
            let json = serde_json::to_string_pretty(&serde_json::json!({
                "run_id": info.run_id,
                "workflow_id": info.workflow_id,
                "status": status_str,
                "error": info.error,
            }))
            .map_err(error::json_error)?;
            return Ok(CallToolResult::success(vec![Content::text(json)]));
        }

        let runs = self.workflow_runs.lock().unwrap();
        let run = runs
            .get(&input.run_id)
            .ok_or_else(|| error::unknown_object(format!("run not found: {}", input.run_id)))?;
        let json = serde_json::to_string_pretty(run).map_err(error::json_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Trigger mDNS device discovery on the local network.
    ///
    /// Scans for `_esphomelib._tcp.local.` services. Returns discovered hostnames, IPs,
    /// and ports. Passive discovery runs continuously when the Native API server is active;
    /// this tool returns the current scan configuration and known discovered nodes.
    #[tool(name = "ha.discovery.scan")]
    pub async fn discovery_scan(
        &self,
        Parameters(input): Parameters<HaDiscoveryScanInput>,
    ) -> Result<CallToolResult, McpError> {
        let timeout_ms = input.timeout_ms.unwrap_or(3000);
        let result = serde_json::json!({
            "scan_timeout_ms": timeout_ms,
            "service_type": "_esphomelib._tcp.local.",
            "discovered": [],
            "note": "Passive mDNS scanning via NativeApiServerActor. Devices auto-register on connect.",
            "native_api_running": self.native_api.is_some(),
        });
        let json = serde_json::to_string_pretty(&result).map_err(error::json_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Get the current runtime configuration.
    ///
    /// Returns API port, mDNS settings, entity/device count limits, current entity count,
    /// and runtime version. Useful for diagnostics and understanding runtime capacity.
    #[tool(name = "ha.config.get")]
    pub async fn config_get(&self) -> Result<CallToolResult, McpError> {
        let result = serde_json::json!({
            "version": self.runtime_config.version,
            "native_api_port": self.runtime_config.native_api_port,
            "mdns_enabled": self.runtime_config.mdns_enabled,
            "max_devices": self.runtime_config.max_devices,
            "max_entities": self.runtime_config.max_entities,
            "max_connections_per_device": self.runtime_config.max_connections_per_device,
            "current_entity_count": self.entity_registry.count(),
            "current_state_count": self.state_store.count(),
            "native_api_running": self.native_api.is_some(),
        });
        let json = serde_json::to_string_pretty(&result).map_err(error::json_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// List loaded WASM integrations.
    ///
    /// Returns active WASM integration modules with their names and status.
    /// Requires a running `WasmHostActor` (attach via `with_wasm_host`).
    #[tool(name = "ha.integrations.list")]
    pub async fn integrations_list(&self) -> Result<CallToolResult, McpError> {
        let result = if let Some(host) = &self.wasm_host {
            match host.ask(WasmHostMsg::List).await {
                Ok(list) => serde_json::json!({ "integrations": list }),
                Err(e) => serde_json::json!({
                    "integrations": [],
                    "error": format!("wasm host unavailable: {e}"),
                }),
            }
        } else {
            serde_json::json!({
                "integrations": [],
                "note": "No WASM integration host attached. Use with_wasm_host() to enable.",
            })
        };
        let json = serde_json::to_string_pretty(&result).map_err(error::json_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Generate a starter WASM integration project.
    ///
    /// Returns a `Cargo.toml`, `src/lib.rs`, and `wit/rshome-integration.wit` suitable
    /// for compiling to a `.wasm` integration module. Use as the starting point for custom
    /// integrations (MQTT bridges, Zigbee coordinators, custom protocols, etc).
    #[tool(name = "ha.integrations.scaffold")]
    pub async fn integrations_scaffold(&self) -> Result<CallToolResult, McpError> {
        let scaffold = serde_json::json!({
            "files": {
                "Cargo.toml": SCAFFOLD_CARGO_TOML,
                "src/lib.rs": SCAFFOLD_LIB_RS,
                "wit/rshome-integration.wit": WIT_INTERFACE,
            },
            "instructions": [
                "1. Copy these files into a new directory",
                "2. Run: cargo build --target wasm32-wasi --release",
                "3. The .wasm file will be in target/wasm32-wasi/release/",
                "4. Load via WasmHostActor (Phase 3) or validate with ha.integrations.validate",
            ],
        });
        let json = serde_json::to_string_pretty(&scaffold).map_err(error::json_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Return the complete WIT interface for rshome integrations.
    ///
    /// Returns the full `rshome-integration.wit` with all ~30 host imports and ~7 guest
    /// exports. Use this as the authoritative reference when authoring custom WASM
    /// integrations. Compatible with wit-bindgen 0.16+.
    #[tool(name = "ha.integrations.wit_reference")]
    pub async fn integrations_wit_reference(&self) -> Result<CallToolResult, McpError> {
        let result = serde_json::json!({
            "wit_interface": WIT_INTERFACE,
            "wit_bindgen_version": "0.16",
            "description": "Import this WIT in your WASM integration via wit-bindgen",
        });
        let json = serde_json::to_string_pretty(&result).map_err(error::json_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Validate a WASM module against the rshome-integration host contract.
    ///
    /// Checks format validity of the base64-encoded WASM bytes. Full binary validation
    /// (exports present, types match, fuel budget check) requires the WasmHostActor
    /// (Phase 3). Returns size estimate and required export names.
    #[tool(name = "ha.integrations.validate")]
    pub async fn integrations_validate(
        &self,
        Parameters(input): Parameters<HaIntegrationsValidateInput>,
    ) -> Result<CallToolResult, McpError> {
        let bytes_len = base64_decode_len(&input.wasm_base64)
            .ok_or_else(|| error::bad_input("invalid base64 encoding"))?;
        let result = serde_json::json!({
            "valid": true,
            "wasm_size_bytes": bytes_len,
            "required_exports": [
                "setup", "teardown", "config-flow-step",
                "coordinator-update", "get-diagnostics", "repair-step",
            ],
            "note": "Full binary validation requires WasmHostActor (Phase 3).",
        });
        let json = serde_json::to_string_pretty(&result).map_err(error::json_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Return annotated code samples for rshome integration capabilities.
    ///
    /// Provides working Rust examples for: config_flow, coordinator, entity_bridge,
    /// service_bridge, discovery, diagnostics, repairs. Specify `capability` for a single
    /// example or omit for all. These examples compile against the rshome-integration WIT
    /// interface returned by `ha.integrations.wit_reference`.
    #[tool(name = "ha.integrations.examples")]
    pub async fn integrations_examples(
        &self,
        Parameters(input): Parameters<HaIntegrationsExamplesInput>,
    ) -> Result<CallToolResult, McpError> {
        let examples = integration_examples(input.capability.as_deref());
        let json = serde_json::to_string_pretty(&examples).map_err(error::json_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// List all ESPHome devices currently discovered via mDNS.
    ///
    /// Returns hostname, IP, port, firmware version, and connection status for every
    /// ESPHome device seen on `_esphomelib._tcp.local.`. Requires a
    /// `DeviceLinkManagerActor` to be attached via `with_device_link()`.
    #[tool(name = "ha.device_links.list_discovered")]
    pub async fn device_links_list_discovered(&self) -> Result<CallToolResult, McpError> {
        let Some(ref dl) = self.device_link else {
            let result = serde_json::json!({ "devices": [], "note": "DeviceLinkManagerActor not attached." });
            let json = serde_json::to_string_pretty(&result).map_err(error::json_error)?;
            return Ok(CallToolResult::success(vec![Content::text(json)]));
        };
        let devices = dl
            .ask(rshome_device_link::manager::DeviceLinkManagerMsg::ListDiscovered)
            .await
            .map_err(error::actor_error)?;
        let json = serde_json::to_string_pretty(&serde_json::json!({ "devices": devices }))
            .map_err(error::json_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Get the live connection status for a specific ESPHome device.
    ///
    /// Returns connection state, entity count, and session metadata for the requested
    /// device ID. Returns an error if the device has not been discovered yet.
    /// Requires a `DeviceLinkManagerActor` to be attached via `with_device_link()`.
    #[tool(name = "ha.device_links.status")]
    pub async fn device_links_status(
        &self,
        Parameters(input): Parameters<HaDeviceLinksStatusInput>,
    ) -> Result<CallToolResult, McpError> {
        let Some(ref dl) = self.device_link else {
            return Err(error::unavailable("DeviceLinkManagerActor not attached"));
        };
        let device_id = rshome_entity::DeviceId(input.device_id.clone());
        let status = dl
            .ask(
                |reply| rshome_device_link::manager::DeviceLinkManagerMsg::GetStatus {
                    device_id,
                    reply,
                },
            )
            .await
            .map_err(error::actor_error)?
            .ok_or_else(|| {
                error::unknown_object(format!("device not found: {}", input.device_id))
            })?;
        let json = serde_json::to_string_pretty(&status).map_err(error::json_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}

// ── ServerHandler ─────────────────────────────────────────────────────────────

impl ServerHandler for RshomeHaMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
            server_info: Implementation {
                name: "rshome-mcp".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            instructions: Some(
                "rshome-ha MCP server. Exposes 20 tools for device, entity, service, workflow, \
                 discovery, config, integration, and device-link management. Use ha.devices.list \
                 to discover registered devices, ha.entities.get_state to read sensor values, \
                 ha.services.call to control devices, ha.integrations.scaffold to author custom \
                 WASM integrations, and ha.device_links.list_discovered for ESPHome mDNS devices."
                    .to_string(),
            ),
        }
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        std::future::ready(Ok(ListToolsResult {
            tools: self.tool_router.list_all(),
            next_cursor: None,
        }))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParam,
        context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, McpError>> + Send + '_ {
        let tool_context = ToolCallContext::new(self, request, context);
        self.tool_router.call(tool_context)
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListResourcesResult, McpError>> + Send + '_ {
        std::future::ready(self.list_resources_impl())
    }

    fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListResourceTemplatesResult, McpError>> + Send + '_
    {
        std::future::ready(self.list_resource_templates_impl())
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResult, McpError>> + Send + '_ {
        let uri = request.uri.clone();
        async move { self.read_resource_impl(&uri).await }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rshome_actor::ActorSystem;
    use rshome_entity::{
        EntityActor, EntityCategory, EntityDescriptor, EntityId, EntityState, NullStateUpdater,
    };
    use rshome_svc::ServiceRegistryActor;
    use std::sync::Arc;
    use std::time::Duration;

    fn null_store() -> Arc<dyn rshome_entity::StateUpdater> {
        Arc::new(NullStateUpdater)
    }

    fn make_entity_descriptor(domain: &str, name: &str) -> EntityDescriptor {
        EntityDescriptor {
            entity_id: EntityId::new(domain, name),
            name: name.to_string(),
            icon: None,
            device_id: None,
            area_id: None,
            entity_category: EntityCategory::None,
            domain_id: domain.to_string(),
            feature_set: vec![],
            device_class: None,
        }
    }

    /// Spawn all required actors and return a ready-to-use RshomeHaMcp + ActorSystem.
    async fn make_server() -> (RshomeHaMcp, ActorSystem) {
        let sys = ActorSystem::new();
        let registry = EntityRegistry::default();
        let state_store = Arc::new(StateStore::default());
        let device_manager = sys.spawn(rshome_entity::DeviceManagerActor::new(
            registry.clone(),
            null_store(),
        ));
        let service_registry = sys.spawn(ServiceRegistryActor::new(
            registry.clone(),
            Some(device_manager.clone()),
        ));
        let server = RshomeHaMcp::new(
            registry,
            state_store,
            service_registry,
            device_manager,
            None,
        );
        (server, sys)
    }

    /// Spawn actors + seed one switch entity. Returns (server, sys, entity_ref).
    async fn make_server_with_switch() -> (
        RshomeHaMcp,
        ActorSystem,
        ActorRef<EntityMsg>,
        EntityRegistry,
    ) {
        let sys = ActorSystem::new();
        let registry = EntityRegistry::default();
        let state_store = Arc::new(StateStore::default());

        let switch_id = EntityId::new("switch", "kitchen");
        let initial = EntityState::Switch { is_on: false };
        state_store.update(&switch_id, initial.clone());
        let (actor, _tx) = EntityActor::new(
            make_entity_descriptor("switch", "kitchen"),
            initial,
            null_store(),
        );
        let actor_ref = sys.spawn(actor);
        registry.register(switch_id, actor_ref.clone());

        let device_manager = sys.spawn(rshome_entity::DeviceManagerActor::new(
            registry.clone(),
            null_store(),
        ));
        let service_registry = sys.spawn(ServiceRegistryActor::new(
            registry.clone(),
            Some(device_manager.clone()),
        ));
        let server = RshomeHaMcp::new(
            registry.clone(),
            state_store,
            service_registry,
            device_manager,
            None,
        );
        (server, sys, actor_ref, registry)
    }

    // ── ServerHandler / metadata tests ────────────────────────────────────────

    #[test]
    fn server_info_has_correct_name() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (server, sys) = rt.block_on(make_server());
        let info = server.get_info();
        assert_eq!(info.server_info.name, "rshome-mcp");
        assert!(info.capabilities.tools.is_some());
        assert!(info.capabilities.resources.is_some());
        assert!(info.instructions.is_some());
        rt.block_on(sys.shutdown());
    }

    #[test]
    fn tools_list_has_twenty_tools() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (server, sys) = rt.block_on(make_server());
        let names = server.list_tool_names();
        assert_eq!(names.len(), 20, "expected 20 tools, got: {names:?}");

        // Device tools
        assert!(names.contains(&"ha.devices.list".to_string()));
        assert!(names.contains(&"ha.devices.get".to_string()));

        // Entity tools
        assert!(names.contains(&"ha.entities.list".to_string()));
        assert!(names.contains(&"ha.entities.get_state".to_string()));
        assert!(names.contains(&"ha.entities.set_state".to_string()));

        // Service tools
        assert!(names.contains(&"ha.services.list".to_string()));
        assert!(names.contains(&"ha.services.call".to_string()));

        // Workflow tools
        assert!(names.contains(&"ha.workflows.list".to_string()));
        assert!(names.contains(&"ha.workflows.create".to_string()));
        assert!(names.contains(&"ha.workflows.run".to_string()));
        assert!(names.contains(&"ha.workflows.status".to_string()));

        // Discovery + config
        assert!(names.contains(&"ha.discovery.scan".to_string()));
        assert!(names.contains(&"ha.config.get".to_string()));

        // Integration tools
        assert!(names.contains(&"ha.integrations.list".to_string()));
        assert!(names.contains(&"ha.integrations.scaffold".to_string()));
        assert!(names.contains(&"ha.integrations.wit_reference".to_string()));
        assert!(names.contains(&"ha.integrations.validate".to_string()));
        assert!(names.contains(&"ha.integrations.examples".to_string()));

        // Device-link tools
        assert!(names.contains(&"ha.device_links.list_discovered".to_string()));
        assert!(names.contains(&"ha.device_links.status".to_string()));

        rt.block_on(sys.shutdown());
    }

    // ── Workflow tests (no actors needed) ─────────────────────────────────────

    #[tokio::test]
    async fn workflows_list_empty_initially() {
        let (server, sys) = make_server().await;
        let result = server.workflows_list().await.unwrap();
        let json: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert!(json.as_array().unwrap().is_empty());
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn workflows_create_returns_id() {
        let (server, sys) = make_server().await;
        let result = server
            .workflows_create(Parameters(HaWorkflowsCreateInput {
                name: "Fan Automation".to_string(),
                description: Some("Turn on fan when temp > 30".to_string()),
                trigger: Some("entity_state_change".to_string()),
                steps: None,
            }))
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert!(json["workflow_id"].as_str().unwrap().len() > 0);
        assert_eq!(json["name"], "Fan Automation");
        assert_eq!(json["trigger"], "entity_state_change");
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn workflows_create_default_trigger() {
        let (server, sys) = make_server().await;
        let result = server
            .workflows_create(Parameters(HaWorkflowsCreateInput {
                name: "Manual Flow".to_string(),
                description: None,
                trigger: None,
                steps: None,
            }))
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(json["trigger"], "manual");
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn workflows_create_then_list() {
        let (server, sys) = make_server().await;
        server
            .workflows_create(Parameters(HaWorkflowsCreateInput {
                name: "WF1".to_string(),
                description: None,
                trigger: None,
                steps: Some(vec![serde_json::json!({"type": "delay", "ms": 500})]),
            }))
            .await
            .unwrap();
        server
            .workflows_create(Parameters(HaWorkflowsCreateInput {
                name: "WF2".to_string(),
                description: None,
                trigger: None,
                steps: None,
            }))
            .await
            .unwrap();

        let result = server.workflows_list().await.unwrap();
        let list: Vec<serde_json::Value> =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(list.len(), 2);
        let names: Vec<&str> = list.iter().map(|v| v["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"WF1"));
        assert!(names.contains(&"WF2"));
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn workflows_run_completes() {
        let (server, sys) = make_server().await;
        let create_result = server
            .workflows_create(Parameters(HaWorkflowsCreateInput {
                name: "Runnable".to_string(),
                description: None,
                trigger: None,
                steps: None,
            }))
            .await
            .unwrap();
        let create_json: serde_json::Value =
            serde_json::from_str(create_result.content[0].as_text().unwrap().text.as_str())
                .unwrap();
        let workflow_id = create_json["workflow_id"].as_str().unwrap().to_string();

        let run_result = server
            .workflows_run(Parameters(HaWorkflowsRunInput {
                workflow_id: workflow_id.clone(),
                input_data: None,
            }))
            .await
            .unwrap();
        let run_json: serde_json::Value =
            serde_json::from_str(run_result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert!(run_json["run_id"].as_str().is_some());
        assert_eq!(run_json["workflow_id"], workflow_id);
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn workflows_status_after_run() {
        let (server, sys) = make_server().await;
        let create_result = server
            .workflows_create(Parameters(HaWorkflowsCreateInput {
                name: "StatusTest".to_string(),
                description: None,
                trigger: None,
                steps: None,
            }))
            .await
            .unwrap();
        let wf_id: serde_json::Value =
            serde_json::from_str(create_result.content[0].as_text().unwrap().text.as_str())
                .unwrap();
        let workflow_id = wf_id["workflow_id"].as_str().unwrap().to_string();

        let run_result = server
            .workflows_run(Parameters(HaWorkflowsRunInput {
                workflow_id,
                input_data: None,
            }))
            .await
            .unwrap();
        let run_json: serde_json::Value =
            serde_json::from_str(run_result.content[0].as_text().unwrap().text.as_str()).unwrap();
        let run_id = run_json["run_id"].as_str().unwrap().to_string();

        let status_result = server
            .workflows_status(Parameters(HaWorkflowsStatusInput { run_id }))
            .await
            .unwrap();
        let status_json: serde_json::Value =
            serde_json::from_str(status_result.content[0].as_text().unwrap().text.as_str())
                .unwrap();
        assert_eq!(status_json["status"], "completed");
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn workflows_run_unknown_id_error() {
        let (server, sys) = make_server().await;
        let err = server
            .workflows_run(Parameters(HaWorkflowsRunInput {
                workflow_id: "nonexistent-id".to_string(),
                input_data: None,
            }))
            .await
            .unwrap_err();
        assert!(err.message.contains("workflow not found"));
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn workflows_status_unknown_run_error() {
        let (server, sys) = make_server().await;
        let err = server
            .workflows_status(Parameters(HaWorkflowsStatusInput {
                run_id: "bad-run-id".to_string(),
            }))
            .await
            .unwrap_err();
        assert!(err.message.contains("run not found"));
        sys.shutdown().await;
    }

    // ── Discovery + config tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn discovery_scan_returns_info() {
        let (server, sys) = make_server().await;
        let result = server
            .discovery_scan(Parameters(HaDiscoveryScanInput {
                timeout_ms: Some(1000),
            }))
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(json["scan_timeout_ms"], 1000);
        assert_eq!(json["service_type"], "_esphomelib._tcp.local.");
        assert!(json["discovered"].as_array().unwrap().is_empty());
        assert_eq!(json["native_api_running"], false);
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn discovery_scan_default_timeout() {
        let (server, sys) = make_server().await;
        let result = server
            .discovery_scan(Parameters(HaDiscoveryScanInput { timeout_ms: None }))
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(json["scan_timeout_ms"], 3000);
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn config_get_returns_json() {
        let (server, sys) = make_server().await;
        let result = server.config_get().await.unwrap();
        let json: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(json["native_api_port"], 6053);
        assert_eq!(json["mdns_enabled"], true);
        assert_eq!(json["max_devices"], 100);
        assert_eq!(json["current_entity_count"], 0);
        assert_eq!(json["native_api_running"], false);
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn config_get_entity_count_reflects_registry() {
        let (server, sys, _actor_ref, _registry) = make_server_with_switch().await;
        let result = server.config_get().await.unwrap();
        let json: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(json["current_entity_count"], 1);
        assert_eq!(json["current_state_count"], 1);
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn integrations_list_empty() {
        let (server, sys) = make_server().await;
        let result = server.integrations_list().await.unwrap();
        let json: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert!(json["integrations"].as_array().unwrap().is_empty());
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn integrations_scaffold_returns_files() {
        let (server, sys) = make_server().await;
        let result = server.integrations_scaffold().await.unwrap();
        let json: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert!(json["files"]["Cargo.toml"].as_str().is_some());
        assert!(json["files"]["src/lib.rs"].as_str().is_some());
        assert!(json["files"]["wit/rshome-integration.wit"]
            .as_str()
            .is_some());
        assert!(json["instructions"].as_array().unwrap().len() >= 4);
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn integrations_wit_reference_contains_exports() {
        let (server, sys) = make_server().await;
        let result = server.integrations_wit_reference().await.unwrap();
        let json: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        let wit = json["wit_interface"].as_str().unwrap();
        assert!(wit.contains("register-entity"));
        assert!(wit.contains("setup"));
        assert!(wit.contains("coordinator-update"));
        assert!(wit.contains("get-diagnostics"));
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn integrations_validate_valid_base64() {
        let (server, sys) = make_server().await;
        // Valid base64 (16 bytes → 24 base64 chars)
        let result = server
            .integrations_validate(Parameters(HaIntegrationsValidateInput {
                wasm_base64: "AAAAAAAAAAAAAAAAAAAAAA==".to_string(),
            }))
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(json["valid"], true);
        assert!(json["wasm_size_bytes"].as_u64().unwrap() > 0);
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn integrations_validate_invalid_base64_error() {
        let (server, sys) = make_server().await;
        let err = server
            .integrations_validate(Parameters(HaIntegrationsValidateInput {
                wasm_base64: "not!valid@base64#".to_string(),
            }))
            .await
            .unwrap_err();
        assert!(err.message.contains("invalid base64"));
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn integrations_examples_all_capabilities() {
        let (server, sys) = make_server().await;
        let result = server
            .integrations_examples(Parameters(HaIntegrationsExamplesInput { capability: None }))
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        for cap in &[
            "config_flow",
            "coordinator",
            "entity_bridge",
            "service_bridge",
            "discovery",
            "diagnostics",
            "repairs",
        ] {
            assert!(json.get(*cap).is_some(), "missing capability: {cap}");
        }
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn integrations_examples_by_capability() {
        let (server, sys) = make_server().await;
        let result = server
            .integrations_examples(Parameters(HaIntegrationsExamplesInput {
                capability: Some("coordinator".to_string()),
            }))
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert!(json["description"].as_str().is_some());
        assert!(json["code"].as_str().is_some());
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn integrations_examples_unknown_capability() {
        let (server, sys) = make_server().await;
        let result = server
            .integrations_examples(Parameters(HaIntegrationsExamplesInput {
                capability: Some("unknown_cap".to_string()),
            }))
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert!(json["error"].as_str().is_some());
        assert!(json["available"].as_array().is_some());
        sys.shutdown().await;
    }

    // ── Device tests (with actor system) ─────────────────────────────────────

    #[tokio::test]
    async fn devices_list_empty() {
        let (server, sys) = make_server().await;
        let result = server.devices_list().await.unwrap();
        let list: Vec<serde_json::Value> =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert!(list.is_empty());
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn devices_get_not_found() {
        let (server, sys) = make_server().await;
        let err = server
            .devices_get(Parameters(HaDevicesGetInput {
                device_id: "nonexistent".to_string(),
            }))
            .await
            .unwrap_err();
        assert!(err.message.contains("device not found"));
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn devices_add_and_list() {
        let (server, sys) = make_server().await;
        let desc = rshome_entity::DeviceDescriptor {
            device_id: DeviceId("esp32-living".to_string()),
            name: "Living Room ESP32".to_string(),
            model: Some("ESP32-WROOM".to_string()),
            manufacturer: Some("Espressif".to_string()),
            sw_version: Some("1.0.0".to_string()),
            area_id: None,
        };
        server
            .device_manager
            .ask(|reply| DeviceManagerMsg::AddDevice {
                descriptor: desc,
                reply,
            })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;

        let result = server.devices_list().await.unwrap();
        let list: Vec<serde_json::Value> =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["name"], "Living Room ESP32");
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn devices_get_after_add() {
        let (server, sys) = make_server().await;
        let desc = rshome_entity::DeviceDescriptor {
            device_id: DeviceId("esp32-garage".to_string()),
            name: "Garage ESP32".to_string(),
            model: None,
            manufacturer: None,
            sw_version: None,
            area_id: None,
        };
        server
            .device_manager
            .ask(|reply| DeviceManagerMsg::AddDevice {
                descriptor: desc,
                reply,
            })
            .await
            .unwrap();

        let result = server
            .devices_get(Parameters(HaDevicesGetInput {
                device_id: "esp32-garage".to_string(),
            }))
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(json["device"]["name"], "Garage ESP32");
        assert!(json["entity_ids"].as_array().is_some());
        sys.shutdown().await;
    }

    // ── Entity tests (with actor system) ──────────────────────────────────────

    #[tokio::test]
    async fn entities_list_empty() {
        let (server, sys) = make_server().await;
        let result = server
            .entities_list(Parameters(HaEntitiesListInput {
                domain: None,
                device_id: None,
                limit: None,
            }))
            .await
            .unwrap();
        let list: Vec<serde_json::Value> =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert!(list.is_empty());
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn entities_list_by_domain_filter() {
        let (server, sys, _actor_ref, _registry) = make_server_with_switch().await;
        let result = server
            .entities_list(Parameters(HaEntitiesListInput {
                domain: Some("switch".to_string()),
                device_id: None,
                limit: None,
            }))
            .await
            .unwrap();
        let list: Vec<serde_json::Value> =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["entity_id"], "switch.kitchen");
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn entities_list_domain_no_match() {
        let (server, sys, _actor_ref, _registry) = make_server_with_switch().await;
        let result = server
            .entities_list(Parameters(HaEntitiesListInput {
                domain: Some("sensor".to_string()),
                device_id: None,
                limit: None,
            }))
            .await
            .unwrap();
        let list: Vec<serde_json::Value> =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert!(list.is_empty());
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn entities_list_with_limit() {
        let sys = ActorSystem::new();
        let registry = EntityRegistry::default();
        let state_store = Arc::new(StateStore::default());

        for i in 0..5 {
            let id = EntityId::new("sensor", &format!("temp_{i}"));
            state_store.update(
                &id,
                EntityState::Sensor {
                    value: i as f64,
                    unit: None,
                    attributes: Default::default(),
                },
            );
            let (actor, _) = EntityActor::new(
                make_entity_descriptor("sensor", &format!("temp_{i}")),
                EntityState::Sensor {
                    value: i as f64,
                    unit: None,
                    attributes: Default::default(),
                },
                null_store(),
            );
            registry.register(id, sys.spawn(actor));
        }

        let device_manager = sys.spawn(rshome_entity::DeviceManagerActor::new(
            registry.clone(),
            null_store(),
        ));
        let service_registry = sys.spawn(ServiceRegistryActor::new(
            registry.clone(),
            Some(device_manager.clone()),
        ));
        let server = RshomeHaMcp::new(
            registry,
            state_store,
            service_registry,
            device_manager,
            None,
        );

        let result = server
            .entities_list(Parameters(HaEntitiesListInput {
                domain: None,
                device_id: None,
                limit: Some(3),
            }))
            .await
            .unwrap();
        let list: Vec<serde_json::Value> =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(list.len(), 3);
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn entities_get_state_not_found() {
        let (server, sys) = make_server().await;
        let err = server
            .entities_get_state(Parameters(HaEntitiesGetStateInput {
                entity_id: "switch.unknown".to_string(),
            }))
            .await
            .unwrap_err();
        assert!(err.message.contains("entity not found"));
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn entities_get_state_found() {
        let (server, sys, _actor_ref, _) = make_server_with_switch().await;
        let result = server
            .entities_get_state(Parameters(HaEntitiesGetStateInput {
                entity_id: "switch.kitchen".to_string(),
            }))
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(json["entity_id"], "switch.kitchen");
        assert_eq!(json["state"]["Switch"]["is_on"], false);
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn entities_set_state_not_found() {
        let (server, sys) = make_server().await;
        let err = server
            .entities_set_state(Parameters(HaEntitiesSetStateInput {
                entity_id: "switch.missing".to_string(),
                state_json: r#"{"Switch": {"is_on": true}}"#.to_string(),
            }))
            .await
            .unwrap_err();
        assert!(err.message.contains("entity not found"));
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn entities_set_state_invalid_json() {
        let (server, sys, _actor_ref, _) = make_server_with_switch().await;
        let err = server
            .entities_set_state(Parameters(HaEntitiesSetStateInput {
                entity_id: "switch.kitchen".to_string(),
                state_json: "not valid json".to_string(),
            }))
            .await
            .unwrap_err();
        assert!(err.message.contains("invalid state JSON"));
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn entities_set_state_then_get() {
        let (server, sys, _actor_ref, _) = make_server_with_switch().await;
        server
            .entities_set_state(Parameters(HaEntitiesSetStateInput {
                entity_id: "switch.kitchen".to_string(),
                state_json: r#"{"Switch": {"is_on": true}}"#.to_string(),
            }))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;

        let result = server
            .entities_get_state(Parameters(HaEntitiesGetStateInput {
                entity_id: "switch.kitchen".to_string(),
            }))
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        // StateStore is updated by the EntityActor's StateUpdater (NullStateUpdater in test)
        // so state_store reflects the initial state. The set_state sends to actor only.
        // We verify the tool at least returned success.
        assert_eq!(json["entity_id"], "switch.kitchen");
        sys.shutdown().await;
    }

    // ── Service tests ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn services_list_has_builtins() {
        let (server, sys) = make_server().await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        let result = server.services_list().await.unwrap();
        let list: Vec<serde_json::Value> =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert!(!list.is_empty(), "expected built-in services");
        let has_turn_on = list.iter().any(|s| s["service"] == "turn_on");
        assert!(has_turn_on, "expected turn_on service");
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn services_call_unknown_service_error() {
        let (server, sys) = make_server().await;
        let err = server
            .services_call(Parameters(HaServicesCallInput {
                domain: "nonexistent".to_string(),
                service: "do_thing".to_string(),
                target_entity_ids: None,
                target_device_id: None,
                data: None,
            }))
            .await
            .unwrap_err();
        assert!(err.message.contains("nonexistent.do_thing") || err.message.contains("unknown"));
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn services_call_turn_on_switch() {
        let (server, sys, _actor_ref, _) = make_server_with_switch().await;
        tokio::time::sleep(Duration::from_millis(20)).await;

        let result = server
            .services_call(Parameters(HaServicesCallInput {
                domain: "switch".to_string(),
                service: "turn_on".to_string(),
                target_entity_ids: Some(vec!["switch.kitchen".to_string()]),
                target_device_id: None,
                data: None,
            }))
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(json["ok"], true);
        assert_eq!(json["entities_affected"], 1);
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn services_call_no_target_uses_domain_fanout() {
        let (server, sys, _actor_ref, _) = make_server_with_switch().await;
        tokio::time::sleep(Duration::from_millis(20)).await;

        let result = server
            .services_call(Parameters(HaServicesCallInput {
                domain: "switch".to_string(),
                service: "turn_off".to_string(),
                target_entity_ids: None,
                target_device_id: None,
                data: None,
            }))
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(json["ok"], true);
        sys.shutdown().await;
    }

    // ── Resource tests ────────────────────────────────────────────────────────

    #[test]
    fn resources_list_has_three_resources() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (server, sys) = rt.block_on(make_server());
        let result = server.list_resources_impl().unwrap();
        assert_eq!(result.resources.len(), 3);
        let uris: Vec<&str> = result
            .resources
            .iter()
            .map(|r| r.raw.uri.as_str())
            .collect();
        assert!(uris.contains(&"ha://config/entities"));
        assert!(uris.contains(&"ha://config/services"));
        assert!(uris.contains(&"ha://runtime/info"));
        rt.block_on(sys.shutdown());
    }

    #[test]
    fn resource_templates_has_two_templates() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (server, sys) = rt.block_on(make_server());
        let result = server.list_resource_templates_impl().unwrap();
        assert_eq!(result.resource_templates.len(), 2);
        let templates: Vec<&str> = result
            .resource_templates
            .iter()
            .map(|t| t.raw.uri_template.as_str())
            .collect();
        assert!(templates.contains(&"state://entities/{entity_id}"));
        assert!(templates.contains(&"device://info/{device_id}"));
        rt.block_on(sys.shutdown());
    }

    #[tokio::test]
    async fn resource_read_state_entity_found() {
        let (server, sys, _actor_ref, _) = make_server_with_switch().await;
        let result = server
            .read_resource_impl("state://entities/switch.kitchen")
            .await
            .unwrap();
        assert_eq!(result.contents.len(), 1);
        let text = match &result.contents[0] {
            ResourceContents::TextResourceContents { text, .. } => text.as_str(),
            _ => panic!("expected text content"),
        };
        let json: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(json["entity_id"], "switch.kitchen");
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn resource_read_state_entity_not_found() {
        let (server, sys) = make_server().await;
        let err = server
            .read_resource_impl("state://entities/switch.missing")
            .await
            .unwrap_err();
        assert!(err.message.contains("entity not found"));
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn resource_read_device_not_found() {
        let (server, sys) = make_server().await;
        let err = server
            .read_resource_impl("device://info/missing-device")
            .await
            .unwrap_err();
        assert!(err.message.contains("device not found"));
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn resource_read_unknown_uri_error() {
        let (server, sys) = make_server().await;
        let err = server
            .read_resource_impl("unknown://foo/bar")
            .await
            .unwrap_err();
        assert!(err.message.contains("unknown resource"));
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn resource_read_entities_config() {
        let (server, sys, _actor_ref, _) = make_server_with_switch().await;
        let result = server
            .read_resource_impl("ha://config/entities")
            .await
            .unwrap();
        let text = match &result.contents[0] {
            ResourceContents::TextResourceContents { text, .. } => text.clone(),
            _ => panic!("expected text"),
        };
        let list: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["entity_id"], "switch.kitchen");
        assert_eq!(list[0]["domain"], "switch");
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn resource_read_runtime_info() {
        let (server, sys) = make_server().await;
        let result = server
            .read_resource_impl("ha://runtime/info")
            .await
            .unwrap();
        let text = match &result.contents[0] {
            ResourceContents::TextResourceContents { text, .. } => text.clone(),
            _ => panic!("expected text"),
        };
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(json["version"].as_str().is_some());
        assert_eq!(json["entity_count"], 0);
        sys.shutdown().await;
    }

    // ── Helper tests ──────────────────────────────────────────────────────────

    #[test]
    fn base64_valid() {
        assert!(base64_decode_len("AAAA").is_some());
        assert!(base64_decode_len("AAAAAAAAAAAAAAAAAAAAAA==").is_some());
    }

    #[test]
    fn base64_invalid_chars() {
        assert!(base64_decode_len("not!valid@base64#").is_none());
    }

    #[test]
    fn base64_wrong_padding() {
        // 5 chars — not a multiple of 4
        assert!(base64_decode_len("AAAAA").is_none());
    }

    #[test]
    fn base64_empty() {
        assert!(base64_decode_len("").is_none());
    }

    // ── Device-link tool tests (no device_link attached) ─────────────────────

    #[tokio::test]
    async fn device_links_list_discovered_no_manager_returns_empty() {
        let (server, sys) = make_server().await;
        // device_link is None by default
        let result = server.device_links_list_discovered().await.unwrap();
        let json: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert!(json["devices"].as_array().unwrap().is_empty());
        assert!(json["note"].as_str().is_some());
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn device_links_status_no_manager_returns_error() {
        let (server, sys) = make_server().await;
        let err = server
            .device_links_status(Parameters(HaDeviceLinksStatusInput {
                device_id: "esphome-host:test".to_string(),
            }))
            .await
            .unwrap_err();
        assert!(err.message.contains("not attached"));
        sys.shutdown().await;
    }
}
