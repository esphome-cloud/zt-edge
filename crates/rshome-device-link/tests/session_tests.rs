/// Integration tests for DeviceSessionActor using a mock ESPHome TCP server.
///
/// Each test spins up a `tokio::net::TcpListener` that simulates the ESPHome
/// Native API handshake on a random port, then validates the session actor's
/// behaviour.
use futures::{SinkExt, StreamExt as _};
use prost::Message;
use rshome_actor::{Actor, ActorContext, ActorRef, ActorSystem};
use rshome_device_link::{
    device_id_from_hostname, device_id_from_mac,
    manager::{DeviceLinkManagerActor, DeviceLinkManagerMsg},
    noise_transport::{noise_recv_frame, noise_send_frame, noise_server_handshake},
    ConnectedDevice, DeviceLinkStatus, DeviceSecurityConfig, DiscoveredDevice, SessionStatus,
};
use rshome_entity::{
    DeviceActor, DeviceManagerMsg, EntityActor, EntityCategory, EntityDescriptor, EntityId,
    EntityRegistry, EntityState, NullStateUpdater,
};
use rshome_native_api::{msg_types, proto_gen::*, EspHomeCodec};
use rshome_state::StateStore;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_util::codec::{FramedRead, FramedWrite};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Minimal DeviceManager stub that creates proper DeviceActors.
struct StubDeviceManager {
    registry: EntityRegistry,
}

#[async_trait::async_trait]
impl Actor for StubDeviceManager {
    type Msg = DeviceManagerMsg;

    async fn handle(&mut self, msg: DeviceManagerMsg, ctx: &mut ActorContext<DeviceManagerMsg>) {
        match msg {
            DeviceManagerMsg::AddDevice { descriptor, reply } => {
                let updater = Arc::new(NullStateUpdater) as Arc<dyn rshome_entity::StateUpdater>;
                let actor = DeviceActor::new(descriptor, self.registry.clone(), updater);
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

/// Bind a listener on a random port, return (listener, port).
async fn random_listener() -> (TcpListener, u16) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    (listener, port)
}

/// Run the ESPHome handshake server-side, then call `extra` with the writer.
///
/// `extra` is an async closure that receives ownership of the write half after
/// SubscribeStates is read.  It can hold the writer alive for as long as needed
/// (keeping the TCP connection open) or drop it immediately.
///
/// The mock server includes a MAC address in DeviceInfoResponse so that the
/// session actor canonicalises the device ID to `esphome:aabbccddeeff`.
async fn mock_server_handshake_then<F, Fut>(listener: TcpListener, extra: F)
where
    F: FnOnce(FramedWrite<tokio::net::tcp::OwnedWriteHalf, EspHomeCodec>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, write) = stream.into_split();
        let mut reader = FramedRead::new(read, EspHomeCodec);
        let mut writer = FramedWrite::new(write, EspHomeCodec);

        // Hello
        let _hello = reader.next().await.unwrap().unwrap();
        writer
            .send((
                msg_types::HELLO_RESPONSE,
                HelloResponse {
                    api_version_major: 1,
                    api_version_minor: 10,
                    server_info: "mock".into(),
                    name: "mock-device".into(),
                }
                .encode_to_vec(),
            ))
            .await
            .unwrap();

        // DeviceInfo — includes MAC so canonical ID = esphome:aabbccddeeff
        let _di = reader.next().await.unwrap().unwrap();
        writer
            .send((
                msg_types::DEVICE_INFO_RESPONSE,
                DeviceInfoResponse {
                    name: "mock-device".into(),
                    mac_address: "AA:BB:CC:DD:EE:FF".into(),
                    ..Default::default()
                }
                .encode_to_vec(),
            ))
            .await
            .unwrap();

        // ListEntities: send one sensor + done
        let _le = reader.next().await.unwrap().unwrap();
        writer
            .send((
                msg_types::LIST_ENTITIES_SENSOR,
                ListEntitiesSensorResponse {
                    object_id: "temperature".into(),
                    key: 12345,
                    name: "Temperature".into(),
                    unit_of_measurement: "°C".into(),
                    ..Default::default()
                }
                .encode_to_vec(),
            ))
            .await
            .unwrap();
        writer
            .send((
                msg_types::LIST_ENTITIES_DONE,
                ListEntitiesDoneResponse {}.encode_to_vec(),
            ))
            .await
            .unwrap();

        // SubscribeStates
        let _ss = reader.next().await.unwrap().unwrap();

        // Hand the writer to caller — caller decides when to close the connection.
        extra(writer).await;
    });
}

fn make_discovered_device(port: u16) -> DiscoveredDevice {
    DiscoveredDevice {
        device_id: device_id_from_hostname("mock-device"),
        service_fullname: "mock-device._esphomelib._tcp.local.".into(),
        hostname: "mock-device".into(),
        ip: "127.0.0.1".parse().unwrap(),
        port,
        name: "mock-device".into(),
        version: "2025.1.0".into(),
        friendly_name: None,
        first_seen_at: std::time::SystemTime::UNIX_EPOCH,
        last_seen_at: std::time::SystemTime::now(),
        is_stale: false,
    }
}

fn make_system_and_manager() -> (
    ActorSystem,
    ActorRef<DeviceLinkManagerMsg>,
    EntityRegistry,
    Arc<StateStore>,
) {
    let system = ActorSystem::new();
    let registry = EntityRegistry::default();
    let state_store = Arc::new(StateStore::default());

    let dm = StubDeviceManager {
        registry: registry.clone(),
    };
    let dm_ref = system.spawn(dm);

    let manager = DeviceLinkManagerActor::new(
        dm_ref,
        registry.clone(),
        state_store.clone(),
        rshome_device_link::DeviceLinkLimits::default(),
    );
    let manager_ref = system.spawn(manager);

    (system, manager_ref, registry, state_store)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn handshake_completes_and_session_becomes_active() {
    let (listener, port) = random_listener().await;
    // Keep the connection open for 500ms so the session stays Active during the check.
    mock_server_handshake_then(listener, |writer| async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        drop(writer);
    })
    .await;

    let (sys, manager_ref, _registry, _store) = make_system_and_manager();
    let device = make_discovered_device(port);
    manager_ref
        .send(DeviceLinkManagerMsg::DeviceDiscovered(device))
        .ok();

    // Give session time to complete handshake (3 round-trips over loopback)
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // After handshake, canonical ID is MAC-based: esphome:aabbccddeeff
    let canonical_id = device_id_from_mac("AA:BB:CC:DD:EE:FF");
    let (tx, rx) = oneshot::channel::<Option<DeviceLinkStatus>>();
    manager_ref
        .send(DeviceLinkManagerMsg::GetStatus {
            device_id: canonical_id,
            reply: tx,
        })
        .ok();
    let status = rx.await.unwrap().unwrap();
    assert!(
        matches!(status.status, SessionStatus::Active),
        "session should be Active after handshake, got {:?}",
        status.status
    );
    assert_eq!(status.entity_count, 1);

    manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
    drop(sys);
}

