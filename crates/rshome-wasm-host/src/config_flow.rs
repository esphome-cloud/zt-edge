//! Config-flow step dispatch.
//!
//! A "config flow" is a multi-step UI wizard that runs when a user first sets
//! up an integration (analogous to Home Assistant's config entries).  Each
//! step returns a JSON schema describing the next form to render.

use std::sync::Arc;

use crate::guest::GuestFunctions;

/// Run one step of the integration's config flow.
///
/// `step_id` identifies the current step (e.g. `"user"`, `"confirm"`).
/// `user_input` carries form field values submitted by the user; `None` on the
/// first call to retrieve the initial form schema.
///
/// Returns JSON-encoded step result bytes on success.
pub fn run_config_flow_step(
    guest: &Arc<dyn GuestFunctions>,
    step_id: &str,
    user_input: Option<Vec<(String, String)>>,
) -> Result<Vec<u8>, String> {
    guest.config_flow_step(step_id, user_input)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guest::MockGuest;

    fn mock_arc() -> (Arc<MockGuest>, Arc<dyn GuestFunctions>) {
        let mock = Arc::new(MockGuest::default());
        let guest: Arc<dyn GuestFunctions> = mock.clone();
        (mock, guest)
    }

    #[test]
    fn config_flow_step_with_user_input() {
        let (mock, guest) = mock_arc();
        let input = vec![("host".into(), "192.168.1.10".into())];
        let result = run_config_flow_step(&guest, "user", Some(input.clone()));
        assert!(result.is_ok());

        let calls = mock.calls.lock();
        assert_eq!(calls.len(), 1);
        match &calls[0] {
            crate::guest::GuestCall::ConfigFlowStep {
                step_id,
                user_input,
            } => {
                assert_eq!(step_id, "user");
                assert_eq!(user_input.as_ref().unwrap(), &input);
            }
            other => panic!("unexpected call: {other:?}"),
        }
    }

    #[test]
    fn config_flow_step_no_input_gets_form_schema() {
        let (mock, guest) = mock_arc();
        let result = run_config_flow_step(&guest, "init", None);
        assert!(result.is_ok());
        let payload = result.unwrap();
        assert!(!payload.is_empty());

        let calls = mock.calls.lock();
        assert_eq!(calls.len(), 1);
        match &calls[0] {
            crate::guest::GuestCall::ConfigFlowStep { user_input, .. } => {
                assert!(user_input.is_none());
            }
            other => panic!("unexpected call: {other:?}"),
        }
    }

    #[test]
    fn config_flow_step_error_propagated() {
        let mut mock = MockGuest::default();
        mock.config_flow_error = Some("network timeout".into());
        let mock = Arc::new(mock);
        let guest: Arc<dyn GuestFunctions> = mock.clone();

        let err = run_config_flow_step(&guest, "user", None).unwrap_err();
        assert_eq!(err, "network timeout");
    }
}
