//! Service registration bridge.
//!
//! Called by the host when a WASM guest invokes `register-service`.

use crate::capability::{Handle, ObjectRegistry, Rights};
use crate::context::ServiceHandle;

/// # Internal ABI
///
/// Register a custom service for this integration.
///
/// Returns a capability handle that can be used to look up or unregister the
/// service later.  `schema` is JSON-encoded service data schema bytes.
///
/// Prefer domain-aware service registration via [`DomainLoweringLayer::lower_register_service`]
/// for validated registration. This function is the raw bridge used by the WIT import layer.
pub fn register_service(
    service_reg: &ObjectRegistry<ServiceHandle>,
    domain: &str,
    name: &str,
    _schema: Vec<u8>,
) -> Handle {
    service_reg.alloc(
        ServiceHandle {
            domain: domain.to_owned(),
            name: name.to_owned(),
        },
        Rights::INVOKE,
    )
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::{CapError, INVALID_HANDLE};

    fn reg() -> ObjectRegistry<ServiceHandle> {
        ObjectRegistry::new()
    }

    #[test]
    fn register_service_returns_valid_handle() {
        let r = reg();
        let h = register_service(&r, "my_domain", "flash_leds", vec![]);
        assert_ne!(h, INVALID_HANDLE);
        let entry = r.get_any(h).unwrap();
        assert_eq!(entry.value.domain, "my_domain");
        assert_eq!(entry.value.name, "flash_leds");
    }

    #[test]
    fn register_two_services_different_handles() {
        let r = reg();
        let h1 = register_service(&r, "domain_a", "svc_1", vec![]);
        let h2 = register_service(&r, "domain_a", "svc_2", vec![]);
        assert_ne!(h1, h2);
    }

    #[test]
    fn service_handle_invalid_after_revoke() {
        let r = reg();
        let h = register_service(&r, "dom", "svc", vec![]);
        r.revoke(h).unwrap();
        assert_eq!(r.get(h, Rights::INVOKE).unwrap_err(), CapError::Revoked);
    }
}
