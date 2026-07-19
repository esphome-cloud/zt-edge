use crate::bindings::{ImportedCommandCapabilities, ImportedEntityBinding};
use crate::commands::state_to_command;
use crate::discovery::{device_id_from_mac, device_slug, DiscoveredDevice};
use crate::error::DeviceLinkError;
use crate::ingest::parse_state_frame;
use crate::manager::{DeviceLinkLimits, DeviceLinkManagerMsg, SessionStatus};
use crate::noise_transport::{noise_handshake, noise_recv_frame, noise_send_frame, SharedNoise};
use crate::security::DeviceSecurityConfig;
use futures::{SinkExt, StreamExt as _};
use parking_lot::Mutex;
use prost::Message as _;
use rshome_actor::{Actor, ActorContext, ActorRef, SupervisorStrategy};
use rshome_entity::{
    DeviceDescriptor, DeviceId, DeviceManagerMsg, DeviceMsg, DomainRegistry, EntityActor,
    EntityCategory, EntityDescriptor, EntityId, EntityMsg, EntityRegistry, EntityState,
    StateUpdater,
};
use rshome_native_api::{codec::EspHomeCodec, msg_types, proto_gen::*};
use rshome_state::StateStore;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio_util::codec::{FramedRead, FramedWrite};

// ── RemoteStateUpdater ────────────────────────────────────────────────────────

/// `StateUpdater` for entities imported from a remote firmware device.
///
/// When `update()` is called by an entity actor, this updater determines whether
/// the change came from a firmware push or from a local command:
/// - **Firmware push**: the new state matches the value we last received from the
///   firmware, so we just store it in the `StateStore` and do nothing else.
/// - **Local command**: the new state differs from the last firmware value,
///   indicating a service-call-driven change. We forward it to the session actor
///   so it can translate and relay the command to the firmware.
pub(crate) struct RemoteStateUpdater {
    inner: Arc<StateStore>,
    firmware_states: Arc<Mutex<HashMap<EntityId, EntityState>>>,
    cmd_tx: mpsc::UnboundedSender<(EntityId, EntityState)>,
}

impl RemoteStateUpdater {
    pub fn new(
        inner: Arc<StateStore>,
        firmware_states: Arc<Mutex<HashMap<EntityId, EntityState>>>,
        cmd_tx: mpsc::UnboundedSender<(EntityId, EntityState)>,
    ) -> Self {
        Self {
            inner,
            firmware_states,
            cmd_tx,
        }
    }
}

impl StateUpdater for RemoteStateUpdater {
    fn update(&self, id: &EntityId, state: EntityState) {
        self.inner.update(id, state.clone());

        if matches!(state, EntityState::Unavailable) {
            return;
        }

        let is_firmware_push = self
            .firmware_states
            .lock()
            .get(id)
            .map(|s| s == &state)
            .unwrap_or(false);

        if !is_firmware_push {
            let _ = self.cmd_tx.send((id.clone(), state));
        }
    }
}

// ── Internal types ────────────────────────────────────────────────────────────

/// Hard cap: reject devices exporting more entities than this limit to prevent
/// resource exhaustion (each entity spawns an actor + watch channel).
pub const MAX_ENTITIES_PER_DEVICE: usize = 512;

/// Abstraction over the Native API transport layer.
///
/// Both the cleartext and Noise variants support sending `(msg_type, payload)` frames.
/// In the Noise variant the TCP write half and Noise state are stored separately so
/// that the reader task can independently decrypt inbound frames.
enum FrameWriter {
    /// Standard cleartext ESPHome wire format (preamble 0x00).
    Cleartext(FramedWrite<OwnedWriteHalf, EspHomeCodec>),
    /// ESPHome Noise encrypted transport (preamble 0x01).
    Noise {
        write_half: OwnedWriteHalf,
        noise: SharedNoise,
    },
}

impl FrameWriter {
    /// Send one Native API frame.  Errors are logged but do not propagate — callers
    /// should proceed even if a single frame fails (the reader task will detect
    /// the broken connection and fire `ReaderClosed`).
    async fn send_frame(&mut self, msg_type: u32, payload: Vec<u8>) {
        match self {
            FrameWriter::Cleartext(w) => {
                let _ = w.send((msg_type, payload)).await;
            }
            FrameWriter::Noise { write_half, noise } => {
                let _ = noise_send_frame(write_half, noise, msg_type, &payload).await;
            }
        }
    }

    /// Attempt to send one Native API frame, returning an error on failure.
    async fn try_send_frame(
        &mut self,
        msg_type: u32,
        payload: Vec<u8>,
    ) -> Result<(), DeviceLinkError> {
        match self {
            FrameWriter::Cleartext(w) => w
                .send((msg_type, payload))
                .await
                .map_err(DeviceLinkError::Connect),
            FrameWriter::Noise { write_half, noise } => {
                noise_send_frame(write_half, noise, msg_type, &payload)
                    .await
                    .map_err(DeviceLinkError::Connect)
            }
        }
    }
}

/// Metadata parsed from a `LIST_ENTITIES_*` frame, used during handshake.
#[derive(Debug)]
pub(crate) struct ParsedEntityMeta {
    pub key: u32,
    pub object_id: String,
    pub unique_id: Option<String>,
    pub name: String,
    pub domain: &'static str,
    pub initial_state: EntityState,
}

/// Handshake sub-state (used inside `SessionFsm::Handshake`).
enum HandshakePhase {
    AwaitHello,
    AwaitDeviceInfo,
    CollectingEntities { entities: Vec<ParsedEntityMeta> },
}

/// The session connection FSM.
enum SessionFsm {
    Idle,
    Handshake {
        writer: FrameWriter,
        phase: HandshakePhase,
    },
    Active {
        writer: FrameWriter,
    },
    Reconnecting,
    Stopped,
}

// ── Messages ──────────────────────────────────────────────────────────────────

#[allow(clippy::module_name_repetitions)]
pub enum DeviceSessionMsg {
    Connect,
    Reconnect,
    InboundFrame {
        msg_type: u32,
        payload: Vec<u8>,
    },
    ReaderClosed,
    /// A malformed inbound frame was received — the error is recorded and the session
    /// reconnects. The offending session is isolated; other sessions are unaffected.
    InboundFrameError {
        error: String,
    },
    /// A local command has updated an entity — relay the new state to firmware.
    RelayCommand {
        local_id: EntityId,
        new_state: EntityState,
    },
    /// Route a command from an external caller (e.g. `DeviceLinkManagerActor::DispatchCommand`).
    SendCommand {
        local_entity_id: EntityId,
        state: EntityState,
        reply: oneshot::Sender<Result<(), DeviceLinkError>>,
    },
    /// Fired every `CLIENT_PING_INTERVAL_SECS` while Active; checks inactivity.
    PingTick,
    Stop,
}

