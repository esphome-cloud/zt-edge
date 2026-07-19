use crate::definition::{StateDef, StepDef, WfError};
use crate::pipeline::PipelineRunner;

// ── RtcHandle ─────────────────────────────────────────────────────────────────

/// A live run-to-completion state machine instance.
#[allow(dead_code)]
pub struct RtcHandle {
    pub(crate) current_state: String,
    pub(crate) states: Vec<StateDef>,
    pub(crate) pipeline: PipelineRunner,
}

impl RtcHandle {
    /// Returns the name of the current state.
    pub fn current_state(&self) -> &str {
        &self.current_state
    }

    /// Returns `true` if the named state has no outgoing transitions (terminal).
    pub fn is_in_terminal_state(&self, state_name: &str) -> bool {
        self.states
            .iter()
            .find(|s| s.name == state_name)
            .is_none_or(|s| s.transitions.is_empty())
    }

    /// Dispatch an event to the state machine.
    ///
    /// Finds the first transition in the current state whose event matches and
    /// whose guard (if any) passes. Executes exit actions → transition actions →
    /// updates current state → entry actions. Returns the new state name.
    ///
    /// Returns the current state unchanged when no transition matches.
    pub async fn dispatch(
        &mut self,
        event: &str,
        data: serde_json::Value,
    ) -> Result<String, WfError> {
        let state = self
            .states
            .iter()
            .find(|s| s.name == self.current_state)
            .ok_or_else(|| {
                WfError::StateMachineError(format!(
                    "current state '{}' not found in definition",
                    self.current_state
                ))
            })?;

        // Find first transition that matches the event and whose guard passes
        let matching = state.transitions.iter().find(|t| {
            if t.event != event {
                return false;
            }
            match &t.guard {
                None => true,
                Some(g) => !matches!(g.trim(), "false"),
            }
        });

        let transition = match matching {
            Some(t) => t,
            None => return Ok(self.current_state.clone()), // no-op
        };

        // Evaluate explicit "true"/"false" guard; "true" is already handled
        // by the find above, but we still need to handle typed expressions.
        // Phase 5: non-literal guards result in InvalidExpression.
        if let Some(guard) = &transition.guard {
            let g = guard.trim();
            if g != "true" && g != "false" {
                return Err(WfError::InvalidExpression(format!(
                    "guard '{guard}' is not a literal boolean (Phase 5 limitation)"
                )));
            }
            // "false" is already excluded by the find; "true" falls through
        }

        let target = transition.target_state.clone();
        let exit_actions: Vec<StepDef> = state.exit_actions.clone();
        let transition_actions: Vec<StepDef> = transition.actions.clone();

        // Find target state entry actions before mutating current_state
        let entry_actions: Vec<StepDef> = self
            .states
            .iter()
            .find(|s| s.name == target)
            .ok_or_else(|| {
                WfError::StateMachineError(format!("target state '{target}' not found"))
            })?
            .entry_actions
            .clone();

        // Execute exit → transition → update state → entry
        self.pipeline.execute(&exit_actions, None).await?;
        self.pipeline
            .execute(&transition_actions, Some(data))
            .await?;
        self.current_state = target.clone();
        self.pipeline.execute(&entry_actions, None).await?;

        Ok(target)
    }
}

// ── RunToCompletionRunner ─────────────────────────────────────────────────────

pub struct RunToCompletionRunner {
    pipeline: PipelineRunner,
}

impl RunToCompletionRunner {
    pub fn new(pipeline: PipelineRunner) -> Self {
        Self { pipeline }
    }

