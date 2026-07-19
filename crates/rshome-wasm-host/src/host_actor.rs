//! WasmHostActor — loads, tracks, and unloads WASM integration modules.
//!
//! The actor manages a registry of active integrations. On `Load`, it:
//!   1. Creates a `GuestFunctions` via the injected factory.
//!   2. Calls `guest.setup(config)` in a blocking task.
//!   3. Spawns an `IntegrationActor` as a child.
//!
//! On `Unload`, it drops the child `ActorRef`, which closes the mailbox and
//! triggers `IntegrationActor::post_stop` → `guest.teardown()`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use rshome_actor::{Actor, ActorContext, ActorRef};
use rshome_entity::{DeviceManagerMsg, EntityRegistry};
use rshome_svc::ServiceMsg;
use tokio::sync::oneshot;

use crate::context::CapabilityContext;
use crate::guest::GuestFunctions;
use crate::integration_actor::{IntegrationActor, IntegrationMsg};

// ── IntegrationId ─────────────────────────────────────────────────────────────

static NEXT_INTEGRATION_ID: AtomicU64 = AtomicU64::new(1);

/// Stable identifier for a loaded integration.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
pub struct IntegrationId(String);

impl IntegrationId {
    fn new_random() -> Self {
        let n = NEXT_INTEGRATION_ID.fetch_add(1, Ordering::Relaxed);
        Self(format!("integration_{n}"))
    }
}

impl std::fmt::Display for IntegrationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ── IntegrationInfo ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum IntegrationStatus {
    Running,
    Failed,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct IntegrationInfo {
    pub id: IntegrationId,
    pub name: String,
    pub status: IntegrationStatus,
}

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum WasmHostError {
    #[error("integration not found: {id}")]
    NotFound { id: IntegrationId },
    #[error("WASM validation error: {0}")]
    WasmValidation(String),
    #[error("setup failed: {0}")]
    SetupFailed(String),
    #[error("actor error: {0}")]
    Actor(String),
}

// ── Message types ─────────────────────────────────────────────────────────────

/// Factory that produces a `GuestFunctions` from raw WASM bytes.
///
/// In production: wraps `WasmGuest::new`.
/// In tests: returns a `MockGuest` regardless of bytes.
pub type GuestFactory =
    Arc<dyn Fn(Vec<u8>) -> Result<Arc<dyn GuestFunctions>, WasmHostError> + Send + Sync>;

pub enum WasmHostMsg {
    Load {
        name: String,
        wasm_bytes: Vec<u8>,
        config: Vec<(String, String)>,
        reply: oneshot::Sender<Result<IntegrationId, WasmHostError>>,
    },
    Unload {
        id: IntegrationId,
        reply: oneshot::Sender<Result<(), WasmHostError>>,
    },
    List(oneshot::Sender<Vec<IntegrationInfo>>),
    GetDiagnostics {
        id: IntegrationId,
        reply: oneshot::Sender<Result<Vec<(String, String)>, WasmHostError>>,
    },
    Stop,
}

// ── Actor ─────────────────────────────────────────────────────────────────────

pub struct WasmHostActor {
    integrations: HashMap<IntegrationId, ActorRef<IntegrationMsg>>,
    info: HashMap<IntegrationId, IntegrationInfo>,
    entity_registry: EntityRegistry,
    device_manager: ActorRef<DeviceManagerMsg>,
    service_registry: ActorRef<ServiceMsg>,
    guest_factory: GuestFactory,
}

impl WasmHostActor {
    pub fn new(
        entity_registry: EntityRegistry,
        device_manager: ActorRef<DeviceManagerMsg>,
        service_registry: ActorRef<ServiceMsg>,
        guest_factory: GuestFactory,
    ) -> Self {
        Self {
            integrations: HashMap::new(),
            info: HashMap::new(),
            entity_registry,
            device_manager,
            service_registry,
            guest_factory,
        }
    }

    /// Production constructor using `WasmGuest`.
    pub fn with_wasm_guests(
        entity_registry: EntityRegistry,
        device_manager: ActorRef<DeviceManagerMsg>,
        service_registry: ActorRef<ServiceMsg>,
    ) -> Self {
        let factory: GuestFactory = Arc::new(|bytes| {
            crate::guest::WasmGuest::new(bytes)
                .map(|g| Arc::new(g) as Arc<dyn GuestFunctions>)
                .map_err(WasmHostError::WasmValidation)
        });
        Self::new(entity_registry, device_manager, service_registry, factory)
    }
}

#[async_trait::async_trait]
impl Actor for WasmHostActor {
    type Msg = WasmHostMsg;

