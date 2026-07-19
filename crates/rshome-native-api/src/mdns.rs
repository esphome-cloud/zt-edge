use mdns_sd::{ServiceDaemon, ServiceInfo};

#[allow(clippy::module_name_repetitions)]
pub struct MdnsHandle {
    daemon: ServiceDaemon,
    fullname: String,
}

impl MdnsHandle {
    pub fn register(device_name: &str, port: u16) -> Result<Self, mdns_sd::Error> {
        let daemon = ServiceDaemon::new()?;

        let service_type = "_esphomelib._tcp.local.";
        let instance_name = device_name;
        // hostname must end with .local.
        let hostname = format!("{device_name}.local.");

        let mut properties = std::collections::HashMap::new();
        properties.insert("version".to_string(), "2025.1.0".to_string());
        properties.insert("friendly_name".to_string(), device_name.to_string());
        properties.insert("network".to_string(), "wifi".to_string());

        let service_info = ServiceInfo::new(
            service_type,
            instance_name,
            &hostname,
            (),
            port,
            Some(properties),
        )?;

        let fullname = service_info.get_fullname().to_string();
        daemon.register(service_info)?;

        Ok(Self { daemon, fullname })
    }

    fn unregister(&self) {
        let _ = self.daemon.unregister(&self.fullname);
    }
}

impl Drop for MdnsHandle {
    fn drop(&mut self) {
        self.unregister();
    }
}