// ── DeviceSessionActor ────────────────────────────────────────────────────────

/// Manages the TCP connection lifecycle for one ESPHome firmware device.
///
/// Lifecycle:
/// 1. `Connect` → TCP handshake (Hello → DeviceInfo → ListEntities → SubscribeStates)
/// 2. Active: ingest pushed state frames into EntityActors; relay commands to firmware
/// 3. On disconnect: mark entities `Unavailable`, schedule reconnect with exponential backoff
/// 4. On `Stop`: mark entities `Unavailable`, stop actor and all child EntityActors
pub struct DeviceSessionActor {
    device: DiscoveredDevice,
    /// Per-device security configuration (Noise PSK if applicable).
    security_config: DeviceSecurityConfig,
    fsm: SessionFsm,
    manager_ref: ActorRef<DeviceLinkManagerMsg>,
    device_manager: ActorRef<DeviceManagerMsg>,
    entity_registry: EntityRegistry,
    state_store: Arc<StateStore>,
    /// remote FNV key → local EntityId  (for ingest)
    key_to_entity: HashMap<u32, EntityId>,
    /// local EntityId → remote FNV key  (for command relay)
    entity_to_key: HashMap<EntityId, u32>,
    /// EntityId → ActorRef for sending SetState
    entity_refs: HashMap<EntityId, ActorRef<EntityMsg>>,
    /// Imported entity bindings keyed by local EntityId.
    bindings: HashMap<EntityId, ImportedEntityBinding>,
    /// DeviceActor for the imported device (created on first handshake)
    device_ref: Option<ActorRef<DeviceMsg>>,
    /// Shared "last firmware state" table — updated before each SetState so
    /// RemoteStateUpdater knows the change is a firmware push, not a command.
    firmware_states: Arc<Mutex<HashMap<EntityId, EntityState>>>,
    /// The state updater shared with all child EntityActors.
    updater: Option<Arc<RemoteStateUpdater>>,
    /// Receives command-relay requests from `RemoteStateUpdater`; consumed in `pre_start`.
    cmd_rx: Option<mpsc::UnboundedReceiver<(EntityId, EntityState)>>,
    /// Number of consecutive reconnect attempts; reset to 0 on Active entry.
    reconnect_attempts: u32,
    /// Tokio instant of the last received frame; used for client-side inactivity timeout.
    last_activity_at: Option<tokio::time::Instant>,
    /// Canonical DeviceId resolved from MAC (set in AwaitDeviceInfo).
    canonical_id: Option<DeviceId>,
    /// Device name from DeviceInfoResponse (used in DeviceDescriptor on first handshake).
    device_info_name: Option<String>,
    /// Device model string from DeviceInfoResponse.
    device_info_model: Option<String>,
    /// Firmware version from DeviceInfoResponse.esphome_version.
    device_info_sw_version: Option<String>,
    /// Runtime ingest limits shared with the device-link manager.
    limits: DeviceLinkLimits,
}

impl DeviceSessionActor {
    pub fn new(
        device: DiscoveredDevice,
        manager_ref: ActorRef<DeviceLinkManagerMsg>,
        device_manager: ActorRef<DeviceManagerMsg>,
        entity_registry: EntityRegistry,
        state_store: Arc<StateStore>,
        limits: DeviceLinkLimits,
    ) -> Self {
        Self::with_security(
            device,
            DeviceSecurityConfig::default(),
            manager_ref,
            device_manager,
            entity_registry,
            state_store,
            limits,
        )
    }

    /// Construct a session with an explicit per-device security configuration.
    pub fn with_security(
        device: DiscoveredDevice,
        security_config: DeviceSecurityConfig,
        manager_ref: ActorRef<DeviceLinkManagerMsg>,
        device_manager: ActorRef<DeviceManagerMsg>,
        entity_registry: EntityRegistry,
        state_store: Arc<StateStore>,
        limits: DeviceLinkLimits,
    ) -> Self {
        let firmware_states = Arc::new(Mutex::new(HashMap::new()));
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let updater = Arc::new(RemoteStateUpdater::new(
            state_store.clone(),
            firmware_states.clone(),
            cmd_tx,
        ));
        Self {
            device,
            security_config,
            fsm: SessionFsm::Idle,
            manager_ref,
            device_manager,
            entity_registry,
            state_store,
            key_to_entity: HashMap::new(),
            entity_to_key: HashMap::new(),
            entity_refs: HashMap::new(),
            bindings: HashMap::new(),
            device_ref: None,
            firmware_states,
            updater: Some(updater),
            cmd_rx: Some(cmd_rx),
            reconnect_attempts: 0,
            last_activity_at: None,
            canonical_id: None,
            device_info_name: None,
            device_info_model: None,
            device_info_sw_version: None,
            limits,
        }
    }