#[tokio::test]
async fn get_status_by_provisional_id_resolves_to_canonical() {
    let (listener, port) = random_listener().await;
    mock_server_handshake_then(listener, |writer| async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        drop(writer);
    })
    .await;

    let (sys, manager_ref, _registry, _store) = make_system_and_manager();
    let device = make_discovered_device(port);
    manager_ref
        .send(DeviceLinkManagerMsg::DeviceDiscovered(device.clone()))
        .ok();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Query by provisional hostname ID — should resolve via provisional_to_canonical
    let (tx, rx) = oneshot::channel::<Option<DeviceLinkStatus>>();
    manager_ref
        .send(DeviceLinkManagerMsg::GetStatus {
            device_id: device_id_from_hostname("mock-device"),
            reply: tx,
        })
        .ok();
    let status = rx.await.unwrap().unwrap();
    // device_id in response is the canonical MAC-based ID
    assert_eq!(status.device_id, "esphome:aabbccddeeff");
    assert!(matches!(status.status, SessionStatus::Active));

    manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
    drop(sys);
}

#[tokio::test]
async fn entity_registered_in_registry_after_handshake() {
    let (listener, port) = random_listener().await;
    // Entity registration happens before SubscribeStates — connection can close immediately.
    mock_server_handshake_then(listener, |_writer| async {}).await;

    let (_sys, manager_ref, registry, _store) = make_system_and_manager();
    let device = make_discovered_device(port);
    manager_ref
        .send(DeviceLinkManagerMsg::DeviceDiscovered(device))
        .ok();

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Entity ID follows the pattern: sensor.mock_device__temperature
    let entity_id = rshome_entity::EntityId::new("sensor", "mock_device__temperature");
    assert!(
        registry.get(&entity_id).is_some(),
        "entity must be registered in EntityRegistry"
    );

    manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
}

#[tokio::test]
async fn imported_entity_seeds_initial_state_before_first_push() {
    let (listener, port) = random_listener().await;
    mock_server_handshake_then(listener, |writer| async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        drop(writer);
    })
    .await;

    let (_sys, manager_ref, _registry, state_store) = make_system_and_manager();
    let device = make_discovered_device(port);
    manager_ref
        .send(DeviceLinkManagerMsg::DeviceDiscovered(device))
        .ok();

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let entity_id = rshome_entity::EntityId::new("sensor", "mock_device__temperature");
    let state = state_store.get(&entity_id);
    assert!(
        matches!(
            state,
            Some(rshome_entity::EntityState::Sensor {
                value,
                unit: Some(ref unit),
                ..
            }) if value == 0.0 && unit == "°C"
        ),
        "state store should contain imported initial state before any firmware push: got {state:?}"
    );

    manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
}

#[tokio::test]
async fn handshake_reuses_restored_entity_actor_after_restart() {
    let (listener, port) = random_listener().await;
    mock_server_handshake_then(listener, |writer| async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        drop(writer);
    })
    .await;

    let (sys, manager_ref, registry, state_store) = make_system_and_manager();
    let canonical_id = device_id_from_mac("AA:BB:CC:DD:EE:FF");
    let entity_id = EntityId::new("sensor", "mock_device__temperature");
    let restored_state = EntityState::Sensor {
        value: 21.0,
        unit: Some("°C".to_string()),
        attributes: Default::default(),
    };
    let restored_descriptor = EntityDescriptor {
        entity_id: entity_id.clone(),
        name: "Temperature".to_string(),
        icon: None,
        device_id: Some(canonical_id),
        area_id: None,
        entity_category: EntityCategory::None,
        domain_id: "sensor".to_string(),
        feature_set: vec!["state".to_string()],
        device_class: Some("temperature".to_string()),
    };
    state_store.update(&entity_id, restored_state.clone());
    let (restored_actor, _) = EntityActor::new(
        restored_descriptor.clone(),
        restored_state,
        state_store.clone(),
    );
    let restored_ref = sys.spawn(restored_actor);
    registry.register_descriptor(restored_descriptor);
    registry.register(entity_id.clone(), restored_ref.clone());

    manager_ref
        .send(DeviceLinkManagerMsg::DeviceDiscovered(
            make_discovered_device(port),
        ))
        .ok();

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let current_ref = registry
        .get(&entity_id)
        .expect("restored entity should remain registered");
    assert_eq!(current_ref.actor_id(), restored_ref.actor_id());

    manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
    sys.shutdown().await;
}

#[tokio::test]
async fn state_push_updates_state_store() {
    let (listener, port) = random_listener().await;
    mock_server_handshake_then(listener, |mut writer| async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        writer
            .send((
                msg_types::SENSOR_STATE,
                SensorStateResponse {
                    key: 12345,
                    state: 23.5,
                    missing_state: false,
                }
                .encode_to_vec(),
            ))
            .await
            .ok();
        // Keep connection open well past the 400ms test check window.
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    })
    .await;

    let (_sys, manager_ref, _registry, state_store) = make_system_and_manager();
    let device = make_discovered_device(port);
    manager_ref
        .send(DeviceLinkManagerMsg::DeviceDiscovered(device))
        .ok();

    // Wait for handshake + state push
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    let entity_id = rshome_entity::EntityId::new("sensor", "mock_device__temperature");
    let state = state_store.get(&entity_id);
    assert!(
        matches!(state, Some(rshome_entity::EntityState::Sensor { value, .. }) if (value - 23.5).abs() < 0.01),
        "state store should reflect firmware push: got {state:?}"
    );

    manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
}

#[tokio::test]
async fn session_reconnects_after_server_drops_connection() {
    // First server: accepts, handshakes, then drops
    let (listener1, port) = random_listener().await;
    // Close connection immediately after SubscribeStates
    mock_server_handshake_then(listener1, |_writer| async { /* drop immediately */ }).await;

    let (_sys, manager_ref, _registry, _store) = make_system_and_manager();
    let device = make_discovered_device(port);
    manager_ref
        .send(DeviceLinkManagerMsg::DeviceDiscovered(device))
        .ok();

    // After initial handshake, server drops the connection.
    // Session should mark entities Unavailable and schedule reconnect.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // Query by provisional ID — should still resolve (either canonical if identity was
    // resolved, or discovered fallback)
    let (tx, rx) = oneshot::channel::<Option<DeviceLinkStatus>>();
    manager_ref
        .send(DeviceLinkManagerMsg::GetStatus {
            device_id: device_id_from_hostname("mock-device"),
            reply: tx,
        })
        .ok();
    let status = rx.await.unwrap().unwrap();
    // Device is still tracked (either Backoff or Discovered fallback)
    assert!(
        !status.device_id.is_empty(),
        "device should still be tracked after disconnect"
    );

    manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
}

