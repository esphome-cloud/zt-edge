use crate::discovery::{BrowserEvent, DiscoveredDevice, MdnsBrowser};
use crate::error::DeviceLinkError;
use crate::security::DeviceSecurityConfig;
use crate::session::{DeviceSessionActor, DeviceSessionMsg};
use rshome_actor::{Actor, ActorContext, ActorRef, SupervisorStrategy};
use rshome_entity::{DeviceId, DeviceManagerMsg, EntityId, EntityRegistry, EntityState};
use rshome_state::StateStore;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::oneshot;

/// Session connectivity status for one ESPHome device link.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum SessionStatus {
    /// Discovered via mDNS; no session started yet (or session not yet identified).
    Discovered,
    /// TCP connect in progress.
    Connecting,
    /// TCP connected; Native API handshake (Hello/DeviceInfo/ListEntities) in progress.
    Handshaking,
    /// Handshake complete; subscribed to state updates.
    Active,
    /// Connection lost; waiting before reconnect attempt.
    Backoff { attempt: u32, delay_secs: u64 },
    /// Device requires password authentication, which is unsupported in v1.
    /// Session is permanently stopped; operator must clear the password on firmware.
    UnsupportedAuth,
    /// Intentionally stopped.
    Unavailable,
}

/// A device whose identity has been confirmed via the Native API handshake.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConnectedDevice {
    /// Canonical device ID: `esphome:<mac>` if MAC was present in DeviceInfoResponse,
    /// otherwise falls back to `esphome-host:<hostname>`.
    pub device_id: DeviceId,
    /// The mDNS discovery record this session was started from.
    pub discovered: DiscoveredDevice,
    /// Name from `DeviceInfoResponse.name`.
    pub name: String,
    /// Human-readable name from `DeviceInfoResponse.friendly_name` (if set).
    pub friendly_name: Option<String>,
    /// MAC address from `DeviceInfoResponse.mac_address`.
    pub mac_address: Option<String>,
    /// Model string from `DeviceInfoResponse.model`.
    pub model: Option<String>,
    /// Firmware version from `DeviceInfoResponse.esphome_version`.
    pub sw_version: Option<String>,
    /// Current session status.
    pub status: SessionStatus,
    /// Number of entities registered for this device.
    pub entity_count: usize,
    /// Authentication mode used for the connection (`"cleartext"` or `"noise"`).
    pub auth_mode: Option<String>,
    /// Last connection error message, if any.
    pub last_error: Option<String>,
}

/// Live status of a single ESPHome device link (returned by `GetStatus`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct DeviceLinkStatus {
    pub device_id: String,
    pub name: String,
    pub hostname: String,
    pub ip: String,
    pub port: u16,
    pub version: String,
    pub status: SessionStatus,
    pub entity_count: usize,
    pub mac_address: Option<String>,
    pub model: Option<String>,
    pub sw_version: Option<String>,
    pub friendly_name: Option<String>,
    /// Authentication mode used for the connection (`"cleartext"` or `"noise"`).
    pub auth_mode: Option<String>,
    /// Last connection error message, if any.
    pub last_error: Option<String>,
}

/// Current resource utilisation snapshot returned by `GetResourceUsage`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ResourceUsage {
    pub discovered_count: usize,
    pub discovered_cap: usize,
    pub active_sessions: usize,
    pub session_cap: usize,
}

/// Runtime limits applied by the standalone rshome-ha device-link ingress.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DeviceLinkLimits {
    pub max_devices: usize,
    pub max_entities: usize,
    pub max_connections_per_device: usize,
}

impl Default for DeviceLinkLimits {
    fn default() -> Self {
        Self {
            max_devices: 100,
            max_entities: 10_000,
            max_connections_per_device: 1,
        }
    }
}

/// Messages for `DeviceLinkManagerActor`.
#[allow(clippy::module_name_repetitions)]
pub enum DeviceLinkManagerMsg {
    // ── External queries ─────────────────────────────────────────────────
    /// Return all currently-discovered ESPHome devices.
    ListDiscovered(oneshot::Sender<Vec<DiscoveredDevice>>),
    /// Return all devices whose Native API identity has been confirmed.
    ListConnected(oneshot::Sender<Vec<ConnectedDevice>>),
    /// Return the connection status for one device (by provisional or canonical ID).
    GetStatus {
        device_id: DeviceId,
        reply: oneshot::Sender<Option<DeviceLinkStatus>>,
    },
    /// Return current resource utilisation (discovered count, session count, caps).
    GetResourceUsage(oneshot::Sender<ResourceUsage>),
    /// Set the security configuration for a specific device (keyed by provisional hostname ID).
    SetDeviceSecurityConfig {
        device_id: DeviceId,
        config: DeviceSecurityConfig,
    },
    /// Seed a previously-imported device so status/provenance survive daemon restarts.
    SeedRestoredDevice(ConnectedDevice),

    // ── Internal: mDNS browser events ────────────────────────────────────
    DeviceDiscovered(DiscoveredDevice),
    DeviceRemoved(DeviceId),

    // ── Internal: session actor events ───────────────────────────────────
    /// Session completed the DeviceInfo exchange; provides canonical identity.
    SessionIdentityResolved {
        provisional_id: DeviceId,
        canonical_id: DeviceId,
        name: String,
        friendly_name: Option<String>,
        mac_address: Option<String>,
        model: Option<String>,
        sw_version: Option<String>,
    },
    /// Session's connection status changed.
    SessionStatusChanged {
        provisional_id: DeviceId,
        status: SessionStatus,
        entity_count: usize,
    },

    // ── Internal: periodic staleness check ───────────────────────────────
    /// Fired every 30 s; marks records stale if not refreshed for ≥ 90 s.
    CheckStaleness,

