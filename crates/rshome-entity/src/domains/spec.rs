//! Domain-first guest contracts.
//!
//! [`DomainSpec`] captures a domain's capabilities (features, device classes,
//! services) in a serialisable form.  [`DomainSpecRegistry`] collects built-in
//! specs (derived from [`DomainDef`]) and allows extension domains to be
//! registered at runtime.

use std::collections::HashMap;
use std::fmt;

use super::DomainDef;

// ── ServiceSpec ──────────────────────────────────────────────────────────────

/// Describes a callable service on a domain.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServiceSpec {
    pub name: String,
    pub schema: Option<serde_json::Value>,
}

// ── DomainSpecError ──────────────────────────────────────────────────────────

/// Validation errors for domain specs and extension registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainSpecError {
    /// A required feature is missing from the provided set.
    MissingRequiredFeature { feature: String },
    /// The domain ID collides with an already-registered domain.
    DomainIdCollision { id: String },
    /// An extension domain feature uses a reserved namespace prefix.
    NamespaceViolation { feature: String },
}

impl fmt::Display for DomainSpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingRequiredFeature { feature } => {
                write!(f, "missing required feature: {feature}")
            }
            Self::DomainIdCollision { id } => {
                write!(f, "domain ID already registered: {id}")
            }
            Self::NamespaceViolation { feature } => {
                write!(f, "feature uses reserved namespace: {feature}")
            }
        }
    }
}

impl std::error::Error for DomainSpecError {}

// ── DomainSpec ───────────────────────────────────────────────────────────────

/// Serialisable description of a domain's capabilities.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DomainSpec {
    pub domain_id: String,
    pub required_features: Vec<String>,
    pub optional_features: Vec<String>,
    pub device_classes: Vec<String>,
    pub services: Vec<ServiceSpec>,
    pub is_built_in: bool,
}

/// Reserved prefixes that extension domains may not use for feature names.
const RESERVED_PREFIXES: &[&str] = &["rshome_", "esphome_", "ha_"];

impl DomainSpec {
    /// Derive a spec from a built-in [`DomainDef`].
    pub fn from_def(def: &dyn DomainDef) -> Self {
        let all_features: Vec<String> = def
            .required_features()
            .iter()
            .chain(def.optional_features().iter())
            .map(|s| s.to_string())
            .collect();
        let services = def
            .services(&all_features)
            .into_iter()
            .map(|name| ServiceSpec {
                name: name.to_string(),
                schema: None,
            })
            .collect();
        Self {
            domain_id: def.id().to_string(),
            required_features: def
                .required_features()
                .iter()
                .map(|s| s.to_string())
                .collect(),
            optional_features: def
                .optional_features()
                .iter()
                .map(|s| s.to_string())
                .collect(),
            device_classes: def.device_classes().iter().map(|s| s.to_string()).collect(),
            services,
            is_built_in: true,
        }
    }

    /// Create a custom extension domain spec.
    pub fn extension(
        id: impl Into<String>,
        required_features: Vec<String>,
        optional_features: Vec<String>,
        device_classes: Vec<String>,
        services: Vec<ServiceSpec>,
    ) -> Self {
        Self {
            domain_id: id.into(),
            required_features,
            optional_features,
            device_classes,
            services,
            is_built_in: false,
        }
    }

    /// Validate that all required features are present in the given set.
    pub fn validate_features(&self, features: &[String]) -> Result<(), DomainSpecError> {
        for req in &self.required_features {
            if !features.iter().any(|f| f == req) {
                return Err(DomainSpecError::MissingRequiredFeature {
                    feature: req.clone(),
                });
            }
        }
        Ok(())
    }

    /// Check whether a device class is valid for this domain.
    pub fn validate_device_class(&self, class: &str) -> bool {
        // Empty device_classes list means any class is acceptable (or none).
        self.device_classes.is_empty() || self.device_classes.iter().any(|c| c == class)
    }

    /// Check whether a service name is valid for this domain.
    pub fn has_service(&self, name: &str) -> bool {
        self.services.iter().any(|s| s.name == name)
    }

    /// All feature names (required + optional).
    pub fn all_features(&self) -> Vec<&str> {
        self.required_features
            .iter()
            .chain(self.optional_features.iter())
            .map(String::as_str)
            .collect()
    }

    /// Validate that extension feature names don't use reserved prefixes.
    pub fn validate_extension_features(features: &[String]) -> Result<(), DomainSpecError> {
        for f in features {
            for prefix in RESERVED_PREFIXES {
                if f.starts_with(prefix) {
                    return Err(DomainSpecError::NamespaceViolation { feature: f.clone() });
                }
            }
        }
        Ok(())
    }
}

// ── DomainSpecRegistry ───────────────────────────────────────────────────────

/// Registry of domain specs (built-in + extension).
pub struct DomainSpecRegistry {
    specs: HashMap<String, DomainSpec>,
}

impl DomainSpecRegistry {
    /// Create a registry pre-populated with all 17 built-in domains.
    pub fn built_in() -> Self {
        let domain_reg = super::DomainRegistry::built_in();
        let mut specs = HashMap::new();
        for def in domain_reg.all_domains() {
            let spec = DomainSpec::from_def(*def);
            specs.insert(spec.domain_id.clone(), spec);
        }
        Self { specs }
    }

    /// Create an empty registry.
    pub fn empty() -> Self {
        Self {
            specs: HashMap::new(),
        }
    }

