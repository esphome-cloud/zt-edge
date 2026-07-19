use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io;
use std::io::Write as _;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use rshome_actor::{ActorError, ActorRef, ActorSystem};
use rshome_device_link::{
    ConnectedDevice, DeviceLinkLimits, DeviceLinkManagerActor, DeviceLinkManagerMsg, SessionStatus,
    MAX_ENTITIES_PER_DEVICE,
};
use rshome_entity::{
    DeviceDescriptor, DeviceId, DeviceManagerActor, DeviceManagerMsg, DeviceMsg, DomainRegistry,
    EntityActor, EntityCategory, EntityDescriptor, EntityId, EntityMsg, EntityRegistry,
    EntityState, StateUpdater,
};
use rshome_state::StateStore;
use rshome_svc::{ServiceMsg, ServiceRegistryActor};
use serde::{Deserialize, Serialize};

use crate::{RshomeHaMcp, RuntimeConfig};

const SNAPSHOT_VERSION: u8 = 3;
const SUPPORTED_SNAPSHOT_VERSIONS: [u8; 2] = [2, SNAPSHOT_VERSION];

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuntimeSnapshot {
    version: u8,
    devices: Vec<DeviceSnapshot>,
    orphan_entities: Vec<EntitySnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeviceSnapshot {
    descriptor: DeviceDescriptor,
    entities: Vec<EntitySnapshot>,
    #[serde(default)]
    link: Option<ConnectedDevice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EntitySnapshot {
    descriptor: EntityDescriptor,
    state: EntityState,
}

/// Running rshome-ha runtime owned by the standalone MCP daemon.
pub struct RshomeHaRuntime {
    pub server: RshomeHaMcp,
    pub system: ActorSystem,
    pub state_store: Arc<StateStore>,
    pub entity_registry: EntityRegistry,
    pub device_manager: ActorRef<DeviceManagerMsg>,
    pub service_registry: ActorRef<ServiceMsg>,
    pub device_link: Option<ActorRef<DeviceLinkManagerMsg>>,
    pub runtime_config: RuntimeConfig,
}

impl RshomeHaRuntime {
    /// Build the default device-ingest runtime used by `rshome-ha-mcp`.
    ///
    /// This runtime wires entity, service, state, and optional mDNS device-link support.
    /// Workflow, WASM, and native API subsystems are intentionally left detached.
    pub fn new_device_ingest(config: RuntimeConfig) -> Self {
        let system = ActorSystem::new();
        let entity_registry = EntityRegistry::default();
        let state_store = Arc::new(StateStore::default());
        let state_updater: Arc<dyn StateUpdater> = state_store.clone();
        let device_link_limits = runtime_config_to_device_link_limits(&config);
        let device_manager = system.spawn(DeviceManagerActor::new(
            entity_registry.clone(),
            state_updater,
        ));
        let service_registry = system.spawn(ServiceRegistryActor::new(
            entity_registry.clone(),
            Some(device_manager.clone()),
        ));

        let device_link = if config.mdns_enabled {
            Some(system.spawn(DeviceLinkManagerActor::new(
                device_manager.clone(),
                entity_registry.clone(),
                state_store.clone(),
                device_link_limits,
            )))
        } else {
            None
        };

        let mut server = RshomeHaMcp::new(
            entity_registry.clone(),
            state_store.clone(),
            service_registry.clone(),
            device_manager.clone(),
            None,
        )
        .with_config(config.clone());

        if let Some(ref device_link_manager) = device_link {
            server = server.with_device_link(device_link_manager.clone());
        }

        Self {
            server,
            system,
            state_store,
            entity_registry,
            device_manager,
            service_registry,
            device_link,
            runtime_config: config,
        }
    }

    pub async fn restore_snapshot(&self, path: &Path) -> Result<usize, io::Error> {
        let json = std::fs::read_to_string(path)?;

        // Sniff: if top-level object has a "version" key, treat as structured snapshot
        // and fail hard on parse/version errors instead of falling through to legacy.
        let is_structured = serde_json::from_str::<serde_json::Value>(&json)
            .ok()
            .and_then(|v| v.get("version").cloned())
            .is_some();

        if is_structured {
            let snapshot: RuntimeSnapshot = serde_json::from_str(&json)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            self.validate_snapshot(&snapshot)?;
            return self.restore_structured_snapshot(snapshot).await;
        }

        self.restore_legacy_snapshot(&json).await
    }

    pub async fn persist_snapshot(&self, path: &Path) -> Result<(), io::Error> {
        let snapshot = self.build_snapshot().await?;
        let json = serde_json::to_string_pretty(&snapshot).map_err(io::Error::other)?;
        atomic_write(path, json.as_bytes())
    }

    pub fn stop_device_ingest(&self) {
        if let Some(device_link_manager) = &self.device_link {
            let _ = device_link_manager.send(DeviceLinkManagerMsg::Stop);
        }
        let _ = self.device_manager.send(DeviceManagerMsg::Stop);
    }

    pub async fn shutdown(&self) {
        self.stop_device_ingest();
        tokio::time::sleep(Duration::from_millis(50)).await;
        self.system.shutdown().await;
    }

    async fn build_snapshot(&self) -> Result<RuntimeSnapshot, io::Error> {
        let link_snapshots = self.snapshot_imported_links().await?;
        let mut device_descriptors = self
            .device_manager
            .ask(DeviceManagerMsg::ListDevices)
            .await
            .map_err(actor_error_to_io)?;
        device_descriptors.sort_by(|left, right| left.device_id.0.cmp(&right.device_id.0));

        let mut seen_entities = HashSet::new();
        let mut devices = Vec::with_capacity(device_descriptors.len());

        for descriptor in device_descriptors {
            let entities = self
                .snapshot_device_entities(&descriptor, &mut seen_entities)
                .await?;
            devices.push(DeviceSnapshot {
                link: link_snapshots.get(&descriptor.device_id).cloned(),
                descriptor,
                entities,
            });
        }

        let orphan_entities = self.snapshot_orphan_entities(&seen_entities).await?;

        Ok(RuntimeSnapshot {
            version: SNAPSHOT_VERSION,
            devices,
            orphan_entities,
        })
    }

    async fn snapshot_device_entities(
        &self,
        descriptor: &DeviceDescriptor,
        seen_entities: &mut HashSet<EntityId>,
    ) -> Result<Vec<EntitySnapshot>, io::Error> {
        let device_ref = self
            .device_manager
            .ask(|reply| DeviceManagerMsg::GetDevice {
                id: descriptor.device_id.clone(),
                reply,
            })
            .await
            .map_err(actor_error_to_io)?;

        let mut entity_ids = if let Some(device_ref) = device_ref {
            device_ref
                .ask(DeviceMsg::GetEntities)
                .await
                .map_err(actor_error_to_io)?
        } else {
            Vec::new()
        };
        entity_ids.sort_by(|left, right| left.0.cmp(&right.0));

        let mut entities = Vec::with_capacity(entity_ids.len());
        for entity_id in entity_ids {
            if let Some(snapshot) = self
                .snapshot_entity(&entity_id, Some(descriptor.device_id.clone()))
                .await?
            {
                seen_entities.insert(entity_id);
                entities.push(snapshot);
            }
        }

        Ok(entities)
    }

    async fn snapshot_orphan_entities(
        &self,
        seen_entities: &HashSet<EntityId>,
    ) -> Result<Vec<EntitySnapshot>, io::Error> {
        let mut orphan_ids: HashSet<EntityId> =
            self.entity_registry.list_all().into_iter().collect();
        orphan_ids.extend(self.state_store.list_ids());

        let mut orphan_ids: Vec<EntityId> = orphan_ids
            .into_iter()
            .filter(|entity_id| !seen_entities.contains(entity_id))
            .collect();
        orphan_ids.sort_by(|left, right| left.0.cmp(&right.0));

        let mut orphans = Vec::with_capacity(orphan_ids.len());
        for entity_id in orphan_ids {
            if let Some(snapshot) = self.snapshot_entity(&entity_id, None).await? {
                orphans.push(snapshot);
            }
        }

        Ok(orphans)
    }

    async fn snapshot_entity(
        &self,
        entity_id: &EntityId,
        fallback_device_id: Option<DeviceId>,
    ) -> Result<Option<EntitySnapshot>, io::Error> {
        let Some(state) = self.load_entity_state(entity_id).await? else {
            return Ok(None);
        };

        let descriptor = self
            .entity_registry
            .get_descriptor(entity_id)
            .unwrap_or_else(|| synthesize_entity_descriptor(entity_id, fallback_device_id));

        Ok(Some(EntitySnapshot { descriptor, state }))
    }

    async fn load_entity_state(
        &self,
        entity_id: &EntityId,
    ) -> Result<Option<EntityState>, io::Error> {
        if let Some(state) = self.state_store.get(entity_id) {
            return Ok(Some(state));
        }

        if let Some(entity_ref) = self.entity_registry.get(entity_id) {
            return entity_ref
                .ask(EntityMsg::GetState)
                .await
                .map(Some)
                .map_err(actor_error_to_io);
        }

        Ok(None)
    }

    async fn snapshot_imported_links(
        &self,
    ) -> Result<HashMap<DeviceId, ConnectedDevice>, io::Error> {
        let Some(device_link) = &self.device_link else {
            return Ok(HashMap::new());
        };

        let links = device_link
            .ask(DeviceLinkManagerMsg::ListConnected)
            .await
            .map_err(actor_error_to_io)?;
        Ok(links
            .into_iter()
            .map(|connected| (connected.device_id.clone(), connected))
            .collect())
    }

    fn validate_snapshot(&self, snapshot: &RuntimeSnapshot) -> Result<(), io::Error> {
        if !SUPPORTED_SNAPSHOT_VERSIONS.contains(&snapshot.version) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unsupported snapshot version {}; supported versions are {:?}",
                    snapshot.version, SUPPORTED_SNAPSHOT_VERSIONS
                ),
            ));
        }

        let total_devices = snapshot.devices.len();
        if total_devices > self.runtime_config.max_devices {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "snapshot contains {total_devices} devices but max_devices is {}",
                    self.runtime_config.max_devices
                ),
            ));
        }

        let total_entities = snapshot
            .devices
            .iter()
            .map(|device| device.entities.len())
            .sum::<usize>()
            + snapshot.orphan_entities.len();
        if total_entities > self.runtime_config.max_entities {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "snapshot contains {total_entities} entities but max_entities is {}",
                    self.runtime_config.max_entities
                ),
            ));
        }

        let per_device_limit = self
            .runtime_config
            .max_entities
            .min(MAX_ENTITIES_PER_DEVICE);
        if let Some(too_large) = snapshot
            .devices
            .iter()
            .find(|device| device.entities.len() > per_device_limit)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "device {} exports {} entities but per-device cap is {}",
                    too_large.descriptor.device_id,
                    too_large.entities.len(),
                    per_device_limit
                ),
            ));
        }

        Ok(())
    }

    async fn restore_structured_snapshot(
        &self,
        snapshot: RuntimeSnapshot,
    ) -> Result<usize, io::Error> {
        let mut restored = 0usize;

        for device in snapshot.devices {
            let DeviceSnapshot {
                descriptor,
                entities,
                link,
            } = device;
            let device_ref = self
                .device_manager
                .ask(|reply| DeviceManagerMsg::AddDevice { descriptor, reply })
                .await
                .map_err(actor_error_to_io)?;

            for entity in entities {
                self.restore_device_entity(&device_ref, entity).await?;
                restored += 1;
            }

            if let (Some(mut link), Some(device_link)) = (link, &self.device_link) {
                // Normalize status to offline — the snapshot may have captured an
                // Active or Handshaking session that no longer exists post-restart.
                link.status = SessionStatus::Unavailable;
                device_link
                    .send(DeviceLinkManagerMsg::SeedRestoredDevice(link))
                    .map_err(actor_error_to_io)?;
            }
        }

        for entity in snapshot.orphan_entities {
            self.restore_orphan_entity(entity);
            restored += 1;
        }

        Ok(restored)
    }

    async fn restore_device_entity(
        &self,
        device_ref: &ActorRef<DeviceMsg>,
        entity: EntitySnapshot,
    ) -> Result<(), io::Error> {
        device_ref
            .ask(|reply| DeviceMsg::AddEntity {
                descriptor: entity.descriptor,
                initial_state: entity.state,
                reply,
            })
            .await
            .map_err(actor_error_to_io)
            .map(|_| ())
    }

    async fn restore_legacy_snapshot(&self, json: &str) -> Result<usize, io::Error> {
        let snapshot: HashMap<String, serde_json::Value> = serde_json::from_str(json)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if snapshot.len() > self.runtime_config.max_entities {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "snapshot contains {} entities but max_entities is {}",
                    snapshot.len(),
                    self.runtime_config.max_entities
                ),
            ));
        }
        let mut restored = 0usize;

        for (entity_id_str, value) in snapshot {
            let state: EntityState = match serde_json::from_value(value) {
                Ok(state) => state,
                Err(_) => continue,
            };

            let Some((domain, object_id)) = entity_id_str.split_once('.') else {
                continue;
            };

            let entity_id = EntityId::new(domain, object_id);
            let descriptor = synthesize_entity_descriptor(&entity_id, None);
            self.restore_orphan_entity(EntitySnapshot { descriptor, state });
            restored += 1;
        }

        Ok(restored)
    }

    fn restore_orphan_entity(&self, entity: EntitySnapshot) {
        self.state_store
            .update(&entity.descriptor.entity_id, entity.state.clone());
        let (actor, _) = EntityActor::new(
            entity.descriptor.clone(),
            entity.state,
            self.state_store.clone(),
        );
        let actor_ref = self.system.spawn(actor);
        self.entity_registry
            .register(entity.descriptor.entity_id.clone(), actor_ref);
        self.entity_registry.register_descriptor(entity.descriptor);
    }
}