    /// Dispatch a command to the active session for a device.
    ///
    /// Routes `local_entity_id` + desired `state` to the `DeviceSessionActor` that
    /// owns the device, translating it into a Native API command frame.
    DispatchCommand {
        /// Target device (canonical or provisional ID accepted).
        device_id: DeviceId,
        /// Local entity ID to command.
        local_entity_id: EntityId,
        /// Desired new state (e.g. `Switch { is_on: true }`).
        state: EntityState,
        /// One-shot reply channel for the command result.
        reply: oneshot::Sender<Result<(), DeviceLinkError>>,
    },

    Stop,
}

/// Maximum number of mDNS discovery records retained by the manager.
pub const MAX_DISCOVERED: usize = 256;
/// Maximum number of simultaneously active device sessions.
pub const MAX_ACTIVE_SESSIONS: usize = 128;
/// Duration (seconds) after which stale discovery records with no active session
/// are evicted from the manager to prevent unbounded accumulation.
pub const STALE_EVICTION_SECS: u64 = 600;

/// Manages mDNS discovery and device session actors for all ESPHome devices
/// visible on the local network.
///
/// On `pre_start`, starts an mDNS browser for `_esphomelib._tcp.local.`. For
/// each discovered device a `DeviceSessionActor` child is spawned automatically.
#[allow(clippy::module_name_repetitions)]
pub struct DeviceLinkManagerActor {
    discovered: HashMap<DeviceId, DiscoveredDevice>,
    /// Canonical connected-device records keyed by canonical DeviceId.
    canonical: HashMap<DeviceId, ConnectedDevice>,
    /// Maps provisional (hostname-based) DeviceId → canonical (MAC-based) DeviceId.
    provisional_to_canonical: HashMap<DeviceId, DeviceId>,
    /// Reverse map: canonical DeviceId → provisional DeviceId (for DispatchCommand routing).
    canonical_to_provisional: HashMap<DeviceId, DeviceId>,
    /// Snapshot-restored imported devices that do not yet have a live session.
    restored: HashMap<DeviceId, ConnectedDevice>,
    sessions: HashMap<DeviceId, ActorRef<DeviceSessionMsg>>,
    device_manager: ActorRef<DeviceManagerMsg>,
    entity_registry: EntityRegistry,
    state_store: Arc<StateStore>,
    limits: DeviceLinkLimits,
    /// Per-device security configurations keyed by provisional device ID.
    security_configs: HashMap<DeviceId, DeviceSecurityConfig>,
    /// Held so the background browse task keeps running
    _browser: Option<MdnsBrowser>,
}

impl DeviceLinkManagerActor {
    pub fn new(
        device_manager: ActorRef<DeviceManagerMsg>,
        entity_registry: EntityRegistry,
        state_store: Arc<StateStore>,
        limits: DeviceLinkLimits,
    ) -> Self {
        Self {
            discovered: HashMap::new(),
            canonical: HashMap::new(),
            provisional_to_canonical: HashMap::new(),
            canonical_to_provisional: HashMap::new(),
            restored: HashMap::new(),
            sessions: HashMap::new(),
            device_manager,
            entity_registry,
            state_store,
            limits,
            security_configs: HashMap::new(),
            _browser: None,
        }
    }

    fn spawn_session(
        &mut self,
        device: &DiscoveredDevice,
        ctx: &mut ActorContext<DeviceLinkManagerMsg>,
    ) {
        if self.sessions.contains_key(&device.device_id) {
            return;
        }
        if self.limits.max_connections_per_device == 0 {
            tracing::warn!(
                device_id = %device.device_id,
                "device-link session creation disabled by max_connections_per_device=0",
            );
            return;
        }
        let session_cap = self.limits.max_devices.min(MAX_ACTIVE_SESSIONS);
        if session_cap == 0 {
            tracing::warn!(
                device_id = %device.device_id,
                "device-link session creation disabled by max_devices=0",
            );
            return;
        }
        // Task 4.3: enforce session cap
        if self.sessions.len() >= session_cap {
            tracing::warn!(
                device_id = %device.device_id,
                cap = session_cap,
                "active session cap reached; not spawning session for new device",
            );
            return;
        }
        let security_config = self
            .security_configs
            .get(&device.device_id)
            .cloned()
            .unwrap_or_default();
        let session = DeviceSessionActor::with_security(
            device.clone(),
            security_config,
            ctx.self_ref().clone(),
            self.device_manager.clone(),
            self.entity_registry.clone(),
            self.state_store.clone(),
            self.limits,
        );
        let session_ref = ctx.spawn_child(session, SupervisorStrategy::default());
        self.sessions.insert(device.device_id.clone(), session_ref);
    }

    /// Build a `DeviceLinkStatus` from a `ConnectedDevice`.
    fn status_from_connected(cd: &ConnectedDevice) -> DeviceLinkStatus {
        DeviceLinkStatus {
            device_id: cd.device_id.to_string(),
            name: cd.name.clone(),
            hostname: cd.discovered.hostname.clone(),
            ip: cd.discovered.ip.to_string(),
            port: cd.discovered.port,
            version: cd.discovered.version.clone(),
            status: cd.status.clone(),
            entity_count: cd.entity_count,
            mac_address: cd.mac_address.clone(),
            model: cd.model.clone(),
            sw_version: cd.sw_version.clone(),
            friendly_name: cd.friendly_name.clone(),
            auth_mode: cd.auth_mode.clone(),
            last_error: cd.last_error.clone(),
        }
    }

