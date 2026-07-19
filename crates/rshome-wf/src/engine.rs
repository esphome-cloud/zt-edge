#![allow(clippy::module_name_repetitions)]

use std::collections::HashMap;

use rshome_actor::{Actor, ActorContext, ActorRef};
use rshome_entity::EntityRegistry;
use rshome_svc::ServiceMsg;
use tokio::sync::oneshot;

use crate::definition::{
    RunStatus, WfError, WorkflowDefinition, WorkflowInfo, WorkflowMode, WorkflowRunInfo,
};
use crate::pipeline::PipelineRunner;
use crate::rtc::{RtcHandle, RunToCompletionRunner};

// ── Messages ──────────────────────────────────────────────────────────────────

pub enum WorkflowEngineMsg {
    Create {
        definition: WorkflowDefinition,
        reply: oneshot::Sender<Result<String, WfError>>,
    },
    List(oneshot::Sender<Vec<WorkflowInfo>>),
    Get {
        workflow_id: String,
        reply: oneshot::Sender<Result<WorkflowDefinition, WfError>>,
    },
    Delete {
        workflow_id: String,
        reply: oneshot::Sender<Result<(), WfError>>,
    },
    Run {
        workflow_id: String,
        input_data: Option<serde_json::Value>,
        reply: oneshot::Sender<Result<String, WfError>>,
    },
    GetRunStatus {
        run_id: String,
        reply: oneshot::Sender<Result<WorkflowRunInfo, WfError>>,
    },
    /// Internal: spawned pipeline tasks report completion back to the actor.
    RunCompleted {
        run_id: String,
        status: RunStatus,
        error: Option<String>,
    },
    /// Dispatch an event to an active run-to-completion state machine.
    DispatchEvent {
        workflow_id: String,
        event_type: String,
        data: serde_json::Value,
    },
    Stop,
}

// ── Actor ─────────────────────────────────────────────────────────────────────

pub struct WorkflowEngineActor {
    workflows: HashMap<String, WorkflowDefinition>,
    runs: HashMap<String, WorkflowRunInfo>,
    /// workflow_id → (run_id, live RTC handle)
    rtc_runs: HashMap<String, (String, RtcHandle)>,
    entity_registry: EntityRegistry,
    service_registry: ActorRef<ServiceMsg>,
}

impl WorkflowEngineActor {
    pub fn new(entity_registry: EntityRegistry, service_registry: ActorRef<ServiceMsg>) -> Self {
        Self {
            workflows: HashMap::new(),
            runs: HashMap::new(),
            rtc_runs: HashMap::new(),
            entity_registry,
            service_registry,
        }
    }
}

#[async_trait::async_trait]
impl Actor for WorkflowEngineActor {
    type Msg = WorkflowEngineMsg;

