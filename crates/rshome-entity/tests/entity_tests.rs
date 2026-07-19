use rshome_actor::ActorSystem;
use rshome_entity::*;
use std::sync::Arc;

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

fn make_device_descriptor(id: &str, name: &str) -> DeviceDescriptor {
    DeviceDescriptor {
        device_id: DeviceId(id.to_string()),
        name: name.to_string(),
        model: None,
        manufacturer: None,
        sw_version: None,
        area_id: None,
    }
}

// ── EntityId tests ────────────────────────────────────────────────────────────

#[test]
fn entity_id_new() {
    let id = EntityId::new("switch", "living_room");
    assert_eq!(id.0, "switch.living_room");
}

#[test]
fn entity_id_domain() {
    let id = EntityId::new("light", "bedroom");
    assert_eq!(id.domain(), "light");
}

#[test]
fn entity_id_object_id() {
    let id = EntityId::new("sensor", "temperature");
    assert_eq!(id.object_id(), "temperature");
}

#[test]
fn entity_id_equality() {
    let a = EntityId::new("switch", "fan");
    let b = EntityId::new("switch", "fan");
    assert_eq!(a, b);
}

#[test]
fn entity_id_display() {
    let id = EntityId::new("light", "kitchen");
    assert_eq!(id.to_string(), "light.kitchen");
}

// ── EntityState tests ──────────────────────────────────────────────────────────

#[test]
fn entity_state_clone() {
    let s = EntityState::Switch { is_on: true };
    let c = s.clone();
    assert_eq!(s, c);
}

#[test]
fn entity_state_serde_roundtrip() {
    let s = EntityState::Sensor {
        value: 23.5,
        unit: Some("°C".to_string()),
        attributes: Default::default(),
    };
    let json = serde_json::to_string(&s).unwrap();
    let back: EntityState = serde_json::from_str(&json).unwrap();
    assert_eq!(s, back);
}

#[test]
fn entity_state_unavailable() {
    let s = EntityState::Unavailable;
    let json = serde_json::to_string(&s).unwrap();
    let back: EntityState = serde_json::from_str(&json).unwrap();
    assert_eq!(s, back);
}

// ── EntityRegistry tests ───────────────────────────────────────────────────────

#[tokio::test]
async fn registry_register_and_get() {
    let sys = ActorSystem::new();
    let registry = EntityRegistry::default();
    let id = EntityId::new("switch", "test");
    let (actor, _tx) = EntityActor::new(
        make_entity_descriptor("switch", "test"),
        EntityState::Switch { is_on: false },
        null_store(),
    );
    let actor_ref = sys.spawn(actor);
    registry.register(id.clone(), actor_ref);
    assert!(registry.get(&id).is_some());
    sys.shutdown().await;
}

#[tokio::test]
async fn registry_remove() {
    let sys = ActorSystem::new();
    let registry = EntityRegistry::default();
    let id = EntityId::new("switch", "test2");
    let (actor, _tx) = EntityActor::new(
        make_entity_descriptor("switch", "test2"),
        EntityState::Switch { is_on: false },
        null_store(),
    );
    let actor_ref = sys.spawn(actor);
    registry.register(id.clone(), actor_ref);
    registry.remove(&id);
    assert!(registry.get(&id).is_none());
    sys.shutdown().await;
}

#[tokio::test]
async fn registry_list_by_domain() {
    let sys = ActorSystem::new();
    let registry = EntityRegistry::default();

    for name in &["a", "b"] {
        let id = EntityId::new("light", name);
        let (actor, _tx) = EntityActor::new(
            make_entity_descriptor("light", name),
            EntityState::Light {
                is_on: false,
                brightness: None,
                color_temp: None,
                rgb: None,
                color_mode: None,
            },
            null_store(),
        );
        registry.register(id, sys.spawn(actor));
    }

    let switch_id = EntityId::new("switch", "s1");
    let (actor, _tx) = EntityActor::new(
        make_entity_descriptor("switch", "s1"),
        EntityState::Switch { is_on: false },
        null_store(),
    );
    registry.register(switch_id, sys.spawn(actor));

    assert_eq!(registry.list_by_domain("light").len(), 2);
    assert_eq!(registry.list_by_domain("switch").len(), 1);
    assert_eq!(registry.count(), 3);
    sys.shutdown().await;
}

// ── EntityActor tests ──────────────────────────────────────────────────────────