    async fn connect_and_send_hello(&mut self, ctx: &mut ActorContext<DeviceSessionMsg>) {
        // Notify manager we're attempting a connection
        let _ = self
            .manager_ref
            .send(DeviceLinkManagerMsg::SessionStatusChanged {
                provisional_id: self.device.device_id.clone(),
                status: SessionStatus::Connecting,
                entity_count: self.key_to_entity.len(),
            });

        let addr = (self.device.ip, self.device.port);
        match TcpStream::connect(addr).await {
            Ok(stream) => {
                let (mut read, mut write) = stream.into_split();

                // ── Select transport (cleartext or Noise) ──────────────────────
                let writer = if self.security_config.uses_noise() {
                    match self.security_config.decode_psk() {
                        Some(Ok(psk)) => match noise_handshake(&mut read, &mut write, &psk).await {
                            Ok(noise) => FrameWriter::Noise {
                                write_half: write,
                                noise,
                            },
                            Err(e) => {
                                tracing::warn!(
                                    device_id = %self.device.device_id,
                                    "Noise handshake failed: {e}",
                                );
                                self.schedule_reconnect(ctx);
                                return;
                            }
                        },
                        Some(Err(e)) => {
                            tracing::warn!(
                                device_id = %self.device.device_id,
                                "invalid Noise PSK configuration: {e}",
                            );
                            self.schedule_reconnect(ctx);
                            return;
                        }
                        None => {
                            // uses_noise() returned true but decode_psk() returned None — shouldn't happen
                            tracing::warn!(
                                device_id = %self.device.device_id,
                                "Noise is enabled but PSK is missing; falling back to cleartext",
                            );
                            FrameWriter::Cleartext(FramedWrite::new(write, EspHomeCodec))
                        }
                    }
                } else {
                    FrameWriter::Cleartext(FramedWrite::new(write, EspHomeCodec))
                };

                // ── Spawn reader task ──────────────────────────────────────────
                let self_ref = ctx.self_ref().clone();
                spawn_reader_task(
                    read,
                    self_ref,
                    self.security_config
                        .uses_noise()
                        .then(|| {
                            // For the Noise reader we need a reference to the shared state.
                            // Extract it from the writer we just created.
                            match &writer {
                                FrameWriter::Noise { noise, .. } => Some(noise.clone()),
                                _ => None,
                            }
                        })
                        .flatten(),
                );

                // ── Send Native API Hello ──────────────────────────────────────
                let mut writer = writer;
                let hello = HelloRequest {
                    client_info: format!("rshome-device-link {}", env!("CARGO_PKG_VERSION")),
                    api_version_major: 1,
                    api_version_minor: 10,
                };
                writer
                    .send_frame(msg_types::HELLO_REQUEST, hello.encode_to_vec())
                    .await;

                // Notify manager we're in the Native API handshake
                let _ = self
                    .manager_ref
                    .send(DeviceLinkManagerMsg::SessionStatusChanged {
                        provisional_id: self.device.device_id.clone(),
                        status: SessionStatus::Handshaking,
                        entity_count: self.key_to_entity.len(),
                    });

                self.fsm = SessionFsm::Handshake {
                    writer,
                    phase: HandshakePhase::AwaitHello,
                };
            }
            Err(e) => {
                tracing::warn!(
                    device_id = %self.device.device_id,
                    "connection to {}:{} failed: {e}",
                    self.device.ip,
                    self.device.port,
                );
                self.schedule_reconnect(ctx);
            }
        }
    }

