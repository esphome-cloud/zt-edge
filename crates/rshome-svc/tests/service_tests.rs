use rshome_actor::ActorSystem;
use rshome_entity::*;
use rshome_svc::*;
use std::sync::Arc;
use std::time::Duration;

fn null_store() -> Arc<dyn StateUpdater> {
    Arc::new(NullStateUpdater)
}

fn make_entity_descriptor(domain: &str, name: &str) -> EntityDescriptor {
    EntityDescriptor {
        entity_id: EntityId::new(domain, name),
        name: name.to_string(),
        icon: None,
        device_id: None,
        area_id: None,
        entity_category: EntityCategory::None,
        domain_id: domain.to_string(),
        feature_set: vec![],
        device_class: None,
    }
}

async fn setup_switch(
    sys: &ActorSystem,
    registry: &EntityRegistry,
    domain: &str,
    name: &str,
    initial_on: bool,
) -> rshome_actor::ActorRef<EntityMsg> {
    let initial_state = EntityState::Switch { is_on: initial_on };
    let (actor, _tx) = EntityActor::new(
        make_entity_descriptor(domain, name),
        initial_state,
        null_store(),
    );
    let actor_ref = sys.spawn(actor);
    registry.register(EntityId::new(domain, name), actor_ref.clone());
    actor_ref
}

// ── ServiceDescriptor tests ───────────────────────────────────────────────────

#[tokio::test]
async fn service_registry_register_custom() {
    let sys = ActorSystem::new();
    let registry = EntityRegistry::default();
    let svc_actor = ServiceRegistryActor::new(registry, None);
    let svc_ref = sys.spawn(svc_actor);

    svc_ref
        .send(ServiceMsg::Register(ServiceDescriptor {
            domain: "custom".to_string(),
            service: "my_action".to_string(),
            description: Some("A custom action".to_string()),
        }))
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;

    let list = svc_ref.ask(ServiceMsg::List).await.unwrap();
    assert!(list
        .iter()
        .any(|s| s.domain == "custom" && s.service == "my_action"));
    sys.shutdown().await;
}

#[tokio::test]
async fn service_registry_lists_builtins() {
    let sys = ActorSystem::new();
    let svc_actor = ServiceRegistryActor::new(EntityRegistry::default(), None);
    let svc_ref = sys.spawn(svc_actor);

    let list = svc_ref.ask(ServiceMsg::List).await.unwrap();
    assert!(list
        .iter()
        .any(|s| s.domain == "switch" && s.service == "turn_on"));
    assert!(list
        .iter()
        .any(|s| s.domain == "light" && s.service == "turn_off"));
    assert!(list
        .iter()
        .any(|s| s.domain == "button" && s.service == "press"));
    sys.shutdown().await;
}

// ── Dispatch EntityIds target ──────────────────────────────────────────────────

#[tokio::test]
async fn service_call_single_entity_by_id() {
    let sys = ActorSystem::new();
    let registry = EntityRegistry::default();
    let entity_ref = setup_switch(&sys, &registry, "switch", "s1", false).await;

    let svc_actor = ServiceRegistryActor::new(registry, None);
    let svc_ref = sys.spawn(svc_actor);

    let result = svc_ref
        .ask(|tx| ServiceMsg::Call {
            domain: "switch".to_string(),
            service: "turn_on".to_string(),
            target: ServiceTarget::EntityIds(vec![EntityId::new("switch", "s1")]),
            data: serde_json::Value::Null,
            reply: tx,
        })
        .await
        .unwrap();
    assert_eq!(result.unwrap(), 1);

    tokio::time::sleep(Duration::from_millis(30)).await;
    let state = entity_ref.ask(EntityMsg::GetState).await.unwrap();
    assert_eq!(state, EntityState::Switch { is_on: true });
    sys.shutdown().await;
}