#[tokio::test]
async fn entity_actor_spawn_and_get_state() {
    let sys = ActorSystem::new();
    let (actor, _tx) = EntityActor::new(
        make_entity_descriptor("switch", "test"),
        EntityState::Switch { is_on: false },
        null_store(),
    );
    let actor_ref = sys.spawn(actor);
    let state = actor_ref.ask(EntityMsg::GetState).await.unwrap();
    assert_eq!(state, EntityState::Switch { is_on: false });
    sys.shutdown().await;
}

#[tokio::test]
async fn entity_actor_set_state() {
    let sys = ActorSystem::new();
    let (actor, _tx) = EntityActor::new(
        make_entity_descriptor("switch", "test"),
        EntityState::Switch { is_on: false },
        null_store(),
    );
    let actor_ref = sys.spawn(actor);
    actor_ref
        .send(EntityMsg::SetState(EntityState::Switch { is_on: true }))
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let state = actor_ref.ask(EntityMsg::GetState).await.unwrap();
    assert_eq!(state, EntityState::Switch { is_on: true });
    sys.shutdown().await;
}

#[tokio::test]
async fn entity_actor_command_turn_on() {
    let sys = ActorSystem::new();
    let (actor, _tx) = EntityActor::new(
        make_entity_descriptor("switch", "test"),
        EntityState::Switch { is_on: false },
        null_store(),
    );
    let actor_ref = sys.spawn(actor);
    actor_ref
        .send(EntityMsg::Command(EntityCommand::TurnOn))
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let state = actor_ref.ask(EntityMsg::GetState).await.unwrap();
    assert_eq!(state, EntityState::Switch { is_on: true });
    sys.shutdown().await;
}

#[tokio::test]
async fn entity_actor_command_toggle() {
    let sys = ActorSystem::new();
    let (actor, _tx) = EntityActor::new(
        make_entity_descriptor("switch", "test"),
        EntityState::Switch { is_on: false },
        null_store(),
    );
    let actor_ref = sys.spawn(actor);
    actor_ref
        .send(EntityMsg::Command(EntityCommand::Toggle))
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let state = actor_ref.ask(EntityMsg::GetState).await.unwrap();
    assert_eq!(state, EntityState::Switch { is_on: true });
    sys.shutdown().await;
}

#[tokio::test]
async fn entity_actor_state_change_broadcast() {
    let sys = ActorSystem::new();
    let (actor, change_tx) = EntityActor::new(
        make_entity_descriptor("switch", "test"),
        EntityState::Switch { is_on: false },
        null_store(),
    );
    let mut rx = change_tx.subscribe();
    let actor_ref = sys.spawn(actor);
    actor_ref
        .send(EntityMsg::SetState(EntityState::Switch { is_on: true }))
        .unwrap();
    let changed = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(changed.old_state, EntityState::Switch { is_on: false });
    assert_eq!(changed.new_state, EntityState::Switch { is_on: true });
    sys.shutdown().await;
}

#[tokio::test]
async fn entity_actor_broadcast_multiple_subscribers() {
    let sys = ActorSystem::new();
    let (actor, change_tx) = EntityActor::new(
        make_entity_descriptor("switch", "test"),
        EntityState::Switch { is_on: false },
        null_store(),
    );
    let mut rx1 = change_tx.subscribe();
    let mut rx2 = change_tx.subscribe();
    let actor_ref = sys.spawn(actor);
    actor_ref
        .send(EntityMsg::SetState(EntityState::Switch { is_on: true }))
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert!(rx1.try_recv().is_ok());
    assert!(rx2.try_recv().is_ok());
    sys.shutdown().await;
}

#[tokio::test]
async fn entity_actor_broadcast_old_vs_new() {
    let sys = ActorSystem::new();
    let (actor, change_tx) = EntityActor::new(
        make_entity_descriptor("switch", "test"),
        EntityState::Switch { is_on: true },
        null_store(),
    );
    let mut rx = change_tx.subscribe();
    let actor_ref = sys.spawn(actor);
    actor_ref
        .send(EntityMsg::SetState(EntityState::Switch { is_on: false }))
        .unwrap();
    let evt = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(evt.old_state, EntityState::Switch { is_on: true });
    assert_eq!(evt.new_state, EntityState::Switch { is_on: false });
    sys.shutdown().await;
}

#[tokio::test]
async fn entity_actor_post_stop_sets_unavailable() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc as StdArc;

    struct TrackingUpdater {
        became_unavailable: StdArc<AtomicBool>,
    }
    impl StateUpdater for TrackingUpdater {
        fn update(&self, _id: &EntityId, state: EntityState) {
            if matches!(state, EntityState::Unavailable) {
                self.became_unavailable.store(true, Ordering::SeqCst);
            }
        }
    }

    let flag = StdArc::new(AtomicBool::new(false));
    let store: Arc<dyn StateUpdater> = Arc::new(TrackingUpdater {
        became_unavailable: flag.clone(),
    });

    let sys = ActorSystem::new();
    let (actor, _tx) = EntityActor::new(
        make_entity_descriptor("switch", "test"),
        EntityState::Switch { is_on: true },
        store,
    );
    let actor_ref = sys.spawn(actor);
    actor_ref.send(EntityMsg::Stop).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(flag.load(Ordering::SeqCst));
    sys.shutdown().await;
}