    async fn handle(&mut self, msg: WasmHostMsg, ctx: &mut ActorContext<WasmHostMsg>) {
        match msg {
            WasmHostMsg::Load {
                name,
                wasm_bytes,
                config,
                reply,
            } => {
                // Check for duplicate name
                if self.info.values().any(|i| i.name == name) {
                    let _ = reply.send(Err(WasmHostError::SetupFailed(format!(
                        "integration '{name}' already loaded"
                    ))));
                    return;
                }

                // Create guest
                let guest = match (self.guest_factory)(wasm_bytes) {
                    Ok(g) => g,
                    Err(e) => {
                        let _ = reply.send(Err(e));
                        return;
                    }
                };

                // Call setup in a blocking context
                let guest_clone = guest.clone();
                let cfg = config.clone();
                let setup_result =
                    tokio::task::spawn_blocking(move || guest_clone.setup(cfg)).await;

                match setup_result {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => {
                        let _ = reply.send(Err(WasmHostError::SetupFailed(e)));
                        return;
                    }
                    Err(e) => {
                        let _ = reply.send(Err(WasmHostError::SetupFailed(e.to_string())));
                        return;
                    }
                }

                // Create capability context and spawn integration actor
                let capability_ctx = Arc::new(CapabilityContext::new(
                    self.entity_registry.clone(),
                    self.device_manager.clone(),
                    self.service_registry.clone(),
                ));
                let actor = IntegrationActor::new(name.clone(), guest, capability_ctx);
                let actor_ref = ctx.spawn_child_default(actor);

                let id = IntegrationId::new_random();
                self.integrations.insert(id.clone(), actor_ref);
                self.info.insert(
                    id.clone(),
                    IntegrationInfo {
                        id: id.clone(),
                        name: name.clone(),
                        status: IntegrationStatus::Running,
                    },
                );

                let _ = reply.send(Ok(id));
            }

            WasmHostMsg::Unload { id, reply } => {
                let actor_ref = match self.integrations.remove(&id) {
                    Some(r) => r,
                    None => {
                        let _ = reply.send(Err(WasmHostError::NotFound { id }));
                        return;
                    }
                };
                self.info.remove(&id);
                // Send Stop so the actor calls ctx.stop() → post_stop → teardown
                let _ = actor_ref.send(IntegrationMsg::Stop);
                let _ = reply.send(Ok(()));
            }

            WasmHostMsg::List(reply) => {
                let list: Vec<IntegrationInfo> = self.info.values().cloned().collect();
                let _ = reply.send(list);
            }

            WasmHostMsg::GetDiagnostics { id, reply } => {
                let actor_ref = match self.integrations.get(&id) {
                    Some(r) => r.clone(),
                    None => {
                        let _ = reply.send(Err(WasmHostError::NotFound { id }));
                        return;
                    }
                };
                let (tx, rx) = oneshot::channel();
                if actor_ref.send(IntegrationMsg::GetDiagnostics(tx)).is_err() {
                    let _ = reply.send(Err(WasmHostError::Actor("actor disconnected".into())));
                    return;
                }
                match rx.await {
                    Ok(diags) => {
                        let _ = reply.send(Ok(diags));
                    }
                    Err(_) => {
                        let _ =
                            reply.send(Err(WasmHostError::Actor("reply channel dropped".into())));
                    }
                }
            }

            WasmHostMsg::Stop => {
                ctx.stop();
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use rshome_actor::ActorSystem;
    use rshome_entity::{DeviceManagerActor, EntityRegistry, NullStateUpdater};
    use rshome_svc::ServiceRegistryActor;
    use std::sync::Arc;

    use super::*;
    use crate::guest::{GuestFunctions, MockGuest};

    fn mock_factory() -> GuestFactory {
        Arc::new(|_bytes| Ok(Arc::new(MockGuest::default()) as Arc<dyn GuestFunctions>))
    }

    async fn make_host() -> (ActorRef<WasmHostMsg>, ActorSystem) {
        let sys = ActorSystem::new();
        let entity_registry = EntityRegistry::default();
        let device_manager = sys.spawn(DeviceManagerActor::new(
            entity_registry.clone(),
            Arc::new(NullStateUpdater),
        ));
        let service_registry = sys.spawn(ServiceRegistryActor::new(entity_registry.clone(), None));
        let actor = WasmHostActor::new(
            entity_registry,
            device_manager,
            service_registry,
            mock_factory(),
        );
        let host_ref = sys.spawn(actor);
        (host_ref, sys)
    }

    async fn load(
        host: &ActorRef<WasmHostMsg>,
        name: &str,
    ) -> Result<IntegrationId, WasmHostError> {
        host.ask(|tx| WasmHostMsg::Load {
            name: name.to_owned(),
            wasm_bytes: vec![0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00], // \0asm stub
            config: vec![],
            reply: tx,
        })
        .await
        .expect("actor disconnected")
    }

    #[tokio::test]
    async fn load_integration_succeeds() {
        let (host, sys) = make_host().await;
        let id = load(&host, "my_integration").await;
        assert!(id.is_ok());
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn list_returns_loaded_integrations() {
        let (host, sys) = make_host().await;
        load(&host, "int_a").await.unwrap();
        load(&host, "int_b").await.unwrap();

        let list = host
            .ask(|tx| WasmHostMsg::List(tx))
            .await
            .expect("actor disconnected");
        assert_eq!(list.len(), 2);
        let names: Vec<&str> = list.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"int_a"));
        assert!(names.contains(&"int_b"));

        sys.shutdown().await;
    }

    #[tokio::test]
    async fn unload_removes_integration() {
        let (host, sys) = make_host().await;
        let id = load(&host, "to_remove").await.unwrap();

        let result = host
            .ask(|tx| WasmHostMsg::Unload { id, reply: tx })
            .await
            .expect("actor disconnected");
        assert!(result.is_ok());

        let list = host.ask(|tx| WasmHostMsg::List(tx)).await.unwrap();
        assert!(list.is_empty());

        sys.shutdown().await;
    }

    #[tokio::test]
    async fn unload_unknown_id_returns_error() {
        let (host, sys) = make_host().await;
        let fake_id = IntegrationId("integration_9999".into());

        let result = host
            .ask(|tx| WasmHostMsg::Unload {
                id: fake_id,
                reply: tx,
            })
            .await
            .expect("actor disconnected");
        assert!(matches!(result, Err(WasmHostError::NotFound { .. })));

        sys.shutdown().await;
    }

    #[tokio::test]
    async fn duplicate_load_same_name_returns_error() {
        let (host, sys) = make_host().await;
        load(&host, "dupe").await.unwrap();
        let result = load(&host, "dupe").await;
        assert!(matches!(result, Err(WasmHostError::SetupFailed(_))));

        sys.shutdown().await;
    }

    #[tokio::test]
    async fn get_diagnostics_returns_values() {
        let (host, sys) = make_host().await;
        let id = load(&host, "diag_int").await.unwrap();

        let result = host
            .ask(|tx| WasmHostMsg::GetDiagnostics { id, reply: tx })
            .await
            .expect("actor disconnected");
        assert!(result.is_ok());
        let diags = result.unwrap();
        // MockGuest default diagnostics: [("status", "ok")]
        assert!(!diags.is_empty());

        sys.shutdown().await;
    }

    #[tokio::test]
    async fn load_with_setup_failure_returns_error() {
        let sys = ActorSystem::new();
        let entity_registry = EntityRegistry::default();
        let device_manager = sys.spawn(DeviceManagerActor::new(
            entity_registry.clone(),
            Arc::new(NullStateUpdater),
        ));
        let service_registry = sys.spawn(ServiceRegistryActor::new(entity_registry.clone(), None));
        let factory: GuestFactory = Arc::new(|_| {
            let mut m = MockGuest::default();
            m.setup_error = Some("bad credentials".into());
            Ok(Arc::new(m) as Arc<dyn GuestFunctions>)
        });
        let actor = WasmHostActor::new(entity_registry, device_manager, service_registry, factory);
        let host = sys.spawn(actor);

        let result = host
            .ask(|tx| WasmHostMsg::Load {
                name: "failing_int".into(),
                wasm_bytes: vec![0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00],
                config: vec![],
                reply: tx,
            })
            .await
            .expect("actor disconnected");
        assert!(matches!(result, Err(WasmHostError::SetupFailed(_))));

        sys.shutdown().await;
    }
}
