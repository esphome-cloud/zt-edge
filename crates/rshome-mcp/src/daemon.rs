use std::future::Future;
use std::path::{Path, PathBuf};

use rmcp::service::ServiceExt;
use tokio::net::UnixListener;
use tokio::task::JoinSet;

use crate::runtime::RshomeHaRuntime;
use crate::RuntimeConfig;

pub type DaemonResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const DEFAULT_SOCKET_PATH: &str = "/run/rshome/rshome-ha-mcp.sock";
const DEFAULT_SNAPSHOT_PATH: &str = "/var/lib/rshome/rshome-ha-mcp-state.json";

#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub socket_path: PathBuf,
    pub snapshot_path: PathBuf,
    pub runtime_config: RuntimeConfig,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            socket_path: PathBuf::from(DEFAULT_SOCKET_PATH),
            snapshot_path: PathBuf::from(DEFAULT_SNAPSHOT_PATH),
            runtime_config: RuntimeConfig::default(),
        }
    }
}

impl DaemonConfig {
    pub fn from_env() -> Self {
        let mut runtime_config = RuntimeConfig::default();
        runtime_config.mdns_enabled =
            parse_env_bool("RSHOME_HA_MDNS_ENABLED").unwrap_or(runtime_config.mdns_enabled);
        runtime_config.max_devices =
            parse_env_usize("RSHOME_HA_MAX_DEVICES").unwrap_or(runtime_config.max_devices);
        runtime_config.max_entities =
            parse_env_usize("RSHOME_HA_MAX_ENTITIES").unwrap_or(runtime_config.max_entities);
        runtime_config.max_connections_per_device =
            parse_env_usize("RSHOME_HA_MAX_CONNECTIONS_PER_DEVICE")
                .unwrap_or(runtime_config.max_connections_per_device);
        if runtime_config.max_connections_per_device > 1 {
            tracing::warn!(
                configured = runtime_config.max_connections_per_device,
                "rshome-ha-mcp supports at most one concurrent device-link connection per device; clamping",
            );
            runtime_config.max_connections_per_device = 1;
        }

        Self {
            socket_path: env_path("RSHOME_HA_MCP_SOCKET_PATH", DEFAULT_SOCKET_PATH),
            snapshot_path: env_path("RSHOME_HA_STATE_SNAPSHOT_PATH", DEFAULT_SNAPSHOT_PATH),
            runtime_config,
        }
    }
}

pub async fn run_daemon(config: DaemonConfig) -> DaemonResult {
    #[cfg(unix)]
    {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        run_daemon_with_shutdown(config, async move {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = sigterm.recv() => {}
            }
        })
        .await
    }

    #[cfg(not(unix))]
    {
        run_daemon_with_shutdown(config, async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
    }
}

pub async fn run_daemon_with_shutdown<F>(config: DaemonConfig, shutdown: F) -> DaemonResult
where
    F: Future<Output = ()> + Send,
{
    prepare_paths(&config)?;

    let runtime = RshomeHaRuntime::new_device_ingest(config.runtime_config.clone());
    if config.snapshot_path.exists() {
        match runtime.restore_snapshot(&config.snapshot_path).await {
            Ok(restored) => tracing::info!(
                path = %config.snapshot_path.display(),
                restored,
                "restored runtime snapshot"
            ),
            Err(error) => tracing::warn!(
                path = %config.snapshot_path.display(),
                %error,
                "failed to restore runtime snapshot; continuing with empty state"
            ),
        }
    }

    let listener = UnixListener::bind(&config.socket_path)?;
    tracing::info!(
        socket = %config.socket_path.display(),
        snapshot = %config.snapshot_path.display(),
        mdns_enabled = config.runtime_config.mdns_enabled,
        "rshome-ha-mcp daemon listening"
    );

    tokio::pin!(shutdown);
    let mut client_tasks = JoinSet::new();

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                tracing::info!("shutdown requested");
                break;
            }
            accept_result = listener.accept() => {
                let (stream, _addr) = accept_result?;
                let server = runtime.server.clone();
                client_tasks.spawn(async move {
                    match server.serve(stream).await {
                        Ok(service) => {
                            if let Err(error) = service.waiting().await {
                                tracing::warn!(%error, "unix socket MCP session ended with error");
                            }
                        }
                        Err(error) => {
                            tracing::warn!(%error, "failed to initialize unix socket MCP session");
                        }
                    }
                });
            }
            join_result = client_tasks.join_next(), if !client_tasks.is_empty() => {
                if let Some(Err(error)) = join_result {
                    tracing::warn!(%error, "unix socket MCP task join error");
                }
            }
        }
    }

    client_tasks.abort_all();
    while client_tasks.join_next().await.is_some() {}

    drop(listener);
    let persist_result = runtime.persist_snapshot(&config.snapshot_path).await;
    if let Err(error) = &persist_result {
        tracing::error!(
            path = %config.snapshot_path.display(),
            %error,
            "failed to persist runtime snapshot"
        );
    }

    runtime.shutdown().await;
    cleanup_socket(&config.socket_path);
    persist_result?;

    Ok(())
}

