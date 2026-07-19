//! Diagnostics collection.
//!
//! Integrations expose diagnostic key-value pairs (e.g. `"status" → "connected"`,
//! `"uptime_s" → "3600"`) that are surfaced in the MCP tool `ha.integrations.list`.

use std::sync::Arc;

use crate::guest::GuestFunctions;

/// Collect diagnostic information from the WASM guest.
pub fn collect_diagnostics(guest: &Arc<dyn GuestFunctions>) -> Vec<(String, String)> {
    guest.get_diagnostics()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guest::MockGuest;

    #[test]
    fn collect_diagnostics_returns_guest_values() {
        let mut mock = MockGuest::default();
        mock.diagnostics = vec![
            ("status".into(), "connected".into()),
            ("uptime_s".into(), "1234".into()),
        ];
        let guest: Arc<dyn GuestFunctions> = Arc::new(mock);
        let diags = collect_diagnostics(&guest);
        assert_eq!(diags.len(), 2);
        assert_eq!(diags[0], ("status".into(), "connected".into()));
        assert_eq!(diags[1], ("uptime_s".into(), "1234".into()));
    }

    #[test]
    fn collect_diagnostics_empty_case() {
        let mut mock = MockGuest::default();
        mock.diagnostics = vec![];
        let guest: Arc<dyn GuestFunctions> = Arc::new(mock);
        let diags = collect_diagnostics(&guest);
        assert!(diags.is_empty());
    }
}