    /// Build a `DeviceLinkStatus` from a `DiscoveredDevice` (pre-handshake fallback).
    fn status_from_discovered(dd: &DiscoveredDevice) -> DeviceLinkStatus {
        DeviceLinkStatus {
            device_id: dd.device_id.to_string(),
            name: dd.name.clone(),
            hostname: dd.hostname.clone(),
            ip: dd.ip.to_string(),
            port: dd.port,
            version: dd.version.clone(),
            status: SessionStatus::Discovered,
            entity_count: 0,
            mac_address: None,
            model: None,
            sw_version: None,
            friendly_name: dd.friendly_name.clone(),
            auth_mode: None,
            last_error: None,
        }
    }
}

#[async_trait::async_trait]
impl Actor for DeviceLinkManagerActor {
    type Msg = DeviceLinkManagerMsg;

    async fn pre_start(&mut self, ctx: &mut ActorContext<Self::Msg>) {
        let self_ref = ctx.self_ref().clone();
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<BrowserEvent>();

        match MdnsBrowser::start(event_tx) {
            Ok(browser) => {
                self._browser = Some(browser);
                tokio::spawn(async move {
                    while let Some(event) = event_rx.recv().await {
                        let msg = match event {
                            BrowserEvent::DeviceFound(d) => {
                                DeviceLinkManagerMsg::DeviceDiscovered(d)
                            }
                            BrowserEvent::DeviceRemoved(id) => {
                                DeviceLinkManagerMsg::DeviceRemoved(id)
                            }
                        };
                        if self_ref.send(msg).is_err() {
                            break;
                        }
                    }
                });
            }
            Err(e) => {
                tracing::warn!("mDNS browser failed to start: {e}");
            }
        }

        // Spawn a periodic staleness-check task (disabled in test builds to avoid
        // background timer interference with deterministic unit tests).
        #[cfg(not(test))]
        {
            let stale_ref = ctx.self_ref().clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(30));
                interval.tick().await; // skip the immediate first tick
                loop {
                    interval.tick().await;
                    if stale_ref
                        .send(DeviceLinkManagerMsg::CheckStaleness)
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }
    }

