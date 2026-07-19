use std::collections::BTreeMap;
use std::time::Duration;

use rshome_actor::ActorRef;
use rshome_entity::{EntityId, EntityMsg, EntityRegistry, EntityState};
use rshome_svc::{ServiceMsg, ServiceTarget};

use crate::definition::{ServiceCallTarget, StepDef, WfError};

// ── PipelineRunner ────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct PipelineRunner {
    pub entity_registry: EntityRegistry,
    pub service_registry: ActorRef<ServiceMsg>,
}

impl PipelineRunner {
    pub fn new(entity_registry: EntityRegistry, service_registry: ActorRef<ServiceMsg>) -> Self {
        Self {
            entity_registry,
            service_registry,
        }
    }

    /// Execute a sequence of steps in order. Returns `Ok(Value::Null)` when all
    /// steps succeed. Returns the first error encountered.
    pub async fn execute(
        &self,
        steps: &[StepDef],
        _input: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, WfError> {
        for step in steps {
            self.execute_step(step).await?;
        }
        Ok(serde_json::Value::Null)
    }

    async fn execute_step(&self, step: &StepDef) -> Result<(), WfError> {
        match step {
            StepDef::Log { message } => {
                tracing::info!("[workflow] {}", message);
            }
            StepDef::Delay { duration_ms } => {
                tokio::time::sleep(Duration::from_millis(*duration_ms)).await;
            }
            StepDef::Condition {
                expression,
                entity_id,
            } => {
                self.eval_condition(expression, entity_id.as_deref())
                    .await?;
            }
            StepDef::ServiceCall {
                domain,
                service,
                target,
                data,
            } => {
                let svc_target = to_service_target(target);
                let count = self
                    .service_registry
                    .ask(|tx| ServiceMsg::Call {
                        domain: domain.clone(),
                        service: service.clone(),
                        target: svc_target,
                        data: data.clone(),
                        reply: tx,
                    })
                    .await
                    .map_err(|e| WfError::ActorError(e.to_string()))?
                    .map_err(|e| WfError::ServiceError(e.to_string()))?;
                tracing::debug!(
                    "[workflow] service call {}.{} affected {} entities",
                    domain,
                    service,
                    count
                );
            }
            StepDef::SetEntityState { entity_id, state } => {
                let eid = EntityId(entity_id.clone());
                let actor = self
                    .entity_registry
                    .get(&eid)
                    .ok_or_else(|| WfError::EntityNotFound(entity_id.clone()))?;
                let new_state: EntityState = serde_json::from_value(state.clone())
                    .map_err(|e| WfError::InvalidExpression(e.to_string()))?;
                actor
                    .send(EntityMsg::SetState(new_state))
                    .map_err(|e| WfError::ActorError(e.to_string()))?;
            }
        }
        Ok(())
    }

    /// Evaluate a condition expression, optionally against entity state.
    ///
    /// Without `entity_id`, only literal `"true"` / `"false"` are accepted.
    /// With `entity_id`, the expression must be `"field op value"` where `field`
    /// is a top-level key of the serialized entity state inner object.
    pub async fn eval_condition(
        &self,
        expression: &str,
        entity_id: Option<&str>,
    ) -> Result<(), WfError> {
        if entity_id.is_none() {
            return match expression.trim() {
                "true" => Ok(()),
                "false" => Err(WfError::ConditionFailed(expression.to_string())),
                _ => Err(WfError::InvalidExpression(format!(
                    "expression '{expression}' requires entity_id"
                ))),
            };
        }

        let eid = EntityId(entity_id.unwrap().to_string());
        let actor = self
            .entity_registry
            .get(&eid)
            .ok_or_else(|| WfError::EntityNotFound(entity_id.unwrap().to_string()))?;

        let state = actor
            .ask(EntityMsg::GetState)
            .await
            .map_err(|e| WfError::ActorError(e.to_string()))?;

        let parts: Vec<&str> = expression.splitn(3, ' ').collect();
        if parts.len() != 3 {
            return Err(WfError::InvalidExpression(format!(
                "expected 'field op value', got: '{expression}'"
            )));
        }
        let (field, op, expected) = (parts[0], parts[1], parts[2]);

        let state_json =
            serde_json::to_value(&state).map_err(|e| WfError::InvalidExpression(e.to_string()))?;

        let fields = extract_entity_fields(&state_json)?;
        let actual = fields.get(field).ok_or_else(|| {
            WfError::InvalidExpression(format!("field '{field}' not found in entity state"))
        })?;

        if compare_field_value(actual, op, expected)? {
            Ok(())
        } else {
            Err(WfError::ConditionFailed(expression.to_string()))
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn to_service_target(t: &ServiceCallTarget) -> ServiceTarget {
    match t {
        ServiceCallTarget::All => ServiceTarget::All,
        ServiceCallTarget::EntityId { id } => ServiceTarget::EntityIds(vec![EntityId(id.clone())]),
        ServiceCallTarget::Domain { domain } => ServiceTarget::Domain(domain.clone()),
    }
}

/// Given a serialized EntityState JSON value, extract the inner object fields.
/// Handles both object-variant enums (`{"BinarySensor": {...}}`) and unit
/// variants (`"Unavailable"`).
fn extract_entity_fields(
    value: &serde_json::Value,
) -> Result<BTreeMap<String, serde_json::Value>, WfError> {
    match value {
        serde_json::Value::Object(map) if map.len() == 1 => {
            let inner = map.values().next().unwrap();
            match inner {
                serde_json::Value::Object(inner_map) => Ok(inner_map
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()),
                _ => Err(WfError::InvalidExpression(
                    "entity state variant has no object fields".to_string(),
                )),
            }
        }
        serde_json::Value::String(s) => Err(WfError::ConditionFailed(format!(
            "entity state is '{s}' (no fields)"
        ))),
        _ => Err(WfError::InvalidExpression(
            "unexpected entity state JSON shape".to_string(),
        )),
    }
}

fn compare_field_value(
    actual: &serde_json::Value,
    op: &str,
    expected: &str,
) -> Result<bool, WfError> {
    let result = match actual {
        serde_json::Value::Bool(b) => {
            let exp = expected == "true";
            match op {
                "==" => *b == exp,
                "!=" => *b != exp,
                _ => {
                    return Err(WfError::InvalidExpression(format!(
                        "op '{op}' not supported for bool"
                    )))
                }
            }
        }
        serde_json::Value::Number(n) => {
            let act_f = n.as_f64().unwrap_or(0.0);
            let exp_f: f64 = expected.parse().map_err(|_| {
                WfError::InvalidExpression(format!("cannot parse '{expected}' as number"))
            })?;
            match op {
                "==" => (act_f - exp_f).abs() < f64::EPSILON,
                "!=" => (act_f - exp_f).abs() >= f64::EPSILON,
                ">" => act_f > exp_f,
                "<" => act_f < exp_f,
                ">=" => act_f >= exp_f,
                "<=" => act_f <= exp_f,
                _ => {
                    return Err(WfError::InvalidExpression(format!(
                        "unknown op '{op}' for number"
                    )))
                }
            }
        }
        serde_json::Value::String(s) => match op {
            "==" => s == expected,
            "!=" => s != expected,
            _ => {
                return Err(WfError::InvalidExpression(format!(
                    "op '{op}' not supported for string"
                )))
            }
        },
        _ => {
            return Err(WfError::InvalidExpression(
                "unsupported JSON value type for comparison".to_string(),
            ))
        }
    };
    Ok(result)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rshome_actor::ActorSystem;
    use rshome_entity::{
        EntityActor, EntityCategory, EntityDescriptor, EntityRegistry, EntityState,
        NullStateUpdater,
    };
    use rshome_svc::ServiceRegistryActor;

    use super::*;
    use crate::definition::ServiceCallTarget;

    fn make_runner(sys: &ActorSystem) -> (EntityRegistry, PipelineRunner) {
        let registry = EntityRegistry::default();
        let svc = sys.spawn(ServiceRegistryActor::new(registry.clone(), None));
        let runner = PipelineRunner::new(registry.clone(), svc);
        (registry, runner)
    }

    fn register_binary_sensor(
        sys: &ActorSystem,
        registry: &EntityRegistry,
        entity_id: &str,
        is_on: bool,
    ) {
        let eid = EntityId(entity_id.to_string());
        let descriptor = EntityDescriptor {
            entity_id: eid.clone(),
            name: entity_id.to_string(),
            icon: None,
            device_id: None,
            area_id: None,
            entity_category: EntityCategory::None,
            domain_id: "binary_sensor".to_string(),
            feature_set: vec![],
            device_class: None,
        };
        let (actor, _tx) = EntityActor::new(
            descriptor,
            EntityState::BinarySensor {
                is_on,
                attributes: HashMap::new(),
            },
            std::sync::Arc::new(NullStateUpdater),
        );
        let actor_ref = sys.spawn(actor);
        registry.register(eid, actor_ref);
    }

    fn register_sensor(sys: &ActorSystem, registry: &EntityRegistry, entity_id: &str, value: f64) {
        let eid = EntityId(entity_id.to_string());
        let descriptor = EntityDescriptor {
            entity_id: eid.clone(),
            name: entity_id.to_string(),
            icon: None,
            device_id: None,
            area_id: None,
            entity_category: EntityCategory::None,
            domain_id: "sensor".to_string(),
            feature_set: vec![],
            device_class: None,
        };
        let (actor, _tx) = EntityActor::new(
            descriptor,
            EntityState::Sensor {
                value,
                unit: None,
                attributes: HashMap::new(),
            },
            std::sync::Arc::new(NullStateUpdater),
        );
        let actor_ref = sys.spawn(actor);
        registry.register(eid, actor_ref);
    }

    #[tokio::test]
    async fn test_empty_steps_ok() {
        let sys = ActorSystem::new();
        let (_reg, runner) = make_runner(&sys);
        let result = runner.execute(&[], None).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), serde_json::Value::Null);
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_log_step() {
        let sys = ActorSystem::new();
        let (_reg, runner) = make_runner(&sys);
        let steps = vec![StepDef::Log {
            message: "test log".to_string(),
        }];
        assert!(runner.execute(&steps, None).await.is_ok());
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_delay_step() {
        let sys = ActorSystem::new();
        let (_reg, runner) = make_runner(&sys);
        let steps = vec![StepDef::Delay { duration_ms: 10 }];
        assert!(runner.execute(&steps, None).await.is_ok());
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_condition_true_binary_sensor() {
        let sys = ActorSystem::new();
        let (reg, runner) = make_runner(&sys);
        register_binary_sensor(&sys, &reg, "binary_sensor.motion", true);
        let steps = vec![StepDef::Condition {
            expression: "is_on == true".to_string(),
            entity_id: Some("binary_sensor.motion".to_string()),
        }];
        assert!(runner.execute(&steps, None).await.is_ok());
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_condition_false_binary_sensor() {
        let sys = ActorSystem::new();
        let (reg, runner) = make_runner(&sys);
        register_binary_sensor(&sys, &reg, "binary_sensor.motion", false);
        let steps = vec![StepDef::Condition {
            expression: "is_on == true".to_string(),
            entity_id: Some("binary_sensor.motion".to_string()),
        }];
        let err = runner.execute(&steps, None).await.unwrap_err();
        assert!(matches!(err, WfError::ConditionFailed(_)));
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_condition_entity_not_found() {
        let sys = ActorSystem::new();
        let (_reg, runner) = make_runner(&sys);
        let steps = vec![StepDef::Condition {
            expression: "is_on == true".to_string(),
            entity_id: Some("binary_sensor.missing".to_string()),
        }];
        let err = runner.execute(&steps, None).await.unwrap_err();
        assert!(matches!(err, WfError::EntityNotFound(_)));
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_condition_numeric_greater_than() {
        let sys = ActorSystem::new();
        let (reg, runner) = make_runner(&sys);
        register_sensor(&sys, &reg, "sensor.temp", 25.0);
        let steps = vec![StepDef::Condition {
            expression: "value > 20.0".to_string(),
            entity_id: Some("sensor.temp".to_string()),
        }];
        assert!(runner.execute(&steps, None).await.is_ok());
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_condition_numeric_less_than_fails() {
        let sys = ActorSystem::new();
        let (reg, runner) = make_runner(&sys);
        register_sensor(&sys, &reg, "sensor.temp", 15.0);
        let steps = vec![StepDef::Condition {
            expression: "value > 20.0".to_string(),
            entity_id: Some("sensor.temp".to_string()),
        }];
        let err = runner.execute(&steps, None).await.unwrap_err();
        assert!(matches!(err, WfError::ConditionFailed(_)));
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_service_call_dispatched() {
        let sys = ActorSystem::new();
        let (reg, runner) = make_runner(&sys);
        // Register a switch entity so the service call has a target
        let eid = EntityId("switch.relay".to_string());
        let descriptor = EntityDescriptor {
            entity_id: eid.clone(),
            name: "relay".to_string(),
            icon: None,
            device_id: None,
            area_id: None,
            entity_category: EntityCategory::None,
            domain_id: "switch".to_string(),
            feature_set: vec![],
            device_class: None,
        };
        let (actor, _tx) = EntityActor::new(
            descriptor,
            EntityState::Switch { is_on: false },
            std::sync::Arc::new(NullStateUpdater),
        );
        reg.register(eid, sys.spawn(actor));

        let steps = vec![StepDef::ServiceCall {
            domain: "switch".to_string(),
            service: "turn_on".to_string(),
            target: ServiceCallTarget::All,
            data: serde_json::Value::Null,
        }];
        assert!(runner.execute(&steps, None).await.is_ok());
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_service_call_unknown_service_fails() {
        let sys = ActorSystem::new();
        let (_reg, runner) = make_runner(&sys);
        let steps = vec![StepDef::ServiceCall {
            domain: "unknown".to_string(),
            service: "do_thing".to_string(),
            target: ServiceCallTarget::All,
            data: serde_json::Value::Null,
        }];
        let err = runner.execute(&steps, None).await.unwrap_err();
        assert!(matches!(err, WfError::ServiceError(_)));
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_set_entity_state_dispatched() {
        let sys = ActorSystem::new();
        let (reg, runner) = make_runner(&sys);
        register_binary_sensor(&sys, &reg, "binary_sensor.door", false);

        let new_state = serde_json::to_value(EntityState::BinarySensor {
            is_on: true,
            attributes: HashMap::new(),
        })
        .unwrap();
        let steps = vec![StepDef::SetEntityState {
            entity_id: "binary_sensor.door".to_string(),
            state: new_state,
        }];
        assert!(runner.execute(&steps, None).await.is_ok());

        // Verify the state was updated
        let actor = reg
            .get(&EntityId("binary_sensor.door".to_string()))
            .unwrap();
        let state = actor.ask(EntityMsg::GetState).await.unwrap();
        assert_eq!(
            state,
            EntityState::BinarySensor {
                is_on: true,
                attributes: HashMap::new()
            }
        );
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_set_entity_state_not_found() {
        let sys = ActorSystem::new();
        let (_reg, runner) = make_runner(&sys);
        let steps = vec![StepDef::SetEntityState {
            entity_id: "sensor.missing".to_string(),
            state: serde_json::json!({"Sensor": {"value": 1.0, "unit": null, "attributes": {}}}),
        }];
        let err = runner.execute(&steps, None).await.unwrap_err();
        assert!(matches!(err, WfError::EntityNotFound(_)));
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_multi_step_pipeline() {
        let sys = ActorSystem::new();
        let (reg, runner) = make_runner(&sys);
        register_binary_sensor(&sys, &reg, "binary_sensor.pir", true);

        let steps = vec![
            StepDef::Log {
                message: "step 1".to_string(),
            },
            StepDef::Condition {
                expression: "is_on == true".to_string(),
                entity_id: Some("binary_sensor.pir".to_string()),
            },
            StepDef::Log {
                message: "step 3".to_string(),
            },
        ];
        assert!(runner.execute(&steps, None).await.is_ok());
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_condition_literal_true_no_entity() {
        let sys = ActorSystem::new();
        let (_reg, runner) = make_runner(&sys);
        let steps = vec![StepDef::Condition {
            expression: "true".to_string(),
            entity_id: None,
        }];
        assert!(runner.execute(&steps, None).await.is_ok());
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_condition_literal_false_no_entity() {
        let sys = ActorSystem::new();
        let (_reg, runner) = make_runner(&sys);
        let steps = vec![StepDef::Condition {
            expression: "false".to_string(),
            entity_id: None,
        }];
        let err = runner.execute(&steps, None).await.unwrap_err();
        assert!(matches!(err, WfError::ConditionFailed(_)));
        sys.shutdown().await;
    }

    #[tokio::test]
    async fn test_condition_invalid_expression() {
        let sys = ActorSystem::new();
        let (_reg, runner) = make_runner(&sys);
        let steps = vec![StepDef::Condition {
            expression: "bad_expr_no_op".to_string(),
            entity_id: None,
        }];
        let err = runner.execute(&steps, None).await.unwrap_err();
        assert!(matches!(err, WfError::InvalidExpression(_)));
        sys.shutdown().await;
    }
}
