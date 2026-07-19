//! HA Entity Export Adapter — thin declarative model for Home Assistant entity contracts.
//!
//! Replaces runtime entity base classes (`rshome_entity_t`, `rshome_sensor_t`, etc.)
//! with a declarative binding model.  Each [`HaEntityExportDefinition`] captures the
//! entity metadata plus command and state bindings that wire to Brookesia
//! `CustomService` functions and events.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ── Entity kind ─────────────────────────────────────────────────────────────

/// HA entity domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum HaEntityKind {
    BinarySensor,
    Sensor,
    Switch,
    Light,
    Climate,
    Button,
    TextSensor,
    Number,
    Select,
}

// ── Bindings ────────────────────────────────────────────────────────────────

/// Maps an HA command (e.g. `"turn_on"`) to a CustomService function call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CommandBinding {
    /// HA command name (e.g. `"turn_on"`, `"set_temperature"`).
    pub ha_command: String,
    /// CustomService function name (e.g. `"gpio_ctrl_0.set"`).
    pub service_function: String,
    /// Static parameter values to pass (e.g. `{"On": true}`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub parameter_map: BTreeMap<String, serde_json::Value>,
}

impl CommandBinding {
    pub fn new(
        ha_command: impl Into<String>,
        service_function: impl Into<String>,
        parameter_map: BTreeMap<String, serde_json::Value>,
    ) -> Self {
        Self {
            ha_command: ha_command.into(),
            service_function: service_function.into(),
            parameter_map,
        }
    }
}

/// Maps a CustomService event to HA entity state updates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StateBinding {
    /// CustomService event name (e.g. `"gpio_ctrl_0.changed"`).
    pub source_event: String,
    /// Maps event field names to HA state field names.
    pub field_map: BTreeMap<String, String>,
}

impl StateBinding {
    pub fn new(source_event: impl Into<String>, field_map: BTreeMap<String, String>) -> Self {
        Self {
            source_event: source_event.into(),
            field_map,
        }
    }
}

// ── Export definition ───────────────────────────────────────────────────────

/// Declarative HA entity contract — metadata + command/state bindings.
///
/// This is the thin replacement for the old runtime entity base classes.
/// Codegen uses this to generate the HA export adapter that wires
/// `CustomService` functions/events to ESPHome Native API protocol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HaEntityExportDefinition {
    /// Entity domain.
    pub kind: HaEntityKind,
    /// Object ID (used in HA entity_id, e.g. `"relay_1"`).
    pub object_id: String,
    /// Explicit unique ID (auto-generated from object_id if absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unique_id: Option<String>,
    /// Human-readable name shown in HA UI.
    pub name: String,
    /// HA device class (e.g. `"switch"`, `"temperature"`, `"humidity"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_class: Option<String>,
    /// Unit of measurement (e.g. `"°C"`, `"%"`, `"lx"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_of_measurement: Option<String>,
    /// Entity category (e.g. `"config"`, `"diagnostic"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_category: Option<String>,
    /// MDI icon (e.g. `"mdi:thermometer"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Command bindings (HA command -> CustomService function).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command_bindings: Vec<CommandBinding>,
    /// State binding (CustomService event -> HA entity state).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_binding: Option<StateBinding>,
    /// Event name that indicates device availability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub availability_event: Option<String>,
}

// ── Convenience constructors ────────────────────────────────────────────────

impl HaEntityExportDefinition {
    /// Create a switch entity with turn_on/turn_off/toggle command bindings.
    pub fn switch_entity(
        object_id: impl Into<String>,
        name: impl Into<String>,
        command_bindings: Vec<CommandBinding>,
        state_binding: StateBinding,
    ) -> Self {
        Self {
            kind: HaEntityKind::Switch,
            object_id: object_id.into(),
            unique_id: None,
            name: name.into(),
            device_class: Some("switch".into()),
            unit_of_measurement: None,
            entity_category: None,
            icon: None,
            command_bindings,
            state_binding: Some(state_binding),
            availability_event: None,
        }
    }