#[tokio::test]
async fn entity_actor_command_set_value_on_number() {
    let sys = ActorSystem::new();
    let (actor, _tx) = EntityActor::new(
        make_entity_descriptor("number", "test"),
        EntityState::Number {
            value: 0.0,
            min: 0.0,
            max: 100.0,
            step: 1.0,
            unit: None,
        },
        null_store(),
    );
    let actor_ref = sys.spawn(actor);
    actor_ref
        .send(EntityMsg::Command(EntityCommand::SetValue(42.0)))
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let state = actor_ref.ask(EntityMsg::GetState).await.unwrap();
    assert!(matches!(state, EntityState::Number { value, .. } if (value - 42.0).abs() < 0.001));
    sys.shutdown().await;
}

#[tokio::test]
async fn entity_actor_command_set_option_on_select() {
    let sys = ActorSystem::new();
    let (actor, _tx) = EntityActor::new(
        make_entity_descriptor("select", "test"),
        EntityState::Select {
            current: "a".to_string(),
            options: vec!["a".to_string(), "b".to_string(), "c".to_string()],
        },
        null_store(),
    );
    let actor_ref = sys.spawn(actor);
    actor_ref
        .send(EntityMsg::Command(EntityCommand::SetOption(
            "b".to_string(),
        )))
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let state = actor_ref.ask(EntityMsg::GetState).await.unwrap();
    assert!(matches!(state, EntityState::Select { current, .. } if current == "b"));
    sys.shutdown().await;
}

#[tokio::test]
async fn entity_actor_command_set_light_brightness() {
    let sys = ActorSystem::new();
    let (actor, _tx) = EntityActor::new(
        make_entity_descriptor("light", "test"),
        EntityState::Light {
            is_on: true,
            brightness: None,
            color_temp: None,
            rgb: None,
            color_mode: None,
        },
        null_store(),
    );
    let actor_ref = sys.spawn(actor);
    actor_ref
        .send(EntityMsg::Command(EntityCommand::SetLightBrightness(0.8)))
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let state = actor_ref.ask(EntityMsg::GetState).await.unwrap();
    assert!(
        matches!(state, EntityState::Light { brightness: Some(b), .. } if (b - 0.8).abs() < 0.001)
    );
    sys.shutdown().await;
}

#[tokio::test]
async fn entity_actor_command_press_button() {
    let sys = ActorSystem::new();
    let (actor, _tx) = EntityActor::new(
        make_entity_descriptor("button", "test"),
        EntityState::Button,
        null_store(),
    );
    let actor_ref = sys.spawn(actor);
    // Press button — state remains Button
    actor_ref
        .send(EntityMsg::Command(EntityCommand::PressButton))
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let state = actor_ref.ask(EntityMsg::GetState).await.unwrap();
    assert_eq!(state, EntityState::Button);
    sys.shutdown().await;
}

// ── DeviceActor tests ──────────────────────────────────────────────────────────

#[tokio::test]
async fn device_actor_add_entity() {
    let sys = ActorSystem::new();
    let registry = EntityRegistry::default();
    let actor = DeviceActor::new(
        make_device_descriptor("dev1", "Device 1"),
        registry.clone(),
        null_store(),
    );
    let device_ref = sys.spawn(actor);
    let descriptor = make_entity_descriptor("switch", "s1");
    let entity_ref = device_ref
        .ask(|tx| DeviceMsg::AddEntity {
            descriptor,
            initial_state: EntityState::Switch { is_on: false },
            reply: tx,
        })
        .await
        .unwrap();
    let state = entity_ref.ask(EntityMsg::GetState).await.unwrap();
    assert_eq!(state, EntityState::Switch { is_on: false });
    assert_eq!(registry.count(), 1);
    sys.shutdown().await;
}

