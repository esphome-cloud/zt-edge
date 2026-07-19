use rmcp::ErrorData as McpError;

pub fn json_error(e: serde_json::Error) -> McpError {
    McpError::internal_error(format!("JSON serialization error: {e}"), None)
}

pub fn actor_error(e: rshome_actor::ActorError) -> McpError {
    McpError::internal_error(
        format!("actor error: {e}"),
        Some(serde_json::json!({ "category": "dispatch_failure" })),
    )
}

/// The caller supplied a malformed or out-of-range parameter value.
pub fn bad_input(msg: impl std::fmt::Display) -> McpError {
    McpError::invalid_params(
        msg.to_string(),
        Some(serde_json::json!({ "category": "bad_input" })),
    )
}

/// The referenced entity / device / workflow / integration does not exist.
pub fn unknown_object(msg: impl std::fmt::Display) -> McpError {
    McpError::invalid_params(
        msg.to_string(),
        Some(serde_json::json!({ "category": "unknown_object" })),
    )
}

/// An optional actor dependency (WasmHost, WorkflowEngine, DeviceLink, …) is not attached.
pub fn unavailable(msg: impl std::fmt::Display) -> McpError {
    McpError::internal_error(
        msg.to_string(),
        Some(serde_json::json!({ "category": "unavailable_dependency" })),
    )
}
