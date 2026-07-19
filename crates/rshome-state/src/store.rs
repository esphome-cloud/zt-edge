use crate::snapshot::StateSnapshot;
use parking_lot::RwLock;
use rshome_entity::{EntityId, EntityState, StateUpdater};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::watch;

type Inner = HashMap<EntityId, (EntityState, SystemTime, watch::Sender<Option<EntityState>>)>;

#[derive(Clone, Default)]
pub struct StateStore {
    pub(crate) inner: Arc<RwLock<Inner>>,
}

impl StateStore {
    pub fn update(&self, id: &EntityId, state: EntityState) {
        let mut map = self.inner.write();
        if let Some(entry) = map.get_mut(id) {
            entry.0 = state.clone();
            entry.1 = SystemTime::now();
            let _ = entry.2.send(Some(state));
        } else {
            let (tx, _rx) = watch::channel(Some(state.clone()));
            map.insert(id.clone(), (state, SystemTime::now(), tx));
        }
    }

    #[must_use]
    pub fn get(&self, id: &EntityId) -> Option<EntityState> {
        self.inner.read().get(id).map(|(s, _, _)| s.clone())
    }

    pub fn get_all(&self) -> HashMap<EntityId, EntityState> {
        self.inner
            .read()
            .iter()
            .map(|(k, (s, _, _))| (k.clone(), s.clone()))
            .collect()
    }

    pub fn subscribe(&self, id: &EntityId) -> watch::Receiver<Option<EntityState>> {
        let mut map = self.inner.write();
        if let Some(entry) = map.get(id) {
            entry.2.subscribe()
        } else {
            let (tx, rx) = watch::channel(None);
            // Insert a placeholder entry with Unavailable state until first update
            map.insert(
                id.clone(),
                (EntityState::Unavailable, SystemTime::now(), tx),
            );
            rx
        }
    }

    pub fn list_ids(&self) -> Vec<EntityId> {
        self.inner.read().keys().cloned().collect()
    }

    pub fn count(&self) -> usize {
        self.inner.read().len()
    }

    pub fn last_updated(&self, id: &EntityId) -> Option<SystemTime> {
        self.inner.read().get(id).map(|(_, t, _)| *t)
    }

    #[must_use]
    pub fn snapshot(&self, id: &EntityId) -> Option<StateSnapshot> {
        let map = self.inner.read();
        map.get(id).map(|(state, t, _)| StateSnapshot {
            entity_id: id.clone(),
            state: state.clone(),
            last_updated: *t,
        })
    }

    pub fn snapshot_all(&self) -> Vec<StateSnapshot> {
        let map = self.inner.read();
        map.iter()
            .map(|(id, (state, t, _))| StateSnapshot {
                entity_id: id.clone(),
                state: state.clone(),
                last_updated: *t,
            })
            .collect()
    }
}

impl StateStore {
    /// Persist all entity states as a JSON snapshot to `path`.
    ///
    /// Format: JSON object mapping `entity_id` → serialized `EntityState`.
    /// This is opt-in; not called automatically on shutdown.
    pub fn persist_snapshot(&self, path: &std::path::Path) -> Result<(), std::io::Error> {
        let map = self.inner.read();
        let snap: std::collections::HashMap<String, serde_json::Value> = map
            .iter()
            .filter_map(|(id, (state, _, _))| {
                serde_json::to_value(state)
                    .ok()
                    .map(|v| (id.to_string(), v))
            })
            .collect();
        let json = serde_json::to_string_pretty(&snap).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }

    /// Restore entity states from a JSON snapshot previously written by `persist_snapshot`.
    ///
    /// Each entry in the snapshot is deserialized and applied via `update()`.
    pub fn restore_snapshot(&self, path: &std::path::Path) -> Result<usize, std::io::Error> {
        let json = std::fs::read_to_string(path)?;
        let snap: std::collections::HashMap<String, serde_json::Value> =
            serde_json::from_str(&json)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let mut count = 0;
        for (id_str, value) in snap {
            let state: EntityState = match serde_json::from_value(value) {
                Ok(s) => s,
                Err(_) => continue,
            };
            // Parse entity_id from "domain.object_id" format
            let entity_id = if let Some((domain, object_id)) = id_str.split_once('.') {
                EntityId::new(domain, object_id)
            } else {
                continue;
            };
            self.update(&entity_id, state);
            count += 1;
        }
        Ok(count)
    }
}

impl StateUpdater for StateStore {
    fn update(&self, id: &EntityId, state: EntityState) {
        StateStore::update(self, id, state);
    }
}