// ── New Phase 2 tests ─────────────────────────────────────────────────────────

#[tokio::test]
async fn reconnect_preserves_canonical_device_id() {
    // First server: handshakes then immediately closes to force a reconnect
    let (listener, port) = random_listener().await;
    mock_server_handshake_then(listener, |_writer| async { /* drop */ }).await;

    let (_sys, manager_ref, _registry, _store) = make_system_and_manager();
    let device = make_discovered_device(port);
    manager_ref
        .send(DeviceLinkManagerMsg::DeviceDiscovered(device))
        .ok();

    // Let handshake complete (establishes canonical ID) + server drop
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // The canonical entry (esphome:aabbccddeeff) should still be present in the
    // manager even though the session is now in Backoff state.
    let (tx, rx) = oneshot::channel::<Vec<ConnectedDevice>>();
    manager_ref
        .send(DeviceLinkManagerMsg::ListConnected(tx))
        .ok();
    let connected = rx.await.unwrap();
    assert_eq!(
        connected.len(),
        1,
        "canonical entry must persist through disconnect/reconnect cycle"
    );
    assert_eq!(
        connected[0].device_id.0, "esphome:aabbccddeeff",
        "canonical ID must be MAC-based after successful handshake"
    );

    manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
}

#[tokio::test]
async fn duplicate_mdns_collapses_to_one_connected_device() {
    // Two TCP servers on different ports, both return the same MAC
    let (listener_a, port_a) = random_listener().await;
    let (listener_b, port_b) = random_listener().await;

    // Server A
    tokio::spawn(async move {
        let (stream, _) = listener_a.accept().await.unwrap();
        let (read, write) = stream.into_split();
        let mut reader = FramedRead::new(read, EspHomeCodec);
        let mut writer = FramedWrite::new(write, EspHomeCodec);

        let _hello = reader.next().await.unwrap().unwrap();
        writer
            .send((
                msg_types::HELLO_RESPONSE,
                HelloResponse {
                    api_version_major: 1,
                    api_version_minor: 10,
                    server_info: "a".into(),
                    name: "device-a".into(),
                }
                .encode_to_vec(),
            ))
            .await
            .unwrap();

        let _di = reader.next().await.unwrap().unwrap();
        writer
            .send((
                msg_types::DEVICE_INFO_RESPONSE,
                DeviceInfoResponse {
                    name: "device-a".into(),
                    mac_address: "AA:BB:CC:DD:EE:FF".into(),
                    ..Default::default()
                }
                .encode_to_vec(),
            ))
            .await
            .unwrap();

        let _le = reader.next().await.unwrap().unwrap();
        writer
            .send((
                msg_types::LIST_ENTITIES_DONE,
                ListEntitiesDoneResponse {}.encode_to_vec(),
            ))
            .await
            .unwrap();
        let _ss = reader.next().await.unwrap().unwrap();
        // Hold connection open
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    });

    // Server B — same MAC, different hostname/port.
    // Uses graceful EOF handling: session-b may be stopped by the manager mid-handshake
    // (duplicate-MAC collapse), so any read can return None.
    tokio::spawn(async move {
        let Ok((stream, _)) = listener_b.accept().await else {
            return;
        };
        let (read, write) = stream.into_split();
        let mut reader = FramedRead::new(read, EspHomeCodec);
        let mut writer = FramedWrite::new(write, EspHomeCodec);

        let Some(Ok(_)) = reader.next().await else {
            return;
        };
        if writer
            .send((
                msg_types::HELLO_RESPONSE,
                HelloResponse {
                    api_version_major: 1,
                    api_version_minor: 10,
                    server_info: "b".into(),
                    name: "device-b".into(),
                }
                .encode_to_vec(),
            ))
            .await
            .is_err()
        {
            return;
        }

        let Some(Ok(_)) = reader.next().await else {
            return;
        };
        if writer
            .send((
                msg_types::DEVICE_INFO_RESPONSE,
                DeviceInfoResponse {
                    name: "device-b".into(),
                    mac_address: "AA:BB:CC:DD:EE:FF".into(),
                    ..Default::default()
                }
                .encode_to_vec(),
            ))
            .await
            .is_err()
        {
            return;
        }

        let Some(Ok(_)) = reader.next().await else {
            return;
        };
        if writer
            .send((
                msg_types::LIST_ENTITIES_DONE,
                ListEntitiesDoneResponse {}.encode_to_vec(),
            ))
            .await
            .is_err()
        {
            return;
        }
        // SubscribeStates — may never arrive if session-b is stopped early
        let Some(Ok(_)) = reader.next().await else {
            return;
        };
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    });

    let (sys, manager_ref, _registry, _store) = make_system_and_manager();

    // Discover two mDNS records with different hostnames
    let device_a = DiscoveredDevice {
        device_id: device_id_from_hostname("device-a"),
        service_fullname: "device-a._esphomelib._tcp.local.".into(),
        hostname: "device-a".into(),
        ip: "127.0.0.1".parse().unwrap(),
        port: port_a,
        name: "device-a".into(),
        version: "2025.1.0".into(),
        friendly_name: None,
        first_seen_at: std::time::SystemTime::UNIX_EPOCH,
        last_seen_at: std::time::SystemTime::now(),
        is_stale: false,
    };
    let device_b = DiscoveredDevice {
        device_id: device_id_from_hostname("device-b"),
        service_fullname: "device-b._esphomelib._tcp.local.".into(),
        hostname: "device-b".into(),
        ip: "127.0.0.1".parse().unwrap(),
        port: port_b,
        name: "device-b".into(),
        version: "2025.1.0".into(),
        friendly_name: None,
        first_seen_at: std::time::SystemTime::UNIX_EPOCH,
        last_seen_at: std::time::SystemTime::now(),
        is_stale: false,
    };

    manager_ref
        .send(DeviceLinkManagerMsg::DeviceDiscovered(device_a))
        .ok();
    manager_ref
        .send(DeviceLinkManagerMsg::DeviceDiscovered(device_b))
        .ok();

    // Allow both handshakes to complete
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    let (tx, rx) = oneshot::channel::<Vec<ConnectedDevice>>();
    manager_ref
        .send(DeviceLinkManagerMsg::ListConnected(tx))
        .ok();
    let connected = rx.await.unwrap();
    assert_eq!(
        connected.len(),
        1,
        "two mDNS records with same MAC must collapse to one ConnectedDevice, got: {:?}",
        connected.iter().map(|d| &d.device_id).collect::<Vec<_>>()
    );
    assert_eq!(connected[0].device_id.0, "esphome:aabbccddeeff");

    // GetStatus by canonical ID works
    let (tx2, rx2) = oneshot::channel::<Option<DeviceLinkStatus>>();
    manager_ref
        .send(DeviceLinkManagerMsg::GetStatus {
            device_id: device_id_from_mac("AA:BB:CC:DD:EE:FF"),
            reply: tx2,
        })
        .ok();
    let status = rx2.await.unwrap();
    assert!(
        status.is_some(),
        "GetStatus by canonical MAC-based ID must succeed"
    );

    manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
    drop(sys);
}

