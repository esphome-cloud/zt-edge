//! Discovery subscription bridge.
//!
//! Called by the host when a WASM guest invokes `subscribe-discovery`.
//! Integrations subscribe to a discovery protocol (e.g. `"mdns"`, `"ssdp"`)
//! and receive events when new devices are found on the network.

use crate::capability::{CapError, Handle, ObjectRegistry, Rights};
use crate::context::DiscoveryHandle;

/// Subscribe to device discovery for the given protocol.
///
/// Returns a capability handle.  Discovery events are delivered out-of-band
/// (Phase 6).
pub fn subscribe_discovery(
    discovery_reg: &ObjectRegistry<DiscoveryHandle>,
    protocol: &str,
) -> Handle {
    discovery_reg.alloc(
        DiscoveryHandle {
            protocol: protocol.to_owned(),
        },
        Rights::READ,
    )
}

/// Unsubscribe by revoking the discovery handle.
pub fn unsubscribe_discovery(
    discovery_reg: &ObjectRegistry<DiscoveryHandle>,
    handle: Handle,
) -> Result<(), CapError> {
    discovery_reg.revoke(handle)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::INVALID_HANDLE;

    fn reg() -> ObjectRegistry<DiscoveryHandle> {
        ObjectRegistry::new()
    }

    #[test]
    fn subscribe_returns_valid_handle() {
        let r = reg();
        let h = subscribe_discovery(&r, "mdns");
        assert_ne!(h, INVALID_HANDLE);
        let entry = r.get_any(h).unwrap();
        assert_eq!(entry.value.protocol, "mdns");
    }

    #[test]
    fn unsubscribe_invalidates_handle() {
        let r = reg();
        let h = subscribe_discovery(&r, "ssdp");
        unsubscribe_discovery(&r, h).unwrap();
        assert_eq!(r.get(h, Rights::READ).unwrap_err(), CapError::Revoked);
    }
}
