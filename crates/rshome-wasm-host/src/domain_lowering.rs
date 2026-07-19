//! Domain-aware lowering layer.
//!
//! Validates entity/service registrations against [`DomainSpec`] definitions
//! and produces correctly-typed [`EntityState`] values from JSON bytes.

use std::collections::HashMap;
use std::fmt;

use rshome_entity::{
    DomainSpec, DomainSpecError, DomainSpecRegistry, EntityCategory, EntityDescriptor, EntityId,
    EntityState,
};

// ── LoweringError ────────────────────────────────────────────────────────────

/// Errors during domain-aware lowering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoweringError {
    UnknownDomain { id: String },
    InvalidFeatures(DomainSpecError),
    InvalidDeviceClass { domain: String, class: String },
    InvalidService { domain: String, service: String },
    StateSerialization(String),
    ExtensionRegistration(DomainSpecError),
}

impl fmt::Display for LoweringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownDomain { id } => write!(f, "unknown domain: {id}"),
            Self::InvalidFeatures(e) => write!(f, "invalid features: {e}"),
            Self::InvalidDeviceClass { domain, class } => {
                write!(f, "invalid device class '{class}' for domain '{domain}'")
            }
            Self::InvalidService { domain, service } => {
                write!(f, "invalid service '{service}' for domain '{domain}'")
            }
            Self::StateSerialization(msg) => write!(f, "state serialization error: {msg}"),
            Self::ExtensionRegistration(e) => write!(f, "extension registration error: {e}"),
        }
    }
}

impl std::error::Error for LoweringError {}

impl From<DomainSpecError> for LoweringError {
    fn from(e: DomainSpecError) -> Self {
        Self::InvalidFeatures(e)
    }
}

// ── EntityRegistration ───────────────────────────────────────────────────────

/// Output of a validated entity registration.
#[derive(Debug, Clone)]
pub struct EntityRegistration {
    pub platform: String,
    pub unique_id: String,
    pub descriptor: EntityDescriptor,
    pub initial_state: EntityState,
}

// ── DomainLoweringLayer ──────────────────────────────────────────────────────

/// Validates entity/service registrations and state updates against domain specs.
pub struct DomainLoweringLayer {
    spec_registry: DomainSpecRegistry,
}

impl DomainLoweringLayer {
    /// Create a new layer backed by the built-in domain spec registry.
    pub fn new() -> Self {
        Self {
            spec_registry: DomainSpecRegistry::built_in(),
        }
    }

    /// Create a layer from a custom spec registry (for testing).
    pub fn with_registry(spec_registry: DomainSpecRegistry) -> Self {
        Self { spec_registry }
    }

    /// Access the underlying spec registry.
    pub fn spec_registry(&self) -> &DomainSpecRegistry {
        &self.spec_registry
    }

    /// Validate an entity registration against the domain spec.
    pub fn validate_entity_registration(
        &self,
        domain_id: &str,
        features: &[String],
        device_class: Option<&str>,
    ) -> Result<(), LoweringError> {
        let spec =
            self.spec_registry
                .get(domain_id)
                .ok_or_else(|| LoweringError::UnknownDomain {
                    id: domain_id.to_string(),
                })?;

        spec.validate_features(features)?;

        if let Some(class) = device_class {
            if !spec.validate_device_class(class) {
                return Err(LoweringError::InvalidDeviceClass {
                    domain: domain_id.to_string(),
                    class: class.to_string(),
                });
            }
        }

        Ok(())
    }

    /// Produce a validated entity registration with descriptor and default state.
    pub fn lower_register_entity(
        &self,
        domain_id: &str,
        unique_id: &str,
        name: &str,
        features: &[String],
        device_class: Option<&str>,
    ) -> Result<EntityRegistration, LoweringError> {
        self.validate_entity_registration(domain_id, features, device_class)?;

        let entity_id = EntityId::new(domain_id, unique_id);
        let descriptor = EntityDescriptor {
            entity_id,
            name: name.to_string(),
            icon: None,
            device_id: None,
            area_id: None,
            entity_category: EntityCategory::None,
            domain_id: domain_id.to_string(),
            feature_set: features.to_vec(),
            device_class: device_class.map(|s| s.to_string()),
        };
        let initial_state = default_state_for_domain(domain_id);
        Ok(EntityRegistration {
            platform: domain_id.to_string(),
            unique_id: unique_id.to_string(),
            descriptor,
            initial_state,
        })
    }

