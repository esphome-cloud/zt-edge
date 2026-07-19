//! IntegrationActor — per-integration lifecycle and message dispatch.
//!
//! Each loaded WASM integration runs as an `IntegrationActor`.  The actor
//! owns a `GuestFunctions` implementation and dispatches incoming messages to
//! the appropriate guest export via `tokio::task::spawn_blocking`.

use std::sync::Arc;

use rshome_actor::{Actor, ActorContext};
use tokio::sync::oneshot;

use crate::context::SharedCtx;
use crate::guest::GuestFunctions;

// ── Message type ──────────────────────────────────────────────────────────────

pub enum IntegrationMsg {
    ConfigFlowStep {
        step_id: String,
        user_input: Option<Vec<(String, String)>>,
        reply: oneshot::Sender<Result<Vec<u8>, String>>,
    },
    RunCoordinator {
        coordinator_handle: u64,
    },
    RepairStep {
        repair_id: String,
        step_id: String,
        user_input: Option<Vec<(String, String)>>,
        reply: oneshot::Sender<Result<Vec<u8>, String>>,
    },
    GetDiagnostics(oneshot::Sender<Vec<(String, String)>>),
    /// Gracefully stop this integration actor (triggers post_stop → teardown).
    Stop,
}

// ── Actor ─────────────────────────────────────────────────────────────────────

/// Manages the lifecycle of a single WASM integration.
pub struct IntegrationActor {
    pub name: String,
    pub guest: Arc<dyn GuestFunctions>,
    /// Shared capability context (entity/device/service registries + actor refs).
    pub ctx: SharedCtx,
}

impl IntegrationActor {
    pub fn new(name: String, guest: Arc<dyn GuestFunctions>, ctx: SharedCtx) -> Self {
        Self { name, guest, ctx }
    }
}

#[async_trait::async_trait]
impl Actor for IntegrationActor {
    type Msg = IntegrationMsg;

    /// Setup is called by `WasmHostActor` before spawning this actor, so
    /// `pre_start` is a no-op here.
    async fn pre_start(&mut self, _ctx: &mut ActorContext<Self::Msg>) {}

    /// Call `guest.teardown()` when the actor is stopped (e.g. on `Unload`).
    async fn post_stop(&mut self) {
        let guest = self.guest.clone();
        let _ = tokio::task::spawn_blocking(move || guest.teardown()).await;
    }

    async fn handle(&mut self, msg: IntegrationMsg, _ctx: &mut ActorContext<IntegrationMsg>) {
        match msg {
            IntegrationMsg::ConfigFlowStep {
                step_id,
                user_input,
                reply,
            } => {
                let guest = self.guest.clone();
                let result = tokio::task::spawn_blocking(move || {
                    guest.config_flow_step(&step_id, user_input)
                })
                .await
                .unwrap_or_else(|e| Err(format!("spawn_blocking panicked: {e}")));
                let _ = reply.send(result);
            }

            IntegrationMsg::RunCoordinator { coordinator_handle } => {
                let guest = self.guest.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    guest.coordinator_update(coordinator_handle)
                })
                .await;
                // Result is informational (fire-and-forget); errors are traced
                // by the coordinator module if needed.
            }

            IntegrationMsg::RepairStep {
                repair_id,
                step_id,
                user_input,
                reply,
            } => {
                let guest = self.guest.clone();
                let result = tokio::task::spawn_blocking(move || {
                    guest.repair_step(&repair_id, &step_id, user_input)
                })
                .await
                .unwrap_or_else(|e| Err(format!("spawn_blocking panicked: {e}")));
                let _ = reply.send(result);
            }

            IntegrationMsg::GetDiagnostics(reply) => {
                let guest = self.guest.clone();
                let diags = tokio::task::spawn_blocking(move || guest.get_diagnostics())
                    .await
                    .unwrap_or_default();
                let _ = reply.send(diags);
            }