    fn schedule_reconnect(&mut self, ctx: &mut ActorContext<DeviceSessionMsg>) {
        let delay = backoff_delay(self.reconnect_attempts);
        let _ = self
            .manager_ref
            .send(DeviceLinkManagerMsg::SessionStatusChanged {
                provisional_id: self.device.device_id.clone(),
                status: SessionStatus::Backoff {
                    attempt: self.reconnect_attempts,
                    delay_secs: delay.as_secs(),
                },
                entity_count: self.key_to_entity.len(),
            });
        self.reconnect_attempts += 1;
        self.fsm = SessionFsm::Reconnecting;
        let self_ref = ctx.self_ref().clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = self_ref.send(DeviceSessionMsg::Reconnect);
        });
    }

    async fn handle_inbound(
        &mut self,
        msg_type: u32,
        payload: Vec<u8>,
        ctx: &mut ActorContext<DeviceSessionMsg>,
    ) {
        // Any inbound frame resets the inactivity timer
        self.last_activity_at = Some(tokio::time::Instant::now());

        let fsm = std::mem::replace(&mut self.fsm, SessionFsm::Stopped);
        match fsm {
            SessionFsm::Handshake { writer, phase } => {
                self.fsm = self
                    .handle_handshake(msg_type, payload, writer, phase, ctx)
                    .await;
            }
            SessionFsm::Active { mut writer } => {
                self.handle_active(msg_type, &payload, &mut writer).await;
                self.fsm = SessionFsm::Active { writer };
            }
            other => {
                self.fsm = other;
            }
        }
    }

    async fn handle_handshake(
        &mut self,
        msg_type: u32,
        payload: Vec<u8>,
        mut writer: FrameWriter,
        phase: HandshakePhase,
        ctx: &mut ActorContext<DeviceSessionMsg>,
    ) -> SessionFsm {
        match phase {
            HandshakePhase::AwaitHello if msg_type == msg_types::HELLO_RESPONSE => {
                writer
                    .send_frame(
                        msg_types::DEVICE_INFO_REQUEST,
                        DeviceInfoRequest {}.encode_to_vec(),
                    )
                    .await;
                SessionFsm::Handshake {
                    writer,
                    phase: HandshakePhase::AwaitDeviceInfo,
                }
            }
            HandshakePhase::AwaitDeviceInfo if msg_type == msg_types::DEVICE_INFO_RESPONSE => {
                let resp = match DeviceInfoResponse::decode(&payload[..]) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!(
                            device_id = %self.device.device_id,
                            "failed to decode DeviceInfoResponse: {e}",
                        );
                        self.schedule_reconnect(ctx);
                        return SessionFsm::Reconnecting;
                    }
                };

                // Store device info for use in finalize_entities
                self.device_info_name = Some(resp.name.clone());
                self.device_info_model = opt_str(resp.model.clone());
                self.device_info_sw_version = opt_str(resp.esphome_version.clone());

                // Derive canonical ID from MAC if present, otherwise keep provisional
                let canonical_id = if !resp.mac_address.is_empty() {
                    device_id_from_mac(&resp.mac_address)
                } else {
                    self.device.device_id.clone()
                };
                self.canonical_id = Some(canonical_id.clone());

                // Task 2.4: password-based auth is unsupported in v1.
                if resp.uses_password {
                    tracing::warn!(
                        device_id = %self.device.device_id,
                        "device requires password auth; marking UnsupportedAuth (permanent stop)",
                    );
                    let _ = self
                        .manager_ref
                        .send(DeviceLinkManagerMsg::SessionIdentityResolved {
                            provisional_id: self.device.device_id.clone(),
                            canonical_id: canonical_id.clone(),
                            name: resp.name.clone(),
                            friendly_name: opt_str(resp.friendly_name.clone()),
                            mac_address: opt_str(resp.mac_address.clone()),
                            model: opt_str(resp.model.clone()),
                            sw_version: opt_str(resp.esphome_version.clone()),
                        });
                    let _ = self
                        .manager_ref
                        .send(DeviceLinkManagerMsg::SessionStatusChanged {
                            provisional_id: self.device.device_id.clone(),
                            status: SessionStatus::UnsupportedAuth,
                            entity_count: 0,
                        });
                    ctx.stop();
                    return SessionFsm::Reconnecting;
                }

                let _ = self
                    .manager_ref
                    .send(DeviceLinkManagerMsg::SessionIdentityResolved {
                        provisional_id: self.device.device_id.clone(),
                        canonical_id,
                        name: resp.name,
                        friendly_name: opt_str(resp.friendly_name),
                        mac_address: opt_str(resp.mac_address),
                        model: opt_str(resp.model),
                        sw_version: opt_str(resp.esphome_version),
                    });

                writer
                    .send_frame(
                        msg_types::LIST_ENTITIES_REQUEST,
                        ListEntitiesRequest {}.encode_to_vec(),
                    )
                    .await;
                SessionFsm::Handshake {
                    writer,
                    phase: HandshakePhase::CollectingEntities {
                        entities: Vec::new(),
                    },
                }
            }
            HandshakePhase::CollectingEntities { mut entities } => {
                if msg_type == msg_types::LIST_ENTITIES_DONE {
                    self.finalize_entities(entities, ctx).await;
                    // Send SubscribeStates — firmware starts pushing immediately
                    writer
                        .send_frame(
                            msg_types::SUBSCRIBE_STATES,
                            SubscribeStatesRequest {}.encode_to_vec(),
                        )
                        .await;
                    let entity_count = self.entity_refs.len();
                    tracing::info!(
                        device_id = %self.device.device_id,
                        entity_count,
                        "device session active",
                    );
                    // Reset backoff counter and record activity time
                    self.reconnect_attempts = 0;
                    self.last_activity_at = Some(tokio::time::Instant::now());

                    let _ = self
                        .manager_ref
                        .send(DeviceLinkManagerMsg::SessionStatusChanged {
                            provisional_id: self.device.device_id.clone(),
                            status: SessionStatus::Active,
                            entity_count,
                        });

                    // Spawn client-side ping ticker
                    let self_ref = ctx.self_ref().clone();
                    tokio::spawn(async move {
                        let mut interval = tokio::time::interval(std::time::Duration::from_secs(
                            rshome_native_api::CLIENT_PING_INTERVAL_SECS,
                        ));
                        interval.tick().await; // skip immediate first tick
                        loop {
                            interval.tick().await;
                            if self_ref.send(DeviceSessionMsg::PingTick).is_err() {
                                break;
                            }
                        }
                    });

                    SessionFsm::Active { writer }
                } else {
                    if let Some(meta) = parse_entity_list_frame(msg_type, &payload) {
                        entities.push(meta);
                    }
                    SessionFsm::Handshake {
                        writer,
                        phase: HandshakePhase::CollectingEntities { entities },
                    }
                }
            }
            // Unexpected frame during handshake — stay in current phase
            other_phase => SessionFsm::Handshake {
                writer,
                phase: other_phase,
            },
        }
    }

    async fn handle_active(&mut self, msg_type: u32, payload: &[u8], writer: &mut FrameWriter) {
        if is_state_frame(msg_type) {
            self.ingest_state(msg_type, payload);
        } else if msg_type == msg_types::PING_REQUEST {
            writer
                .send_frame(msg_types::PING_RESPONSE, PingResponse {}.encode_to_vec())
                .await;
        }
        // PING_RESPONSE and other frame types are silently ignored
    }

    async fn finalize_entities(
        &mut self,
        entities: Vec<ParsedEntityMeta>,
        ctx: &mut ActorContext<DeviceSessionMsg>,
    ) {
        let slug = device_slug(&self.device.name);
        let device_id = self
            .canonical_id
            .clone()
            .unwrap_or_else(|| self.device.device_id.clone());

        // Create DeviceActor once (reuse on reconnect).
        // Use device info from DeviceInfoResponse when available.
        if self.device_ref.is_none() {
            let name = self
                .device_info_name
                .clone()
                .unwrap_or_else(|| self.device.name.clone());
            let sw_version = self
                .device_info_sw_version
                .clone()
                .or_else(|| Some(self.device.version.clone()));
            let desc = DeviceDescriptor {
                device_id: device_id.clone(),
                name,
                model: self.device_info_model.clone(),
                manufacturer: Some("ESPHome-compatible".to_string()),
                sw_version,
                area_id: None,
            };
            match self
                .device_manager
                .ask(|reply| DeviceManagerMsg::AddDevice {
                    descriptor: desc,
                    reply,
                })
                .await
            {
                Ok(device_ref) => self.device_ref = Some(device_ref),
                Err(e) => {
                    tracing::warn!("failed to register device: {e}");
                    return;
                }
            }
        }

        if entities.is_empty() {
            return;
        }

        // Hard entity cap: refuse to import if the device exports too many entities.
        // This prevents a misbehaving firmware from spawning thousands of actors.
        let per_device_limit = self.limits.max_entities.min(MAX_ENTITIES_PER_DEVICE);
        if entities.len() > per_device_limit {
            tracing::error!(
                device_id = %device_id,
                entity_count = entities.len(),
                limit = per_device_limit,
                "device exports more entities than the hard cap; marking unavailable",
            );
            self.mark_entities_unavailable();
            self.schedule_reconnect(ctx);
            return;
        }

        let new_entities_needed = entities
            .iter()
            .filter(|meta| {
                let object_id = format!("{slug}__{}", meta.object_id);
                let entity_id = EntityId::new(meta.domain, &object_id);
                !self.entity_refs.contains_key(&entity_id)
                    && self.entity_registry.get(&entity_id).is_none()
            })
            .count();
        let current_entities = self.entity_registry.count();
        if current_entities.saturating_add(new_entities_needed) > self.limits.max_entities {
            tracing::error!(
                device_id = %device_id,
                current_entities,
                new_entities_needed,
                max_entities = self.limits.max_entities,
                "import would exceed the configured entity limit; marking device unavailable",
            );
            self.mark_entities_unavailable();
            self.schedule_reconnect(ctx);
            return;
        }

        let updater: Arc<dyn StateUpdater> = match &self.updater {
            Some(u) => u.clone() as Arc<dyn StateUpdater>,
            None => return,
        };

        // Track which entity IDs are in the new list so we can detect disappearing entities.
        let mut new_entity_ids: std::collections::HashSet<EntityId> =
            std::collections::HashSet::new();

        for meta in entities {
            let object_id = format!("{slug}__{}", meta.object_id);
            let entity_id = EntityId::new(meta.domain, &object_id);
            new_entity_ids.insert(entity_id.clone());

            // Update key mappings (handles firmware updates that change keys)
            self.key_to_entity.insert(meta.key, entity_id.clone());
            self.entity_to_key.insert(entity_id.clone(), meta.key);

            // Determine command capabilities from domain
            let command_capabilities = command_capabilities_for_domain(meta.domain);

            // Create or update the ImportedEntityBinding
            let binding = ImportedEntityBinding {
                local_entity_id: entity_id.clone(),
                device_id: device_id.clone(),
                remote_key: meta.key,
                remote_object_id: meta.object_id.clone(),
                remote_unique_id: meta.unique_id,
                remote_domain: meta.domain.to_string(),
                command_capabilities,
            };
            self.bindings.insert(entity_id.clone(), binding);

            if self.entity_refs.contains_key(&entity_id) {
                // Reuse existing entity actor across reconnects
                continue;
            }

            // Resolve domain metadata — skip entities with unrecognised wire types.
            let Some((domain_id, feature_set)) =
                DomainRegistry::built_in().resolve_wire_type(meta.domain)
            else {
                tracing::warn!(
                    device_id = %device_id,
                    wire_type = meta.domain,
                    "skipping entity with unregistered domain wire type",
                );
                continue;
            };

            let entity_desc = EntityDescriptor {
                entity_id: entity_id.clone(),
                name: meta.name,
                icon: None,
                device_id: Some(device_id.clone()),
                area_id: None,
                entity_category: EntityCategory::None,
                domain_id: domain_id.to_string(),
                feature_set,
                device_class: None,
            };

            if let Some(existing_ref) = self.entity_registry.get(&entity_id) {
                self.entity_registry
                    .register_descriptor(entity_desc.clone());
                if let Some(device_ref) = &self.device_ref {
                    if let Err(error) = device_ref.send(DeviceMsg::AttachEntity {
                        descriptor: entity_desc,
                        entity_ref: existing_ref.clone(),
                    }) {
                        tracing::warn!(
                            device_id = %device_id,
                            entity_id = %entity_id,
                            %error,
                            "failed to reattach restored entity to device actor"
                        );
                    }
                }
                self.entity_refs.insert(entity_id.clone(), existing_ref);
                continue;
            }

            let initial_state = meta.initial_state;
            self.state_store.update(&entity_id, initial_state.clone());

            let (entity_actor, _) =
                EntityActor::new(entity_desc.clone(), initial_state, updater.clone());
            let entity_ref = ctx.spawn_child(entity_actor, SupervisorStrategy::default());

            self.entity_registry
                .register_descriptor(entity_desc.clone());
            self.entity_registry
                .register(entity_id.clone(), entity_ref.clone());
            if let Some(device_ref) = &self.device_ref {
                if let Err(error) = device_ref.send(DeviceMsg::AttachEntity {
                    descriptor: entity_desc,
                    entity_ref: entity_ref.clone(),
                }) {
                    tracing::warn!(
                        device_id = %device_id,
                        entity_id = %entity_id,
                        %error,
                        "failed to attach imported entity to device actor"
                    );
                }
            }
            self.entity_refs.insert(entity_id.clone(), entity_ref);
        }

        // Entities that existed before but are not in the new list have already been
        // marked Unavailable by mark_entities_unavailable() on disconnect.  We leave
        // them in entity_refs (PRD: "mark unavailable instead of deleting immediately"),
        // but we can explicitly ensure they remain Unavailable in case of a reconnect
        // that arrives without a prior disconnect event.
        let disappeared: Vec<EntityId> = self
            .entity_refs
            .keys()
            .filter(|id| !new_entity_ids.contains(*id))
            .cloned()
            .collect();
        for entity_id in disappeared {
            let mut fw = self.firmware_states.lock();
            fw.remove(&entity_id);
            drop(fw);
            if let Some(entity_ref) = self.entity_refs.get(&entity_id) {
                let _ = entity_ref.send(EntityMsg::SetState(EntityState::Unavailable));
            }
        }
    }

    fn ingest_state(&mut self, msg_type: u32, payload: &[u8]) {
        let ingested = match parse_state_frame(msg_type, payload) {
            Ok(Some(s)) => s,
            _ => return,
        };

        let entity_id = match self.key_to_entity.get(&ingested.key) {
            Some(id) => id.clone(),
            None => return,
        };

        let entity_ref = match self.entity_refs.get(&entity_id) {
            Some(r) => r.clone(),
            None => return,
        };

        // Record the firmware-pushed state so RemoteStateUpdater won't echo it back
        self.firmware_states
            .lock()
            .insert(entity_id, ingested.state.clone());
        let _ = entity_ref.send(EntityMsg::SetState(ingested.state));
    }

    async fn relay_command_to_firmware(&mut self, local_id: EntityId, new_state: EntityState) {
        let key = match self.entity_to_key.get(&local_id) {
            Some(k) => *k,
            None => return,
        };

        let (msg_type, payload) = match state_to_command(key, &new_state) {
            Some((t, p)) => (t, p),
            None => return,
        };

        let fsm = std::mem::replace(&mut self.fsm, SessionFsm::Stopped);
        if let SessionFsm::Active { mut writer } = fsm {
            writer.send_frame(msg_type, payload).await;
            self.fsm = SessionFsm::Active { writer };
        } else {
            self.fsm = fsm;
        }
    }

    async fn send_command_to_firmware(
        &mut self,
        local_entity_id: EntityId,
        state: EntityState,
        reply: oneshot::Sender<Result<(), DeviceLinkError>>,
    ) {
        // Resolve entity → remote key
        let key = match self.entity_to_key.get(&local_entity_id) {
            Some(k) => *k,
            None => {
                let _ = reply.send(Err(DeviceLinkError::EntityNotFound(
                    local_entity_id.to_string(),
                )));
                return;
            }
        };

        // Convert state to command frame
        let (msg_type, payload) = match state_to_command(key, &state) {
            Some(f) => f,
            None => {
                let _ = reply.send(Err(DeviceLinkError::CommandNotSupported(
                    local_entity_id.to_string(),
                )));
                return;
            }
        };

        // Session must be Active to send
        let fsm = std::mem::replace(&mut self.fsm, SessionFsm::Stopped);
        if let SessionFsm::Active { mut writer } = fsm {
            let result = writer.try_send_frame(msg_type, payload).await;
            self.fsm = SessionFsm::Active { writer };
            let _ = reply.send(result);
        } else {
            self.fsm = fsm;
            let _ = reply.send(Err(DeviceLinkError::SessionNotActive));
        }
    }

    fn mark_entities_unavailable(&self) {
        let mut fw = self.firmware_states.lock();
        for (entity_id, entity_ref) in &self.entity_refs {
            fw.remove(entity_id);
            let _ = entity_ref.send(EntityMsg::SetState(EntityState::Unavailable));
        }
    }
}