fn prepare_paths(config: &DaemonConfig) -> Result<(), std::io::Error> {
    ensure_parent_dir(&config.socket_path)?;
    ensure_parent_dir(&config.snapshot_path)?;

    if config.socket_path.exists() {
        std::fs::remove_file(&config.socket_path)?;
    }

    Ok(())
}

fn ensure_parent_dir(path: &Path) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn cleanup_socket(path: &Path) {
    if let Err(error) = std::fs::remove_file(path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(socket = %path.display(), %error, "failed to remove unix socket");
        }
    }
}

fn env_path(key: &str, default: &str) -> PathBuf {
    std::env::var_os(key)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default))
}

fn parse_env_usize(key: &str) -> Option<usize> {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
}

fn parse_env_bool(key: &str) -> Option<bool> {
    std::env::var(key)
        .ok()
        .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener as StdUnixListener;

    use rmcp::{
        handler::client::ClientHandler,
        model::{CallToolRequestParam, ClientInfo},
        service::ServiceExt,
    };
    use rshome_entity::{
        DeviceDescriptor, DeviceId, DeviceManagerMsg, DeviceMsg, EntityCategory, EntityDescriptor,
        EntityId, EntityState,
    };

    #[derive(Debug, Clone, Default)]
    struct DummyClientHandler;

    impl ClientHandler for DummyClientHandler {
        fn get_info(&self) -> ClientInfo {
            ClientInfo::default()
        }
    }

    fn temp_dir() -> PathBuf {
        let short_id = uuid::Uuid::new_v4().simple().to_string();
        let dir = PathBuf::from("/tmp").join(format!("rhmcpd-{}", &short_id[..8]));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn unix_socket_bind_supported(dir: &Path) -> bool {
        let probe_path = dir.join("probe.sock");
        match StdUnixListener::bind(&probe_path) {
            Ok(listener) => {
                drop(listener);
                std::fs::remove_file(&probe_path).ok();
                true
            }
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => false,
            Err(error) => panic!(
                "unexpected unix socket probe failure for {}: {error}",
                probe_path.display()
            ),
        }
    }

    async fn wait_for_socket(path: &Path, handle: &mut tokio::task::JoinHandle<DaemonResult>) {
        for _ in 0..100 {
            match tokio::net::UnixStream::connect(path).await {
                Ok(stream) => {
                    drop(stream);
                    return;
                }
                Err(_) => {}
            }
            if handle.is_finished() {
                match handle.await {
                    Ok(Ok(())) => panic!("daemon exited before socket became ready"),
                    Ok(Err(error)) => panic!("daemon exited before socket became ready: {error}"),
                    Err(error) => {
                        panic!("daemon task panicked before socket became ready: {error}")
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("socket did not appear: {}", path.display());
    }

    fn text_content(result: &rmcp::model::CallToolResult) -> &str {
        result
            .content
            .first()
            .and_then(|content| content.raw.as_text())
            .map(|text| text.text.as_str())
            .expect("expected text content")
    }

    fn make_device_descriptor(device_id: &str, name: &str) -> DeviceDescriptor {
        DeviceDescriptor {
            device_id: DeviceId(device_id.to_string()),
            name: name.to_string(),
            model: Some("test-model".into()),
            manufacturer: Some("test-manufacturer".into()),
            sw_version: Some("1.0.0".into()),
            area_id: None,
        }
    }

    fn make_entity_descriptor(
        device_id: &DeviceId,
        domain: &str,
        object_id: &str,
    ) -> EntityDescriptor {
        EntityDescriptor {
            entity_id: EntityId::new(domain, object_id),
            name: object_id.to_string(),
            icon: None,
            device_id: Some(device_id.clone()),
            area_id: None,
            entity_category: EntityCategory::None,
            domain_id: domain.to_string(),
            feature_set: Vec::new(),
            device_class: None,
        }
    }

    #[tokio::test]
    async fn unix_socket_daemon_serves_tools_and_cleans_stale_socket() {
        let dir = temp_dir();
        if !unix_socket_bind_supported(&dir) {
            std::fs::remove_dir(&dir).ok();
            return;
        }

        let socket_path = dir.join("rshome-ha-mcp.sock");
        let snapshot_path = dir.join("state.json");

        std::fs::write(&socket_path, b"stale").unwrap();

        let seed_store = crate::runtime::RshomeHaRuntime::new_device_ingest(RuntimeConfig {
            mdns_enabled: false,
            ..RuntimeConfig::default()
        });
        seed_store.state_store.update(
            &EntityId::new("sensor", "restored"),
            EntityState::Sensor {
                value: 42.0,
                unit: Some("C".into()),
                attributes: Default::default(),
            },
        );
        seed_store.persist_snapshot(&snapshot_path).await.unwrap();
        seed_store.shutdown().await;

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let config = DaemonConfig {
            socket_path: socket_path.clone(),
            snapshot_path: snapshot_path.clone(),
            runtime_config: RuntimeConfig {
                mdns_enabled: false,
                ..RuntimeConfig::default()
            },
        };

        let mut handle = tokio::spawn(async move {
            run_daemon_with_shutdown(config, async move {
                let _ = shutdown_rx.await;
            })
            .await
        });

        wait_for_socket(&socket_path, &mut handle).await;

        let stream = tokio::net::UnixStream::connect(&socket_path).await.unwrap();
        let client = DummyClientHandler.serve(stream).await.unwrap();

        let config_result = client
            .call_tool(CallToolRequestParam {
                name: "ha.config.get".into(),
                arguments: None,
            })
            .await
            .unwrap();
        assert!(text_content(&config_result).contains("\"mdns_enabled\": false"));

        let devices_result = client
            .call_tool(CallToolRequestParam {
                name: "ha.devices.list".into(),
                arguments: None,
            })
            .await
            .unwrap();
        assert_eq!(text_content(&devices_result).trim(), "[]");

        let restored_state = client
            .call_tool(CallToolRequestParam {
                name: "ha.entities.get_state".into(),
                arguments: Some(
                    serde_json::json!({ "entity_id": "sensor.restored" })
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            })
            .await
            .unwrap();
        assert!(text_content(&restored_state).contains("\"entity_id\": \"sensor.restored\""));

        client.cancel().await.unwrap();
        let _ = shutdown_tx.send(());
        handle.await.unwrap().unwrap();

        assert!(!socket_path.exists());

        std::fs::remove_file(&snapshot_path).ok();
        std::fs::remove_dir(&dir).ok();
    }

    #[tokio::test]
    async fn daemon_shutdown_persists_structured_snapshot_before_teardown() {
        let dir = temp_dir();
        if !unix_socket_bind_supported(&dir) {
            std::fs::remove_dir(&dir).ok();
            return;
        }

        let socket_path = dir.join("rshome-ha-mcp.sock");
        let snapshot_path = dir.join("state.json");

        let seed_runtime = crate::runtime::RshomeHaRuntime::new_device_ingest(RuntimeConfig {
            mdns_enabled: false,
            ..RuntimeConfig::default()
        });
        let device = make_device_descriptor("dev-1", "Living Room");
        let entity = make_entity_descriptor(&device.device_id, "switch", "lamp");
        let device_ref = seed_runtime
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
        seed_runtime.persist_snapshot(&snapshot_path).await.unwrap();
        seed_runtime.shutdown().await;

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let config = DaemonConfig {
            socket_path: socket_path.clone(),
            snapshot_path: snapshot_path.clone(),
            runtime_config: RuntimeConfig {
                mdns_enabled: false,
                ..RuntimeConfig::default()
            },
        };

        let mut handle = tokio::spawn(async move {
            run_daemon_with_shutdown(config, async move {
                let _ = shutdown_rx.await;
            })
            .await
        });

        wait_for_socket(&socket_path, &mut handle).await;
        let _ = shutdown_tx.send(());
        handle.await.unwrap().unwrap();

        let snapshot: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&snapshot_path).unwrap()).unwrap();
        assert_eq!(snapshot["devices"].as_array().unwrap().len(), 1);
        assert_eq!(snapshot["devices"][0]["descriptor"]["device_id"], "dev-1");
        assert_eq!(
            snapshot["devices"][0]["entities"].as_array().unwrap().len(),
            1
        );
        assert_eq!(
            snapshot["devices"][0]["entities"][0]["descriptor"]["entity_id"],
            "switch.lamp"
        );
        assert_eq!(
            snapshot["devices"][0]["entities"][0]["state"]["Switch"]["is_on"],
            true
        );

        std::fs::remove_file(&snapshot_path).ok();
        std::fs::remove_dir(&dir).ok();
    }
}