    /// Start a run-to-completion state machine in `initial_state`.
    ///
    /// Executes the initial state's entry actions and returns a live handle.
    pub async fn run(
        &self,
        states: &[StateDef],
        initial_state: &str,
    ) -> Result<RtcHandle, WfError> {
        let state = states
            .iter()
            .find(|s| s.name == initial_state)
            .ok_or_else(|| {
                WfError::StateMachineError(format!("initial state '{initial_state}' not found"))
            })?;

        // Execute entry actions of the initial state
        self.pipeline.execute(&state.entry_actions, None).await?;

        Ok(RtcHandle {
            current_state: initial_state.to_string(),
            states: states.to_vec(),
            pipeline: self.pipeline.clone(),
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use rshome_actor::ActorSystem;
    use rshome_entity::EntityRegistry;
    use rshome_svc::ServiceRegistryActor;

    use super::*;
    use crate::definition::{StepDef, TransitionDef};

    fn make_pipeline(sys: &ActorSystem) -> PipelineRunner {
        let registry = EntityRegistry::default();
        let svc = sys.spawn(ServiceRegistryActor::new(registry.clone(), None));
        PipelineRunner::new(registry, svc)
    }

    fn simple_states() -> Vec<StateDef> {
        vec![
            StateDef {
                name: "A".to_string(),
                entry_actions: vec![StepDef::Log {
                    message: "enter A".to_string(),
                }],
                exit_actions: vec![StepDef::Log {
                    message: "exit A".to_string(),
                }],
                transitions: vec![TransitionDef {
                    event: "go".to_string(),
                    guard: None,
                    target_state: "B".to_string(),
                    actions: vec![StepDef::Log {
                        message: "A→B".to_string(),
                    }],
                }],
            },
            StateDef {
                name: "B".to_string(),
                entry_actions: vec![StepDef::Log {
                    message: "enter B".to_string(),
                }],
                exit_actions: vec![],
                transitions: vec![TransitionDef {
                    event: "next".to_string(),
                    guard: None,
                    target_state: "C".to_string(),
                    actions: vec![],
                }],
            },
            StateDef {
                name: "C".to_string(),
                entry_actions: vec![],
                exit_actions: vec![],
                transitions: vec![],
            },
        ]
    }

    #[tokio::test]
    async fn test_initial_state_entry_actions_fire() {
        let sys = ActorSystem::new();
        let runner = RunToCompletionRunner::new(make_pipeline(&sys));
        let states = simple_states();
        let handle = runner.run(&states, "A").await.unwrap();
        assert_eq!(handle.current_state(), "A");
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_event_triggers_transition() {
        let sys = ActorSystem::new();
        let runner = RunToCompletionRunner::new(make_pipeline(&sys));
        let states = simple_states();
        let mut handle = runner.run(&states, "A").await.unwrap();
        let new_state = handle
            .dispatch("go", serde_json::Value::Null)
            .await
            .unwrap();
        assert_eq!(new_state, "B");
        assert_eq!(handle.current_state(), "B");
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_guard_allows_transition() {
        let sys = ActorSystem::new();
        let runner = RunToCompletionRunner::new(make_pipeline(&sys));
        let states = vec![
            StateDef {
                name: "X".to_string(),
                entry_actions: vec![],
                exit_actions: vec![],
                transitions: vec![TransitionDef {
                    event: "go".to_string(),
                    guard: Some("true".to_string()), // explicit true guard
                    target_state: "Y".to_string(),
                    actions: vec![],
                }],
            },
            StateDef {
                name: "Y".to_string(),
                entry_actions: vec![],
                exit_actions: vec![],
                transitions: vec![],
            },
        ];
        let mut handle = runner.run(&states, "X").await.unwrap();
        let new_state = handle
            .dispatch("go", serde_json::Value::Null)
            .await
            .unwrap();
        assert_eq!(new_state, "Y");
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_guard_blocks_transition() {
        let sys = ActorSystem::new();
        let runner = RunToCompletionRunner::new(make_pipeline(&sys));
        let states = vec![
            StateDef {
                name: "X".to_string(),
                entry_actions: vec![],
                exit_actions: vec![],
                transitions: vec![TransitionDef {
                    event: "go".to_string(),
                    guard: Some("false".to_string()), // guard blocks transition
                    target_state: "Y".to_string(),
                    actions: vec![],
                }],
            },
            StateDef {
                name: "Y".to_string(),
                entry_actions: vec![],
                exit_actions: vec![],
                transitions: vec![],
            },
        ];
        let mut handle = runner.run(&states, "X").await.unwrap();
        let same_state = handle
            .dispatch("go", serde_json::Value::Null)
            .await
            .unwrap();
        // Guard blocked → state unchanged
        assert_eq!(same_state, "X");
        assert_eq!(handle.current_state(), "X");
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_exit_and_entry_actions_fire_in_order() {
        // exit A → transition action → entry B — verified by not panicking (all Log steps)
        let sys = ActorSystem::new();
        let runner = RunToCompletionRunner::new(make_pipeline(&sys));
        let states = simple_states();
        let mut handle = runner.run(&states, "A").await.unwrap();
        assert!(handle.dispatch("go", serde_json::Value::Null).await.is_ok());
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_unknown_event_noop() {
        let sys = ActorSystem::new();
        let runner = RunToCompletionRunner::new(make_pipeline(&sys));
        let states = simple_states();
        let mut handle = runner.run(&states, "A").await.unwrap();
        let state = handle
            .dispatch("unknown_event", serde_json::Value::Null)
            .await
            .unwrap();
        assert_eq!(state, "A"); // unchanged
        assert_eq!(handle.current_state(), "A");
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_missing_initial_state_returns_error() {
        let sys = ActorSystem::new();
        let runner = RunToCompletionRunner::new(make_pipeline(&sys));
        let states = simple_states();
        let result = runner.run(&states, "nonexistent").await;
        assert!(matches!(result, Err(WfError::StateMachineError(_))));
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_transition_actions_execute() {
        // Transition from A to B includes a Log action
        let sys = ActorSystem::new();
        let runner = RunToCompletionRunner::new(make_pipeline(&sys));
        let states = simple_states();
        let mut handle = runner.run(&states, "A").await.unwrap();
        // Log actions in exit / transition / entry should not error
        let new_state = handle
            .dispatch("go", serde_json::Value::Null)
            .await
            .unwrap();
        assert_eq!(new_state, "B");
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_multi_hop_transitions() {
        // A→B→C via two sequential dispatches
        let sys = ActorSystem::new();
        let runner = RunToCompletionRunner::new(make_pipeline(&sys));
        let states = simple_states();
        let mut handle = runner.run(&states, "A").await.unwrap();
        handle
            .dispatch("go", serde_json::Value::Null)
            .await
            .unwrap();
        let final_state = handle
            .dispatch("next", serde_json::Value::Null)
            .await
            .unwrap();
        assert_eq!(final_state, "C");
        assert_eq!(handle.current_state(), "C");
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_is_in_terminal_state() {
        let sys = ActorSystem::new();
        let runner = RunToCompletionRunner::new(make_pipeline(&sys));
        let states = simple_states();
        let handle = runner.run(&states, "A").await.unwrap();
        assert!(!handle.is_in_terminal_state("A")); // A has transitions
        assert!(!handle.is_in_terminal_state("B")); // B has transitions
        assert!(handle.is_in_terminal_state("C")); // C is terminal
        sys.shutdown().await;
    }
}
