use rshome_entity::{DeviceId, EntityId};

/// Whether an imported entity supports outbound Native API commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportedCommandCapabilities {
    /// Entity accepts state-change commands (Switch, Light, Climate, Fan, Cover, Number,
    /// Select, Button).
    Controllable,
    /// Entity is read-only (Sensor, BinarySensor, TextSensor).
    ReadOnly,
}

/// Pairs a local runtime entity with its remote transport identity.
///
/// This is the single canonical location for transport-specific remote keying.
/// All command routing from service callers → `DeviceSessionActor` → firmware
/// must resolve through this binding.
#[derive(Debug, Clone)]
pub struct ImportedEntityBinding {
    /// Local entity ID in the rshome-ha runtime: `<domain>.<device_slug>__<remote_object_id>`.
    pub local_entity_id: EntityId,
    /// Canonical device ID of the owning device (`esphome:<mac>` or `esphome-host:<hostname>`).
    pub device_id: DeviceId,
    /// ESPHome FNV-1a wire key from the `ListEntities` response.
    pub remote_key: u32,
    /// Remote `object_id` string from `ListEntities`.
    pub remote_object_id: String,
    /// Remote `unique_id` from `ListEntities`, if provided by firmware.
    pub remote_unique_id: Option<String>,
    /// Remote entity domain string (e.g. `"sensor"`, `"switch"`, `"light"`).
    pub remote_domain: String,
    /// Whether this entity accepts outbound commands.
    pub command_capabilities: ImportedCommandCapabilities,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_fields_are_accessible() {
        let binding = ImportedEntityBinding {
            local_entity_id: EntityId::new("switch", "my_esp32__relay"),
            device_id: DeviceId("esphome:aabbccddeeff".into()),
            remote_key: 42,
            remote_object_id: "relay".into(),
            remote_unique_id: Some("relay-unique".into()),
            remote_domain: "switch".into(),
            command_capabilities: ImportedCommandCapabilities::Controllable,
        };
        assert_eq!(binding.remote_key, 42);
        assert_eq!(
            binding.command_capabilities,
            ImportedCommandCapabilities::Controllable
        );
    }

    #[test]
    fn read_only_capability_for_sensor() {
        let binding = ImportedEntityBinding {
            local_entity_id: EntityId::new("sensor", "my_esp32__temp"),
            device_id: DeviceId("esphome:aabbccddeeff".into()),
            remote_key: 99,
            remote_object_id: "temp".into(),
            remote_unique_id: None,
            remote_domain: "sensor".into(),
            command_capabilities: ImportedCommandCapabilities::ReadOnly,
        };
        assert_eq!(
            binding.command_capabilities,
            ImportedCommandCapabilities::ReadOnly
        );
        assert!(binding.remote_unique_id.is_none());
    }
}