#[tokio::test]
async fn service_call_multiple_entities_by_id() {
    let sys = ActorSystem::new();
    let registry = EntityRegistry::default();
    let r1 = setup_switch(&sys, &registry, "switch", "m1", false).await;
    let r2 = setup_switch(&sys, &registry, "switch", "m2", false).await;

    let svc_actor = ServiceRegistryActor::new(registry, None);
    let svc_ref = sys.spawn(svc_actor);

    let result = svc_ref
        .ask(|tx| ServiceMsg::Call {
            domain: "switch".to_string(),
            service: "turn_on".to_string(),
            target: ServiceTarget::EntityIds(vec![
                EntityId::new("switch", "m1"),
                EntityId::new("switch", "m2"),
            ]),
            data: serde_json::Value::Null,
            reply: tx,
        })
        .await
        .unwrap();
    assert_eq!(result.unwrap(), 2);
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(
        r1.ask(EntityMsg::GetState).await.unwrap(),
        EntityState::Switch { is_on: true }
    );
    assert_eq!(
        r2.ask(EntityMsg::GetState).await.unwrap(),
        EntityState::Switch { is_on: true }
    );
    sys.shutdown().await;
}

#[tokio::test]
async fn service_call_unknown_entity_id_returns_no_targets() {
    let sys = ActorSystem::new();
    let registry = EntityRegistry::default();
    let svc_actor = ServiceRegistryActor::new(registry, None);
    let svc_ref = sys.spawn(svc_actor);

    let result = svc_ref
        .ask(|tx| ServiceMsg::Call {
            domain: "switch".to_string(),
            service: "turn_on".to_string(),
            target: ServiceTarget::EntityIds(vec![EntityId::new("switch", "nonexistent")]),
            data: serde_json::Value::Null,
            reply: tx,
        })
        .await
        .unwrap();
    assert!(matches!(result, Err(ServiceError::NoTargets)));
    sys.shutdown().await;
}

// ── Dispatch Domain target ────────────────────────────────────────────────────

#[tokio::test]
async fn service_call_domain_turns_on_all_switches() {
    let sys = ActorSystem::new();
    let registry = EntityRegistry::default();
    let r1 = setup_switch(&sys, &registry, "switch", "d1", false).await;
    let r2 = setup_switch(&sys, &registry, "switch", "d2", false).await;

    let svc_actor = ServiceRegistryActor::new(registry, None);
    let svc_ref = sys.spawn(svc_actor);

    let result = svc_ref
        .ask(|tx| ServiceMsg::Call {
            domain: "switch".to_string(),
            service: "turn_on".to_string(),
            target: ServiceTarget::Domain("switch".to_string()),
            data: serde_json::Value::Null,
            reply: tx,
        })
        .await
        .unwrap();
    assert_eq!(result.unwrap(), 2);
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(
        r1.ask(EntityMsg::GetState).await.unwrap(),
        EntityState::Switch { is_on: true }
    );
    assert_eq!(
        r2.ask(EntityMsg::GetState).await.unwrap(),
        EntityState::Switch { is_on: true }
    );
    sys.shutdown().await;
}

#[tokio::test]
async fn service_call_empty_domain_returns_no_targets() {
    let sys = ActorSystem::new();
    let registry = EntityRegistry::default();
    let svc_actor = ServiceRegistryActor::new(registry, None);
    let svc_ref = sys.spawn(svc_actor);

    let result = svc_ref
        .ask(|tx| ServiceMsg::Call {
            domain: "switch".to_string(),
            service: "turn_on".to_string(),
            target: ServiceTarget::Domain("switch".to_string()),
            data: serde_json::Value::Null,
            reply: tx,
        })
        .await
        .unwrap();
    assert!(matches!(result, Err(ServiceError::NoTargets)));
    sys.shutdown().await;
}

#[tokio::test]
async fn service_call_domain_does_not_affect_other_domains() {
    let sys = ActorSystem::new();
    let registry = EntityRegistry::default();
    setup_switch(&sys, &registry, "switch", "mixed_sw", false).await;
    let (light_actor, _) = EntityActor::new(
        make_entity_descriptor("light", "mixed_l"),
        EntityState::Light {
            is_on: false,
            brightness: None,
            color_temp: None,
            rgb: None,
            color_mode: None,
        },
        null_store(),
    );
    registry.register(EntityId::new("light", "mixed_l"), sys.spawn(light_actor));

    let svc_actor = ServiceRegistryActor::new(registry, None);
    let svc_ref = sys.spawn(svc_actor);

    let result = svc_ref
        .ask(|tx| ServiceMsg::Call {
            domain: "switch".to_string(),
            service: "turn_on".to_string(),
            target: ServiceTarget::Domain("switch".to_string()),
            data: serde_json::Value::Null,
            reply: tx,
        })
        .await
        .unwrap();
    assert_eq!(result.unwrap(), 1);
    sys.shutdown().await;
}