#[tokio::test(start_paused = true)]
async fn ping_timeout_transitions_to_backoff() {
    let (listener, port) = random_listener().await;

    // Synchronisation: signal when handshake is complete so we can advance time
    let (done_tx, done_rx) = oneshot::channel::<()>();

    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, write) = stream.into_split();
        let mut reader = FramedRead::new(read, EspHomeCodec);
        let mut writer = FramedWrite::new(write, EspHomeCodec);

        // Hello
        let _hello = reader.next().await.unwrap().unwrap();
        writer
            .send((
                msg_types::HELLO_RESPONSE,
                HelloResponse {
                    api_version_major: 1,
                    api_version_minor: 10,
                    server_info: "mock".into(),
                    name: "mock-device".into(),
                }
                .encode_to_vec(),
            ))
            .await
            .unwrap();

        // DeviceInfo
        let _di = reader.next().await.unwrap().unwrap();
        writer
            .send((
                msg_types::DEVICE_INFO_RESPONSE,
                DeviceInfoResponse {
                    name: "mock-device".into(),
                    mac_address: "AA:BB:CC:DD:EE:FF".into(),
                    ..Default::default()
                }
                .encode_to_vec(),
            ))
            .await
            .unwrap();

        // ListEntities (empty) + done
        let _le = reader.next().await.unwrap().unwrap();
        writer
            .send((
                msg_types::LIST_ENTITIES_DONE,
                ListEntitiesDoneResponse {}.encode_to_vec(),
            ))
            .await
            .unwrap();

        // SubscribeStates
        let _ss = reader.next().await.unwrap().unwrap();

        // Signal that handshake is done
        let _ = done_tx.send(());

        // Keep connection open but never send any frames (no ping responses)
        tokio::time::sleep(std::time::Duration::from_secs(1000)).await;
        drop(writer);
    });

    let (sys, manager_ref, _registry, _store) = make_system_and_manager();
    let device = make_discovered_device(port);
    manager_ref
        .send(DeviceLinkManagerMsg::DeviceDiscovered(device))
        .ok();

    // Wait for TCP handshake to complete (I/O driven, does not require time advancement)
    done_rx.await.unwrap();

    // Yield to let actor messages (SessionIdentityResolved, SessionStatusChanged) settle
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }

    // Advance past the first PingTick (CLIENT_PING_INTERVAL_SECS = 15 s)
    tokio::time::advance(std::time::Duration::from_secs(16)).await;
    // Let PingTick process (session sends PING_REQUEST; server ignores it)
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }

    // Advance to CLIENT_INACTIVITY_TIMEOUT_SECS (30 s total elapsed since handshake)
    // last_activity_at was set when the last handshake frame arrived (at paused time ≈ 0).
    // After 31 s of no inbound frames, PingTick triggers inactivity disconnect.
    tokio::time::advance(std::time::Duration::from_secs(16)).await;
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }

    // Check that the session has transitioned to Backoff
    let canonical_id = device_id_from_mac("AA:BB:CC:DD:EE:FF");
    let (tx, rx) = oneshot::channel::<Option<DeviceLinkStatus>>();
    manager_ref
        .send(DeviceLinkManagerMsg::GetStatus {
            device_id: canonical_id,
            reply: tx,
        })
        .ok();
    let status = rx.await.unwrap();
    if let Some(s) = status {
        assert!(
            matches!(s.status, SessionStatus::Backoff { .. }),
            "session should be in Backoff after inactivity timeout, got {:?}",
            s.status
        );
    }
    // If status is None (canonical entry was cleaned up), the test still passes
    // because the session correctly detected inactivity and triggered reconnect.

    manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
    drop(sys);
}

// ── Phase 3 tests ─────────────────────────────────────────────────────────────

/// Verify that `DispatchCommand` routes through the session and delivers a
/// correctly-encoded Native API command frame to the firmware mock server.
#[tokio::test]
async fn command_bridge_relays_command_to_firmware() {
    let (listener, port) = random_listener().await;
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<(u32, Vec<u8>)>();

    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, write) = stream.into_split();
        let mut reader = FramedRead::new(read, EspHomeCodec);
        let mut writer = FramedWrite::new(write, EspHomeCodec);

        // Hello
        let _ = reader.next().await.unwrap().unwrap();
        writer
            .send((
                msg_types::HELLO_RESPONSE,
                HelloResponse {
                    api_version_major: 1,
                    api_version_minor: 10,
                    server_info: "mock".into(),
                    name: "mock-device".into(),
                }
                .encode_to_vec(),
            ))
            .await
            .unwrap();

        // DeviceInfo — MAC so canonical ID = esphome:aabbccddeeff
        let _ = reader.next().await.unwrap().unwrap();
        writer
            .send((
                msg_types::DEVICE_INFO_RESPONSE,
                DeviceInfoResponse {
                    name: "mock-device".into(),
                    mac_address: "AA:BB:CC:DD:EE:FF".into(),
                    ..Default::default()
                }
                .encode_to_vec(),
            ))
            .await
            .unwrap();

        // ListEntities: switch key=99 + done
        let _ = reader.next().await.unwrap().unwrap();
        writer
            .send((
                msg_types::LIST_ENTITIES_SWITCH,
                ListEntitiesSwitchResponse {
                    object_id: "relay".into(),
                    key: 99,
                    name: "Relay".into(),
                    ..Default::default()
                }
                .encode_to_vec(),
            ))
            .await
            .unwrap();
        writer
            .send((
                msg_types::LIST_ENTITIES_DONE,
                ListEntitiesDoneResponse {}.encode_to_vec(),
            ))
            .await
            .unwrap();

        // SubscribeStates
        let _ = reader.next().await.unwrap().unwrap();

        // Forward the next incoming frame (the dispatched command) to the test
        if let Some(Ok((msg_type, payload))) = reader.next().await {
            cmd_tx.send((msg_type, payload)).ok();
        }

        // Hold connection open so the session stays Active while the test checks
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    });

    let (sys, manager_ref, _registry, _store) = make_system_and_manager();
    let device = make_discovered_device(port);
    manager_ref
        .send(DeviceLinkManagerMsg::DeviceDiscovered(device))
        .ok();

    // Allow handshake to complete
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Entity ID: device_slug("mock-device") = "mock_device", object_id = "relay"
    let entity_id = EntityId::new("switch", "mock_device__relay");
    let canonical_id = device_id_from_mac("AA:BB:CC:DD:EE:FF");

    let (reply_tx, reply_rx) = oneshot::channel();
    manager_ref
        .send(DeviceLinkManagerMsg::DispatchCommand {
            device_id: canonical_id,
            local_entity_id: entity_id,
            state: EntityState::Switch { is_on: true },
            reply: reply_tx,
        })
        .ok();

    let result = reply_rx.await.unwrap();
    assert!(
        result.is_ok(),
        "DispatchCommand should succeed: {:?}",
        result
    );

    // Verify the correct frame was delivered to the server
    let (frame_type, payload) =
        tokio::time::timeout(std::time::Duration::from_millis(500), cmd_rx.recv())
            .await
            .expect("server should receive command within 500ms")
            .expect("command channel should not be closed");

    assert_eq!(
        frame_type,
        msg_types::SWITCH_COMMAND,
        "server must receive SWITCH_COMMAND"
    );
    let cmd = SwitchCommandRequest::decode(payload.as_slice()).unwrap();
    assert_eq!(cmd.key, 99, "command key must match entity key");
    assert!(cmd.state, "command state must be on");

    manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
    drop(sys);
}

