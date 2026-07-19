#[cfg(not(test))]
use mdns_sd::{ServiceDaemon, ServiceEvent};
use rshome_entity::DeviceId;
use std::net::IpAddr;
use std::time::SystemTime;

/// A device discovered via mDNS browsing `_esphomelib._tcp.local.`
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DiscoveredDevice {
    /// Canonical ID: `esphome:<mac>` or `esphome-host:<hostname>`
    pub device_id: DeviceId,
    /// Fully-qualified mDNS service name, e.g. `"my-device._esphomelib._tcp.local."`
    pub service_fullname: String,
    /// Hostname without `.local.` suffix
    pub hostname: String,
    /// Resolved IP address
    pub ip: IpAddr,
    /// TCP port (usually 6053)
    pub port: u16,
    /// mDNS instance name (e.g. "my-device")
    pub name: String,
    /// ESPHome firmware version string from TXT record
    pub version: String,
    /// Human-readable friendly name from TXT `friendly_name` property
    pub friendly_name: Option<String>,
    /// Wall-clock time when this device was first seen via mDNS
    pub first_seen_at: SystemTime,
    /// Wall-clock time of the most recent mDNS announcement for this device
    pub last_seen_at: SystemTime,
    /// True when no mDNS announcement has been received for ≥ 90 seconds
    pub is_stale: bool,
}

/// A URL-safe slug derived from a device name, used when building entity IDs.
///
/// e.g., "My ESP32 Device-01" → `"my_esp32_device_01"`
pub fn device_slug(name: &str) -> String {
    let raw: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    // Collapse consecutive underscores and strip leading/trailing ones
    raw.split('_')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

/// Derive DeviceId from a MAC address string.
///
/// Strips any separators (`:`, `-`) and lowercases hex digits.
/// `"AA:BB:CC:DD:EE:FF"` → `DeviceId("esphome:aabbccddeeff")`
pub fn device_id_from_mac(mac: &str) -> DeviceId {
    let clean: String = mac
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect::<String>()
        .to_lowercase();
    DeviceId(format!("esphome:{clean}"))
}

/// Derive DeviceId from a hostname (with or without `.local.` suffix).
///
/// `"my-device.local."` → `DeviceId("esphome-host:my-device")`
pub fn device_id_from_hostname(hostname: &str) -> DeviceId {
    let clean = hostname.trim_end_matches('.').trim_end_matches(".local");
    DeviceId(format!("esphome-host:{clean}"))
}

/// An event emitted by the mDNS browser.
pub enum BrowserEvent {
    /// A new (or updated) ESPHome device was resolved.
    DeviceFound(DiscoveredDevice),
    /// An ESPHome device was deregistered or went offline.
    DeviceRemoved(DeviceId),
}

/// mDNS browser that watches `_esphomelib._tcp.local.` advertisements.
///
/// Keep the returned `MdnsBrowser` alive to continue receiving events; drop it
/// to stop browsing.
pub struct MdnsBrowser;

impl MdnsBrowser {
    #[cfg(not(test))]
    const SERVICE_TYPE: &'static str = "_esphomelib._tcp.local.";

    /// Start browsing for ESPHome devices.
    ///
    /// Discovered and removed events are sent to `event_tx`. This function
    /// returns immediately; in production discovery runs in a detached OS thread
    /// so that tokio runtime shutdown is not blocked.  In `#[cfg(test)]` builds
    /// the method is a no-op (no network activity, no threads).
    pub fn start(
        event_tx: tokio::sync::mpsc::UnboundedSender<BrowserEvent>,
    ) -> Result<Self, mdns_sd::Error> {
        #[cfg(test)]
        {
            // Skip real mDNS in unit tests — no network browsing needed.
            drop(event_tx);
            return Ok(Self);
        }
        #[cfg(not(test))]
        {
            let service_type = Self::SERVICE_TYPE;
            // Use a regular OS thread (not spawn_blocking) so the tokio runtime
            // is not forced to wait for this thread on shutdown.
            std::thread::spawn(move || {
                let daemon = match ServiceDaemon::new() {
                    Ok(d) => d,
                    Err(e) => {
                        tracing::warn!("mDNS daemon init failed: {e}");
                        return;
                    }
                };
                let receiver = match daemon.browse(service_type) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!("mDNS browse failed: {e}");
                        return;
                    }
                };
                // Keep daemon alive for the thread's duration.
                let _daemon = daemon;
                loop {
                    match receiver.recv() {
                        Ok(ServiceEvent::ServiceResolved(info)) => {
                            if let Some(device) = build_discovered(&info) {
                                if event_tx.send(BrowserEvent::DeviceFound(device)).is_err() {
                                    break;
                                }
                            }
                        }
                        Ok(ServiceEvent::ServiceRemoved(_, fullname)) => {
                            let device_id = fullname_to_device_id(&fullname);
                            if event_tx
                                .send(BrowserEvent::DeviceRemoved(device_id))
                                .is_err()
                            {
                                break;
                            }
                        }
                        Ok(_) => {}
                        Err(_) => break,
                    }
                }
            });
            Ok(Self)
        }
    }
}

