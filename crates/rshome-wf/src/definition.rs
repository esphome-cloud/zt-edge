use serde::{Deserialize, Serialize};
use thiserror::Error;

// ── Trigger ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TriggerDef {
    Manual,
    EntityStateChange {
        entity_id: String,
        from: Option<String>,
        to: Option<String>,
    },
    /// Stored, not evaluated in Phase 5.
    TimePattern {
        cron: String,
    },
    /// Stored, not evaluated in Phase 5.
    Webhook {
        path: String,
    },
}

impl TriggerDef {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::EntityStateChange { .. } => "entity_state_change",
            Self::TimePattern { .. } => "time_pattern",
            Self::Webhook { .. } => "webhook",
        }
    }
}

// ── Service call target ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ServiceCallTarget {
    All,
    EntityId { id: String },
    Domain { domain: String },
}

// ── Step ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StepDef {
    /// Evaluate `expression` against entity state. Fails the step when false.
    ///
    /// `expression` format: `"field op value"`, e.g. `"is_on == true"`.
    /// `entity_id` is required for field lookups; without it only literal
    /// `"true"` / `"false"` expressions are accepted.
    Condition {
        expression: String,
        entity_id: Option<String>,
    },
    ServiceCall {
        domain: String,
        service: String,
        target: ServiceCallTarget,
        data: serde_json::Value,
    },
    Delay {
        duration_ms: u64,
    },
    SetEntityState {
        entity_id: String,
        state: serde_json::Value,
    },
    Log {
        message: String,
    },
}

// ── State machine ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TransitionDef {
    pub event: String,
    /// Simple condition expression. Only `"true"` / `"false"` literals are
    /// evaluated in Phase 5; entity-backed expressions require Phase 6.
    pub guard: Option<String>,
    pub target_state: String,
    pub actions: Vec<StepDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StateDef {
    pub name: String,
    pub entry_actions: Vec<StepDef>,
    pub exit_actions: Vec<StepDef>,
    pub transitions: Vec<TransitionDef>,
}

// ── Workflow mode ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum WorkflowMode {
    Pipeline { steps: Vec<StepDef> },
    RunToCompletion { states: Vec<StateDef> },
}

impl WorkflowMode {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Pipeline { .. } => "pipeline",
            Self::RunToCompletion { .. } => "run_to_completion",
        }
    }
}

// ── Workflow definition ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    pub workflow_id: String,
    pub name: String,
    pub description: Option<String>,
    pub trigger: TriggerDef,
    pub mode: WorkflowMode,
}

// ── List / status types ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowInfo {
    pub workflow_id: String,
    pub name: String,
    pub trigger_type: String,
    pub mode_type: String,
}

impl WorkflowInfo {
    pub fn from_definition(def: &WorkflowDefinition) -> Self {
        Self {
            workflow_id: def.workflow_id.clone(),
            name: def.name.clone(),
            trigger_type: def.trigger.type_name().to_string(),
            mode_type: def.mode.type_name().to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRunInfo {
    pub run_id: String,
    pub workflow_id: String,
    pub status: RunStatus,
    pub error: Option<String>,
}

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum WfError {
    #[error("workflow not found: {0}")]
    WorkflowNotFound(String),
    #[error("run not found: {0}")]
    RunNotFound(String),
    #[error("workflow already has active runs")]
    ActiveRunsExist,
    #[error("condition failed: {0}")]
    ConditionFailed(String),
    #[error("invalid expression: {0}")]
    InvalidExpression(String),
    #[error("entity not found: {0}")]
    EntityNotFound(String),
    #[error("state machine error: {0}")]
    StateMachineError(String),
    #[error("actor error: {0}")]
    ActorError(String),
    #[error("service error: {0}")]
    ServiceError(String),
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pipeline_def(id: &str) -> WorkflowDefinition {
        WorkflowDefinition {
            workflow_id: id.to_string(),
            name: "Test Pipeline".to_string(),
            description: Some("desc".to_string()),
            trigger: TriggerDef::Manual,
            mode: WorkflowMode::Pipeline {
                steps: vec![
                    StepDef::Log {
                        message: "hello".to_string(),
                    },
                    StepDef::Delay { duration_ms: 100 },
                ],
            },
        }
    }

    fn sample_rtc_def(id: &str) -> WorkflowDefinition {
        WorkflowDefinition {
            workflow_id: id.to_string(),
            name: "Test RTC".to_string(),
            description: None,
            trigger: TriggerDef::EntityStateChange {
                entity_id: "sensor.temp".to_string(),
                from: None,
                to: Some("on".to_string()),
            },
            mode: WorkflowMode::RunToCompletion {
                states: vec![
                    StateDef {
                        name: "idle".to_string(),
                        entry_actions: vec![StepDef::Log {
                            message: "entering idle".to_string(),
                        }],
                        exit_actions: vec![],
                        transitions: vec![TransitionDef {
                            event: "start".to_string(),
                            guard: None,
                            target_state: "running".to_string(),
                            actions: vec![],
                        }],
                    },
                    StateDef {
                        name: "running".to_string(),
                        entry_actions: vec![],
                        exit_actions: vec![StepDef::Log {
                            message: "exiting running".to_string(),
                        }],
                        transitions: vec![],
                    },
                ],
            },
        }
    }

    #[test]
    fn test_pipeline_def_serde_roundtrip() {
        let original = sample_pipeline_def("wf-1");
        let json = serde_json::to_string(&original).unwrap();
        let restored: WorkflowDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(original.workflow_id, restored.workflow_id);
        assert_eq!(original.name, restored.name);
        assert_eq!(original.trigger, restored.trigger);
        assert_eq!(original.mode, restored.mode);
    }

    #[test]
    fn test_rtc_def_serde_roundtrip() {
        let original = sample_rtc_def("wf-2");
        let json = serde_json::to_string(&original).unwrap();
        let restored: WorkflowDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(original.mode, restored.mode);
    }

    #[test]
    fn test_trigger_def_type_names() {
        assert_eq!(TriggerDef::Manual.type_name(), "manual");
        assert_eq!(
            TriggerDef::EntityStateChange {
                entity_id: "x".into(),
                from: None,
                to: None
            }
            .type_name(),
            "entity_state_change"
        );
        assert_eq!(
            TriggerDef::TimePattern {
                cron: "* * * * *".into()
            }
            .type_name(),
            "time_pattern"
        );
        assert_eq!(
            TriggerDef::Webhook {
                path: "/hook".into()
            }
            .type_name(),
            "webhook"
        );
    }

    #[test]
    fn test_workflow_info_from_pipeline() {
        let def = sample_pipeline_def("wf-3");
        let info = WorkflowInfo::from_definition(&def);
        assert_eq!(info.trigger_type, "manual");
        assert_eq!(info.mode_type, "pipeline");
    }

    #[test]
    fn test_workflow_info_from_rtc() {
        let def = sample_rtc_def("wf-4");
        let info = WorkflowInfo::from_definition(&def);
        assert_eq!(info.mode_type, "run_to_completion");
    }

    #[test]
    fn test_run_status_serde() {
        let statuses = [RunStatus::Running, RunStatus::Completed, RunStatus::Failed];
        for s in &statuses {
            let json = serde_json::to_string(s).unwrap();
            let back: RunStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(s, &back);
        }
    }
}