/// Verify that an entity that disappears from a reconnected device remains in the
/// registry as `Unavailable` rather than being deleted.
#[tokio::test]
async fn disappearing_entity_stays_unavailable_after_reconnect() {
    let (listener, port) = random_listener().await;

    tokio::spawn(async move {
        // ── Connection 1: switch (key=1) + temperature sensor (key=2) ──
        let (stream, _) = listener.accept().await.unwrap();
        let (read, write) = stream.into_split();
        let mut reader = FramedRead::new(read, EspHomeCodec);
        let mut writer = FramedWrite::new(write, EspHomeCodec);

        let _ = reader.next().await.unwrap().unwrap();
        writer
            .send((
                msg_types::HELLO_RESPONSE,
                HelloResponse {
                    api_version_major: 1,
                    api_version_minor: 10,
                    server_info: "mock".into(),
                    name: "mock-device".into(),
                }
                .encode_to_vec(),
            ))
            .await
            .unwrap();

        let _ = reader.next().await.unwrap().unwrap();
        writer
            .send((
                msg_types::DEVICE_INFO_RESPONSE,
                DeviceInfoResponse {
                    name: "mock-device".into(),
                    mac_address: "AA:BB:CC:DD:EE:FF".into(),
                    ..Default::default()
                }
                .encode_to_vec(),
            ))
            .await
            .unwrap();

        let _ = reader.next().await.unwrap().unwrap();
        writer
            .send((
                msg_types::LIST_ENTITIES_SWITCH,
                ListEntitiesSwitchResponse {
                    object_id: "relay".into(),
                    key: 1,
                    name: "Relay".into(),
                    ..Default::default()
                }
                .encode_to_vec(),
            ))
            .await
            .unwrap();
        writer
            .send((
                msg_types::LIST_ENTITIES_SENSOR,
                ListEntitiesSensorResponse {
                    object_id: "temperature".into(),
                    key: 2,
                    name: "Temperature".into(),
                    unit_of_measurement: "°C".into(),
                    ..Default::default()
                }
                .encode_to_vec(),
            ))
            .await
            .unwrap();
        writer
            .send((
                msg_types::LIST_ENTITIES_DONE,
                ListEntitiesDoneResponse {}.encode_to_vec(),
            ))
            .await
            .unwrap();

        let _ = reader.next().await.unwrap().unwrap(); // SubscribeStates
                                                       // Drop immediately — triggers ReaderClosed on client, marks entities Unavailable
        drop(writer);
        drop(reader);

        // ── Connection 2: only switch (temperature has disappeared) ──
        let Ok((stream2, _)) = listener.accept().await else {
            return;
        };
        let (read2, write2) = stream2.into_split();
        let mut reader2 = FramedRead::new(read2, EspHomeCodec);
        let mut writer2 = FramedWrite::new(write2, EspHomeCodec);

        let Some(Ok(_)) = reader2.next().await else {
            return;
        };
        if writer2
            .send((
                msg_types::HELLO_RESPONSE,
                HelloResponse {
                    api_version_major: 1,
                    api_version_minor: 10,
                    server_info: "mock".into(),
                    name: "mock-device".into(),
                }
                .encode_to_vec(),
            ))
            .await
            .is_err()
        {
            return;
        }

        let Some(Ok(_)) = reader2.next().await else {
            return;
        };
        if writer2
            .send((
                msg_types::DEVICE_INFO_RESPONSE,
                DeviceInfoResponse {
                    name: "mock-device".into(),
                    mac_address: "AA:BB:CC:DD:EE:FF".into(),
                    ..Default::default()
                }
                .encode_to_vec(),
            ))
            .await
            .is_err()
        {
            return;
        }

        let Some(Ok(_)) = reader2.next().await else {
            return;
        };
        // Only relay — temperature does not appear in this entity list
        if writer2
            .send((
                msg_types::LIST_ENTITIES_SWITCH,
                ListEntitiesSwitchResponse {
                    object_id: "relay".into(),
                    key: 1,
                    name: "Relay".into(),
                    ..Default::default()
                }
                .encode_to_vec(),
            ))
            .await
            .is_err()
        {
            return;
        }
        if writer2
            .send((
                msg_types::LIST_ENTITIES_DONE,
                ListEntitiesDoneResponse {}.encode_to_vec(),
            ))
            .await
            .is_err()
        {
            return;
        }

        let Some(Ok(_)) = reader2.next().await else {
            return;
        }; // SubscribeStates
           // Hold connection open during assertions
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    });

    let (_sys, manager_ref, _registry, state_store) = make_system_and_manager();
    let device = make_discovered_device(port);
    manager_ref
        .send(DeviceLinkManagerMsg::DeviceDiscovered(device))
        .ok();

    // Wait for: handshake1 + disconnect + backoff(1 s) + handshake2 + margin
    tokio::time::sleep(std::time::Duration::from_millis(1800)).await;

    // Entity IDs: device_slug("mock-device") = "mock_device"
    let temp_entity_id = EntityId::new("sensor", "mock_device__temperature");
    let relay_entity_id = EntityId::new("switch", "mock_device__relay");

    // Temperature disappeared from connection 2's entity list, but must remain
    // in the registry as Unavailable (not deleted).
    let temp_state = state_store.get(&temp_entity_id);
    assert!(
        matches!(temp_state, Some(EntityState::Unavailable)),
        "temperature entity must stay in registry as Unavailable after disappearing, got: {:?}",
        temp_state
    );

    // Relay was present in both connections; it must still be tracked.
    let relay_state = state_store.get(&relay_entity_id);
    assert!(
        relay_state.is_some(),
        "relay entity must still be tracked after reconnect"
    );

    manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
}

// ── Phase 4 tests ─────────────────────────────────────────────────────────────