    async fn handle(&mut self, msg: DeviceLinkManagerMsg, ctx: &mut ActorContext<Self::Msg>) {
        match msg {
            DeviceLinkManagerMsg::ListDiscovered(reply) => {
                let list: Vec<DiscoveredDevice> = self.discovered.values().cloned().collect();
                let _ = reply.send(list);
            }
            DeviceLinkManagerMsg::ListConnected(reply) => {
                let mut list: Vec<ConnectedDevice> = self.canonical.values().cloned().collect();
                for (device_id, restored) in &self.restored {
                    if !self.canonical.contains_key(device_id) {
                        list.push(restored.clone());
                    }
                }
                let _ = reply.send(list);
            }
            DeviceLinkManagerMsg::GetStatus { device_id, reply } => {
                // 1. Direct lookup in canonical (canonical_id matches query)
                if let Some(cd) = self.canonical.get(&device_id) {
                    let _ = reply.send(Some(Self::status_from_connected(cd)));
                    return;
                }
                // 2. Resolve provisional_id → canonical_id
                if let Some(canonical_id) = self.provisional_to_canonical.get(&device_id) {
                    if let Some(cd) = self.canonical.get(canonical_id) {
                        let _ = reply.send(Some(Self::status_from_connected(cd)));
                        return;
                    }
                }
                // 3. Fall back to discovered record (pre-handshake)
                let status = self
                    .discovered
                    .get(&device_id)
                    .map(Self::status_from_discovered);
                if status.is_some() {
                    let _ = reply.send(status);
                    return;
                }
                let restored = self
                    .restored
                    .get(&device_id)
                    .cloned()
                    .map(|cd| Self::status_from_connected(&cd))
                    .or_else(|| {
                        self.restored
                            .values()
                            .find(|cd| cd.discovered.device_id == device_id)
                            .map(Self::status_from_connected)
                    });
                let _ = reply.send(restored);
            }
            DeviceLinkManagerMsg::SeedRestoredDevice(device) => {
                self.restored.insert(device.device_id.clone(), device);
            }
            DeviceLinkManagerMsg::GetResourceUsage(reply) => {
                let session_cap = self.limits.max_devices.min(MAX_ACTIVE_SESSIONS);
                let _ = reply.send(ResourceUsage {
                    discovered_count: self.discovered.len(),
                    discovered_cap: MAX_DISCOVERED,
                    active_sessions: self.sessions.len(),
                    session_cap,
                });
            }
            DeviceLinkManagerMsg::DeviceDiscovered(device) => {
                tracing::info!(
                    device_id = %device.device_id,
                    name = %device.name,
                    ip = %device.ip,
                    "ESPHome device discovered",
                );
                if let Some(existing) = self.discovered.get_mut(&device.device_id) {
                    // Re-announcement: refresh timestamp, clear stale flag, keep first_seen_at.
                    existing.last_seen_at = SystemTime::now();
                    existing.is_stale = false;
                } else {
                    // Task 4.3: enforce discovery record cap
                    if self.discovered.len() >= MAX_DISCOVERED {
                        tracing::warn!(
                            device_id = %device.device_id,
                            cap = MAX_DISCOVERED,
                            "discovery record cap reached; ignoring new mDNS record",
                        );
                        return;
                    }
                    self.spawn_session(&device, ctx);
                    self.discovered.insert(device.device_id.clone(), device);
                }
            }
            DeviceLinkManagerMsg::DeviceRemoved(provisional_id) => {
                tracing::info!(device_id = %provisional_id, "ESPHome device removed from mDNS");

                // Task 4.4: active sessions survive temporary mDNS browse silence.
                // Only mark the discovery record stale; do not stop a healthy session.
                let session_is_active = self
                    .provisional_to_canonical
                    .get(&provisional_id)
                    .and_then(|cid| self.canonical.get(cid))
                    .map(|cd| matches!(cd.status, SessionStatus::Active))
                    .unwrap_or(false);

                if session_is_active {
                    // Keep session alive; just mark the discovery record stale so
                    // callers know the mDNS advertisement has disappeared.
                    if let Some(record) = self.discovered.get_mut(&provisional_id) {
                        record.is_stale = true;
                    }
                    tracing::info!(
                        device_id = %provisional_id,
                        "mDNS record removed but session is Active; marking stale, keeping session",
                    );
                    return;
                }

                // Non-active session: clean up fully.
                self.discovered.remove(&provisional_id);
                if let Some(canonical_id) = self.provisional_to_canonical.remove(&provisional_id) {
                    self.canonical_to_provisional.remove(&canonical_id);
                    self.canonical.remove(&canonical_id);
                }
                if let Some(session) = self.sessions.remove(&provisional_id) {
                    let _ = session.send(DeviceSessionMsg::Stop);
                }
            }
            DeviceLinkManagerMsg::SetDeviceSecurityConfig { device_id, config } => {
                self.security_configs.insert(device_id, config);
            }
            DeviceLinkManagerMsg::SessionIdentityResolved {
                provisional_id,
                canonical_id,
                name,
                friendly_name,
                mac_address,
                model,
                sw_version,
            } => {
                self.restored.remove(&canonical_id);

                // Duplicate-MAC collapse: if another session already owns this canonical ID,
                // stop the redundant session and do not insert a second canonical entry.
                if self.canonical.contains_key(&canonical_id) {
                    tracing::info!(
                        provisional_id = %provisional_id,
                        canonical_id = %canonical_id,
                        "duplicate MAC detected, stopping redundant session",
                    );
                    if let Some(session) = self.sessions.get(&provisional_id) {
                        let _ = session.send(DeviceSessionMsg::Stop);
                    }
                    return;
                }

                let discovered = match self.discovered.get(&provisional_id).cloned() {
                    Some(d) => d,
                    None => return, // device was removed concurrently
                };

                let auth_mode = self
                    .security_configs
                    .get(&provisional_id)
                    .and_then(|cfg| {
                        if cfg.noise_psk.is_some() {
                            Some("noise".to_string())
                        } else {
                            None
                        }
                    })
                    .or_else(|| Some("cleartext".to_string()));
                let connected = ConnectedDevice {
                    device_id: canonical_id.clone(),
                    discovered,
                    name,
                    friendly_name,
                    mac_address,
                    model,
                    sw_version,
                    status: SessionStatus::Handshaking,
                    entity_count: 0,
                    auth_mode,
                    last_error: None,
                };
                self.canonical.insert(canonical_id.clone(), connected);
                self.provisional_to_canonical
                    .insert(provisional_id.clone(), canonical_id.clone());
                self.canonical_to_provisional
                    .insert(canonical_id, provisional_id);
            }
            DeviceLinkManagerMsg::SessionStatusChanged {
                provisional_id,
                status,
                entity_count,
            } => {
                let canonical_id = self
                    .provisional_to_canonical
                    .get(&provisional_id)
                    .cloned()
                    .unwrap_or_else(|| provisional_id.clone());

                if let Some(device) = self.canonical.get_mut(&canonical_id) {
                    device.status = status;
                    device.entity_count = entity_count;
                }
                // If no canonical entry exists yet (pre-handshake), the update is a no-op.
            }
            DeviceLinkManagerMsg::CheckStaleness => {
                let stale_threshold = Duration::from_secs(90);
                let eviction_threshold = Duration::from_secs(STALE_EVICTION_SECS);
                // Phase 1: mark records stale after 90s without re-announcement
                for device in self.discovered.values_mut() {
                    if device.last_seen_at.elapsed().unwrap_or(Duration::MAX) >= stale_threshold {
                        device.is_stale = true;
                    }
                }
                // Phase 2: evict stale records older than 10 minutes that have no
                // canonical entry (i.e. no active session owns them).
                let to_evict: Vec<DeviceId> = self
                    .discovered
                    .iter()
                    .filter(|(_, d)| {
                        d.is_stale
                            && d.last_seen_at.elapsed().unwrap_or(Duration::MAX)
                                >= eviction_threshold
                            && !self.provisional_to_canonical.contains_key(&d.device_id)
                    })
                    .map(|(id, _)| id.clone())
                    .collect();
                for id in to_evict {
                    self.discovered.remove(&id);
                    if let Some(session) = self.sessions.remove(&id) {
                        let _ = session.send(DeviceSessionMsg::Stop);
                    }
                }
            }
            DeviceLinkManagerMsg::DispatchCommand {
                device_id,
                local_entity_id,
                state,
                reply,
            } => {
                // Resolve device_id: try canonical_to_provisional first (canonical id input),
                // then fall back to treating it as a provisional id directly.
                let provisional_id = self
                    .canonical_to_provisional
                    .get(&device_id)
                    .cloned()
                    .unwrap_or_else(|| device_id.clone());

                let session = match self.sessions.get(&provisional_id) {
                    Some(s) => s.clone(),
                    None => {
                        let _ = reply.send(Err(DeviceLinkError::SessionNotActive));
                        return;
                    }
                };

                let (cmd_reply_tx, cmd_reply_rx) = oneshot::channel();
                if session
                    .send(DeviceSessionMsg::SendCommand {
                        local_entity_id,
                        state,
                        reply: cmd_reply_tx,
                    })
                    .is_err()
                {
                    let _ = reply.send(Err(DeviceLinkError::SessionNotActive));
                    return;
                }

                tokio::spawn(async move {
                    let result = cmd_reply_rx
                        .await
                        .unwrap_or(Err(DeviceLinkError::SessionNotActive));
                    let _ = reply.send(result);
                });
            }
            DeviceLinkManagerMsg::Stop => {
                for (_, session) in self.sessions.drain() {
                    let _ = session.send(DeviceSessionMsg::Stop);
                }
                self.canonical.clear();
                self.restored.clear();
                self.provisional_to_canonical.clear();
                self.canonical_to_provisional.clear();
                ctx.stop();
            }
        }
    }