    /// Register an extension domain. Fails on ID collision or namespace
    /// violations in feature names.
    pub fn register_extension(&mut self, spec: DomainSpec) -> Result<(), DomainSpecError> {
        if self.specs.contains_key(&spec.domain_id) {
            return Err(DomainSpecError::DomainIdCollision {
                id: spec.domain_id.clone(),
            });
        }
        let all: Vec<String> = spec
            .required_features
            .iter()
            .chain(spec.optional_features.iter())
            .cloned()
            .collect();
        DomainSpec::validate_extension_features(&all)?;
        self.specs.insert(spec.domain_id.clone(), spec);
        Ok(())
    }

    /// Look up a spec by domain ID.
    pub fn get(&self, domain_id: &str) -> Option<&DomainSpec> {
        self.specs.get(domain_id)
    }

    /// Iterate all registered specs.
    pub fn all_specs(&self) -> impl Iterator<Item = &DomainSpec> {
        self.specs.values()
    }

    /// Number of registered specs.
    pub fn len(&self) -> usize {
        self.specs.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::DomainRegistry;

    #[test]
    fn built_in_has_17_specs() {
        let reg = DomainSpecRegistry::built_in();
        assert_eq!(reg.len(), 17);
    }

    #[test]
    fn from_def_sensor() {
        let def = DomainRegistry::built_in().get("sensor").unwrap();
        let spec = DomainSpec::from_def(def);
        assert_eq!(spec.domain_id, "sensor");
        assert!(spec.is_built_in);
        assert!(spec.required_features.contains(&"state".to_string()));
        assert!(spec.optional_features.contains(&"unit".to_string()));
        assert!(spec.device_classes.contains(&"temperature".to_string()));
        assert!(spec.services.is_empty()); // sensor is read-only
    }

    #[test]
    fn from_def_switch_has_services() {
        let def = DomainRegistry::built_in().get("switch").unwrap();
        let spec = DomainSpec::from_def(def);
        assert_eq!(spec.domain_id, "switch");
        let svc_names: Vec<&str> = spec.services.iter().map(|s| s.name.as_str()).collect();
        assert!(svc_names.contains(&"turn_on"));
        assert!(svc_names.contains(&"turn_off"));
        assert!(svc_names.contains(&"toggle"));
    }

    #[test]
    fn extension_spec() {
        let spec = DomainSpec::extension(
            "custom_irrigation",
            vec!["state".into()],
            vec!["schedule".into()],
            vec!["sprinkler".into()],
            vec![ServiceSpec {
                name: "start_zone".into(),
                schema: None,
            }],
        );
        assert!(!spec.is_built_in);
        assert_eq!(spec.domain_id, "custom_irrigation");
        assert!(spec.has_service("start_zone"));
    }

    #[test]
    fn validate_features_ok() {
        let spec = DomainSpec::from_def(DomainRegistry::built_in().get("sensor").unwrap());
        assert!(spec
            .validate_features(&["state".into(), "unit".into()])
            .is_ok());
    }

    #[test]
    fn validate_features_missing_required() {
        let spec = DomainSpec::from_def(DomainRegistry::built_in().get("sensor").unwrap());
        let err = spec.validate_features(&["unit".into()]).unwrap_err();
        assert!(
            matches!(err, DomainSpecError::MissingRequiredFeature { feature } if feature == "state")
        );
    }

    #[test]
    fn validate_device_class_known() {
        let spec = DomainSpec::from_def(DomainRegistry::built_in().get("sensor").unwrap());
        assert!(spec.validate_device_class("temperature"));
        assert!(!spec.validate_device_class("nonexistent_class"));
    }

    #[test]
    fn validate_device_class_empty_list_accepts_any() {
        let spec = DomainSpec::from_def(DomainRegistry::built_in().get("light").unwrap());
        assert!(spec.device_classes.is_empty());
        assert!(spec.validate_device_class("anything"));
    }

    #[test]
    fn register_extension_ok() {
        let mut reg = DomainSpecRegistry::built_in();
        let spec =
            DomainSpec::extension("custom_pool", vec!["state".into()], vec![], vec![], vec![]);
        assert!(reg.register_extension(spec).is_ok());
        assert_eq!(reg.len(), 18);
        assert!(reg.get("custom_pool").is_some());
    }

    #[test]
    fn register_extension_collision() {
        let mut reg = DomainSpecRegistry::built_in();
        let spec = DomainSpec::extension("sensor", vec![], vec![], vec![], vec![]);
        let err = reg.register_extension(spec).unwrap_err();
        assert!(matches!(err, DomainSpecError::DomainIdCollision { id } if id == "sensor"));
    }

    #[test]
    fn register_extension_namespace_violation() {
        let mut reg = DomainSpecRegistry::built_in();
        let spec = DomainSpec::extension(
            "custom_bad",
            vec!["rshome_internal".into()],
            vec![],
            vec![],
            vec![],
        );
        let err = reg.register_extension(spec).unwrap_err();
        assert!(matches!(
            err,
            DomainSpecError::NamespaceViolation { feature } if feature == "rshome_internal"
        ));
    }

    #[test]
    fn all_specs_iterates() {
        let reg = DomainSpecRegistry::built_in();
        let count = reg.all_specs().count();
        assert_eq!(count, 17);
    }

    #[test]
    fn has_service_check() {
        let spec = DomainSpec::from_def(DomainRegistry::built_in().get("switch").unwrap());
        assert!(spec.has_service("turn_on"));
        assert!(!spec.has_service("nonexistent"));
    }

    #[test]
    fn all_features() {
        let spec = DomainSpec::from_def(DomainRegistry::built_in().get("sensor").unwrap());
        let all = spec.all_features();
        assert!(all.contains(&"state"));
        assert!(all.contains(&"unit"));
    }
}