    /// Validate that a service name is valid for the given domain.
    pub fn lower_register_service(
        &self,
        domain_id: &str,
        service_name: &str,
    ) -> Result<(), LoweringError> {
        let spec =
            self.spec_registry
                .get(domain_id)
                .ok_or_else(|| LoweringError::UnknownDomain {
                    id: domain_id.to_string(),
                })?;

        if !spec.has_service(service_name) {
            return Err(LoweringError::InvalidService {
                domain: domain_id.to_string(),
                service: service_name.to_string(),
            });
        }
        Ok(())
    }

    /// Deserialize JSON bytes into the correct [`EntityState`] variant for a domain.
    pub fn lower_state_update(
        &self,
        domain_id: &str,
        state_bytes: &[u8],
    ) -> Result<EntityState, LoweringError> {
        let _ = self
            .spec_registry
            .get(domain_id)
            .ok_or_else(|| LoweringError::UnknownDomain {
                id: domain_id.to_string(),
            })?;

        // The state_bytes are expected to be a JSON-serialised EntityState variant.
        // EntityState is tagged externally by serde, so the JSON looks like:
        //   {"Sensor": {"value": 22.5, "unit": "°C", "attributes": {}}}
        serde_json::from_slice(state_bytes)
            .map_err(|e| LoweringError::StateSerialization(e.to_string()))
    }

    /// Register an extension domain.
    pub fn register_extension_domain(&mut self, spec: DomainSpec) -> Result<(), LoweringError> {
        self.spec_registry
            .register_extension(spec)
            .map_err(LoweringError::ExtensionRegistration)
    }
}

impl Default for DomainLoweringLayer {
    fn default() -> Self {
        Self::new()
    }
}

// ── Default state helpers ────────────────────────────────────────────────────