// ── Dispatch DeviceId target ──────────────────────────────────────────────────

#[tokio::test]
async fn service_call_device_id_fans_out() {
    let sys = ActorSystem::new();
    let registry = EntityRegistry::default();
    let dm_actor = DeviceManagerActor::new(registry.clone(), null_store());
    let dm_ref = sys.spawn(dm_actor);

    let device_id = DeviceId("device_svc_1".to_string());
    let dev_ref = dm_ref
        .ask(|tx| DeviceManagerMsg::AddDevice {
            descriptor: DeviceDescriptor {
                device_id: device_id.clone(),
                name: "Test Device".to_string(),
                model: None,
                manufacturer: None,
                sw_version: None,
                area_id: None,
            },
            reply: tx,
        })
        .await
        .unwrap();

    dev_ref
        .ask(|tx| DeviceMsg::AddEntity {
            descriptor: make_entity_descriptor("switch", "dev_sw1"),
            initial_state: EntityState::Switch { is_on: false },
            reply: tx,
        })
        .await
        .unwrap();
    dev_ref
        .ask(|tx| DeviceMsg::AddEntity {
            descriptor: make_entity_descriptor("switch", "dev_sw2"),
            initial_state: EntityState::Switch { is_on: false },
            reply: tx,
        })
        .await
        .unwrap();

    let svc_actor = ServiceRegistryActor::new(registry, Some(dm_ref));
    let svc_ref = sys.spawn(svc_actor);

    let result = svc_ref
        .ask(|tx| ServiceMsg::Call {
            domain: "switch".to_string(),
            service: "turn_on".to_string(),
            target: ServiceTarget::DeviceId(device_id),
            data: serde_json::Value::Null,
            reply: tx,
        })
        .await
        .unwrap();
    assert_eq!(result.unwrap(), 2);
    sys.shutdown().await;
}

#[tokio::test]
async fn service_call_unknown_device_returns_no_targets() {
    let sys = ActorSystem::new();
    let registry = EntityRegistry::default();
    let dm_actor = DeviceManagerActor::new(registry.clone(), null_store());
    let dm_ref = sys.spawn(dm_actor);

    let svc_actor = ServiceRegistryActor::new(registry, Some(dm_ref));
    let svc_ref = sys.spawn(svc_actor);

    let result = svc_ref
        .ask(|tx| ServiceMsg::Call {
            domain: "switch".to_string(),
            service: "turn_on".to_string(),
            target: ServiceTarget::DeviceId(DeviceId("unknown_device".to_string())),
            data: serde_json::Value::Null,
            reply: tx,
        })
        .await
        .unwrap();
    assert!(matches!(result, Err(ServiceError::NoTargets)));
    sys.shutdown().await;
}

// ── Built-in service tests ────────────────────────────────────────────────────

#[tokio::test]
async fn builtin_turn_on_switch() {
    let sys = ActorSystem::new();
    let registry = EntityRegistry::default();
    let entity_ref = setup_switch(&sys, &registry, "switch", "bi_s1", false).await;
    let svc_ref = sys.spawn(ServiceRegistryActor::new(registry, None));

    svc_ref
        .ask(|tx| ServiceMsg::Call {
            domain: "switch".to_string(),
            service: "turn_on".to_string(),
            target: ServiceTarget::EntityIds(vec![EntityId::new("switch", "bi_s1")]),
            data: serde_json::Value::Null,
            reply: tx,
        })
        .await
        .unwrap()
        .unwrap();

    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(
        entity_ref.ask(EntityMsg::GetState).await.unwrap(),
        EntityState::Switch { is_on: true }
    );
    sys.shutdown().await;
}

