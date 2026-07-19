//! CapabilityContext — per-integration shared state.
//!
//! Each loaded integration gets its own `CapabilityContext` containing its
//! capability registries (entity, device, service, coordinator, discovery) and
//! shared actor refs used by bridge functions to communicate with the runtime.

use std::sync::Arc;

use rshome_actor::ActorRef;
use rshome_entity::{DeviceManagerMsg, EntityRegistry};
use rshome_svc::ServiceMsg;

use crate::capability::{Handle, ObjectRegistry, Rights};
use crate::domain_lowering::{DomainLoweringLayer, EntityRegistration, LoweringError};

// ── Handle value types ────────────────────────────────────────────────────────

/// Value stored in the entity capability registry.
#[derive(Debug, Clone)]
pub struct EntityHandle {
    pub platform: String,
    pub unique_id: String,
}

/// Value stored in the device capability registry.
#[derive(Debug, Clone)]
pub struct DeviceHandle {
    pub name: String,
    pub model: Option<String>,
    pub manufacturer: Option<String>,
    pub unique_id: String,
}

/// Value stored in the service capability registry.
#[derive(Debug, Clone)]
pub struct ServiceHandle {
    pub domain: String,
    pub name: String,
}

/// Value stored in the coordinator capability registry.
#[derive(Debug, Clone)]
pub struct CoordinatorHandle {
    pub name: String,
    pub interval_ms: u64,
}

/// Value stored in the discovery capability registry.
#[derive(Debug, Clone)]
pub struct DiscoveryHandle {
    pub protocol: String,
}

// ── CapabilityContext ─────────────────────────────────────────────────────────

/// Per-integration context passed to all bridge functions.
pub struct CapabilityContext {
    /// Capability registry for registered entities.
    pub entity_reg: ObjectRegistry<EntityHandle>,
    /// Capability registry for registered devices.
    pub device_reg: ObjectRegistry<DeviceHandle>,
    /// Capability registry for registered services.
    pub service_reg: ObjectRegistry<ServiceHandle>,
    /// Capability registry for data-update coordinators.
    pub coordinator_reg: ObjectRegistry<CoordinatorHandle>,
    /// Capability registry for discovery subscriptions.
    pub discovery_reg: ObjectRegistry<DiscoveryHandle>,

    /// Domain-aware lowering layer — validates registrations against domain specs.
    pub domain_lowering: DomainLoweringLayer,

    /// Shared entity registry — used for state lookups by ID.
    pub entity_registry: EntityRegistry,
    /// Device manager actor — used to add/remove devices.
    pub device_manager: ActorRef<DeviceManagerMsg>,
    /// Service registry actor — used to register/call services.
    pub service_registry: ActorRef<ServiceMsg>,
}

impl CapabilityContext {
    pub fn new(
        entity_registry: EntityRegistry,
        device_manager: ActorRef<DeviceManagerMsg>,
        service_registry: ActorRef<ServiceMsg>,
    ) -> Self {
        Self {
            entity_reg: ObjectRegistry::new(),
            device_reg: ObjectRegistry::new(),
            service_reg: ObjectRegistry::new(),
            coordinator_reg: ObjectRegistry::new(),
            discovery_reg: ObjectRegistry::new(),
            domain_lowering: DomainLoweringLayer::new(),
            entity_registry,
            device_manager,
            service_registry,
        }
    }

    /// Allocate an entity handle.
    pub fn alloc_entity(&self, platform: String, unique_id: String) -> Handle {
        self.entity_reg.alloc(
            EntityHandle {
                platform,
                unique_id,
            },
            Rights::READ | Rights::WRITE,
        )
    }

    /// Allocate a device handle.
    pub fn alloc_device(
        &self,
        name: String,
        model: Option<String>,
        manufacturer: Option<String>,
        unique_id: String,
    ) -> Handle {
        self.device_reg.alloc(
            DeviceHandle {
                name,
                model,
                manufacturer,
                unique_id,
            },
            Rights::READ | Rights::WRITE,
        )
    }

    /// Allocate a service handle.
    pub fn alloc_service(&self, domain: String, name: String) -> Handle {
        self.service_reg
            .alloc(ServiceHandle { domain, name }, Rights::INVOKE)
    }

    /// Allocate a coordinator handle.
    pub fn alloc_coordinator(&self, name: String, interval_ms: u64) -> Handle {
        self.coordinator_reg
            .alloc(CoordinatorHandle { name, interval_ms }, Rights::INVOKE)
    }

