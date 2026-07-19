//! Entity and device registration bridge.
//!
//! Called by the host when a WASM guest invokes `register-entity`,
//! `update-entity-state`, or `register-device` host imports.

use rshome_entity::{EntityId, EntityMsg, EntityRegistry, EntityState};

use crate::capability::{CapError, Handle, ObjectRegistry, Rights};
use crate::context::{DeviceHandle, EntityHandle};

// ── Entity bridge ─────────────────────────────────────────────────────────────

/// # Internal ABI
///
/// Register a new entity for this integration.
///
/// Allocates a capability handle in `entity_reg` and returns it to the guest.
/// The entity is identified by `platform.unique_id` (e.g. `"sensor.temp_01"`).
///
/// Prefer [`CapabilityContext::domain_aware_register_entity`] for domain-validated
/// registrations. This function is the raw bridge used by the WIT import layer.
pub fn register_entity(
    entity_reg: &ObjectRegistry<EntityHandle>,
    platform: &str,
    unique_id: &str,
    _config: Vec<u8>,
) -> Handle {
    entity_reg.alloc(
        EntityHandle {
            platform: platform.to_owned(),
            unique_id: unique_id.to_owned(),
        },
        Rights::READ | Rights::WRITE,
    )
}

/// # Internal ABI
///
/// Push a state update for an entity identified by `entity_handle`.
///
/// Resolves the handle to an `EntityId`, looks up the actor in `entity_registry`,
/// and sends `EntityMsg::SetState`.  Returns `CapError::NotFound` if the actor
/// is not registered.
pub fn update_entity_state(
    entity_reg: &ObjectRegistry<EntityHandle>,
    entity_registry: &EntityRegistry,
    entity_handle: Handle,
    state: EntityState,
) -> Result<(), CapError> {
    let entry = entity_reg.get(entity_handle, Rights::WRITE)?;
    let entity_id = EntityId::new(&entry.value.platform, &entry.value.unique_id);
    let actor_ref = entity_registry.get(&entity_id).ok_or(CapError::NotFound)?;
    // Fire-and-forget: if actor is gone the error is intentionally ignored.
    let _ = actor_ref.send(EntityMsg::SetState(state));
    Ok(())
}

// ── Device bridge ─────────────────────────────────────────────────────────────

/// # Internal ABI
///
/// Register a new device for this integration.
///
/// Allocates a capability handle in `device_reg` and returns it to the guest.
pub fn register_device(
    device_reg: &ObjectRegistry<DeviceHandle>,
    name: &str,
    model: Option<String>,
    manufacturer: Option<String>,
    unique_id: &str,
) -> Handle {
    device_reg.alloc(
        DeviceHandle {
            name: name.to_owned(),
            model,
            manufacturer,
            unique_id: unique_id.to_owned(),
        },
        Rights::READ | Rights::WRITE,
    )
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::INVALID_HANDLE;

    fn entity_reg() -> ObjectRegistry<EntityHandle> {
        ObjectRegistry::new()
    }

    fn device_reg() -> ObjectRegistry<DeviceHandle> {
        ObjectRegistry::new()
    }

    #[test]
    fn register_entity_returns_valid_handle() {
        let reg = entity_reg();
        let h = register_entity(&reg, "sensor", "temp_01", vec![]);
        assert_ne!(h, INVALID_HANDLE);
        let entry = reg.get_any(h).unwrap();
        assert_eq!(entry.value.platform, "sensor");
        assert_eq!(entry.value.unique_id, "temp_01");
    }

    #[test]
    fn register_entity_different_unique_ids_different_handles() {
        let reg = entity_reg();
        let h1 = register_entity(&reg, "sensor", "temp_01", vec![]);
        let h2 = register_entity(&reg, "sensor", "temp_02", vec![]);
        assert_ne!(h1, h2);
        assert_eq!(reg.len(), 2);
    }

    #[test]
    fn update_entity_state_unknown_handle_returns_error() {
        let entity_reg = entity_reg();
        let entity_registry = EntityRegistry::default();
        // Handle not in registry — should fail with NotFound
        let err = update_entity_state(
            &entity_reg,
            &entity_registry,
            0xDEAD_0001,
            EntityState::Unavailable,
        )
        .unwrap_err();
        assert_eq!(err, CapError::NotFound);
    }

    #[test]
    fn register_device_returns_valid_handle() {
        let reg = device_reg();
        let h = register_device(
            &reg,
            "Acme Sensor",
            Some("Model X".into()),
            None,
            "acme_001",
        );
        assert_ne!(h, INVALID_HANDLE);
        let entry = reg.get_any(h).unwrap();
        assert_eq!(entry.value.name, "Acme Sensor");
        assert_eq!(entry.value.model.as_deref(), Some("Model X"));
        assert_eq!(entry.value.unique_id, "acme_001");
    }

    #[test]
    fn register_entity_revoke_then_update_returns_error() {
        let entity_reg = entity_reg();
        let entity_registry = EntityRegistry::default();
        let h = register_entity(&entity_reg, "switch", "relay_1", vec![]);
        entity_reg.revoke(h).unwrap();

        let err = update_entity_state(&entity_reg, &entity_registry, h, EntityState::Unavailable)
            .unwrap_err();
        assert_eq!(err, CapError::Revoked);
    }
}