#[tokio::test]
async fn device_actor_remove_entity() {
    let sys = ActorSystem::new();
    let registry = EntityRegistry::default();
    let actor = DeviceActor::new(
        make_device_descriptor("dev2", "Device 2"),
        registry.clone(),
        null_store(),
    );
    let device_ref = sys.spawn(actor);
    let id = EntityId::new("switch", "s2");
    let descriptor = EntityDescriptor {
        entity_id: id.clone(),
        name: "s2".to_string(),
        icon: None,
        device_id: None,
        area_id: None,
        entity_category: EntityCategory::None,
        domain_id: "switch".to_string(),
        feature_set: vec![],
        device_class: None,
    };
    device_ref
        .ask(|tx| DeviceMsg::AddEntity {
            descriptor,
            initial_state: EntityState::Switch { is_on: false },
            reply: tx,
        })
        .await
        .unwrap();
    assert_eq!(registry.count(), 1);
    device_ref
        .send(DeviceMsg::RemoveEntity(id.clone()))
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert!(registry.get(&id).is_none());
    sys.shutdown().await;
}

#[tokio::test]
async fn device_actor_get_info() {
    let sys = ActorSystem::new();
    let actor = DeviceActor::new(
        make_device_descriptor("dev3", "Device 3"),
        EntityRegistry::default(),
        null_store(),
    );
    let device_ref = sys.spawn(actor);
    let info = device_ref.ask(DeviceMsg::GetInfo).await.unwrap();
    assert_eq!(info.name, "Device 3");
    sys.shutdown().await;
}

#[tokio::test]
async fn device_actor_get_entities() {
    let sys = ActorSystem::new();
    let actor = DeviceActor::new(
        make_device_descriptor("dev4", "Device 4"),
        EntityRegistry::default(),
        null_store(),
    );
    let device_ref = sys.spawn(actor);
    device_ref
        .ask(|tx| DeviceMsg::AddEntity {
            descriptor: make_entity_descriptor("switch", "e1"),
            initial_state: EntityState::Switch { is_on: false },
            reply: tx,
        })
        .await
        .unwrap();
    device_ref
        .ask(|tx| DeviceMsg::AddEntity {
            descriptor: make_entity_descriptor("light", "e2"),
            initial_state: EntityState::Light {
                is_on: false,
                brightness: None,
                color_temp: None,
                rgb: None,
                color_mode: None,
            },
            reply: tx,
        })
        .await
        .unwrap();
    let ids = device_ref.ask(DeviceMsg::GetEntities).await.unwrap();
    assert_eq!(ids.len(), 2);
    sys.shutdown().await;
}

#[tokio::test]
async fn device_actor_entity_removed_on_stop() {
    let sys = ActorSystem::new();
    let registry = EntityRegistry::default();
    let actor = DeviceActor::new(
        make_device_descriptor("dev5", "Device 5"),
        registry.clone(),
        null_store(),
    );
    let device_ref = sys.spawn(actor);
    let id = EntityId::new("switch", "s5");
    let desc = EntityDescriptor {
        entity_id: id.clone(),
        name: "s5".to_string(),
        icon: None,
        device_id: None,
        area_id: None,
        entity_category: EntityCategory::None,
        domain_id: "switch".to_string(),
        feature_set: vec![],
        device_class: None,
    };
    device_ref
        .ask(|tx| DeviceMsg::AddEntity {
            descriptor: desc,
            initial_state: EntityState::Switch { is_on: false },
            reply: tx,
        })
        .await
        .unwrap();
    assert_eq!(registry.count(), 1);
    device_ref.send(DeviceMsg::Stop).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(registry.get(&id).is_none());
    sys.shutdown().await;
}

// ── DeviceManagerActor tests ──────────────────────────────────────────────────

#[tokio::test]
async fn device_manager_add_device() {
    let sys = ActorSystem::new();
    let actor = DeviceManagerActor::new(EntityRegistry::default(), null_store());
    let dm_ref = sys.spawn(actor);
    let device_ref = dm_ref
        .ask(|tx| DeviceManagerMsg::AddDevice {
            descriptor: make_device_descriptor("dm1", "DM Device 1"),
            reply: tx,
        })
        .await
        .unwrap();
    let info = device_ref.ask(DeviceMsg::GetInfo).await.unwrap();
    assert_eq!(info.name, "DM Device 1");
    sys.shutdown().await;
}