    /// Allocate a discovery subscription handle.
    pub fn alloc_discovery(&self, protocol: String) -> Handle {
        self.discovery_reg
            .alloc(DiscoveryHandle { protocol }, Rights::READ)
    }

    /// Register an entity through the domain-aware lowering layer.
    ///
    /// Validates the registration against the domain spec, allocates a
    /// capability handle, and returns the full [`EntityRegistration`].
    pub fn domain_aware_register_entity(
        &self,
        domain_id: &str,
        unique_id: &str,
        name: &str,
        features: &[String],
        device_class: Option<&str>,
    ) -> Result<(Handle, EntityRegistration), LoweringError> {
        let reg = self.domain_lowering.lower_register_entity(
            domain_id,
            unique_id,
            name,
            features,
            device_class,
        )?;
        let handle = self.entity_reg.alloc(
            EntityHandle {
                platform: reg.platform.clone(),
                unique_id: reg.unique_id.clone(),
            },
            Rights::READ | Rights::WRITE,
        );
        Ok((handle, reg))
    }
}

/// Convenience alias for a shared, thread-safe capability context.
pub type SharedCtx = Arc<CapabilityContext>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::INVALID_HANDLE;
    use rshome_entity::{DeviceManagerActor, EntityRegistry, EntityState, NullStateUpdater};
    use rshome_svc::ServiceRegistryActor;
    use std::sync::Arc;

    fn make_ctx() -> CapabilityContext {
        let entity_registry = EntityRegistry::default();
        let sys = rshome_actor::ActorSystem::new();
        let device_manager = sys.spawn(DeviceManagerActor::new(
            entity_registry.clone(),
            Arc::new(NullStateUpdater),
        ));
        let service_registry = sys.spawn(ServiceRegistryActor::new(entity_registry.clone(), None));
        CapabilityContext::new(entity_registry, device_manager, service_registry)
    }

    #[tokio::test]
    async fn domain_aware_register_entity_ok() {
        let ctx = make_ctx();
        let (handle, reg) = ctx
            .domain_aware_register_entity(
                "sensor",
                "temp_01",
                "Temperature",
                &["state".into()],
                Some("temperature"),
            )
            .unwrap();
        assert_ne!(handle, INVALID_HANDLE);
        assert_eq!(reg.platform, "sensor");
        assert_eq!(reg.unique_id, "temp_01");
        assert_eq!(reg.descriptor.domain_id, "sensor");
        assert_eq!(reg.descriptor.name, "Temperature");
        assert_eq!(reg.descriptor.device_class.as_deref(), Some("temperature"));
        assert!(matches!(reg.initial_state, EntityState::Sensor { .. }));
    }

    #[tokio::test]
    async fn domain_aware_register_entity_unknown_domain() {
        let ctx = make_ctx();
        let err = ctx
            .domain_aware_register_entity("nonexistent", "x", "X", &[], None)
            .unwrap_err();
        assert!(matches!(err, LoweringError::UnknownDomain { .. }));
    }

    #[tokio::test]
    async fn domain_aware_register_entity_bad_device_class() {
        let ctx = make_ctx();
        let err = ctx
            .domain_aware_register_entity(
                "sensor",
                "x",
                "X",
                &["state".into()],
                Some("nonexistent"),
            )
            .unwrap_err();
        assert!(matches!(err, LoweringError::InvalidDeviceClass { .. }));
    }

    #[tokio::test]
    async fn domain_aware_register_entity_missing_feature() {
        let ctx = make_ctx();
        let err = ctx
            .domain_aware_register_entity("sensor", "x", "X", &[], None)
            .unwrap_err();
        assert!(matches!(err, LoweringError::InvalidFeatures(_)));
    }

    #[tokio::test]
    async fn domain_aware_register_entity_handle_is_resolvable() {
        let ctx = make_ctx();
        let (handle, _) = ctx
            .domain_aware_register_entity(
                "switch",
                "relay_1",
                "Relay",
                &["state".into(), "toggle".into()],
                None,
            )
            .unwrap();
        let entry = ctx.entity_reg.get(handle, Rights::READ).unwrap();
        assert_eq!(entry.value.platform, "switch");
        assert_eq!(entry.value.unique_id, "relay_1");
    }

    #[tokio::test]
    async fn domain_lowering_field_accessible() {
        let ctx = make_ctx();
        assert!(ctx.domain_lowering.spec_registry().get("sensor").is_some());
    }
}