    /// Create a sensor entity with a state binding (no commands).
    pub fn sensor_entity(
        object_id: impl Into<String>,
        name: impl Into<String>,
        device_class: impl Into<String>,
        unit: impl Into<String>,
        state_binding: StateBinding,
    ) -> Self {
        Self {
            kind: HaEntityKind::Sensor,
            object_id: object_id.into(),
            unique_id: None,
            name: name.into(),
            device_class: Some(device_class.into()),
            unit_of_measurement: Some(unit.into()),
            entity_category: None,
            icon: None,
            command_bindings: vec![],
            state_binding: Some(state_binding),
            availability_event: None,
        }
    }

    /// Create a climate entity with command and state bindings.
    pub fn climate_entity(
        object_id: impl Into<String>,
        name: impl Into<String>,
        command_bindings: Vec<CommandBinding>,
        state_binding: StateBinding,
    ) -> Self {
        Self {
            kind: HaEntityKind::Climate,
            object_id: object_id.into(),
            unique_id: None,
            name: name.into(),
            device_class: None,
            unit_of_measurement: Some("°C".into()),
            entity_category: None,
            icon: None,
            command_bindings,
            state_binding: Some(state_binding),
            availability_event: None,
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_entity_construction() {
        let entity = HaEntityExportDefinition::switch_entity(
            "relay_1",
            "Relay 1",
            vec![
                CommandBinding::new(
                    "turn_on",
                    "gpio_ctrl_0.set",
                    BTreeMap::from([("On".into(), serde_json::json!(true))]),
                ),
                CommandBinding::new(
                    "turn_off",
                    "gpio_ctrl_0.set",
                    BTreeMap::from([("On".into(), serde_json::json!(false))]),
                ),
                CommandBinding::new("toggle", "gpio_ctrl_0.toggle", BTreeMap::new()),
            ],
            StateBinding::new(
                "gpio_ctrl_0.changed",
                BTreeMap::from([("on".into(), "state".into())]),
            ),
        );
        assert_eq!(entity.kind, HaEntityKind::Switch);
        assert_eq!(entity.command_bindings.len(), 3);
        assert!(entity.state_binding.is_some());
    }

    #[test]
    fn sensor_entity_construction() {
        let entity = HaEntityExportDefinition::sensor_entity(
            "room_temp",
            "Room Temperature",
            "temperature",
            "°C",
            StateBinding::new(
                "bme280_0.updated",
                BTreeMap::from([("temperature".into(), "value".into())]),
            ),
        );
        assert_eq!(entity.kind, HaEntityKind::Sensor);
        assert!(entity.command_bindings.is_empty());
        assert_eq!(entity.unit_of_measurement.as_deref(), Some("°C"));
    }

    #[test]
    fn serialization_round_trip() {
        let entity = HaEntityExportDefinition::switch_entity(
            "relay_1",
            "Relay 1",
            vec![CommandBinding::new(
                "turn_on",
                "gpio.set",
                BTreeMap::from([("On".into(), serde_json::json!(true))]),
            )],
            StateBinding::new(
                "gpio.changed",
                BTreeMap::from([("on".into(), "state".into())]),
            ),
        );
        let json = serde_json::to_string(&entity).unwrap();
        let back: HaEntityExportDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(back, entity);
    }

    #[test]
    fn empty_optional_fields_skip_serialization() {
        let entity = HaEntityExportDefinition {
            kind: HaEntityKind::Button,
            object_id: "restart".into(),
            unique_id: None,
            name: "Restart".into(),
            device_class: None,
            unit_of_measurement: None,
            entity_category: Some("config".into()),
            icon: Some("mdi:restart".into()),
            command_bindings: vec![],
            state_binding: None,
            availability_event: None,
        };
        let json = serde_json::to_string(&entity).unwrap();
        assert!(!json.contains("unique_id"));
        assert!(!json.contains("device_class"));
        assert!(!json.contains("command_bindings"));
        assert!(!json.contains("state_binding"));
        assert!(json.contains("entity_category"));
        assert!(json.contains("icon"));
    }
}
