use rshome_entity::{EntityId, EntityState};
use rshome_state::StateStore;
use std::time::{Duration, SystemTime};

fn make_sensor(v: f64) -> EntityState {
    EntityState::Sensor {
        value: v,
        unit: Some("°C".to_string()),
        attributes: Default::default(),
    }
}

// ── StateStore basic tests ────────────────────────────────────────────────────

#[test]
fn store_update_and_get() {
    let store = StateStore::default();
    let id = EntityId::new("sensor", "temp");
    store.update(&id, make_sensor(21.0));
    assert_eq!(store.get(&id), Some(make_sensor(21.0)));
}

#[test]
fn store_get_missing() {
    let store = StateStore::default();
    let id = EntityId::new("sensor", "missing");
    assert_eq!(store.get(&id), None);
}

#[test]
fn store_list_ids() {
    let store = StateStore::default();
    store.update(&EntityId::new("sensor", "a"), make_sensor(1.0));
    store.update(&EntityId::new("sensor", "b"), make_sensor(2.0));
    let ids = store.list_ids();
    assert_eq!(ids.len(), 2);
}

#[test]
fn store_get_all() {
    let store = StateStore::default();
    store.update(
        &EntityId::new("switch", "s1"),
        EntityState::Switch { is_on: true },
    );
    store.update(
        &EntityId::new("switch", "s2"),
        EntityState::Switch { is_on: false },
    );
    let all = store.get_all();
    assert_eq!(all.len(), 2);
}

#[test]
fn store_count() {
    let store = StateStore::default();
    assert_eq!(store.count(), 0);
    store.update(
        &EntityId::new("light", "l1"),
        EntityState::Light {
            is_on: true,
            brightness: None,
            color_temp: None,
            rgb: None,
            color_mode: None,
        },
    );
    assert_eq!(store.count(), 1);
}

// ── Watch / subscribe tests ───────────────────────────────────────────────────

#[tokio::test]
async fn store_subscribe_fires_on_update() {
    let store = StateStore::default();
    let id = EntityId::new("sensor", "watch_test");
    let mut rx = store.subscribe(&id);
    store.update(&id, make_sensor(30.0));
    // The receiver should have a new value
    tokio::time::timeout(Duration::from_millis(100), rx.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rx.borrow().as_ref().unwrap().clone(), make_sensor(30.0));
}

#[tokio::test]
async fn store_multiple_watchers() {
    let store = StateStore::default();
    let id = EntityId::new("sensor", "multi_watch");
    let mut rx1 = store.subscribe(&id);
    let mut rx2 = store.subscribe(&id);
    store.update(&id, make_sensor(25.0));
    tokio::time::timeout(Duration::from_millis(100), rx1.changed())
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_millis(100), rx2.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rx1.borrow().as_ref().unwrap().clone(), make_sensor(25.0));
    assert_eq!(rx2.borrow().as_ref().unwrap().clone(), make_sensor(25.0));
}

#[test]
fn store_subscribe_initial_none_for_unknown() {
    let store = StateStore::default();
    let id = EntityId::new("sensor", "unknown");
    let rx = store.subscribe(&id);
    // Before any update, value is None
    assert!(rx.borrow().is_none());
}

#[tokio::test]
async fn store_subscribe_gets_initial_value_if_exists() {
    let store = StateStore::default();
    let id = EntityId::new("sensor", "preexisting");
    store.update(&id, make_sensor(10.0));
    let rx = store.subscribe(&id);
    // Should immediately have the current value
    assert_eq!(rx.borrow().as_ref().unwrap().clone(), make_sensor(10.0));
}

// ── Snapshot tests ────────────────────────────────────────────────────────────

#[test]
fn store_snapshot_single() {
    let store = StateStore::default();
    let id = EntityId::new("sensor", "snap1");
    store.update(&id, make_sensor(15.0));
    let snap = store.snapshot(&id).unwrap();
    assert_eq!(snap.entity_id, id);
    assert_eq!(snap.state, make_sensor(15.0));
}

#[test]
fn store_snapshot_all() {
    let store = StateStore::default();
    store.update(&EntityId::new("sensor", "sn1"), make_sensor(1.0));
    store.update(&EntityId::new("sensor", "sn2"), make_sensor(2.0));
    let snaps = store.snapshot_all();
    assert_eq!(snaps.len(), 2);
}

#[test]
fn store_snapshot_last_updated_timestamp() {
    let store = StateStore::default();
    let id = EntityId::new("sensor", "ts1");
    let before = SystemTime::now();
    store.update(&id, make_sensor(99.0));
    let snap = store.snapshot(&id).unwrap();
    let after = SystemTime::now();
    assert!(snap.last_updated >= before);
    assert!(snap.last_updated <= after);
}

// ── StateUpdater trait impl test ──────────────────────────────────────────────

#[test]
fn store_implements_state_updater_trait() {
    use rshome_entity::StateUpdater;
    let store = StateStore::default();
    let id = EntityId::new("switch", "trait_test");
    // Use via trait object
    let updater: &dyn StateUpdater = &store;
    updater.update(&id, EntityState::Switch { is_on: true });
    assert_eq!(store.get(&id), Some(EntityState::Switch { is_on: true }));
}

// ── History tests (feature-gated) ─────────────────────────────────────────────

#[cfg(feature = "history")]
mod history_tests {
    use super::*;
    use rshome_state::HistoryRecorder;

    #[test]
    fn history_record_and_query() {
        let recorder = HistoryRecorder::with_in_memory().unwrap();
        let id = EntityId::new("sensor", "hist1");
        let t1 = SystemTime::now();
        recorder.record(&id, &make_sensor(10.0), t1);
        recorder.record(&id, &make_sensor(20.0), t1 + Duration::from_secs(1));
        let results = recorder.query(&id, t1);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn history_query_time_range() {
        let recorder = HistoryRecorder::with_in_memory().unwrap();
        let id = EntityId::new("sensor", "hist2");
        let t0 = SystemTime::now();
        recorder.record(&id, &make_sensor(1.0), t0);
        recorder.record(&id, &make_sensor(2.0), t0 + Duration::from_secs(5));
        recorder.record(&id, &make_sensor(3.0), t0 + Duration::from_secs(10));
        let t_mid = t0 + Duration::from_secs(3);
        let results = recorder.query(&id, t_mid);
        assert_eq!(results.len(), 2); // only the last two
    }

    #[test]
    fn history_in_memory_backend() {
        let recorder = HistoryRecorder::with_in_memory().unwrap();
        let id = EntityId::new("sensor", "hist3");
        let t = SystemTime::now();
        recorder.record(&id, &make_sensor(42.0), t);
        let results = recorder.query(&id, t);
        assert!(!results.is_empty());
    }
}