    async fn post_stop(&mut self) {
        // Drop the mDNS browser to stop the background browse task.
        self._browser = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rshome_actor::{Actor, ActorContext, ActorSystem};
    use rshome_entity::{DeviceId, NullStateUpdater};
    use rshome_state::StateStore;
    use std::sync::Arc;
    use tokio::sync::oneshot;

    // Minimal DeviceManager stub that just responds Ok to AddDevice
    struct MockDeviceManager;

    #[async_trait::async_trait]
    impl Actor for MockDeviceManager {
        type Msg = DeviceManagerMsg;
        async fn handle(
            &mut self,
            msg: DeviceManagerMsg,
            ctx: &mut ActorContext<DeviceManagerMsg>,
        ) {
            match msg {
                DeviceManagerMsg::AddDevice { descriptor, reply } => {
                    // Spawn a trivial DeviceActor child and reply
                    use rshome_entity::{DeviceActor, EntityRegistry};
                    let reg = EntityRegistry::default();
                    let updater =
                        Arc::new(NullStateUpdater) as Arc<dyn rshome_entity::StateUpdater>;
                    let actor = DeviceActor::new(descriptor, reg, updater);
                    let actor_ref = ctx.spawn_child_default(actor);
                    let _ = reply.send(actor_ref);
                }
                DeviceManagerMsg::ListDevices(reply) => {
                    let _ = reply.send(vec![]);
                }
                DeviceManagerMsg::GetDevice { reply, .. } => {
                    let _ = reply.send(None);
                }
                DeviceManagerMsg::GetEntitiesForDevice { reply, .. } => {
                    let _ = reply.send(vec![]);
                }
                DeviceManagerMsg::RemoveDevice(_) => {}
                DeviceManagerMsg::Stop => ctx.stop(),
            }
        }
    }

    fn make_manager() -> (ActorSystem, ActorRef<DeviceLinkManagerMsg>) {
        let system = ActorSystem::new();
        let dm_ref = system.spawn(MockDeviceManager);
        let registry = EntityRegistry::default();
        let state_store = Arc::new(StateStore::default());
        let manager =
            DeviceLinkManagerActor::new(dm_ref, registry, state_store, DeviceLinkLimits::default());
        let manager_ref = system.spawn(manager);
        (system, manager_ref)
    }

    fn make_device(name: &str, port: u16) -> DiscoveredDevice {
        use crate::discovery::device_id_from_hostname;
        DiscoveredDevice {
            device_id: device_id_from_hostname(name),
            service_fullname: format!("{name}._esphomelib._tcp.local."),
            hostname: name.to_string(),
            ip: "127.0.0.1".parse().unwrap(),
            port,
            name: name.to_string(),
            version: "2025.1.0".to_string(),
            friendly_name: None,
            first_seen_at: std::time::SystemTime::UNIX_EPOCH,
            last_seen_at: std::time::SystemTime::now(),
            is_stale: false,
        }
    }

    fn make_connected_device(
        canonical_id: &str,
        provisional_name: &str,
        status: SessionStatus,
    ) -> ConnectedDevice {
        ConnectedDevice {
            device_id: DeviceId(canonical_id.to_string()),
            discovered: make_device(provisional_name, 6053),
            name: provisional_name.to_string(),
            friendly_name: Some(provisional_name.to_string()),
            mac_address: Some("AA:BB:CC:DD:EE:FF".to_string()),
            model: Some("ESP32".to_string()),
            sw_version: Some("2026.03".to_string()),
            status,
            entity_count: 1,
            auth_mode: Some("cleartext".to_string()),
            last_error: None,
        }
    }

    #[tokio::test]
    async fn list_discovered_initially_empty() {
        let (sys, manager_ref) = make_manager();

        let (tx, rx) = oneshot::channel();
        manager_ref
            .send(DeviceLinkManagerMsg::ListDiscovered(tx))
            .ok();
        let list = rx.await.unwrap();
        assert!(list.is_empty());

        manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
        drop(sys);
    }

    #[tokio::test]
    async fn list_connected_initially_empty() {
        let (sys, manager_ref) = make_manager();

        let (tx, rx) = oneshot::channel();
        manager_ref
            .send(DeviceLinkManagerMsg::ListConnected(tx))
            .ok();
        let list = rx.await.unwrap();
        assert!(list.is_empty());

        manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
        drop(sys);
    }

    #[tokio::test]
    async fn seeded_restored_device_appears_in_connected_and_status() {
        let (sys, manager_ref) = make_manager();
        let connected = make_connected_device(
            "esphome:aabbccddeeff",
            "living-room",
            SessionStatus::Unavailable,
        );

        manager_ref
            .send(DeviceLinkManagerMsg::SeedRestoredDevice(connected.clone()))
            .ok();

        let (list_tx, list_rx) = oneshot::channel();
        manager_ref
            .send(DeviceLinkManagerMsg::ListConnected(list_tx))
            .ok();
        let list = list_rx.await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].device_id, connected.device_id);

        let (status_tx, status_rx) = oneshot::channel();
        manager_ref
            .send(DeviceLinkManagerMsg::GetStatus {
                device_id: connected.device_id.clone(),
                reply: status_tx,
            })
            .ok();
        let status = status_rx.await.unwrap().unwrap();
        assert_eq!(status.device_id, connected.device_id.to_string());
        assert!(matches!(status.status, SessionStatus::Unavailable));

        manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
        drop(sys);
    }

    #[tokio::test]
    async fn device_discovered_appears_in_list() {
        let (sys, manager_ref) = make_manager();

        let device = make_device("esp32-sensor", 6053);
        manager_ref
            .send(DeviceLinkManagerMsg::DeviceDiscovered(device.clone()))
            .ok();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let (tx, rx) = oneshot::channel();
        manager_ref
            .send(DeviceLinkManagerMsg::ListDiscovered(tx))
            .ok();
        let list = rx.await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "esp32-sensor");

        manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
        drop(sys);
    }

    #[tokio::test]
    async fn device_removed_disappears_from_list() {
        let (sys, manager_ref) = make_manager();

        let device = make_device("node1", 6053);
        manager_ref
            .send(DeviceLinkManagerMsg::DeviceDiscovered(device.clone()))
            .ok();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        manager_ref
            .send(DeviceLinkManagerMsg::DeviceRemoved(
                device.device_id.clone(),
            ))
            .ok();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let (tx, rx) = oneshot::channel();
        manager_ref
            .send(DeviceLinkManagerMsg::ListDiscovered(tx))
            .ok();
        let list = rx.await.unwrap();
        assert!(list.is_empty());

        manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
        drop(sys);
    }

    #[tokio::test]
    async fn get_status_unknown_device_returns_none() {
        let (sys, manager_ref) = make_manager();

        let (tx, rx) = oneshot::channel();
        manager_ref
            .send(DeviceLinkManagerMsg::GetStatus {
                device_id: DeviceId("esphome-host:unknown".into()),
                reply: tx,
            })
            .ok();
        let status = rx.await.unwrap();
        assert!(status.is_none());

        manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
        drop(sys);
    }

    #[tokio::test]
    async fn get_status_known_device_not_yet_connected() {
        let (sys, manager_ref) = make_manager();

        let device = make_device("heater", 6053);
        manager_ref
            .send(DeviceLinkManagerMsg::DeviceDiscovered(device.clone()))
            .ok();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let (tx, rx) = oneshot::channel();
        manager_ref
            .send(DeviceLinkManagerMsg::GetStatus {
                device_id: device.device_id.clone(),
                reply: tx,
            })
            .ok();
        let status = rx.await.unwrap().unwrap();
        assert_eq!(status.name, "heater");
        // Not yet in canonical map → falls back to Discovered status
        assert!(
            !matches!(status.status, SessionStatus::Active),
            "not yet active"
        );
        assert_eq!(status.entity_count, 0);

        manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
        drop(sys);
    }

    #[tokio::test]
    async fn session_identity_and_status_update() {
        let (sys, manager_ref) = make_manager();

        let device = make_device("lamp", 6053);
        manager_ref
            .send(DeviceLinkManagerMsg::DeviceDiscovered(device.clone()))
            .ok();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Simulate session reporting identity (no MAC → canonical_id == provisional_id)
        manager_ref
            .send(DeviceLinkManagerMsg::SessionIdentityResolved {
                provisional_id: device.device_id.clone(),
                canonical_id: device.device_id.clone(),
                name: "lamp".into(),
                friendly_name: None,
                mac_address: None,
                model: None,
                sw_version: None,
            })
            .ok();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Simulate session reporting Active
        manager_ref
            .send(DeviceLinkManagerMsg::SessionStatusChanged {
                provisional_id: device.device_id.clone(),
                status: SessionStatus::Active,
                entity_count: 3,
            })
            .ok();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let (tx, rx) = oneshot::channel();
        manager_ref
            .send(DeviceLinkManagerMsg::GetStatus {
                device_id: device.device_id.clone(),
                reply: tx,
            })
            .ok();
        let status = rx.await.unwrap().unwrap();
        assert!(matches!(status.status, SessionStatus::Active));
        assert_eq!(status.entity_count, 3);

        manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
        drop(sys);
    }

    #[tokio::test]
    async fn device_rediscovered_updates_last_seen() {
        let (sys, manager_ref) = make_manager();

        // First announcement with UNIX_EPOCH as last_seen_at
        let mut device = make_device("thermostat", 6053);
        device.last_seen_at = std::time::SystemTime::UNIX_EPOCH;
        manager_ref
            .send(DeviceLinkManagerMsg::DeviceDiscovered(device.clone()))
            .ok();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Re-announcement (same device_id) — last_seen_at should be updated to now
        manager_ref
            .send(DeviceLinkManagerMsg::DeviceDiscovered(device.clone()))
            .ok();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let (tx, rx) = oneshot::channel();
        manager_ref
            .send(DeviceLinkManagerMsg::ListDiscovered(tx))
            .ok();
        let list = rx.await.unwrap();
        assert_eq!(list.len(), 1);
        // last_seen_at should be after UNIX_EPOCH (i.e. updated to now)
        assert!(
            list[0].last_seen_at > std::time::SystemTime::UNIX_EPOCH,
            "last_seen_at must be updated on re-announcement"
        );

        manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
        drop(sys);
    }

    #[tokio::test]
    async fn stale_after_check_staleness() {
        let (sys, manager_ref) = make_manager();

        // Insert a device with last_seen_at in the distant past (> STALE_EVICTION_SECS).
        // Since it has no canonical entry, CheckStaleness will both mark it stale
        // AND evict it from the discovered list.
        let mut device = make_device("old-sensor", 6053);
        device.last_seen_at = std::time::SystemTime::UNIX_EPOCH;
        manager_ref
            .send(DeviceLinkManagerMsg::DeviceDiscovered(device.clone()))
            .ok();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Trigger the staleness check manually
        manager_ref.send(DeviceLinkManagerMsg::CheckStaleness).ok();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let (tx, rx) = oneshot::channel();
        manager_ref
            .send(DeviceLinkManagerMsg::ListDiscovered(tx))
            .ok();
        let list = rx.await.unwrap();
        // Record is evicted because it's stale for > 600s with no canonical entry
        assert!(
            list.is_empty(),
            "very old stale record without canonical entry should be evicted"
        );

        manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
        drop(sys);
    }

    #[tokio::test]
    async fn stale_mark_without_eviction_when_recent() {
        let (sys, manager_ref) = make_manager();

        // Insert a device that is stale (>90s) but NOT yet evictable (<600s).
        // Use a last_seen_at that is 120 seconds ago.
        let mut device = make_device("recent-stale", 6053);
        device.last_seen_at = std::time::SystemTime::now() - std::time::Duration::from_secs(120);
        manager_ref
            .send(DeviceLinkManagerMsg::DeviceDiscovered(device.clone()))
            .ok();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        manager_ref.send(DeviceLinkManagerMsg::CheckStaleness).ok();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let (tx, rx) = oneshot::channel();
        manager_ref
            .send(DeviceLinkManagerMsg::ListDiscovered(tx))
            .ok();
        let list = rx.await.unwrap();
        assert_eq!(
            list.len(),
            1,
            "recently stale record should still be present"
        );
        assert!(
            list[0].is_stale,
            "device not refreshed for >90s must be stale"
        );

        manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
        drop(sys);
    }

    #[tokio::test]
    async fn canonical_id_resolved_via_provisional_lookup() {
        let (sys, manager_ref) = make_manager();

        let device = make_device("sensor", 6053);
        manager_ref
            .send(DeviceLinkManagerMsg::DeviceDiscovered(device.clone()))
            .ok();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // MAC-based canonical ID differs from hostname provisional ID
        let mac_canonical = DeviceId("esphome:aabbccddeeff".into());
        manager_ref
            .send(DeviceLinkManagerMsg::SessionIdentityResolved {
                provisional_id: device.device_id.clone(),
                canonical_id: mac_canonical.clone(),
                name: "sensor".into(),
                friendly_name: None,
                mac_address: Some("AA:BB:CC:DD:EE:FF".into()),
                model: None,
                sw_version: None,
            })
            .ok();
        manager_ref
            .send(DeviceLinkManagerMsg::SessionStatusChanged {
                provisional_id: device.device_id.clone(),
                status: SessionStatus::Active,
                entity_count: 2,
            })
            .ok();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // GetStatus by provisional ID should resolve to canonical entry
        let (tx, rx) = oneshot::channel();
        manager_ref
            .send(DeviceLinkManagerMsg::GetStatus {
                device_id: device.device_id.clone(),
                reply: tx,
            })
            .ok();
        let status = rx.await.unwrap().unwrap();
        assert_eq!(status.device_id, "esphome:aabbccddeeff");
        assert!(matches!(status.status, SessionStatus::Active));
        assert_eq!(status.mac_address.as_deref(), Some("AA:BB:CC:DD:EE:FF"));

        // GetStatus by canonical ID directly should also work
        let (tx2, rx2) = oneshot::channel();
        manager_ref
            .send(DeviceLinkManagerMsg::GetStatus {
                device_id: mac_canonical,
                reply: tx2,
            })
            .ok();
        let status2 = rx2.await.unwrap().unwrap();
        assert!(matches!(status2.status, SessionStatus::Active));

        manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
        drop(sys);
    }

    #[tokio::test]
    async fn device_removed_keeps_active_session_alive() {
        let (sys, manager_ref) = make_manager();

        let device = make_device("light", 6053);
        manager_ref
            .send(DeviceLinkManagerMsg::DeviceDiscovered(device.clone()))
            .ok();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Simulate handshake completing → canonical entry created
        let canonical_id = DeviceId("esphome:112233445566".into());
        manager_ref
            .send(DeviceLinkManagerMsg::SessionIdentityResolved {
                provisional_id: device.device_id.clone(),
                canonical_id: canonical_id.clone(),
                name: "light".into(),
                friendly_name: None,
                mac_address: Some("11:22:33:44:55:66".into()),
                model: None,
                sw_version: None,
            })
            .ok();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Mark session Active
        manager_ref
            .send(DeviceLinkManagerMsg::SessionStatusChanged {
                provisional_id: device.device_id.clone(),
                status: SessionStatus::Active,
                entity_count: 2,
            })
            .ok();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // mDNS record disappears — session must survive
        manager_ref
            .send(DeviceLinkManagerMsg::DeviceRemoved(
                device.device_id.clone(),
            ))
            .ok();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Canonical entry must still exist and be Active
        let (tx, rx) = oneshot::channel();
        manager_ref
            .send(DeviceLinkManagerMsg::GetStatus {
                device_id: canonical_id.clone(),
                reply: tx,
            })
            .ok();
        let status = rx.await.unwrap();
        assert!(
            status.is_some(),
            "canonical entry must survive DeviceRemoved when Active"
        );
        assert!(
            matches!(status.as_ref().unwrap().status, SessionStatus::Active),
            "session must remain Active, got {:?}",
            status.as_ref().unwrap().status
        );

        // Discovery record must be marked stale, not removed
        let (tx2, rx2) = oneshot::channel();
        manager_ref
            .send(DeviceLinkManagerMsg::ListDiscovered(tx2))
            .ok();
        let discovered = rx2.await.unwrap();
        assert_eq!(
            discovered.len(),
            1,
            "discovery record must remain (marked stale)"
        );
        assert!(
            discovered[0].is_stale,
            "discovery record must be marked stale"
        );

        manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
        drop(sys);
    }

    #[tokio::test]
    async fn device_removed_non_active_session_cleaned_up() {
        let (sys, manager_ref) = make_manager();

        let device = make_device("fan", 6053);
        manager_ref
            .send(DeviceLinkManagerMsg::DeviceDiscovered(device.clone()))
            .ok();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Simulate handshake completing
        let canonical_id = DeviceId("esphome:ffeeddccbbaa".into());
        manager_ref
            .send(DeviceLinkManagerMsg::SessionIdentityResolved {
                provisional_id: device.device_id.clone(),
                canonical_id: canonical_id.clone(),
                name: "fan".into(),
                friendly_name: None,
                mac_address: Some("FF:EE:DD:CC:BB:AA".into()),
                model: None,
                sw_version: None,
            })
            .ok();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Session in Backoff (not Active) — DeviceRemoved should clean up fully
        manager_ref
            .send(DeviceLinkManagerMsg::SessionStatusChanged {
                provisional_id: device.device_id.clone(),
                status: SessionStatus::Backoff {
                    attempt: 1,
                    delay_secs: 2,
                },
                entity_count: 0,
            })
            .ok();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        manager_ref
            .send(DeviceLinkManagerMsg::DeviceRemoved(
                device.device_id.clone(),
            ))
            .ok();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Canonical entry must be gone
        let (tx, rx) = oneshot::channel();
        manager_ref
            .send(DeviceLinkManagerMsg::GetStatus {
                device_id: canonical_id,
                reply: tx,
            })
            .ok();
        let status = rx.await.unwrap();
        assert!(
            status.is_none(),
            "canonical entry must be removed when session was in Backoff"
        );

        // Discovery record must also be gone
        let (tx2, rx2) = oneshot::channel();
        manager_ref
            .send(DeviceLinkManagerMsg::ListDiscovered(tx2))
            .ok();
        let discovered = rx2.await.unwrap();
        assert!(
            discovered.is_empty(),
            "discovery record must be removed for non-active session"
        );

        manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
        drop(sys);
    }

    #[tokio::test]
    async fn set_security_config_stored() {
        let (sys, manager_ref) = make_manager();

        let device = make_device("encrypted-sensor", 6053);
        let config = crate::security::DeviceSecurityConfig {
            noise_psk: Some("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into()),
        };
        manager_ref
            .send(DeviceLinkManagerMsg::SetDeviceSecurityConfig {
                device_id: device.device_id.clone(),
                config,
            })
            .ok();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Verify the config is accepted without error (no panic/crash)
        let (tx, rx) = oneshot::channel();
        manager_ref
            .send(DeviceLinkManagerMsg::GetStatus {
                device_id: device.device_id.clone(),
                reply: tx,
            })
            .ok();
        // Device hasn't been discovered yet → None
        assert!(rx.await.unwrap().is_none());

        manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
        drop(sys);
    }

    #[tokio::test]
    async fn duplicate_mac_collapses_canonical_entry() {
        let (sys, manager_ref) = make_manager();

        let device_a = make_device("device-a", 6053);
        let device_b = make_device("device-b", 6054);
        manager_ref
            .send(DeviceLinkManagerMsg::DeviceDiscovered(device_a.clone()))
            .ok();
        manager_ref
            .send(DeviceLinkManagerMsg::DeviceDiscovered(device_b.clone()))
            .ok();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let mac_canonical = DeviceId("esphome:aabbccddeeff".into());

        // First session identity resolves successfully
        manager_ref
            .send(DeviceLinkManagerMsg::SessionIdentityResolved {
                provisional_id: device_a.device_id.clone(),
                canonical_id: mac_canonical.clone(),
                name: "device-a".into(),
                friendly_name: None,
                mac_address: Some("AA:BB:CC:DD:EE:FF".into()),
                model: None,
                sw_version: None,
            })
            .ok();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Second session with same MAC — should be stopped, no second canonical entry
        manager_ref
            .send(DeviceLinkManagerMsg::SessionIdentityResolved {
                provisional_id: device_b.device_id.clone(),
                canonical_id: mac_canonical.clone(),
                name: "device-b".into(),
                friendly_name: None,
                mac_address: Some("AA:BB:CC:DD:EE:FF".into()),
                model: None,
                sw_version: None,
            })
            .ok();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let (tx, rx) = oneshot::channel();
        manager_ref
            .send(DeviceLinkManagerMsg::ListConnected(tx))
            .ok();
        let connected = rx.await.unwrap();
        assert_eq!(
            connected.len(),
            1,
            "duplicate MAC must collapse to one canonical entry"
        );

        manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
        drop(sys);
    }

    #[tokio::test]
    async fn max_devices_zero_prevents_session_spawn() {
        let system = ActorSystem::new();
        let dm_ref = system.spawn(MockDeviceManager);
        let registry = EntityRegistry::default();
        let state_store = Arc::new(StateStore::default());
        let manager = DeviceLinkManagerActor::new(
            dm_ref,
            registry,
            state_store,
            DeviceLinkLimits {
                max_devices: 0,
                ..DeviceLinkLimits::default()
            },
        );
        let manager_ref = system.spawn(manager);

        manager_ref
            .send(DeviceLinkManagerMsg::DeviceDiscovered(make_device(
                "sensor-zero",
                6053,
            )))
            .ok();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let (tx, rx) = oneshot::channel();
        manager_ref
            .send(DeviceLinkManagerMsg::GetResourceUsage(tx))
            .ok();
        let usage = rx.await.unwrap();
        assert_eq!(usage.active_sessions, 0);
        assert_eq!(usage.session_cap, 0);

        manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
        drop(system);
    }
}