/// Return the zero-value default state for a built-in domain.
pub fn default_state_for_domain(domain_id: &str) -> EntityState {
    match domain_id {
        "sensor" => EntityState::Sensor {
            value: 0.0,
            unit: None,
            attributes: HashMap::new(),
        },
        "binary_sensor" => EntityState::BinarySensor {
            is_on: false,
            attributes: HashMap::new(),
        },
        "switch" => EntityState::Switch { is_on: false },
        "light" => EntityState::Light {
            is_on: false,
            brightness: None,
            color_temp: None,
            rgb: None,
            color_mode: None,
        },
        "climate" => EntityState::Climate {
            mode: "off".to_string(),
            current_temp: None,
            target_temp: None,
            hvac_action: None,
        },
        "fan" => EntityState::Fan {
            is_on: false,
            speed: None,
            oscillating: None,
            direction: None,
        },
        "cover" => EntityState::Cover {
            state: rshome_entity::CoverState::Closed,
            position: None,
            tilt: None,
        },
        "lock" => EntityState::Lock {
            state: rshome_entity::LockState::Locked,
        },
        "number" => EntityState::Number {
            value: 0.0,
            min: 0.0,
            max: 100.0,
            step: 1.0,
            unit: None,
        },
        "select" => EntityState::Select {
            current: String::new(),
            options: vec![],
        },
        "text" => EntityState::Text {
            value: String::new(),
        },
        "button" => EntityState::Button,
        "event" => EntityState::Event {
            event_type: String::new(),
            event_data: HashMap::new(),
        },
        "media_player" => EntityState::MediaPlayer {
            state: rshome_entity::MediaPlayerState::Idle,
            volume: None,
            muted: None,
            media_title: None,
        },
        "alarm_control_panel" => EntityState::AlarmControlPanel {
            state: rshome_entity::AlarmState::Disarmed,
            code_format: None,
        },
        "text_sensor" => EntityState::TextSensor {
            value: String::new(),
        },
        "update" => EntityState::Update {
            installed_version: String::new(),
            latest_version: None,
            in_progress: false,
        },
        _ => EntityState::Unavailable,
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rshome_entity::ServiceSpec;

    fn layer() -> DomainLoweringLayer {
        DomainLoweringLayer::new()
    }

    #[test]
    fn validate_entity_ok() {
        let l = layer();
        assert!(l
            .validate_entity_registration("sensor", &["state".into()], None)
            .is_ok());
    }

    #[test]
    fn validate_entity_unknown_domain() {
        let l = layer();
        let err = l
            .validate_entity_registration("nonexistent", &[], None)
            .unwrap_err();
        assert!(matches!(err, LoweringError::UnknownDomain { id } if id == "nonexistent"));
    }

    #[test]
    fn validate_entity_missing_feature() {
        let l = layer();
        let err = l
            .validate_entity_registration("sensor", &[], None)
            .unwrap_err();
        assert!(matches!(err, LoweringError::InvalidFeatures(_)));
    }

    #[test]
    fn validate_entity_bad_device_class() {
        let l = layer();
        let err = l
            .validate_entity_registration("sensor", &["state".into()], Some("nonexistent_class"))
            .unwrap_err();
        assert!(matches!(err, LoweringError::InvalidDeviceClass { .. }));
    }

    #[test]
    fn validate_entity_good_device_class() {
        let l = layer();
        assert!(l
            .validate_entity_registration("sensor", &["state".into()], Some("temperature"),)
            .is_ok());
    }

    #[test]
    fn validate_entity_empty_device_class_list() {
        let l = layer();
        // light has no device classes → accepts any
        assert!(l
            .validate_entity_registration(
                "light",
                &["state".into(), "toggle".into()],
                Some("custom_class"),
            )
            .is_ok());
    }

    #[test]
    fn lower_register_entity_ok() {
        let l = layer();
        let reg = l
            .lower_register_entity(
                "switch",
                "relay_1",
                "Relay",
                &["state".into(), "toggle".into()],
                None,
            )
            .unwrap();
        assert_eq!(reg.platform, "switch");
        assert_eq!(reg.unique_id, "relay_1");
        assert_eq!(reg.descriptor.domain_id, "switch");
        assert_eq!(reg.descriptor.name, "Relay");
        assert!(matches!(
            reg.initial_state,
            EntityState::Switch { is_on: false }
        ));
    }

    #[test]
    fn lower_register_entity_with_device_class() {
        let l = layer();
        let reg = l
            .lower_register_entity(
                "sensor",
                "temp_01",
                "Temperature",
                &["state".into()],
                Some("temperature"),
            )
            .unwrap();
        assert_eq!(reg.descriptor.device_class.as_deref(), Some("temperature"));
    }

    #[test]
    fn lower_register_service_ok() {
        let l = layer();
        assert!(l.lower_register_service("switch", "turn_on").is_ok());
    }

    #[test]
    fn lower_register_service_unknown_domain() {
        let l = layer();
        let err = l.lower_register_service("nonexistent", "foo").unwrap_err();
        assert!(matches!(err, LoweringError::UnknownDomain { .. }));
    }

    #[test]
    fn lower_register_service_invalid() {
        let l = layer();
        let err = l.lower_register_service("switch", "fly_away").unwrap_err();
        assert!(matches!(err, LoweringError::InvalidService { .. }));
    }

    #[test]
    fn lower_state_update_ok() {
        let l = layer();
        let json = serde_json::to_vec(&EntityState::Sensor {
            value: 22.5,
            unit: Some("°C".to_string()),
            attributes: HashMap::new(),
        })
        .unwrap();
        let state = l.lower_state_update("sensor", &json).unwrap();
        assert!(
            matches!(state, EntityState::Sensor { value, .. } if (value - 22.5).abs() < f64::EPSILON)
        );
    }

    #[test]
    fn lower_state_update_unknown_domain() {
        let l = layer();
        let err = l.lower_state_update("nonexistent", b"{}").unwrap_err();
        assert!(matches!(err, LoweringError::UnknownDomain { .. }));
    }

    #[test]
    fn lower_state_update_bad_json() {
        let l = layer();
        let err = l.lower_state_update("sensor", b"not json").unwrap_err();
        assert!(matches!(err, LoweringError::StateSerialization(_)));
    }

    #[test]
    fn register_extension_ok() {
        let mut l = layer();
        let spec = DomainSpec::extension(
            "custom_pool",
            vec!["state".into()],
            vec![],
            vec![],
            vec![ServiceSpec {
                name: "fill".into(),
                schema: None,
            }],
        );
        assert!(l.register_extension_domain(spec).is_ok());
        assert!(l.lower_register_service("custom_pool", "fill").is_ok());
    }

    #[test]
    fn register_extension_collision() {
        let mut l = layer();
        let spec = DomainSpec::extension("sensor", vec![], vec![], vec![], vec![]);
        let err = l.register_extension_domain(spec).unwrap_err();
        assert!(matches!(err, LoweringError::ExtensionRegistration(_)));
    }

    #[test]
    fn default_state_all_17_domains() {
        let domains = [
            "sensor",
            "binary_sensor",
            "switch",
            "light",
            "climate",
            "fan",
            "cover",
            "lock",
            "number",
            "select",
            "text",
            "button",
            "event",
            "media_player",
            "alarm_control_panel",
            "text_sensor",
            "update",
        ];
        for d in &domains {
            let state = default_state_for_domain(d);
            assert!(
                !matches!(state, EntityState::Unavailable),
                "domain {d} should have a default state, got Unavailable"
            );
        }
    }

    #[test]
    fn default_state_unknown_domain_returns_unavailable() {
        let state = default_state_for_domain("nonexistent");
        assert!(matches!(state, EntityState::Unavailable));
    }
}