#[cfg(not(test))]
fn build_discovered(info: &mdns_sd::ServiceInfo) -> Option<DiscoveredDevice> {
    let ip = info.get_addresses().iter().next().copied()?;
    let port = info.get_port();

    let hostname = info
        .get_hostname()
        .trim_end_matches('.')
        .trim_end_matches(".local")
        .to_string();

    let props = info.get_properties();

    let version = props
        .get("version")
        .map(|v| v.val_str().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let friendly_name = props.get("friendly_name").map(|v| v.val_str().to_string());

    // Prefer MAC-based canonical ID when available in TXT record
    let device_id = if let Some(mac_prop) = props.get("mac") {
        device_id_from_mac(mac_prop.val_str())
    } else {
        device_id_from_hostname(&hostname)
    };

    // Instance name is the first label of the fully-qualified service name
    // e.g. "my-device._esphomelib._tcp.local." → "my-device"
    let name = info
        .get_fullname()
        .split('.')
        .next()
        .map(|s| s.to_string())
        .unwrap_or_else(|| hostname.clone());

    Some(DiscoveredDevice {
        device_id,
        service_fullname: info.get_fullname().to_string(),
        hostname,
        ip,
        port,
        name,
        version,
        friendly_name,
        first_seen_at: SystemTime::now(),
        last_seen_at: SystemTime::now(),
        is_stale: false,
    })
}

#[cfg(not(test))]
fn fullname_to_device_id(fullname: &str) -> DeviceId {
    let hostname = fullname.split('.').next().unwrap_or(fullname);
    device_id_from_hostname(hostname)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Convenience constructor for tests — avoids repeating all new fields.
    pub fn new_for_test(name: &str, port: u16) -> DiscoveredDevice {
        DiscoveredDevice {
            device_id: device_id_from_hostname(name),
            service_fullname: format!("{name}._esphomelib._tcp.local."),
            hostname: name.to_string(),
            ip: "127.0.0.1".parse().unwrap(),
            port,
            name: name.to_string(),
            version: "2025.1.0".to_string(),
            friendly_name: None,
            first_seen_at: SystemTime::UNIX_EPOCH,
            last_seen_at: SystemTime::now(),
            is_stale: false,
        }
    }

    #[test]
    fn mac_with_colons_gives_esphome_id() {
        let id = device_id_from_mac("AA:BB:CC:DD:EE:FF");
        assert_eq!(id.0, "esphome:aabbccddeeff");
    }

    #[test]
    fn mac_with_dashes_gives_esphome_id() {
        let id = device_id_from_mac("AA-BB-CC-DD-EE-FF");
        assert_eq!(id.0, "esphome:aabbccddeeff");
    }

    #[test]
    fn mac_already_clean_gives_esphome_id() {
        let id = device_id_from_mac("aabbccddeeff");
        assert_eq!(id.0, "esphome:aabbccddeeff");
    }

    #[test]
    fn hostname_with_local_suffix_stripped() {
        let id = device_id_from_hostname("my-device.local.");
        assert_eq!(id.0, "esphome-host:my-device");
    }

    #[test]
    fn hostname_bare_no_suffix() {
        let id = device_id_from_hostname("sensor-node");
        assert_eq!(id.0, "esphome-host:sensor-node");
    }

    #[test]
    fn device_slug_lowercases_and_replaces_spaces() {
        assert_eq!(device_slug("My Device"), "my_device");
    }

    #[test]
    fn device_slug_replaces_hyphens_and_numbers() {
        assert_eq!(device_slug("ESP32-Sensor-01"), "esp32_sensor_01");
    }

    #[test]
    fn device_slug_collapses_consecutive_separators() {
        assert_eq!(device_slug("my  device"), "my_device");
    }

    #[test]
    fn service_fullname_preserved_in_helper() {
        let d = new_for_test("living-room", 6053);
        assert_eq!(d.service_fullname, "living-room._esphomelib._tcp.local.");
    }

    #[test]
    fn fresh_record_is_not_stale() {
        let d = new_for_test("kitchen-sensor", 6053);
        assert!(!d.is_stale);
    }
}
