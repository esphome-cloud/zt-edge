//! Repair flow dispatch.
//!
//! Repairs are multi-step wizards that guide users through fixing a broken
//! integration (analogous to Home Assistant's repair flows).

use std::sync::Arc;

use crate::guest::GuestFunctions;

/// Drive one step of a repair flow.
///
/// `repair_id` identifies the active repair issue; `step_id` is the current
/// step within that repair; `user_input` carries form values.
///
/// Returns JSON-encoded step result bytes on success.
pub fn run_repair_step(
    guest: &Arc<dyn GuestFunctions>,
    repair_id: &str,
    step_id: &str,
    user_input: Option<Vec<(String, String)>>,
) -> Result<Vec<u8>, String> {
    guest.repair_step(repair_id, step_id, user_input)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guest::MockGuest;

    #[test]
    fn repair_step_success() {
        let mock = Arc::new(MockGuest::default());
        let guest: Arc<dyn GuestFunctions> = mock.clone();
        let result = run_repair_step(&guest, "broken_auth", "reenter_credentials", None);
        assert!(result.is_ok());

        let calls = mock.calls.lock();
        assert_eq!(calls.len(), 1);
        match &calls[0] {
            crate::guest::GuestCall::RepairStep {
                repair_id,
                step_id,
                user_input,
            } => {
                assert_eq!(repair_id, "broken_auth");
                assert_eq!(step_id, "reenter_credentials");
                assert!(user_input.is_none());
            }
            other => panic!("unexpected call: {other:?}"),
        }
    }

    #[test]
    fn repair_step_error_propagated() {
        let mut mock = MockGuest::default();
        mock.repair_error = Some("repair not supported".into());
        let guest: Arc<dyn GuestFunctions> = Arc::new(mock);
        let err = run_repair_step(&guest, "r1", "s1", None).unwrap_err();
        assert_eq!(err, "repair not supported");
    }
}