#[tokio::test]
async fn device_manager_add_device_reuses_existing_actor_and_updates_descriptor() {
    let sys = ActorSystem::new();
    let actor = DeviceManagerActor::new(EntityRegistry::default(), null_store());
    let dm_ref = sys.spawn(actor);

    let first_ref = dm_ref
        .ask(|tx| DeviceManagerMsg::AddDevice {
            descriptor: make_device_descriptor("dm1", "Original Name"),
            reply: tx,
        })
        .await
        .unwrap();
    let second_ref = dm_ref
        .ask(|tx| DeviceManagerMsg::AddDevice {
            descriptor: DeviceDescriptor {
                device_id: DeviceId("dm1".to_string()),
                name: "Updated Name".to_string(),
                model: Some("Model X".to_string()),
                manufacturer: Some("Vendor".to_string()),
                sw_version: Some("2026.3".to_string()),
                area_id: None,
            },
            reply: tx,
        })
        .await
        .unwrap();

    assert_eq!(first_ref.actor_id(), second_ref.actor_id());

    let info = second_ref.ask(DeviceMsg::GetInfo).await.unwrap();
    assert_eq!(info.name, "Updated Name");
    assert_eq!(info.model.as_deref(), Some("Model X"));

    let devices = dm_ref.ask(DeviceManagerMsg::ListDevices).await.unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].name, "Updated Name");

    sys.shutdown().await;
}

#[tokio::test]
async fn device_manager_remove_device() {
    let sys = ActorSystem::new();
    let actor = DeviceManagerActor::new(EntityRegistry::default(), null_store());
    let dm_ref = sys.spawn(actor);
    let id = DeviceId("dm2".to_string());
    dm_ref
        .ask(|tx| DeviceManagerMsg::AddDevice {
            descriptor: DeviceDescriptor {
                device_id: id.clone(),
                name: "DM2".to_string(),
                model: None,
                manufacturer: None,
                sw_version: None,
                area_id: None,
            },
            reply: tx,
        })
        .await
        .unwrap();
    dm_ref
        .send(DeviceManagerMsg::RemoveDevice(id.clone()))
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    let result = dm_ref
        .ask(|tx| DeviceManagerMsg::GetDevice { id, reply: tx })
        .await
        .unwrap();
    assert!(result.is_none());
    sys.shutdown().await;
}

#[tokio::test]
async fn device_manager_get_device() {
    let sys = ActorSystem::new();
    let actor = DeviceManagerActor::new(EntityRegistry::default(), null_store());
    let dm_ref = sys.spawn(actor);
    let id = DeviceId("dm3".to_string());
    dm_ref
        .ask(|tx| DeviceManagerMsg::AddDevice {
            descriptor: DeviceDescriptor {
                device_id: id.clone(),
                name: "DM3".to_string(),
                model: None,
                manufacturer: None,
                sw_version: None,
                area_id: None,
            },
            reply: tx,
        })
        .await
        .unwrap();
    let result = dm_ref
        .ask(|tx| DeviceManagerMsg::GetDevice { id, reply: tx })
        .await
        .unwrap();
    assert!(result.is_some());
    sys.shutdown().await;
}

#[tokio::test]
async fn device_manager_list_devices() {
    let sys = ActorSystem::new();
    let actor = DeviceManagerActor::new(EntityRegistry::default(), null_store());
    let dm_ref = sys.spawn(actor);
    for i in 0..3u32 {
        dm_ref
            .ask(|tx| DeviceManagerMsg::AddDevice {
                descriptor: DeviceDescriptor {
                    device_id: DeviceId(format!("list_dev{i}")),
                    name: format!("Dev {i}"),
                    model: None,
                    manufacturer: None,
                    sw_version: None,
                    area_id: None,
                },
                reply: tx,
            })
            .await
            .unwrap();
    }
    let list = dm_ref.ask(DeviceManagerMsg::ListDevices).await.unwrap();
    assert_eq!(list.len(), 3);
    sys.shutdown().await;
}

#[tokio::test]
async fn device_manager_get_entities_for_device() {
    let sys = ActorSystem::new();
    let registry = EntityRegistry::default();
    let actor = DeviceManagerActor::new(registry.clone(), null_store());
    let dm_ref = sys.spawn(actor);
    let device_id = DeviceId("dm5".to_string());
    let device_ref = dm_ref
        .ask(|tx| DeviceManagerMsg::AddDevice {
            descriptor: DeviceDescriptor {
                device_id: device_id.clone(),
                name: "DM5".to_string(),
                model: None,
                manufacturer: None,
                sw_version: None,
                area_id: None,
            },
            reply: tx,
        })
        .await
        .unwrap();
    device_ref
        .ask(|tx| DeviceMsg::AddEntity {
            descriptor: make_entity_descriptor("switch", "dm5_s1"),
            initial_state: EntityState::Switch { is_on: false },
            reply: tx,
        })
        .await
        .unwrap();
    let ids = dm_ref
        .ask(|tx| DeviceManagerMsg::GetEntitiesForDevice {
            device_id,
            reply: tx,
        })
        .await
        .unwrap();
    assert_eq!(ids.len(), 1);
    sys.shutdown().await;
}