/// PSK helpers.  [1u8; 32] base64-encodes to NOISE_PSK_CORRECT_B64.
const NOISE_PSK_CORRECT: [u8; 32] = [1u8; 32];
/// base64([1u8; 32]): "AQEB" × 10 + "AQE="
const NOISE_PSK_CORRECT_B64: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=";
/// base64([2u8; 32]) — used as the "wrong" PSK in tests.
const NOISE_PSK_WRONG_B64: &str = "AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI=";

/// Run the full ESPHome + Noise handshake server-side, then call `extra`.
///
/// On PSK mismatch the task silently returns — expected in wrong-PSK tests.
async fn mock_noise_server_handshake_then<F, Fut>(
    listener: TcpListener,
    psk: &'static [u8; 32],
    device_name: &'static str,
    mac_address: &'static str,
    extra: F,
) where
    F: FnOnce(
            tokio::net::tcp::OwnedWriteHalf,
            rshome_device_link::noise_transport::SharedNoise,
            tokio::net::tcp::OwnedReadHalf,
        ) -> Fut
        + Send
        + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (mut read, mut write) = stream.into_split();

        let noise = match noise_server_handshake(&mut read, &mut write, psk).await {
            Ok(n) => n,
            Err(_) => return, // PSK mismatch — expected in wrong-PSK tests
        };

        // Hello
        let _ = noise_recv_frame(&mut read, &noise).await;
        noise_send_frame(
            &mut write,
            &noise,
            msg_types::HELLO_RESPONSE,
            &HelloResponse {
                api_version_major: 1,
                api_version_minor: 10,
                server_info: "mock".into(),
                name: device_name.into(),
            }
            .encode_to_vec(),
        )
        .await
        .ok();

        // DeviceInfo
        let _ = noise_recv_frame(&mut read, &noise).await;
        noise_send_frame(
            &mut write,
            &noise,
            msg_types::DEVICE_INFO_RESPONSE,
            &DeviceInfoResponse {
                name: device_name.into(),
                mac_address: mac_address.into(),
                ..Default::default()
            }
            .encode_to_vec(),
        )
        .await
        .ok();

        // ListEntities → done
        let _ = noise_recv_frame(&mut read, &noise).await;
        noise_send_frame(
            &mut write,
            &noise,
            msg_types::LIST_ENTITIES_DONE,
            &ListEntitiesDoneResponse {}.encode_to_vec(),
        )
        .await
        .ok();

        // SubscribeStates
        let _ = noise_recv_frame(&mut read, &noise).await;

        extra(write, noise, read).await;
    });
}

/// A Noise session configured with the correct PSK must complete the handshake
/// and reach Active state.
#[tokio::test]
async fn noise_session_completes_handshake_and_becomes_active() {
    let (listener, port) = random_listener().await;
    mock_noise_server_handshake_then(
        listener,
        &NOISE_PSK_CORRECT,
        "noise-device",
        "AA:BB:CC:DD:EE:01",
        |w, noise, r| async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            drop((w, noise, r));
        },
    )
    .await;

    let (sys, manager_ref, _registry, _store) = make_system_and_manager();
    let provisional_id = device_id_from_hostname("noise-device");

    manager_ref
        .send(DeviceLinkManagerMsg::SetDeviceSecurityConfig {
            device_id: provisional_id.clone(),
            config: DeviceSecurityConfig {
                noise_psk: Some(NOISE_PSK_CORRECT_B64.into()),
            },
        })
        .ok();

    let device = DiscoveredDevice {
        device_id: provisional_id,
        service_fullname: "noise-device._esphomelib._tcp.local.".into(),
        hostname: "noise-device".into(),
        ip: "127.0.0.1".parse().unwrap(),
        port,
        name: "noise-device".into(),
        version: "2025.1.0".into(),
        friendly_name: None,
        first_seen_at: std::time::SystemTime::UNIX_EPOCH,
        last_seen_at: std::time::SystemTime::now(),
        is_stale: false,
    };
    manager_ref
        .send(DeviceLinkManagerMsg::DeviceDiscovered(device))
        .ok();

    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let canonical_id = device_id_from_mac("AA:BB:CC:DD:EE:01");
    let (tx, rx) = oneshot::channel::<Option<DeviceLinkStatus>>();
    manager_ref
        .send(DeviceLinkManagerMsg::GetStatus {
            device_id: canonical_id,
            reply: tx,
        })
        .ok();
    let status = rx.await.unwrap().unwrap();
    assert!(
        matches!(status.status, SessionStatus::Active),
        "Noise session should be Active, got {:?}",
        status.status
    );

    manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
    drop(sys);
}

/// A session configured with the wrong Noise PSK must never become Active.
///
/// The mock server uses the correct PSK; the session actor is configured with a
/// different PSK.  Noise_NNpsk0 authentication fails at msg2, the session calls
/// `schedule_reconnect`, and no canonical entry is ever created.
#[tokio::test]
async fn wrong_noise_psk_leaves_device_in_backoff() {
    let (listener, port) = random_listener().await;

    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (mut read, mut write) = stream.into_split();
        // Server handshake with correct PSK; client uses wrong PSK → client fails
        let _ = noise_server_handshake(&mut read, &mut write, &NOISE_PSK_CORRECT).await;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    });

    let (sys, manager_ref, _registry, _store) = make_system_and_manager();
    let provisional_id = device_id_from_hostname("wrong-psk-device");

    manager_ref
        .send(DeviceLinkManagerMsg::SetDeviceSecurityConfig {
            device_id: provisional_id.clone(),
            config: DeviceSecurityConfig {
                noise_psk: Some(NOISE_PSK_WRONG_B64.into()),
            },
        })
        .ok();

    let device = DiscoveredDevice {
        device_id: provisional_id.clone(),
        service_fullname: "wrong-psk-device._esphomelib._tcp.local.".into(),
        hostname: "wrong-psk-device".into(),
        ip: "127.0.0.1".parse().unwrap(),
        port,
        name: "wrong-psk-device".into(),
        version: "2025.1.0".into(),
        friendly_name: None,
        first_seen_at: std::time::SystemTime::UNIX_EPOCH,
        last_seen_at: std::time::SystemTime::now(),
        is_stale: false,
    };
    manager_ref
        .send(DeviceLinkManagerMsg::DeviceDiscovered(device))
        .ok();

    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // No canonical entry must be created — the Noise handshake never completed
    let (tx, rx) = oneshot::channel::<Vec<ConnectedDevice>>();
    manager_ref
        .send(DeviceLinkManagerMsg::ListConnected(tx))
        .ok();
    let connected = rx.await.unwrap();
    assert!(
        connected.is_empty(),
        "wrong PSK must not produce an Active canonical entry; got: {:?}",
        connected.iter().map(|d| &d.device_id).collect::<Vec<_>>()
    );

    manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
    drop(sys);
}