#[tokio::test]
async fn builtin_turn_off_light() {
    let sys = ActorSystem::new();
    let registry = EntityRegistry::default();
    let (actor, _) = EntityActor::new(
        make_entity_descriptor("light", "bi_l1"),
        EntityState::Light {
            is_on: true,
            brightness: None,
            color_temp: None,
            rgb: None,
            color_mode: None,
        },
        null_store(),
    );
    let entity_ref = sys.spawn(actor);
    registry.register(EntityId::new("light", "bi_l1"), entity_ref.clone());

    let svc_ref = sys.spawn(ServiceRegistryActor::new(registry, None));

    svc_ref
        .ask(|tx| ServiceMsg::Call {
            domain: "light".to_string(),
            service: "turn_off".to_string(),
            target: ServiceTarget::EntityIds(vec![EntityId::new("light", "bi_l1")]),
            data: serde_json::Value::Null,
            reply: tx,
        })
        .await
        .unwrap()
        .unwrap();

    tokio::time::sleep(Duration::from_millis(30)).await;
    let state = entity_ref.ask(EntityMsg::GetState).await.unwrap();
    assert!(matches!(state, EntityState::Light { is_on: false, .. }));
    sys.shutdown().await;
}

#[tokio::test]
async fn builtin_toggle() {
    let sys = ActorSystem::new();
    let registry = EntityRegistry::default();
    let entity_ref = setup_switch(&sys, &registry, "switch", "bi_tog", true).await;
    let svc_ref = sys.spawn(ServiceRegistryActor::new(registry, None));

    svc_ref
        .ask(|tx| ServiceMsg::Call {
            domain: "switch".to_string(),
            service: "toggle".to_string(),
            target: ServiceTarget::EntityIds(vec![EntityId::new("switch", "bi_tog")]),
            data: serde_json::Value::Null,
            reply: tx,
        })
        .await
        .unwrap()
        .unwrap();

    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(
        entity_ref.ask(EntityMsg::GetState).await.unwrap(),
        EntityState::Switch { is_on: false }
    );
    sys.shutdown().await;
}

#[tokio::test]
async fn builtin_set_value_number() {
    let sys = ActorSystem::new();
    let registry = EntityRegistry::default();
    let (actor, _) = EntityActor::new(
        make_entity_descriptor("number", "bi_n1"),
        EntityState::Number {
            value: 0.0,
            min: 0.0,
            max: 100.0,
            step: 1.0,
            unit: None,
        },
        null_store(),
    );
    let entity_ref = sys.spawn(actor);
    registry.register(EntityId::new("number", "bi_n1"), entity_ref.clone());

    let svc_ref = sys.spawn(ServiceRegistryActor::new(registry, None));
    svc_ref
        .ask(|tx| ServiceMsg::Call {
            domain: "number".to_string(),
            service: "set_value".to_string(),
            target: ServiceTarget::EntityIds(vec![EntityId::new("number", "bi_n1")]),
            data: serde_json::json!({ "value": 75.0 }),
            reply: tx,
        })
        .await
        .unwrap()
        .unwrap();

    tokio::time::sleep(Duration::from_millis(30)).await;
    let state = entity_ref.ask(EntityMsg::GetState).await.unwrap();
    assert!(matches!(state, EntityState::Number { value, .. } if (value - 75.0).abs() < 0.001));
    sys.shutdown().await;
}

#[tokio::test]
async fn builtin_press_button() {
    let sys = ActorSystem::new();
    let registry = EntityRegistry::default();
    let (actor, _) = EntityActor::new(
        make_entity_descriptor("button", "bi_btn"),
        EntityState::Button,
        null_store(),
    );
    let entity_ref = sys.spawn(actor);
    registry.register(EntityId::new("button", "bi_btn"), entity_ref.clone());

    let svc_ref = sys.spawn(ServiceRegistryActor::new(registry, None));
    svc_ref
        .ask(|tx| ServiceMsg::Call {
            domain: "button".to_string(),
            service: "press".to_string(),
            target: ServiceTarget::EntityIds(vec![EntityId::new("button", "bi_btn")]),
            data: serde_json::Value::Null,
            reply: tx,
        })
        .await
        .unwrap()
        .unwrap();

    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(
        entity_ref.ask(EntityMsg::GetState).await.unwrap(),
        EntityState::Button
    );
    sys.shutdown().await;
}

