mod alarm_panel;
mod binary_sensor;
mod button;
mod climate;
mod cover;
mod event;
mod fan;
mod light;
mod lock;
mod media_player;
mod number;
mod select;
mod sensor;
pub mod spec;
mod switch;
mod text;
mod text_sensor;
mod update;

use crate::entity_state::EntityState;
use crate::messages::EntityCommand;
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::sync::LazyLock;

/// Error returned when a domain rejects a command.
#[derive(Debug)]
pub enum DomainError {
    CommandNotApplicable {
        domain: &'static str,
        command: String,
    },
    ReadOnly {
        domain: &'static str,
    },
}

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CommandNotApplicable { domain, command } => {
                write!(f, "command {command} not applicable to domain {domain}")
            }
            Self::ReadOnly { domain } => write!(f, "domain {domain} is read-only"),
        }
    }
}

impl std::error::Error for DomainError {}

/// Per-domain definition. Implemented as a ZST for each ESPHome domain.
pub trait DomainDef: Send + Sync + 'static {
    fn id(&self) -> &'static str;
    fn required_features(&self) -> &'static [&'static str];
    fn optional_features(&self) -> &'static [&'static str];
    fn device_classes(&self) -> &'static [&'static str];
    fn services(&self, features: &[String]) -> Vec<&'static str>;
    fn apply_command(
        &self,
        state: &EntityState,
        cmd: &EntityCommand,
    ) -> Result<EntityState, DomainError>;
    fn encode_command(&self, service: &str, data: &Value) -> Option<EntityCommand>;
}

/// Central registry of all known domains.
pub struct DomainRegistry {
    map: HashMap<&'static str, &'static dyn DomainDef>,
}

static BUILT_IN: LazyLock<DomainRegistry> = LazyLock::new(|| {
    let domains: Vec<&'static dyn DomainDef> = vec![
        &sensor::Sensor,
        &binary_sensor::BinarySensor,
        &switch::Switch,
        &light::Light,
        &climate::Climate,
        &fan::Fan,
        &cover::Cover,
        &lock::Lock,
        &number::Number,
        &select::Select,
        &text::Text,
        &button::Button,
        &media_player::MediaPlayer,
        &alarm_panel::AlarmPanel,
        &text_sensor::TextSensor,
        &update::Update,
        &event::Event,
    ];
    let mut map = HashMap::new();
    for d in domains {
        map.insert(d.id(), d);
    }
    DomainRegistry { map }
});

impl DomainRegistry {
    /// Returns the built-in registry covering all ESPHome-originated domains.
    pub fn built_in() -> &'static DomainRegistry {
        &BUILT_IN
    }

    /// Look up a domain by its ID.
    pub fn get(&self, domain_id: &str) -> Option<&'static dyn DomainDef> {
        self.map.get(domain_id).copied()
    }

    /// Iterate all registered domains.
    pub fn all_domains(&self) -> impl Iterator<Item = &&'static dyn DomainDef> {
        self.map.values()
    }

    /// Resolve a wire type string to `(domain_id, feature_set)`.
    pub fn resolve_wire_type(&self, wire_type: &str) -> Option<(&str, Vec<String>)> {
        let def = self.get(wire_type)?;
        let features: Vec<String> = def
            .required_features()
            .iter()
            .chain(def.optional_features().iter())
            .map(|s| s.to_string())
            .collect();
        Some((def.id(), features))
    }

    /// Return only the required features for a domain.
    pub fn required_features(&self, domain_id: &str) -> Option<Vec<String>> {
        self.get(domain_id).map(|d| {
            d.required_features()
                .iter()
                .map(|s| s.to_string())
                .collect()
        })
    }

    /// Return the callable service names for `(domain, feature_set)`.
    pub fn services_for(&self, domain_id: &str, features: &[String]) -> Vec<String> {
        let feature_set: std::collections::HashSet<&str> =
            features.iter().map(String::as_str).collect();
        let mut services = Vec::new();
        if let Some(def) = self.get(domain_id) {
            for &feat in def
                .required_features()
                .iter()
                .chain(def.optional_features().iter())
            {
                if !feature_set.contains(feat) {
                    continue;
                }
                match feat {
                    "toggle" => {
                        services.push("turn_on".into());
                        services.push("turn_off".into());
                        services.push("toggle".into());
                    }
                    "set_mode" => services.push("set_hvac_mode".into()),
                    "set_text" => services.push("set_value".into()),
                    "set_option" => services.push("select_option".into()),
                    "target_temp" => services.push("set_temperature".into()),
                    "set_value" | "press" | "lock" | "unlock" | "volume_set" | "open_cover"
                    | "close_cover" | "set_cover_position" | "set_percentage" => {
                        services.push(feat.into())
                    }
                    _ => {}
                }
            }
        }
        services
    }

    /// Check whether `(domain_id, features)` is a legal combination.
    pub fn is_legal(&self, domain_id: &str, features: &[String]) -> bool {
        let Some(def) = self.get(domain_id) else {
            return false;
        };
        let present: std::collections::HashSet<&str> =
            features.iter().map(String::as_str).collect();
        def.required_features().iter().all(|r| present.contains(*r))
    }
}

// ── Helper for read-only domains ────────────────────────────────────────────

fn read_only_apply(
    domain: &'static str,
    _state: &EntityState,
    _cmd: &EntityCommand,
) -> Result<EntityState, DomainError> {
    Err(DomainError::ReadOnly { domain })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_has_17_domains() {
        assert_eq!(DomainRegistry::built_in().map.len(), 17);
    }

    #[test]
    fn resolve_known_wire_type() {
        let (domain, features) = DomainRegistry::built_in()
            .resolve_wire_type("sensor")
            .unwrap();
        assert_eq!(domain, "sensor");
        assert!(features.contains(&"state".to_string()));
    }

    #[test]
    fn resolve_unknown_wire_type_returns_none() {
        assert!(DomainRegistry::built_in()
            .resolve_wire_type("unknown_xyz")
            .is_none());
    }

    #[test]
    fn is_legal_with_required_features() {
        assert!(DomainRegistry::built_in().is_legal("sensor", &["state".into()]));
    }

    #[test]
    fn is_legal_missing_required_feature() {
        assert!(!DomainRegistry::built_in().is_legal("sensor", &[]));
    }

    #[test]
    fn is_legal_unknown_domain() {
        assert!(!DomainRegistry::built_in().is_legal("unknown_xyz", &["state".into()]));
    }

    #[test]
    fn get_returns_correct_domain() {
        let def = DomainRegistry::built_in().get("switch").unwrap();
        assert_eq!(def.id(), "switch");
    }

    #[test]
    fn all_domains_iterates() {
        let count = DomainRegistry::built_in().all_domains().count();
        assert_eq!(count, 17);
    }
}