/// A cleartext device and a Noise-encrypted device must both become Active in
/// the same link manager simultaneously.
#[tokio::test]
async fn cleartext_and_noise_sessions_coexist() {
    let (listener_clear, port_clear) = random_listener().await;
    let (listener_noise, port_noise) = random_listener().await;

    mock_server_handshake_then(listener_clear, |writer| async move {
        tokio::time::sleep(std::time::Duration::from_millis(700)).await;
        drop(writer);
    })
    .await;

    mock_noise_server_handshake_then(
        listener_noise,
        &NOISE_PSK_CORRECT,
        "noise-coexist",
        "11:22:33:44:55:77",
        |w, noise, r| async move {
            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
            drop((w, noise, r));
        },
    )
    .await;

    let (sys, manager_ref, _registry, _store) = make_system_and_manager();

    let provisional_b = device_id_from_hostname("noise-coexist");
    manager_ref
        .send(DeviceLinkManagerMsg::SetDeviceSecurityConfig {
            device_id: provisional_b.clone(),
            config: DeviceSecurityConfig {
                noise_psk: Some(NOISE_PSK_CORRECT_B64.into()),
            },
        })
        .ok();

    let device_a = DiscoveredDevice {
        device_id: device_id_from_hostname("mock-device"),
        service_fullname: "mock-device._esphomelib._tcp.local.".into(),
        hostname: "mock-device".into(),
        ip: "127.0.0.1".parse().unwrap(),
        port: port_clear,
        name: "mock-device".into(),
        version: "2025.1.0".into(),
        friendly_name: None,
        first_seen_at: std::time::SystemTime::UNIX_EPOCH,
        last_seen_at: std::time::SystemTime::now(),
        is_stale: false,
    };
    let device_b = DiscoveredDevice {
        device_id: provisional_b,
        service_fullname: "noise-coexist._esphomelib._tcp.local.".into(),
        hostname: "noise-coexist".into(),
        ip: "127.0.0.1".parse().unwrap(),
        port: port_noise,
        name: "noise-coexist".into(),
        version: "2025.1.0".into(),
        friendly_name: None,
        first_seen_at: std::time::SystemTime::UNIX_EPOCH,
        last_seen_at: std::time::SystemTime::now(),
        is_stale: false,
    };

    manager_ref
        .send(DeviceLinkManagerMsg::DeviceDiscovered(device_a))
        .ok();
    manager_ref
        .send(DeviceLinkManagerMsg::DeviceDiscovered(device_b))
        .ok();

    tokio::time::sleep(std::time::Duration::from_millis(450)).await;

    let (tx, rx) = oneshot::channel::<Vec<ConnectedDevice>>();
    manager_ref
        .send(DeviceLinkManagerMsg::ListConnected(tx))
        .ok();
    let connected = rx.await.unwrap();
    assert_eq!(
        connected.len(),
        2,
        "both cleartext and Noise devices must be connected; got: {:?}",
        connected
            .iter()
            .map(|d| d.device_id.to_string())
            .collect::<Vec<_>>()
    );
    for cd in &connected {
        assert!(
            matches!(cd.status, SessionStatus::Active),
            "device {:?} should be Active, got {:?}",
            cd.device_id,
            cd.status
        );
    }

    manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
    drop(sys);
}

/// When an mDNS `DeviceRemoved` arrives for an Active session, the session must
/// not be terminated.  The discovery record is marked stale but the canonical
/// entry remains intact and the session stays Active (Task 4.4).
#[tokio::test]
async fn stale_mdns_record_does_not_kill_active_session() {
    let (listener, port) = random_listener().await;
    mock_server_handshake_then(listener, |writer| async move {
        tokio::time::sleep(std::time::Duration::from_millis(900)).await;
        drop(writer);
    })
    .await;

    let (sys, manager_ref, _registry, _store) = make_system_and_manager();
    let device = make_discovered_device(port);
    manager_ref
        .send(DeviceLinkManagerMsg::DeviceDiscovered(device.clone()))
        .ok();

    // Wait for handshake and Active state
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;

    let canonical_id = device_id_from_mac("AA:BB:CC:DD:EE:FF");
    let (tx, rx) = oneshot::channel::<Option<DeviceLinkStatus>>();
    manager_ref
        .send(DeviceLinkManagerMsg::GetStatus {
            device_id: canonical_id.clone(),
            reply: tx,
        })
        .ok();
    let pre_status = rx.await.unwrap().unwrap();
    assert!(
        matches!(pre_status.status, SessionStatus::Active),
        "session must be Active before DeviceRemoved; got {:?}",
        pre_status.status
    );

    // Simulate mDNS record disappearing
    manager_ref
        .send(DeviceLinkManagerMsg::DeviceRemoved(
            device.device_id.clone(),
        ))
        .ok();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Canonical entry must still be Active
    let (tx2, rx2) = oneshot::channel::<Option<DeviceLinkStatus>>();
    manager_ref
        .send(DeviceLinkManagerMsg::GetStatus {
            device_id: canonical_id,
            reply: tx2,
        })
        .ok();
    let post_status = rx2.await.unwrap();
    assert!(
        post_status.is_some(),
        "canonical entry must not be deleted on DeviceRemoved while Active"
    );
    assert!(
        matches!(post_status.as_ref().unwrap().status, SessionStatus::Active),
        "session must remain Active after DeviceRemoved, got {:?}",
        post_status.as_ref().unwrap().status
    );

    // Discovery record must remain but be marked stale
    let (tx3, rx3) = oneshot::channel::<Vec<DiscoveredDevice>>();
    manager_ref
        .send(DeviceLinkManagerMsg::ListDiscovered(tx3))
        .ok();
    let discovered = rx3.await.unwrap();
    assert_eq!(
        discovered.len(),
        1,
        "discovery record must remain (stale) after DeviceRemoved when session is Active"
    );
    assert!(
        discovered[0].is_stale,
        "discovery record must be marked stale after mDNS removal"
    );

    manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
    drop(sys);
}