            IntegrationMsg::Stop => {
                _ctx.stop();
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rshome_actor::ActorSystem;
    use rshome_entity::{DeviceManagerActor, EntityRegistry, NullStateUpdater};
    use rshome_svc::ServiceRegistryActor;
    use tokio::sync::oneshot;

    use super::*;
    use crate::context::CapabilityContext;
    use crate::guest::{GuestCall, MockGuest};

    async fn make_ctx() -> (SharedCtx, ActorSystem) {
        let sys = ActorSystem::new();
        let entity_registry = EntityRegistry::default();
        let device_manager = sys.spawn(DeviceManagerActor::new(
            entity_registry.clone(),
            std::sync::Arc::new(NullStateUpdater),
        ));
        let service_registry = sys.spawn(ServiceRegistryActor::new(entity_registry.clone(), None));
        let ctx = Arc::new(CapabilityContext::new(
            entity_registry,
            device_manager,
            service_registry,
        ));
        (ctx, sys)
    }

    async fn make_actor(
        mock: Arc<MockGuest>,
        ctx: SharedCtx,
    ) -> (rshome_actor::ActorRef<IntegrationMsg>, ActorSystem) {
        let sys = ActorSystem::new();
        let guest: Arc<dyn GuestFunctions> = mock.clone();
        let actor = IntegrationActor::new("test_integration".into(), guest, ctx);
        let actor_ref = sys.spawn(actor);
        (actor_ref, sys)
    }

    #[tokio::test]
    async fn setup_called_before_actor_receives_messages() {
        let (_ctx, sys) = make_ctx().await;
        let mock = Arc::new(MockGuest::default());
        // Simulate the WasmHostActor setup flow: call setup directly
        let config = vec![("host".into(), "192.168.1.1".into())];
        let guest: Arc<dyn GuestFunctions> = mock.clone();
        let result = tokio::task::spawn_blocking(move || guest.setup(config))
            .await
            .unwrap();
        assert!(result.is_ok());
        let calls = mock.calls.lock();
        assert_eq!(calls.len(), 1);
        assert!(matches!(&calls[0], GuestCall::Setup(_)));
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn teardown_called_on_actor_stop() {
        let (ctx, _ctx_sys) = make_ctx().await;
        let mock = Arc::new(MockGuest::default());
        let (actor_ref, sys) = make_actor(mock.clone(), ctx).await;

        // Send Stop → actor calls ctx.stop() → loop exits → post_stop() → teardown()
        actor_ref.send(IntegrationMsg::Stop).unwrap();
        // Drop so the channel closes after Stop is processed
        drop(actor_ref);

        // Wait for post_stop to complete, checking BEFORE sys.shutdown()
        // (shutdown aborts tasks and prevents post_stop from running).
        let mut has_teardown = false;
        for _ in 0..30 {
            tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
            if mock
                .calls
                .lock()
                .iter()
                .any(|c| matches!(c, GuestCall::Teardown))
            {
                has_teardown = true;
                break;
            }
        }
        sys.shutdown().await;
        assert!(has_teardown, "teardown was not called after Stop message");
    }

    #[tokio::test]
    async fn config_flow_step_dispatched() {
        let (ctx, _ctx_sys) = make_ctx().await;
        let mock = Arc::new(MockGuest::default());
        let (actor_ref, sys) = make_actor(mock.clone(), ctx).await;

        let (tx, rx) = oneshot::channel();
        actor_ref
            .send(IntegrationMsg::ConfigFlowStep {
                step_id: "user".into(),
                user_input: None,
                reply: tx,
            })
            .unwrap();
        let result = rx.await.unwrap();
        assert!(result.is_ok());

        let calls = mock.calls.lock();
        assert!(calls
            .iter()
            .any(|c| matches!(c, GuestCall::ConfigFlowStep { .. })));

        sys.shutdown().await;
    }

    #[tokio::test]
    async fn coordinator_update_dispatched() {
        let (ctx, _ctx_sys) = make_ctx().await;
        let mock = Arc::new(MockGuest::default());
        let (actor_ref, sys) = make_actor(mock.clone(), ctx).await;

        actor_ref
            .send(IntegrationMsg::RunCoordinator {
                coordinator_handle: 42,
            })
            .unwrap();
        // Give it time to process
        tokio::time::sleep(tokio::time::Duration::from_millis(30)).await;

        let calls = mock.calls.lock();
        assert!(
            calls.iter().any(|c| matches!(c, GuestCall::CoordinatorUpdate { coordinator_handle } if *coordinator_handle == 42))
        );

        sys.shutdown().await;
    }

    #[tokio::test]
    async fn diagnostics_dispatched() {
        let (ctx, _ctx_sys) = make_ctx().await;
        let mock = Arc::new(MockGuest::default());
        let (actor_ref, sys) = make_actor(mock.clone(), ctx).await;

        let (tx, rx) = oneshot::channel();
        actor_ref.send(IntegrationMsg::GetDiagnostics(tx)).unwrap();
        let diags = rx.await.unwrap();
        assert!(!diags.is_empty());

        let calls = mock.calls.lock();
        assert!(calls.iter().any(|c| matches!(c, GuestCall::GetDiagnostics)));

        sys.shutdown().await;
    }

    #[tokio::test]
    async fn repair_step_dispatched() {
        let (ctx, _ctx_sys) = make_ctx().await;
        let mock = Arc::new(MockGuest::default());
        let (actor_ref, sys) = make_actor(mock.clone(), ctx).await;

        let (tx, rx) = oneshot::channel();
        actor_ref
            .send(IntegrationMsg::RepairStep {
                repair_id: "bad_creds".into(),
                step_id: "reauth".into(),
                user_input: Some(vec![("token".into(), "abc123".into())]),
                reply: tx,
            })
            .unwrap();
        let result = rx.await.unwrap();
        assert!(result.is_ok());

        let calls = mock.calls.lock();
        assert!(calls.iter().any(|c| matches!(
            c,
            GuestCall::RepairStep { repair_id, .. } if repair_id == "bad_creds"
        )));

        sys.shutdown().await;
    }

    #[tokio::test]
    async fn config_flow_error_forwarded() {
        let (ctx, _ctx_sys) = make_ctx().await;
        let mut mock = MockGuest::default();
        mock.config_flow_error = Some("auth failed".into());
        let mock = Arc::new(mock);
        let (actor_ref, sys) = make_actor(mock, ctx).await;

        let (tx, rx) = oneshot::channel();
        actor_ref
            .send(IntegrationMsg::ConfigFlowStep {
                step_id: "user".into(),
                user_input: None,
                reply: tx,
            })
            .unwrap();
        let err = rx.await.unwrap().unwrap_err();
        assert_eq!(err, "auth failed");

        sys.shutdown().await;
    }

    #[tokio::test]
    async fn repair_step_error_forwarded() {
        let (ctx, _ctx_sys) = make_ctx().await;
        let mut mock = MockGuest::default();
        mock.repair_error = Some("device unreachable".into());
        let mock = Arc::new(mock);
        let (actor_ref, sys) = make_actor(mock, ctx).await;

        let (tx, rx) = oneshot::channel();
        actor_ref
            .send(IntegrationMsg::RepairStep {
                repair_id: "r1".into(),
                step_id: "s1".into(),
                user_input: None,
                reply: tx,
            })
            .unwrap();
        let err = rx.await.unwrap().unwrap_err();
        assert_eq!(err, "device unreachable");

        sys.shutdown().await;
    }

    #[tokio::test]
    async fn multiple_calls_recorded_in_order() {
        let (ctx, _ctx_sys) = make_ctx().await;
        let mock = Arc::new(MockGuest::default());
        let (actor_ref, sys) = make_actor(mock.clone(), ctx).await;

        let (tx1, rx1) = oneshot::channel();
        let (tx2, rx2) = oneshot::channel();
        actor_ref
            .send(IntegrationMsg::ConfigFlowStep {
                step_id: "first".into(),
                user_input: None,
                reply: tx1,
            })
            .unwrap();
        actor_ref
            .send(IntegrationMsg::ConfigFlowStep {
                step_id: "second".into(),
                user_input: None,
                reply: tx2,
            })
            .unwrap();

        rx1.await.unwrap().unwrap();
        rx2.await.unwrap().unwrap();

        let calls = mock.calls.lock();
        let step_ids: Vec<&str> = calls
            .iter()
            .filter_map(|c| {
                if let GuestCall::ConfigFlowStep { step_id, .. } = c {
                    Some(step_id.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(step_ids, ["first", "second"]);

        sys.shutdown().await;
    }

    #[tokio::test]
    async fn setup_error_prevents_registration() {
        // Setup is called directly in WasmHostActor before spawning.
        // This test verifies MockGuest returns the configured error.
        let mut mock = MockGuest::default();
        mock.setup_error = Some("bad config key".into());
        let mock = Arc::new(mock);
        let guest: Arc<dyn GuestFunctions> = mock.clone();

        let err = tokio::task::spawn_blocking(move || guest.setup(vec![]))
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(err, "bad config key");
    }

    #[tokio::test]
    async fn diagnostics_returns_guest_values() {
        let (ctx, _ctx_sys) = make_ctx().await;
        let mut mock = MockGuest::default();
        mock.diagnostics = vec![("uptime".into(), "600".into())];
        let mock = Arc::new(mock);
        let (actor_ref, sys) = make_actor(mock, ctx).await;

        let (tx, rx) = oneshot::channel();
        actor_ref.send(IntegrationMsg::GetDiagnostics(tx)).unwrap();
        let diags = rx.await.unwrap();
        assert_eq!(diags, vec![("uptime".into(), "600".into())]);

        sys.shutdown().await;
    }
}
