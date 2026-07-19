#[cfg(feature = "history")]
use rshome_entity::{EntityId, EntityState};
#[cfg(feature = "history")]
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(feature = "history")]
const HISTORY_TABLE: redb::TableDefinition<'static, u128, &[u8]> =
    redb::TableDefinition::new("history");

#[cfg(feature = "history")]
pub struct HistoryRecorder {
    db: redb::Database,
}

#[cfg(feature = "history")]
impl HistoryRecorder {
    pub fn new(path: &std::path::Path) -> Result<Self, redb::Error> {
        let db = redb::Database::create(path)?;
        Ok(Self { db })
    }

    pub fn with_in_memory() -> Result<Self, redb::Error> {
        let db =
            redb::Database::builder().create_with_backend(redb::backends::InMemoryBackend::new())?;
        Ok(Self { db })
    }

    pub fn record(&self, id: &EntityId, state: &EntityState, at: SystemTime) {
        let ts = at
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_micros();
        let key = ts;
        // Encode as JSON bytes containing entity_id and state
        let json = serde_json::to_vec(&(id, state)).unwrap_or_default();
        if let Ok(write_txn) = self.db.begin_write() {
            if let Ok(mut table) = write_txn.open_table(HISTORY_TABLE) {
                let _ = table.insert(key, json.as_slice());
            }
            let _ = write_txn.commit();
        }
    }

    pub fn query(&self, id: &EntityId, since: SystemTime) -> Vec<(SystemTime, EntityState)> {
        let since_ts = since
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_micros();
        let Ok(read_txn) = self.db.begin_read() else {
            return vec![];
        };
        let Ok(table) = read_txn.open_table(HISTORY_TABLE) else {
            return vec![];
        };
        let mut results = Vec::new();
        if let Ok(iter) = table.range(since_ts..) {
            for item in iter.flatten() {
                let bytes = item.1.value().to_vec();
                if let Ok((entry_id, state)) =
                    serde_json::from_slice::<(EntityId, EntityState)>(&bytes)
                {
                    if &entry_id == id {
                        let ts_micros = item.0.value();
                        let t = UNIX_EPOCH + Duration::from_micros(ts_micros as u64);
                        results.push((t, state));
                    }
                }
            }
        }
        results
    }
}