#[tokio::test]
async fn builtin_set_climate_mode() {
    let sys = ActorSystem::new();
    let registry = EntityRegistry::default();
    let (actor, _) = EntityActor::new(
        make_entity_descriptor("climate", "bi_cl"),
        EntityState::Climate {
            mode: "off".to_string(),
            current_temp: Some(20.0),
            target_temp: Some(22.0),
            hvac_action: None,
        },
        null_store(),
    );
    let entity_ref = sys.spawn(actor);
    registry.register(EntityId::new("climate", "bi_cl"), entity_ref.clone());

    let svc_ref = sys.spawn(ServiceRegistryActor::new(registry, None));
    svc_ref
        .ask(|tx| ServiceMsg::Call {
            domain: "climate".to_string(),
            service: "set_hvac_mode".to_string(),
            target: ServiceTarget::EntityIds(vec![EntityId::new("climate", "bi_cl")]),
            data: serde_json::json!({ "hvac_mode": "heat" }),
            reply: tx,
        })
        .await
        .unwrap()
        .unwrap();

    tokio::time::sleep(Duration::from_millis(30)).await;
    let state = entity_ref.ask(EntityMsg::GetState).await.unwrap();
    assert!(matches!(state, EntityState::Climate { mode, .. } if mode == "heat"));
    sys.shutdown().await;
}

// ── Error tests ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn service_call_unknown_service_error() {
    let sys = ActorSystem::new();
    let registry = EntityRegistry::default();
    setup_switch(&sys, &registry, "switch", "err_sw", false).await;
    let svc_ref = sys.spawn(ServiceRegistryActor::new(registry, None));

    let result = svc_ref
        .ask(|tx| ServiceMsg::Call {
            domain: "switch".to_string(),
            service: "nonexistent_service".to_string(),
            target: ServiceTarget::EntityIds(vec![EntityId::new("switch", "err_sw")]),
            data: serde_json::Value::Null,
            reply: tx,
        })
        .await
        .unwrap();
    assert!(matches!(result, Err(ServiceError::NotFound { .. })));
    sys.shutdown().await;
}

#[tokio::test]
async fn service_call_no_targets_error() {
    let sys = ActorSystem::new();
    let registry = EntityRegistry::default();
    let svc_ref = sys.spawn(ServiceRegistryActor::new(registry, None));

    let result = svc_ref
        .ask(|tx| ServiceMsg::Call {
            domain: "switch".to_string(),
            service: "turn_on".to_string(),
            target: ServiceTarget::EntityIds(vec![]),
            data: serde_json::Value::Null,
            reply: tx,
        })
        .await
        .unwrap();
    assert!(matches!(result, Err(ServiceError::NoTargets)));
    sys.shutdown().await;
}

#[tokio::test]
async fn service_call_returns_entity_count() {
    let sys = ActorSystem::new();
    let registry = EntityRegistry::default();
    for i in 0..5u32 {
        setup_switch(&sys, &registry, "switch", &format!("cnt_{i}"), false).await;
    }
    let svc_ref = sys.spawn(ServiceRegistryActor::new(registry, None));

    let result = svc_ref
        .ask(|tx| ServiceMsg::Call {
            domain: "switch".to_string(),
            service: "turn_on".to_string(),
            target: ServiceTarget::Domain("switch".to_string()),
            data: serde_json::Value::Null,
            reply: tx,
        })
        .await
        .unwrap();
    assert_eq!(result.unwrap(), 5);
    sys.shutdown().await;
}

#[tokio::test]
async fn service_call_unknown_service_has_correct_fields() {
    let sys = ActorSystem::new();
    let svc_ref = sys.spawn(ServiceRegistryActor::new(EntityRegistry::default(), None));

    let result = svc_ref
        .ask(|tx| ServiceMsg::Call {
            domain: "foo".to_string(),
            service: "bar".to_string(),
            target: ServiceTarget::All,
            data: serde_json::Value::Null,
            reply: tx,
        })
        .await
        .unwrap();

    match result {
        Err(ServiceError::NotFound { domain, service }) => {
            assert_eq!(domain, "foo");
            assert_eq!(service, "bar");
        }
        _ => panic!("expected NotFound"),
    }
    sys.shutdown().await;
}