fn runtime_config_to_device_link_limits(config: &RuntimeConfig) -> DeviceLinkLimits {
    DeviceLinkLimits {
        max_devices: config.max_devices,
        max_entities: config.max_entities,
        max_connections_per_device: config.max_connections_per_device,
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), io::Error> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("snapshot path has no parent directory: {}", path.display()),
        )
    })?;
    fs::create_dir_all(parent)?;

    let file_name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("snapshot path has no file name: {}", path.display()),
        )
    })?;
    let temp_path = parent.join(format!(
        ".{}.tmp-{}",
        file_name.to_string_lossy(),
        uuid::Uuid::new_v4().simple()
    ));

    let mut file = File::options()
        .create_new(true)
        .write(true)
        .open(&temp_path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);

    fs::rename(&temp_path, path)?;
    sync_parent_dir(parent)?;
    Ok(())
}

fn sync_parent_dir(path: &Path) -> Result<(), io::Error> {
    let dir = File::open(path)?;
    dir.sync_all()
}

fn actor_error_to_io(error: ActorError) -> io::Error {
    io::Error::other(error.to_string())
}

fn synthesize_entity_descriptor(
    entity_id: &EntityId,
    device_id: Option<DeviceId>,
) -> EntityDescriptor {
    let (domain_id, feature_set) = DomainRegistry::built_in()
        .resolve_wire_type(entity_id.domain())
        .map(|(domain_id, feature_set)| (domain_id.to_string(), feature_set))
        .unwrap_or_else(|| (entity_id.domain().to_string(), Vec::new()));

    EntityDescriptor {
        entity_id: entity_id.clone(),
        name: entity_id.object_id().replace('_', " "),
        icon: None,
        device_id,
        area_id: None,
        entity_category: EntityCategory::None,
        domain_id,
        feature_set,
        device_class: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::handler::server::tool::Parameters;
    use rshome_device_link::{ConnectedDevice, DeviceLinkManagerMsg, SessionStatus};
    use rshome_entity::{DeviceDescriptor, EntityState};

    use crate::{HaDeviceLinksStatusInput, HaEntitiesListInput};

    fn temp_path(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rhmcprt-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    fn make_device_descriptor(device_id: &str, name: &str) -> DeviceDescriptor {
        DeviceDescriptor {
            device_id: rshome_entity::DeviceId(device_id.to_string()),
            name: name.to_string(),
            model: Some("test-model".into()),
            manufacturer: Some("test-manufacturer".into()),
            sw_version: Some("1.0.0".into()),
            area_id: None,
        }
    }

    fn make_entity_descriptor(
        device_id: &rshome_entity::DeviceId,
        domain: &str,
        object_id: &str,
    ) -> EntityDescriptor {
        let (_, feature_set) = DomainRegistry::built_in()
            .resolve_wire_type(domain)
            .unwrap_or((domain, Vec::new()));

        EntityDescriptor {
            entity_id: EntityId::new(domain, object_id),
            name: object_id.to_string(),
            icon: None,
            device_id: Some(device_id.clone()),
            area_id: None,
            entity_category: EntityCategory::None,
            domain_id: domain.to_string(),
            feature_set,
            device_class: None,
        }
    }

    fn make_connected_device(
        canonical_id: &str,
        provisional_name: &str,
        status: SessionStatus,
        entity_count: usize,
    ) -> ConnectedDevice {
        ConnectedDevice {
            device_id: DeviceId(canonical_id.to_string()),
            discovered: rshome_device_link::DiscoveredDevice {
                device_id: DeviceId(format!("esphome-host:{provisional_name}")),
                service_fullname: format!("{provisional_name}._esphomelib._tcp.local."),
                hostname: provisional_name.to_string(),
                ip: "127.0.0.1".parse().unwrap(),
                port: 6053,
                name: provisional_name.to_string(),
                version: "2026.03".to_string(),
                friendly_name: Some("Living Room".to_string()),
                first_seen_at: std::time::SystemTime::UNIX_EPOCH,
                last_seen_at: std::time::SystemTime::now(),
                is_stale: false,
            },
            name: "Living Room".to_string(),
            friendly_name: Some("Living Room".to_string()),
            mac_address: Some("AA:BB:CC:DD:EE:FF".to_string()),
            model: Some("ESP32".to_string()),
            sw_version: Some("2026.03".to_string()),
            status,
            entity_count,
            auth_mode: Some("cleartext".to_string()),
            last_error: None,
        }
    }

    fn text_content(result: &rmcp::model::CallToolResult) -> &str {
        result
            .content
            .first()
            .and_then(|content| content.raw.as_text())
            .map(|text| text.text.as_str())
            .expect("expected text content")
    }

    #[tokio::test]
    async fn device_ingest_runtime_attaches_device_link_by_default() {
        let runtime = RshomeHaRuntime::new_device_ingest(RuntimeConfig::default());

        assert!(runtime.device_link.is_some());
        assert!(runtime
            .server
            .list_tool_names()
            .contains(&"ha.devices.list".to_string()));
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn device_ingest_runtime_can_disable_mdns() {
        let runtime = RshomeHaRuntime::new_device_ingest(RuntimeConfig {
            mdns_enabled: false,
            ..RuntimeConfig::default()
        });

        assert!(runtime.device_link.is_none());
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn snapshot_roundtrip_restores_enumerable_device_and_entity_views() {
        let path = temp_path("state.json");

        let runtime = RshomeHaRuntime::new_device_ingest(RuntimeConfig {
            mdns_enabled: false,
            ..RuntimeConfig::default()
        });
        let device = make_device_descriptor("dev-1", "Living Room");
        let entity = make_entity_descriptor(&device.device_id, "switch", "lamp");
        let device_ref = runtime
            .device_manager
            .ask(|reply| DeviceManagerMsg::AddDevice {
                descriptor: device.clone(),
                reply,
            })
            .await
            .unwrap();
        device_ref
            .ask(|reply| DeviceMsg::AddEntity {
                descriptor: entity.clone(),
                initial_state: EntityState::Switch { is_on: true },
                reply,
            })
            .await
            .unwrap();

        runtime.persist_snapshot(&path).await.unwrap();
        runtime.shutdown().await;

        let restored_runtime = RshomeHaRuntime::new_device_ingest(RuntimeConfig {
            mdns_enabled: false,
            ..RuntimeConfig::default()
        });
        let restored = restored_runtime.restore_snapshot(&path).await.unwrap();
        assert_eq!(restored, 1);

        let devices_json: serde_json::Value = serde_json::from_str(text_content(
            &restored_runtime.server.devices_list().await.unwrap(),
        ))
        .unwrap();
        assert_eq!(devices_json.as_array().unwrap().len(), 1);
        assert_eq!(devices_json[0]["device_id"], "dev-1");

        let entities_json: serde_json::Value = serde_json::from_str(text_content(
            &restored_runtime
                .server
                .entities_list(Parameters(HaEntitiesListInput {
                    domain: None,
                    device_id: None,
                    limit: None,
                }))
                .await
                .unwrap(),
        ))
        .unwrap();
        assert_eq!(entities_json.as_array().unwrap().len(), 1);
        assert_eq!(entities_json[0]["entity_id"], "switch.lamp");

        let config_json: serde_json::Value = serde_json::from_str(text_content(
            &restored_runtime.server.config_get().await.unwrap(),
        ))
        .unwrap();
        assert_eq!(config_json["current_entity_count"], 1);
        assert_eq!(config_json["current_state_count"], 1);
        assert!(matches!(
            restored_runtime.state_store.get(&entity.entity_id),
            Some(EntityState::Switch { is_on: true })
        ));

        restored_runtime.shutdown().await;
        std::fs::remove_file(&path).ok();
        std::fs::remove_dir(path.parent().unwrap()).ok();
    }

    #[tokio::test]
    async fn legacy_snapshot_restore_rebuilds_entity_registry_view() {
        let path = temp_path("legacy-state.json");
        let legacy_snapshot = serde_json::json!({
            "sensor.outdoor": serde_json::to_value(EntityState::Sensor {
                value: 21.5,
                unit: Some("C".into()),
                attributes: std::collections::HashMap::new(),
            }).unwrap(),
        });
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&legacy_snapshot).unwrap(),
        )
        .unwrap();

        let runtime = RshomeHaRuntime::new_device_ingest(RuntimeConfig {
            mdns_enabled: false,
            ..RuntimeConfig::default()
        });
        let restored = runtime.restore_snapshot(&path).await.unwrap();
        assert_eq!(restored, 1);

        let entities_json: serde_json::Value = serde_json::from_str(text_content(
            &runtime
                .server
                .entities_list(Parameters(HaEntitiesListInput {
                    domain: None,
                    device_id: None,
                    limit: None,
                }))
                .await
                .unwrap(),
        ))
        .unwrap();
        assert_eq!(entities_json.as_array().unwrap().len(), 1);
        assert_eq!(entities_json[0]["entity_id"], "sensor.outdoor");

        let config_json: serde_json::Value =
            serde_json::from_str(text_content(&runtime.server.config_get().await.unwrap()))
                .unwrap();
        assert_eq!(config_json["current_entity_count"], 1);
        assert_eq!(config_json["current_state_count"], 1);

        runtime.shutdown().await;
        std::fs::remove_file(&path).ok();
        std::fs::remove_dir(path.parent().unwrap()).ok();
    }

    #[tokio::test]
    async fn snapshot_roundtrip_restores_imported_device_provenance() {
        let path = temp_path("imported-state.json");

        let runtime = RshomeHaRuntime::new_device_ingest(RuntimeConfig::default());
        let device = make_device_descriptor("esphome:aabbccddeeff", "Living Room");
        let entity = make_entity_descriptor(&device.device_id, "switch", "lamp");
        let device_ref = runtime
            .device_manager
            .ask(|reply| DeviceManagerMsg::AddDevice {
                descriptor: device.clone(),
                reply,
            })
            .await
            .unwrap();
        device_ref
            .ask(|reply| DeviceMsg::AddEntity {
                descriptor: entity.clone(),
                initial_state: EntityState::Switch { is_on: true },
                reply,
            })
            .await
            .unwrap();

        runtime
            .device_link
            .as_ref()
            .unwrap()
            .send(DeviceLinkManagerMsg::SeedRestoredDevice(
                make_connected_device(
                    &device.device_id.0,
                    "living-room",
                    SessionStatus::Unavailable,
                    1,
                ),
            ))
            .unwrap();

        runtime.persist_snapshot(&path).await.unwrap();
        runtime.shutdown().await;

        let restored_runtime = RshomeHaRuntime::new_device_ingest(RuntimeConfig::default());
        restored_runtime.restore_snapshot(&path).await.unwrap();

        let devices_json: serde_json::Value = serde_json::from_str(text_content(
            &restored_runtime.server.devices_list().await.unwrap(),
        ))
        .unwrap();
        assert_eq!(devices_json[0]["origin"], "imported");
        assert_eq!(devices_json[0]["session_status"], "disconnected");

        let link_json: serde_json::Value = serde_json::from_str(text_content(
            &restored_runtime
                .server
                .device_links_status(Parameters(HaDeviceLinksStatusInput {
                    device_id: device.device_id.to_string(),
                }))
                .await
                .unwrap(),
        ))
        .unwrap();
        assert_eq!(link_json["device_id"], device.device_id.to_string());
        assert_eq!(link_json["hostname"], "living-room");

        restored_runtime.shutdown().await;
        std::fs::remove_file(&path).ok();
        std::fs::remove_dir(path.parent().unwrap()).ok();
    }

    #[tokio::test]
    async fn restore_snapshot_rejects_unsupported_structured_version() {
        let path = temp_path("unsupported-version.json");
        std::fs::write(
            &path,
            serde_json::json!({
                "version": 99,
                "devices": [],
                "orphan_entities": [],
            })
            .to_string(),
        )
        .unwrap();

        let runtime = RshomeHaRuntime::new_device_ingest(RuntimeConfig::default());
        let error = runtime.restore_snapshot(&path).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("unsupported snapshot version"));

        runtime.shutdown().await;
        std::fs::remove_file(&path).ok();
        std::fs::remove_dir(path.parent().unwrap()).ok();
    }

    #[tokio::test]
    async fn restore_normalizes_imported_device_status_to_unavailable() {
        let path = temp_path("active-status.json");

        // Build a snapshot with an Active imported device.
        let snapshot = serde_json::json!({
            "version": 3,
            "devices": [{
                "descriptor": {
                    "device_id": "esphome:aabbccddeeff",
                    "name": "Living Room",
                    "model": "ESP32",
                    "manufacturer": null,
                    "sw_version": "2026.03",
                    "area_id": null,
                },
                "entities": [],
                "link": {
                    "device_id": "esphome:aabbccddeeff",
                    "discovered": {
                        "device_id": "esphome-host:living-room",
                        "service_fullname": "living-room._esphomelib._tcp.local.",
                        "hostname": "living-room",
                        "ip": "192.168.1.10",
                        "port": 6053,
                        "name": "living-room",
                        "version": "2026.03",
                        "friendly_name": "Living Room",
                        "first_seen_at": { "secs_since_epoch": 0, "nanos_since_epoch": 0 },
                        "last_seen_at": { "secs_since_epoch": 0, "nanos_since_epoch": 0 },
                        "is_stale": false,
                    },
                    "name": "Living Room",
                    "friendly_name": "Living Room",
                    "mac_address": "AA:BB:CC:DD:EE:FF",
                    "model": "ESP32",
                    "sw_version": "2026.03",
                    "status": "Active",
                    "entity_count": 0,
                    "auth_mode": "cleartext",
                    "last_error": null,
                },
            }],
            "orphan_entities": [],
        });
        std::fs::write(&path, snapshot.to_string()).unwrap();

        let runtime = RshomeHaRuntime::new_device_ingest(RuntimeConfig::default());
        runtime.restore_snapshot(&path).await.unwrap();

        // Verify the restored device is normalized to Unavailable
        // (the post-restore state), not Active. The endpoint
        // serializes the raw `SessionStatus` enum so the JSON
        // literal matches the variant name (`"Unavailable"`) — not
        // the user-friendly "disconnected" string that the
        // `ha.device_links.list` enumeration would map to. The
        // invariant the test asserts is the same one the function
        // name claims: "normalize imported device status to
        // unavailable."
        let status_json: serde_json::Value = serde_json::from_str(text_content(
            &runtime
                .server
                .device_links_status(Parameters(HaDeviceLinksStatusInput {
                    device_id: "esphome:aabbccddeeff".to_string(),
                }))
                .await
                .unwrap(),
        ))
        .unwrap();
        assert_eq!(
            status_json["status"], "Unavailable",
            "restored device should be normalized to Unavailable (SessionStatus enum variant); got {status_json:#?}"
        );

        runtime.shutdown().await;
        std::fs::remove_file(&path).ok();
        std::fs::remove_dir(path.parent().unwrap()).ok();
    }

    #[tokio::test]
    async fn restore_rejects_malformed_structured_snapshot() {
        let path = temp_path("malformed-structured.json");
        // Has "version" key so it's detected as structured, but "devices" is wrong type.
        std::fs::write(
            &path,
            r#"{"version": 3, "devices": "not-an-array", "orphan_entities": []}"#,
        )
        .unwrap();

        let runtime = RshomeHaRuntime::new_device_ingest(RuntimeConfig::default());
        let error = runtime.restore_snapshot(&path).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);

        runtime.shutdown().await;
        std::fs::remove_file(&path).ok();
        std::fs::remove_dir(path.parent().unwrap()).ok();
    }

    #[tokio::test]
    async fn validate_snapshot_rejects_excess_total_devices() {
        let runtime = RshomeHaRuntime::new_device_ingest(RuntimeConfig {
            mdns_enabled: false,
            max_devices: 2,
            ..RuntimeConfig::default()
        });

        // 3 devices (mix of local and imported) exceeds max_devices=2.
        let snapshot = RuntimeSnapshot {
            version: SNAPSHOT_VERSION,
            devices: vec![
                DeviceSnapshot {
                    descriptor: make_device_descriptor("dev-local-1", "Local 1"),
                    entities: vec![],
                    link: None,
                },
                DeviceSnapshot {
                    descriptor: make_device_descriptor("dev-local-2", "Local 2"),
                    entities: vec![],
                    link: None,
                },
                DeviceSnapshot {
                    descriptor: make_device_descriptor("esphome:aabb", "Imported"),
                    entities: vec![],
                    link: Some(make_connected_device(
                        "esphome:aabb",
                        "imported",
                        SessionStatus::Unavailable,
                        0,
                    )),
                },
            ],
            orphan_entities: vec![],
        };

        let error = runtime.validate_snapshot(&snapshot).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("3 devices"));
        assert!(error.to_string().contains("max_devices is 2"));

        runtime.shutdown().await;
    }
}