    async fn handle(&mut self, msg: WorkflowEngineMsg, ctx: &mut ActorContext<WorkflowEngineMsg>) {
        match msg {
            WorkflowEngineMsg::Create {
                mut definition,
                reply,
            } => {
                let workflow_id = uuid::Uuid::new_v4().to_string();
                definition.workflow_id = workflow_id.clone();
                self.workflows.insert(workflow_id.clone(), definition);
                let _ = reply.send(Ok(workflow_id));
            }

            WorkflowEngineMsg::List(reply) => {
                let list = self
                    .workflows
                    .values()
                    .map(WorkflowInfo::from_definition)
                    .collect();
                let _ = reply.send(list);
            }

            WorkflowEngineMsg::Get { workflow_id, reply } => {
                let result = self
                    .workflows
                    .get(&workflow_id)
                    .cloned()
                    .ok_or_else(|| WfError::WorkflowNotFound(workflow_id));
                let _ = reply.send(result);
            }

            WorkflowEngineMsg::Delete { workflow_id, reply } => {
                if !self.workflows.contains_key(&workflow_id) {
                    let _ = reply.send(Err(WfError::WorkflowNotFound(workflow_id)));
                    return;
                }
                // Reject if there are active (Running) runs for this workflow
                let has_active = self
                    .runs
                    .values()
                    .any(|r| r.workflow_id == workflow_id && r.status == RunStatus::Running);
                if has_active {
                    let _ = reply.send(Err(WfError::ActiveRunsExist));
                    return;
                }
                self.workflows.remove(&workflow_id);
                let _ = reply.send(Ok(()));
            }

            WorkflowEngineMsg::Run {
                workflow_id,
                input_data,
                reply,
            } => {
                let workflow = match self.workflows.get(&workflow_id).cloned() {
                    Some(w) => w,
                    None => {
                        let _ = reply.send(Err(WfError::WorkflowNotFound(workflow_id)));
                        return;
                    }
                };

                let run_id = uuid::Uuid::new_v4().to_string();
                self.runs.insert(
                    run_id.clone(),
                    WorkflowRunInfo {
                        run_id: run_id.clone(),
                        workflow_id: workflow_id.clone(),
                        status: RunStatus::Running,
                        error: None,
                    },
                );

                match workflow.mode {
                    WorkflowMode::Pipeline { steps } => {
                        let runner = PipelineRunner::new(
                            self.entity_registry.clone(),
                            self.service_registry.clone(),
                        );
                        let self_ref = ctx.self_ref().clone();
                        let run_id_task = run_id.clone();
                        tokio::spawn(async move {
                            let result = runner.execute(&steps, input_data).await;
                            let (status, error) = match result {
                                Ok(_) => (RunStatus::Completed, None),
                                Err(e) => (RunStatus::Failed, Some(e.to_string())),
                            };
                            let _ = self_ref.send(WorkflowEngineMsg::RunCompleted {
                                run_id: run_id_task,
                                status,
                                error,
                            });
                        });
                    }

                    WorkflowMode::RunToCompletion { states } => {
                        let pipeline = PipelineRunner::new(
                            self.entity_registry.clone(),
                            self.service_registry.clone(),
                        );
                        let runner = RunToCompletionRunner::new(pipeline);
                        let initial = states.first().map(|s| s.name.clone()).unwrap_or_default();
                        match runner.run(&states, &initial).await {
                            Ok(handle) => {
                                self.rtc_runs
                                    .insert(workflow_id.clone(), (run_id.clone(), handle));
                            }
                            Err(e) => {
                                if let Some(r) = self.runs.get_mut(&run_id) {
                                    r.status = RunStatus::Failed;
                                    r.error = Some(e.to_string());
                                }
                            }
                        }
                    }
                }

                let _ = reply.send(Ok(run_id));
            }

            WorkflowEngineMsg::GetRunStatus { run_id, reply } => {
                let result = self
                    .runs
                    .get(&run_id)
                    .cloned()
                    .ok_or_else(|| WfError::RunNotFound(run_id));
                let _ = reply.send(result);
            }

            WorkflowEngineMsg::RunCompleted {
                run_id,
                status,
                error,
            } => {
                if let Some(run) = self.runs.get_mut(&run_id) {
                    run.status = status;
                    run.error = error;
                }
            }

            WorkflowEngineMsg::DispatchEvent {
                workflow_id,
                event_type,
                data,
            } => {
                // Remove the handle, dispatch, then re-insert if not terminal
                if let Some((run_id, mut handle)) = self.rtc_runs.remove(&workflow_id) {
                    match handle.dispatch(&event_type, data).await {
                        Ok(new_state) => {
                            if handle.is_in_terminal_state(&new_state) {
                                if let Some(r) = self.runs.get_mut(&run_id) {
                                    r.status = RunStatus::Completed;
                                }
                            } else {
                                self.rtc_runs.insert(workflow_id, (run_id, handle));
                            }
                        }
                        Err(e) => {
                            if let Some(r) = self.runs.get_mut(&run_id) {
                                r.status = RunStatus::Failed;
                                r.error = Some(e.to_string());
                            }
                        }
                    }
                }
            }

            WorkflowEngineMsg::Stop => {
                ctx.stop();
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rshome_actor::ActorSystem;
    use rshome_entity::EntityRegistry;
    use rshome_svc::ServiceRegistryActor;

    use super::*;
    use crate::definition::{StateDef, StepDef, TransitionDef, TriggerDef, WorkflowMode};

    fn make_engine(sys: &ActorSystem) -> ActorRef<WorkflowEngineMsg> {
        let registry = EntityRegistry::default();
        let svc = sys.spawn(ServiceRegistryActor::new(registry.clone(), None));
        sys.spawn(WorkflowEngineActor::new(registry, svc))
    }

    fn pipeline_def(name: &str, steps: Vec<StepDef>) -> WorkflowDefinition {
        WorkflowDefinition {
            workflow_id: String::new(), // assigned by engine on Create
            name: name.to_string(),
            description: None,
            trigger: TriggerDef::Manual,
            mode: WorkflowMode::Pipeline { steps },
        }
    }

    fn rtc_def_ab() -> WorkflowDefinition {
        WorkflowDefinition {
            workflow_id: String::new(),
            name: "rtc-ab".to_string(),
            description: None,
            trigger: TriggerDef::Manual,
            mode: WorkflowMode::RunToCompletion {
                states: vec![
                    StateDef {
                        name: "A".to_string(),
                        entry_actions: vec![],
                        exit_actions: vec![],
                        transitions: vec![TransitionDef {
                            event: "go".to_string(),
                            guard: None,
                            target_state: "B".to_string(),
                            actions: vec![],
                        }],
                    },
                    StateDef {
                        name: "B".to_string(),
                        entry_actions: vec![],
                        exit_actions: vec![],
                        transitions: vec![], // terminal
                    },
                ],
            },
        }
    }

    #[tokio::test]
    async fn test_create_returns_id() {
        let sys = ActorSystem::new();
        let engine = make_engine(&sys);
        let id = engine
            .ask(|tx| WorkflowEngineMsg::Create {
                definition: pipeline_def("wf", vec![]),
                reply: tx,
            })
            .await
            .unwrap()
            .unwrap();
        assert!(!id.is_empty());
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_list_empty() {
        let sys = ActorSystem::new();
        let engine = make_engine(&sys);
        let list = engine.ask(WorkflowEngineMsg::List).await.unwrap();
        assert!(list.is_empty());
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_list_after_create() {
        let sys = ActorSystem::new();
        let engine = make_engine(&sys);
        engine
            .ask(|tx| WorkflowEngineMsg::Create {
                definition: pipeline_def("wf1", vec![]),
                reply: tx,
            })
            .await
            .unwrap()
            .unwrap();
        engine
            .ask(|tx| WorkflowEngineMsg::Create {
                definition: pipeline_def("wf2", vec![]),
                reply: tx,
            })
            .await
            .unwrap()
            .unwrap();
        let list = engine.ask(WorkflowEngineMsg::List).await.unwrap();
        assert_eq!(list.len(), 2);
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_get_ok() {
        let sys = ActorSystem::new();
        let engine = make_engine(&sys);
        let id = engine
            .ask(|tx| WorkflowEngineMsg::Create {
                definition: pipeline_def("my-wf", vec![]),
                reply: tx,
            })
            .await
            .unwrap()
            .unwrap();
        let def = engine
            .ask(|tx| WorkflowEngineMsg::Get {
                workflow_id: id.clone(),
                reply: tx,
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(def.name, "my-wf");
        assert_eq!(def.workflow_id, id);
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_get_not_found() {
        let sys = ActorSystem::new();
        let engine = make_engine(&sys);
        let err = engine
            .ask(|tx| WorkflowEngineMsg::Get {
                workflow_id: "missing".to_string(),
                reply: tx,
            })
            .await
            .unwrap()
            .unwrap_err();
        assert!(matches!(err, WfError::WorkflowNotFound(_)));
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_delete_ok() {
        let sys = ActorSystem::new();
        let engine = make_engine(&sys);
        let id = engine
            .ask(|tx| WorkflowEngineMsg::Create {
                definition: pipeline_def("wf", vec![]),
                reply: tx,
            })
            .await
            .unwrap()
            .unwrap();
        engine
            .ask(|tx| WorkflowEngineMsg::Delete {
                workflow_id: id.clone(),
                reply: tx,
            })
            .await
            .unwrap()
            .unwrap();
        let list = engine.ask(WorkflowEngineMsg::List).await.unwrap();
        assert!(list.is_empty());
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_delete_not_found() {
        let sys = ActorSystem::new();
        let engine = make_engine(&sys);
        let err = engine
            .ask(|tx| WorkflowEngineMsg::Delete {
                workflow_id: "missing".to_string(),
                reply: tx,
            })
            .await
            .unwrap()
            .unwrap_err();
        assert!(matches!(err, WfError::WorkflowNotFound(_)));
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_run_pipeline_returns_run_id() {
        let sys = ActorSystem::new();
        let engine = make_engine(&sys);
        let wf_id = engine
            .ask(|tx| WorkflowEngineMsg::Create {
                definition: pipeline_def("wf", vec![]),
                reply: tx,
            })
            .await
            .unwrap()
            .unwrap();
        let run_id = engine
            .ask(|tx| WorkflowEngineMsg::Run {
                workflow_id: wf_id,
                input_data: None,
                reply: tx,
            })
            .await
            .unwrap()
            .unwrap();
        assert!(!run_id.is_empty());
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_run_status_completes() {
        let sys = ActorSystem::new();
        let engine = make_engine(&sys);
        let wf_id = engine
            .ask(|tx| WorkflowEngineMsg::Create {
                definition: pipeline_def(
                    "wf",
                    vec![StepDef::Log {
                        message: "ok".to_string(),
                    }],
                ),
                reply: tx,
            })
            .await
            .unwrap()
            .unwrap();
        let run_id = engine
            .ask(|tx| WorkflowEngineMsg::Run {
                workflow_id: wf_id,
                input_data: None,
                reply: tx,
            })
            .await
            .unwrap()
            .unwrap();

        // Allow the spawned pipeline task to complete and send RunCompleted back
        tokio::time::sleep(Duration::from_millis(50)).await;

        let info = engine
            .ask(|tx| WorkflowEngineMsg::GetRunStatus { run_id, reply: tx })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(info.status, RunStatus::Completed);
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_run_failure_captured() {
        let sys = ActorSystem::new();
        let engine = make_engine(&sys);
        // A condition with no entity_id that is neither "true" nor "false"
        let steps = vec![StepDef::Condition {
            expression: "field == value".to_string(),
            entity_id: None,
        }];
        let wf_id = engine
            .ask(|tx| WorkflowEngineMsg::Create {
                definition: pipeline_def("fail-wf", steps),
                reply: tx,
            })
            .await
            .unwrap()
            .unwrap();
        let run_id = engine
            .ask(|tx| WorkflowEngineMsg::Run {
                workflow_id: wf_id,
                input_data: None,
                reply: tx,
            })
            .await
            .unwrap()
            .unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;

        let info = engine
            .ask(|tx| WorkflowEngineMsg::GetRunStatus { run_id, reply: tx })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(info.status, RunStatus::Failed);
        assert!(info.error.is_some());
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_run_unknown_workflow() {
        let sys = ActorSystem::new();
        let engine = make_engine(&sys);
        let err = engine
            .ask(|tx| WorkflowEngineMsg::Run {
                workflow_id: "missing".to_string(),
                input_data: None,
                reply: tx,
            })
            .await
            .unwrap()
            .unwrap_err();
        assert!(matches!(err, WfError::WorkflowNotFound(_)));
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_delete_with_active_run_blocked() {
        let sys = ActorSystem::new();
        let engine = make_engine(&sys);
        // Workflow with a delay so the run stays in Running state
        let steps = vec![StepDef::Delay { duration_ms: 5_000 }];
        let wf_id = engine
            .ask(|tx| WorkflowEngineMsg::Create {
                definition: pipeline_def("long-wf", steps),
                reply: tx,
            })
            .await
            .unwrap()
            .unwrap();
        // Start the run (it will be Running for 5 seconds)
        let _run_id = engine
            .ask(|tx| WorkflowEngineMsg::Run {
                workflow_id: wf_id.clone(),
                input_data: None,
                reply: tx,
            })
            .await
            .unwrap()
            .unwrap();

        // Immediately try to delete — should fail with ActiveRunsExist
        let err = engine
            .ask(|tx| WorkflowEngineMsg::Delete {
                workflow_id: wf_id.clone(),
                reply: tx,
            })
            .await
            .unwrap()
            .unwrap_err();
        assert!(matches!(err, WfError::ActiveRunsExist));
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_rtc_dispatch_event_to_terminal() {
        let sys = ActorSystem::new();
        let engine = make_engine(&sys);

        let wf_id = engine
            .ask(|tx| WorkflowEngineMsg::Create {
                definition: rtc_def_ab(),
                reply: tx,
            })
            .await
            .unwrap()
            .unwrap();

        let run_id = engine
            .ask(|tx| WorkflowEngineMsg::Run {
                workflow_id: wf_id.clone(),
                input_data: None,
                reply: tx,
            })
            .await
            .unwrap()
            .unwrap();

        // Run starts in state A (Running)
        let info = engine
            .ask(|tx| WorkflowEngineMsg::GetRunStatus {
                run_id: run_id.clone(),
                reply: tx,
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(info.status, RunStatus::Running);

        // Dispatch "go" → transitions A→B (terminal) → run becomes Completed
        engine
            .send(WorkflowEngineMsg::DispatchEvent {
                workflow_id: wf_id,
                event_type: "go".to_string(),
                data: serde_json::Value::Null,
            })
            .unwrap();

        // The GetRunStatus ask is queued AFTER DispatchEvent, so the engine
        // processes DispatchEvent first (FIFO mailbox).
        let info = engine
            .ask(|tx| WorkflowEngineMsg::GetRunStatus { run_id, reply: tx })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(info.status, RunStatus::Completed);
        sys.shutdown().await;
    }
}
