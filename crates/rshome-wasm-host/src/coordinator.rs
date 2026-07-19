//! DataUpdateCoordinator — periodic fetch loop for integrations.
//!
//! Coordinators are registered by the WASM guest via the `create-coordinator`
//! host import.  The host then periodically calls `coordinator-update` on the
//! guest, which fetches fresh data from the device or service.

use std::sync::Arc;

use crate::capability::{Handle, ObjectRegistry, Rights};
use crate::context::CoordinatorHandle;
use crate::guest::GuestFunctions;
use crate::integration_actor::IntegrationMsg;
use rshome_actor::ActorRef;

/// Register a new coordinator in the context's coordinator registry.
///
/// Returns the handle passed back to the WASM guest.  The caller is
/// responsible for starting the coordinator tick loop (see `start_coordinator_loop`).
pub fn create_coordinator(
    coordinator_reg: &ObjectRegistry<CoordinatorHandle>,
    name: &str,
    interval_ms: u64,
) -> Handle {
    coordinator_reg.alloc(
        CoordinatorHandle {
            name: name.to_owned(),
            interval_ms,
        },
        Rights::INVOKE,
    )
}

/// Invoke `coordinator_update` on the guest for a given handle.
///
/// The result (JSON bytes) is returned to the caller which may use it to
/// push entity state updates.
pub fn run_coordinator_update(
    guest: &Arc<dyn GuestFunctions>,
    coordinator_handle: Handle,
) -> Result<Vec<u8>, String> {
    guest.coordinator_update(coordinator_handle)
}

/// Spawn a background task that periodically sends `RunCoordinator` to
/// the integration actor.  The task exits when the actor's channel is closed.
pub fn start_coordinator_loop(
    actor_ref: ActorRef<IntegrationMsg>,
    coordinator_handle: Handle,
    interval_ms: u64,
) {
    tokio::spawn(async move {
        let period = tokio::time::Duration::from_millis(interval_ms.max(1));
        let mut interval = tokio::time::interval(period);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            if actor_ref
                .send_async(IntegrationMsg::RunCoordinator { coordinator_handle })
                .await
                .is_err()
            {
                break; // actor stopped
            }
        }
    });
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guest::MockGuest;

    #[test]
    fn create_coordinator_returns_valid_handle() {
        let reg: ObjectRegistry<CoordinatorHandle> = ObjectRegistry::new();
        let h = create_coordinator(&reg, "weather", 60_000);
        assert_ne!(h, crate::capability::INVALID_HANDLE);
        let entry = reg.get_any(h).unwrap();
        assert_eq!(entry.value.name, "weather");
        assert_eq!(entry.value.interval_ms, 60_000);
    }

    #[test]
    fn coordinator_update_calls_guest() {
        let reg: ObjectRegistry<CoordinatorHandle> = ObjectRegistry::new();
        let mock = Arc::new(MockGuest::default());
        let guest: Arc<dyn GuestFunctions> = mock.clone();

        let h = create_coordinator(&reg, "temp_sensor", 5_000);
        let result = run_coordinator_update(&guest, h);
        assert!(result.is_ok());

        let calls = mock.calls.lock();
        assert_eq!(calls.len(), 1);
        match &calls[0] {
            crate::guest::GuestCall::CoordinatorUpdate { coordinator_handle } => {
                assert_eq!(*coordinator_handle, h);
            }
            other => panic!("unexpected call: {other:?}"),
        }
    }

    #[test]
    fn coordinator_update_error_propagated() {
        let mut mock = MockGuest::default();
        mock.coordinator_error = Some("device offline".into());
        let mock = Arc::new(mock);
        let guest: Arc<dyn GuestFunctions> = mock.clone();

        let reg: ObjectRegistry<CoordinatorHandle> = ObjectRegistry::new();
        let h = create_coordinator(&reg, "sensor", 1_000);
        let err = run_coordinator_update(&guest, h).unwrap_err();
        assert_eq!(err, "device offline");
    }
}