#[async_trait::async_trait]
impl Actor for DeviceSessionActor {
    type Msg = DeviceSessionMsg;

    async fn pre_start(&mut self, ctx: &mut ActorContext<Self::Msg>) {
        // Bridge the command relay channel into the actor mailbox
        if let Some(mut cmd_rx) = self.cmd_rx.take() {
            let self_ref = ctx.self_ref().clone();
            tokio::spawn(async move {
                while let Some((id, state)) = cmd_rx.recv().await {
                    if self_ref
                        .send(DeviceSessionMsg::RelayCommand {
                            local_id: id,
                            new_state: state,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }
        // Start connection immediately
        ctx.self_ref().send(DeviceSessionMsg::Connect).ok();
    }

    async fn handle(&mut self, msg: DeviceSessionMsg, ctx: &mut ActorContext<Self::Msg>) {
        match msg {
            DeviceSessionMsg::Connect | DeviceSessionMsg::Reconnect => {
                self.connect_and_send_hello(ctx).await;
            }
            DeviceSessionMsg::InboundFrame { msg_type, payload } => {
                self.handle_inbound(msg_type, payload, ctx).await;
            }
            DeviceSessionMsg::ReaderClosed => {
                tracing::warn!(device_id = %self.device.device_id, "connection lost, reconnecting");
                self.mark_entities_unavailable();
                self.schedule_reconnect(ctx);
            }
            DeviceSessionMsg::InboundFrameError { error } => {
                tracing::warn!(
                    device_id = %self.device.device_id,
                    error = %error,
                    "malformed inbound frame, closing session and reconnecting",
                );
                // Store the error for status reporting
                let _ = self
                    .manager_ref
                    .send(DeviceLinkManagerMsg::SessionStatusChanged {
                        provisional_id: self.device.device_id.clone(),
                        status: SessionStatus::Backoff {
                            attempt: self.reconnect_attempts,
                            delay_secs: backoff_delay(self.reconnect_attempts).as_secs(),
                        },
                        entity_count: self.key_to_entity.len(),
                    });
                self.mark_entities_unavailable();
                self.schedule_reconnect(ctx);
            }
            DeviceSessionMsg::RelayCommand {
                local_id,
                new_state,
            } => {
                self.relay_command_to_firmware(local_id, new_state).await;
            }
            DeviceSessionMsg::SendCommand {
                local_entity_id,
                state,
                reply,
            } => {
                self.send_command_to_firmware(local_entity_id, state, reply)
                    .await;
            }
            DeviceSessionMsg::PingTick => {
                // Only act when in Active state
                if !matches!(self.fsm, SessionFsm::Active { .. }) {
                    return;
                }
                // Check inactivity timeout
                if let Some(last) = self.last_activity_at {
                    if last.elapsed()
                        >= std::time::Duration::from_secs(
                            rshome_native_api::CLIENT_INACTIVITY_TIMEOUT_SECS,
                        )
                    {
                        tracing::warn!(
                            device_id = %self.device.device_id,
                            "inactivity timeout ({} s), reconnecting",
                            rshome_native_api::CLIENT_INACTIVITY_TIMEOUT_SECS,
                        );
                        self.mark_entities_unavailable();
                        self.schedule_reconnect(ctx);
                        return;
                    }
                }
                // Send ping to firmware
                let fsm = std::mem::replace(&mut self.fsm, SessionFsm::Stopped);
                if let SessionFsm::Active { mut writer } = fsm {
                    writer
                        .send_frame(msg_types::PING_REQUEST, PingRequest {}.encode_to_vec())
                        .await;
                    self.fsm = SessionFsm::Active { writer };
                }
            }
            DeviceSessionMsg::Stop => {
                self.mark_entities_unavailable();
                self.fsm = SessionFsm::Stopped;
                ctx.stop();
            }
        }
    }

    async fn post_stop(&mut self) {
        self.mark_entities_unavailable();
        // Explicitly close the TCP writer (if any) by transitioning FSM to Stopped.
        self.fsm = SessionFsm::Stopped;
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Spawn the reader task for a device session.
///
/// For cleartext sessions `noise` is `None` and the task uses `FramedRead<_, EspHomeCodec>`.
/// For Noise sessions `noise` is `Some(SharedNoise)` and the task decrypts each frame
/// before forwarding it as an `InboundFrame` message.
fn spawn_reader_task(
    read: OwnedReadHalf,
    self_ref: ActorRef<DeviceSessionMsg>,
    noise: Option<SharedNoise>,
) {
    if let Some(shared_noise) = noise {
        tokio::spawn(async move {
            let mut r = read;
            loop {
                match noise_recv_frame(&mut r, &shared_noise).await {
                    Some(Ok((msg_type, payload))) => {
                        if self_ref
                            .send(DeviceSessionMsg::InboundFrame { msg_type, payload })
                            .is_err()
                        {
                            break;
                        }
                    }
                    _ => {
                        let _ = self_ref.send(DeviceSessionMsg::ReaderClosed);
                        break;
                    }
                }
            }
        });
    } else {
        tokio::spawn(async move {
            let mut framed = FramedRead::new(read, EspHomeCodec);
            loop {
                match framed.next().await {
                    Some(Ok((msg_type, payload))) => {
                        if self_ref
                            .send(DeviceSessionMsg::InboundFrame { msg_type, payload })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Some(Err(e)) if e.kind() == std::io::ErrorKind::InvalidData => {
                        // Malformed frame: isolate this session by sending the error
                        // detail, then close the reader. Other sessions are unaffected.
                        let _ = self_ref.send(DeviceSessionMsg::InboundFrameError {
                            error: e.to_string(),
                        });
                        break;
                    }
                    _ => {
                        let _ = self_ref.send(DeviceSessionMsg::ReaderClosed);
                        break;
                    }
                }
            }
        });
    }
}

/// Convert an empty string to `None`; non-empty string to `Some`.
fn opt_str(s: String) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Exponential backoff delay for reconnect attempts.
/// Sequence (capped at 30 s): 1, 2, 4, 8, 16, 30, 30, ...
fn backoff_delay(attempt: u32) -> std::time::Duration {
    const DELAYS: [u64; 6] = [1, 2, 4, 8, 16, 30];
    let idx = (attempt as usize).min(DELAYS.len() - 1);
    std::time::Duration::from_secs(DELAYS[idx])
}

/// Map an entity domain string to its command capabilities.
fn command_capabilities_for_domain(domain: &str) -> ImportedCommandCapabilities {
    match domain {
        "sensor" | "binary_sensor" | "text_sensor" => ImportedCommandCapabilities::ReadOnly,
        _ => ImportedCommandCapabilities::Controllable,
    }
}

// ── Entity list frame parsing ─────────────────────────────────────────────────

/// Parse a `LIST_ENTITIES_*` frame into entity metadata.
/// Returns `None` for unknown message types.
pub(crate) fn parse_entity_list_frame(msg_type: u32, payload: &[u8]) -> Option<ParsedEntityMeta> {
    use rshome_entity::{CoverState, EntityState};
    use std::collections::HashMap;

    match msg_type {
        t if t == msg_types::LIST_ENTITIES_SENSOR => {
            let m = ListEntitiesSensorResponse::decode(payload).ok()?;
            let unit = if m.unit_of_measurement.is_empty() {
                None
            } else {
                Some(m.unit_of_measurement)
            };
            Some(ParsedEntityMeta {
                key: m.key,
                object_id: m.object_id,
                unique_id: opt_str(m.unique_id),
                name: m.name,
                domain: "sensor",
                initial_state: EntityState::Sensor {
                    value: 0.0,
                    unit,
                    attributes: HashMap::new(),
                },
            })
        }
        t if t == msg_types::LIST_ENTITIES_BINARY_SENSOR => {
            let m = ListEntitiesBinarySensorResponse::decode(payload).ok()?;
            Some(ParsedEntityMeta {
                key: m.key,
                object_id: m.object_id,
                unique_id: opt_str(m.unique_id),
                name: m.name,
                domain: "binary_sensor",
                initial_state: EntityState::BinarySensor {
                    is_on: false,
                    attributes: HashMap::new(),
                },
            })
        }
        t if t == msg_types::LIST_ENTITIES_SWITCH => {
            let m = ListEntitiesSwitchResponse::decode(payload).ok()?;
            Some(ParsedEntityMeta {
                key: m.key,
                object_id: m.object_id,
                unique_id: opt_str(m.unique_id),
                name: m.name,
                domain: "switch",
                initial_state: EntityState::Switch { is_on: false },
            })
        }
        t if t == msg_types::LIST_ENTITIES_LIGHT => {
            let m = ListEntitiesLightResponse::decode(payload).ok()?;
            let brightness = if m.legacy_supports_brightness {
                Some(0.0)
            } else {
                None
            };
            Some(ParsedEntityMeta {
                key: m.key,
                object_id: m.object_id,
                unique_id: opt_str(m.unique_id),
                name: m.name,
                domain: "light",
                initial_state: EntityState::Light {
                    is_on: false,
                    brightness,
                    color_temp: None,
                    rgb: None,
                    color_mode: None,
                },
            })
        }
        t if t == msg_types::LIST_ENTITIES_FAN => {
            let m = ListEntitiesFanResponse::decode(payload).ok()?;
            Some(ParsedEntityMeta {
                key: m.key,
                object_id: m.object_id,
                unique_id: opt_str(m.unique_id),
                name: m.name,
                domain: "fan",
                initial_state: EntityState::Fan {
                    is_on: false,
                    speed: None,
                    oscillating: None,
                    direction: None,
                },
            })
        }
        t if t == msg_types::LIST_ENTITIES_COVER => {
            let m = ListEntitiesCoverResponse::decode(payload).ok()?;
            Some(ParsedEntityMeta {
                key: m.key,
                object_id: m.object_id,
                unique_id: opt_str(m.unique_id),
                name: m.name,
                domain: "cover",
                initial_state: EntityState::Cover {
                    state: CoverState::Stopped,
                    position: None,
                    tilt: None,
                },
            })
        }
        t if t == msg_types::LIST_ENTITIES_NUMBER => {
            let m = ListEntitiesNumberResponse::decode(payload).ok()?;
            let unit = if m.unit_of_measurement.is_empty() {
                None
            } else {
                Some(m.unit_of_measurement)
            };
            Some(ParsedEntityMeta {
                key: m.key,
                object_id: m.object_id,
                unique_id: opt_str(m.unique_id),
                name: m.name,
                domain: "number",
                initial_state: EntityState::Number {
                    value: f64::from(m.min_value),
                    min: f64::from(m.min_value),
                    max: f64::from(m.max_value),
                    step: f64::from(m.step),
                    unit,
                },
            })
        }
        t if t == msg_types::LIST_ENTITIES_SELECT => {
            let m = ListEntitiesSelectResponse::decode(payload).ok()?;
            let current = m.options.first().cloned().unwrap_or_default();
            Some(ParsedEntityMeta {
                key: m.key,
                object_id: m.object_id,
                unique_id: opt_str(m.unique_id),
                name: m.name,
                domain: "select",
                initial_state: EntityState::Select {
                    current,
                    options: m.options,
                },
            })
        }
        t if t == msg_types::LIST_ENTITIES_TEXT_SENSOR => {
            let m = ListEntitiesTextSensorResponse::decode(payload).ok()?;
            Some(ParsedEntityMeta {
                key: m.key,
                object_id: m.object_id,
                unique_id: opt_str(m.unique_id),
                name: m.name,
                domain: "text_sensor",
                initial_state: EntityState::TextSensor {
                    value: String::new(),
                },
            })
        }
        t if t == msg_types::CLIMATE_LIST => {
            let m = ListEntitiesClimateResponse::decode(payload).ok()?;
            Some(ParsedEntityMeta {
                key: m.key,
                object_id: m.object_id,
                unique_id: opt_str(m.unique_id),
                name: m.name,
                domain: "climate",
                initial_state: EntityState::Climate {
                    mode: "off".into(),
                    current_temp: None,
                    target_temp: None,
                    hvac_action: None,
                },
            })
        }
        _ => None,
    }
}

fn is_state_frame(msg_type: u32) -> bool {
    msg_type == msg_types::SENSOR_STATE
        || msg_type == msg_types::BINARY_SENSOR_STATE
        || msg_type == msg_types::SWITCH_STATE
        || msg_type == msg_types::LIGHT_STATE
        || msg_type == msg_types::FAN_STATE
        || msg_type == msg_types::COVER_STATE
        || msg_type == msg_types::NUMBER_STATE
        || msg_type == msg_types::SELECT_STATE
        || msg_type == msg_types::TEXT_SENSOR_STATE
        || msg_type == msg_types::CLIMATE_STATE
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn backoff_delay_sequence() {
        let delays: Vec<u64> = (0u32..8).map(|i| backoff_delay(i).as_secs()).collect();
        assert_eq!(delays, vec![1, 2, 4, 8, 16, 30, 30, 30]);
    }

    #[test]
    fn opt_str_empty_is_none() {
        assert_eq!(opt_str(String::new()), None);
    }

    #[test]
    fn opt_str_non_empty_is_some() {
        assert_eq!(opt_str("hi".into()), Some("hi".into()));
    }

    #[test]
    fn command_capabilities_sensor_is_read_only() {
        assert_eq!(
            command_capabilities_for_domain("sensor"),
            ImportedCommandCapabilities::ReadOnly
        );
        assert_eq!(
            command_capabilities_for_domain("binary_sensor"),
            ImportedCommandCapabilities::ReadOnly
        );
        assert_eq!(
            command_capabilities_for_domain("text_sensor"),
            ImportedCommandCapabilities::ReadOnly
        );
    }

    #[test]
    fn command_capabilities_switch_is_controllable() {
        assert_eq!(
            command_capabilities_for_domain("switch"),
            ImportedCommandCapabilities::Controllable
        );
        assert_eq!(
            command_capabilities_for_domain("light"),
            ImportedCommandCapabilities::Controllable
        );
        assert_eq!(
            command_capabilities_for_domain("climate"),
            ImportedCommandCapabilities::Controllable
        );
    }

    #[test]
    fn parse_sensor_entity_list() {
        let frame = ListEntitiesSensorResponse {
            object_id: "temperature".into(),
            key: 12345,
            name: "Temperature".into(),
            unit_of_measurement: "°C".into(),
            unique_id: "temp-unique-001".into(),
            ..Default::default()
        }
        .encode_to_vec();
        let meta = parse_entity_list_frame(msg_types::LIST_ENTITIES_SENSOR, &frame).unwrap();
        assert_eq!(meta.key, 12345);
        assert_eq!(meta.domain, "sensor");
        assert_eq!(meta.object_id, "temperature");
        assert_eq!(meta.unique_id.as_deref(), Some("temp-unique-001"));
        assert!(
            matches!(meta.initial_state, EntityState::Sensor { unit: Some(ref u), .. } if u == "°C")
        );
    }

    #[test]
    fn parse_sensor_entity_list_empty_unique_id_becomes_none() {
        let frame = ListEntitiesSensorResponse {
            object_id: "temperature".into(),
            key: 1,
            name: "Temperature".into(),
            ..Default::default()
        }
        .encode_to_vec();
        let meta = parse_entity_list_frame(msg_types::LIST_ENTITIES_SENSOR, &frame).unwrap();
        assert!(meta.unique_id.is_none(), "empty unique_id should be None");
    }

    #[test]
    fn parse_switch_entity_list() {
        let frame = ListEntitiesSwitchResponse {
            object_id: "relay".into(),
            key: 99,
            name: "Relay".into(),
            ..Default::default()
        }
        .encode_to_vec();
        let meta = parse_entity_list_frame(msg_types::LIST_ENTITIES_SWITCH, &frame).unwrap();
        assert_eq!(meta.domain, "switch");
        assert!(matches!(
            meta.initial_state,
            EntityState::Switch { is_on: false }
        ));
    }

    #[test]
    fn parse_light_supports_brightness() {
        let frame = ListEntitiesLightResponse {
            object_id: "led".into(),
            key: 7,
            name: "LED".into(),
            legacy_supports_brightness: true,
            ..Default::default()
        }
        .encode_to_vec();
        let meta = parse_entity_list_frame(msg_types::LIST_ENTITIES_LIGHT, &frame).unwrap();
        assert!(matches!(
            meta.initial_state,
            EntityState::Light {
                brightness: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn parse_number_entity_list() {
        let frame = ListEntitiesNumberResponse {
            object_id: "setpoint".into(),
            key: 55,
            name: "Setpoint".into(),
            min_value: 10.0,
            max_value: 30.0,
            step: 0.5,
            ..Default::default()
        }
        .encode_to_vec();
        let meta = parse_entity_list_frame(msg_types::LIST_ENTITIES_NUMBER, &frame).unwrap();
        assert!(
            matches!(meta.initial_state, EntityState::Number { min, max, step, .. }
                if (min - 10.0).abs() < 0.01 && (max - 30.0).abs() < 0.01 && (step - 0.5).abs() < 0.01)
        );
    }

    #[test]
    fn parse_select_entity_list_first_option_as_current() {
        let frame = ListEntitiesSelectResponse {
            object_id: "mode".into(),
            key: 3,
            name: "Mode".into(),
            options: vec!["auto".into(), "manual".into()],
            ..Default::default()
        }
        .encode_to_vec();
        let meta = parse_entity_list_frame(msg_types::LIST_ENTITIES_SELECT, &frame).unwrap();
        assert!(
            matches!(&meta.initial_state, EntityState::Select { current, .. } if current == "auto")
        );
    }

    #[test]
    fn unknown_entity_type_returns_none() {
        let result = parse_entity_list_frame(9999, &[]);
        assert!(result.is_none());
    }

    #[test]
    fn is_state_frame_recognises_known_types() {
        assert!(is_state_frame(msg_types::SENSOR_STATE));
        assert!(is_state_frame(msg_types::SWITCH_STATE));
        assert!(is_state_frame(msg_types::CLIMATE_STATE));
        assert!(!is_state_frame(msg_types::HELLO_RESPONSE));
        assert!(!is_state_frame(msg_types::DEVICE_INFO_RESPONSE));
    }

    #[test]
    fn remote_state_updater_forwards_command_changes() {
        let store = Arc::new(rshome_state::StateStore::default());
        let firmware_states = Arc::new(Mutex::new(HashMap::new()));
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel();

        let updater = RemoteStateUpdater::new(store, firmware_states.clone(), cmd_tx);

        let id = EntityId::new("switch", "relay");
        let state = EntityState::Switch { is_on: true };

        // No firmware state recorded → this looks like a command change
        updater.update(&id, state.clone());
        assert!(
            cmd_rx.try_recv().is_ok(),
            "command change should be forwarded"
        );
    }

    #[test]
    fn remote_state_updater_ignores_firmware_push() {
        let store = Arc::new(rshome_state::StateStore::default());
        let firmware_states = Arc::new(Mutex::new(HashMap::new()));
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel();

        let updater = RemoteStateUpdater::new(store, firmware_states.clone(), cmd_tx);

        let id = EntityId::new("switch", "relay");
        let state = EntityState::Switch { is_on: true };

        // Pre-populate firmware state — simulates session actor marking a SetState
        firmware_states.lock().insert(id.clone(), state.clone());

        updater.update(&id, state);
        assert!(
            cmd_rx.try_recv().is_err(),
            "firmware push must not be forwarded"
        );
    }
}