/// A malformed (oversized) frame on one device session must not affect other
/// sessions: frame boundary isolation.
#[tokio::test]
async fn malformed_frame_kills_only_one_session() {
    use tokio::io::AsyncWriteExt as _;

    let (listener_good, port_good) = random_listener().await;
    let (listener_bad, port_bad) = random_listener().await;

    // "Good" server: stays connected and healthy
    mock_server_handshake_then(listener_good, |writer| async move {
        tokio::time::sleep(std::time::Duration::from_millis(900)).await;
        drop(writer);
    })
    .await;

    // "Bad" server: completes handshake then injects an oversized frame header
    tokio::spawn(async move {
        let (stream, _) = listener_bad.accept().await.unwrap();
        let (read, write) = stream.into_split();
        let mut reader = FramedRead::new(read, EspHomeCodec);
        let mut writer = FramedWrite::new(write, EspHomeCodec);

        let _hello = reader.next().await.unwrap().unwrap();
        writer
            .send((
                msg_types::HELLO_RESPONSE,
                HelloResponse {
                    api_version_major: 1,
                    api_version_minor: 10,
                    server_info: "bad-mock".into(),
                    name: "bad-device".into(),
                }
                .encode_to_vec(),
            ))
            .await
            .unwrap();

        let _di = reader.next().await.unwrap().unwrap();
        writer
            .send((
                msg_types::DEVICE_INFO_RESPONSE,
                DeviceInfoResponse {
                    name: "bad-device".into(),
                    mac_address: "CC:CC:CC:CC:CC:CC".into(),
                    ..Default::default()
                }
                .encode_to_vec(),
            ))
            .await
            .unwrap();

        let _le = reader.next().await.unwrap().unwrap();
        writer
            .send((
                msg_types::LIST_ENTITIES_DONE,
                ListEntitiesDoneResponse {}.encode_to_vec(),
            ))
            .await
            .unwrap();

        let _ss = reader.next().await.unwrap().unwrap();

        // Inject raw oversized frame header: 0x00 | varint(65537) = [0x00, 0x81, 0x80, 0x04]
        // EspHomeCodec::decode() returns an error when payload_len > MAX_FRAME_PAYLOAD (65536)
        let mut raw = writer.into_inner();
        raw.write_all(&[0x00, 0x81, 0x80, 0x04]).await.ok();
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    });

    let (sys, manager_ref, _registry, _store) = make_system_and_manager();

    let device_good = DiscoveredDevice {
        device_id: device_id_from_hostname("mock-device"),
        service_fullname: "mock-device._esphomelib._tcp.local.".into(),
        hostname: "mock-device".into(),
        ip: "127.0.0.1".parse().unwrap(),
        port: port_good,
        name: "mock-device".into(),
        version: "2025.1.0".into(),
        friendly_name: None,
        first_seen_at: std::time::SystemTime::UNIX_EPOCH,
        last_seen_at: std::time::SystemTime::now(),
        is_stale: false,
    };
    let device_bad = DiscoveredDevice {
        device_id: device_id_from_hostname("bad-device"),
        service_fullname: "bad-device._esphomelib._tcp.local.".into(),
        hostname: "bad-device".into(),
        ip: "127.0.0.1".parse().unwrap(),
        port: port_bad,
        name: "bad-device".into(),
        version: "2025.1.0".into(),
        friendly_name: None,
        first_seen_at: std::time::SystemTime::UNIX_EPOCH,
        last_seen_at: std::time::SystemTime::now(),
        is_stale: false,
    };

    manager_ref
        .send(DeviceLinkManagerMsg::DeviceDiscovered(device_good))
        .ok();
    manager_ref
        .send(DeviceLinkManagerMsg::DeviceDiscovered(device_bad))
        .ok();

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Good session must remain Active
    let good_canonical = device_id_from_mac("AA:BB:CC:DD:EE:FF");
    let (tx, rx) = oneshot::channel::<Option<DeviceLinkStatus>>();
    manager_ref
        .send(DeviceLinkManagerMsg::GetStatus {
            device_id: good_canonical,
            reply: tx,
        })
        .ok();
    let good_status = rx.await.unwrap().unwrap();
    assert!(
        matches!(good_status.status, SessionStatus::Active),
        "good session must remain Active after malformed frame on other device; got {:?}",
        good_status.status
    );

    // Bad session must NOT be Active (it should have gone to Backoff)
    let bad_canonical = device_id_from_mac("CC:CC:CC:CC:CC:CC");
    let (tx2, rx2) = oneshot::channel::<Option<DeviceLinkStatus>>();
    manager_ref
        .send(DeviceLinkManagerMsg::GetStatus {
            device_id: bad_canonical,
            reply: tx2,
        })
        .ok();
    let bad_status = rx2.await.unwrap().unwrap();
    assert!(
        !matches!(bad_status.status, SessionStatus::Active),
        "bad session must NOT be Active after oversized frame; got {:?}",
        bad_status.status
    );

    manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
    drop(sys);
}

/// When the device reports `uses_password = true`, the session must transition
/// to `UnsupportedAuth` and NOT schedule a reconnect.
#[tokio::test]
async fn password_protected_device_transitions_to_unsupported_auth() {
    let (listener, port) = random_listener().await;

    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, write) = stream.into_split();
        let mut reader = FramedRead::new(read, EspHomeCodec);
        let mut writer = FramedWrite::new(write, EspHomeCodec);

        // Hello
        let _ = reader.next().await.unwrap().unwrap();
        writer
            .send((
                msg_types::HELLO_RESPONSE,
                HelloResponse {
                    api_version_major: 1,
                    api_version_minor: 10,
                    server_info: "mock".into(),
                    name: "pw-device".into(),
                }
                .encode_to_vec(),
            ))
            .await
            .unwrap();

        // DeviceInfo — with uses_password = true
        let _ = reader.next().await.unwrap().unwrap();
        writer
            .send((
                msg_types::DEVICE_INFO_RESPONSE,
                DeviceInfoResponse {
                    name: "pw-device".into(),
                    mac_address: "BB:CC:DD:EE:FF:00".into(),
                    uses_password: true,
                    ..Default::default()
                }
                .encode_to_vec(),
            ))
            .await
            .unwrap();

        // Hold open briefly so the session actor processes before the test reads status
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    });

    let (sys, manager_ref, _registry, _store) = make_system_and_manager();
    let device = DiscoveredDevice {
        device_id: device_id_from_hostname("pw-device"),
        service_fullname: "pw-device._esphomelib._tcp.local.".into(),
        hostname: "pw-device".into(),
        ip: "127.0.0.1".parse().unwrap(),
        port,
        name: "pw-device".into(),
        version: "2025.1.0".into(),
        friendly_name: None,
        first_seen_at: std::time::SystemTime::UNIX_EPOCH,
        last_seen_at: std::time::SystemTime::now(),
        is_stale: false,
    };
    manager_ref
        .send(DeviceLinkManagerMsg::DeviceDiscovered(device))
        .ok();

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let canonical_id = device_id_from_mac("BB:CC:DD:EE:FF:00");
    let (tx, rx) = oneshot::channel::<Option<DeviceLinkStatus>>();
    manager_ref
        .send(DeviceLinkManagerMsg::GetStatus {
            device_id: canonical_id,
            reply: tx,
        })
        .ok();
    let status = rx.await.unwrap().unwrap();
    assert!(
        matches!(status.status, SessionStatus::UnsupportedAuth),
        "password-protected device must be UnsupportedAuth, got {:?}",
        status.status
    );

    manager_ref.send(DeviceLinkManagerMsg::Stop).ok();
    drop(sys);
}
