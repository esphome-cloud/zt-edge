//! Typed guest SDK generation.
//!
//! Generates Rust source modules from [`DomainSpec`] definitions so that
//! WASM integration authors get type-safe state structs, command enums,
//! and service constants rather than raw byte manipulation.

use rshome_entity::{DomainSpec, DomainSpecRegistry};

// ── Public API ───────────────────────────────────────────────────────────────

/// Generate a complete Rust module for a single domain spec.
pub fn generate_guest_sdk(spec: &DomainSpec) -> String {
    let mod_name = &spec.domain_id;
    let type_name = domain_id_to_type_name(&spec.domain_id);

    let mut out = String::new();

    // Module header
    out.push_str(&format!(
        "/// Auto-generated guest SDK for the `{mod_name}` domain.\n"
    ));
    out.push_str(&format!("pub mod {mod_name} {{\n"));

    // State struct
    let fields = state_fields_for_domain(&spec.domain_id);
    if fields.is_empty() {
        // Unit-like state (e.g. Button)
        out.push_str(&format!("    /// State for the `{mod_name}` domain.\n"));
        out.push_str("    #[derive(Debug, Clone, Default)]\n");
        out.push_str(&format!("    pub struct {type_name}State;\n\n"));
    } else {
        out.push_str(&format!("    /// State for the `{mod_name}` domain.\n"));
        out.push_str("    #[derive(Debug, Clone)]\n");
        out.push_str(&format!("    pub struct {type_name}State {{\n"));
        for (name, ty) in &fields {
            out.push_str(&format!("        pub {name}: {ty},\n"));
        }
        out.push_str("    }\n\n");
    }

    // Device class enum (if any)
    if !spec.device_classes.is_empty() {
        out.push_str(&format!(
            "    /// Device classes for the `{mod_name}` domain.\n"
        ));
        out.push_str("    #[derive(Debug, Clone, Copy, PartialEq, Eq)]\n");
        out.push_str(&format!("    pub enum {type_name}DeviceClass {{\n"));
        for class in &spec.device_classes {
            let variant = snake_to_pascal(class);
            out.push_str(&format!("        {variant},\n"));
        }
        out.push_str("    }\n\n");

        out.push_str(&format!(
            "    impl {type_name}DeviceClass {{\n\
             \x20       pub fn as_str(&self) -> &'static str {{\n\
             \x20           match self {{\n"
        ));
        for class in &spec.device_classes {
            let variant = snake_to_pascal(class);
            out.push_str(&format!(
                "                Self::{variant} => \"{class}\",\n"
            ));
        }
        out.push_str(
            "            }\n\
             \x20       }\n\
             \x20   }\n\n",
        );
    }

    // Service name constants
    if !spec.services.is_empty() {
        out.push_str("    /// Service name constants.\n");
        out.push_str("    pub mod services {\n");
        for svc in &spec.services {
            let const_name = svc.name.to_uppercase();
            out.push_str(&format!(
                "        pub const {const_name}: &str = \"{}\";\n",
                svc.name
            ));
        }
        out.push_str("    }\n\n");
    }

    // Feature constants
    out.push_str("    /// Feature constants.\n");
    out.push_str("    pub mod features {\n");
    for f in &spec.required_features {
        let const_name = f.to_uppercase();
        out.push_str(&format!(
            "        pub const {const_name}: &str = \"{f}\";\n"
        ));
    }
    for f in &spec.optional_features {
        let const_name = f.to_uppercase();
        out.push_str(&format!(
            "        pub const {const_name}: &str = \"{f}\";\n"
        ));
    }
    out.push_str("    }\n");

    // Close module
    out.push_str("}\n");

    out
}

/// Generate SDK modules for all domains in a registry.
pub fn generate_all_sdks(registry: &DomainSpecRegistry) -> String {
    let mut out = String::from("//! Auto-generated domain guest SDKs.\n\n");
    let mut specs: Vec<&DomainSpec> = registry.all_specs().collect();
    specs.sort_by_key(|s| &s.domain_id);
    for spec in specs {
        out.push_str(&generate_guest_sdk(spec));
        out.push('\n');
    }
    out
}

/// Convert a domain_id like `"binary_sensor"` to a type name like `"BinarySensor"`.
pub fn domain_id_to_type_name(domain_id: &str) -> String {
    snake_to_pascal(domain_id)
}

/// Return the state fields for a built-in domain, matching [`EntityState`] variants.
///
/// Each entry is `(field_name, rust_type)`. Returns empty vec for unit-like
/// states (e.g. `Button`).
pub fn state_fields_for_domain(domain_id: &str) -> Vec<(&'static str, &'static str)> {
    match domain_id {
        "sensor" => vec![
            ("value", "f64"),
            ("unit", "Option<String>"),
            (
                "attributes",
                "std::collections::HashMap<String, serde_json::Value>",
            ),
        ],
        "binary_sensor" => vec![
            ("is_on", "bool"),
            (
                "attributes",
                "std::collections::HashMap<String, serde_json::Value>",
            ),
        ],
        "switch" => vec![("is_on", "bool")],
        "light" => vec![
            ("is_on", "bool"),
            ("brightness", "Option<f64>"),
            ("color_temp", "Option<u16>"),
            ("rgb", "Option<[u8; 3]>"),
            ("color_mode", "Option<String>"),
        ],
        "climate" => vec![
            ("mode", "String"),
            ("current_temp", "Option<f64>"),
            ("target_temp", "Option<f64>"),
            ("hvac_action", "Option<String>"),
        ],
        "fan" => vec![
            ("is_on", "bool"),
            ("speed", "Option<u8>"),
            ("oscillating", "Option<bool>"),
            ("direction", "Option<String>"),
        ],
        "cover" => vec![
            ("state", "String"),
            ("position", "Option<u8>"),
            ("tilt", "Option<u8>"),
        ],
        "lock" => vec![("state", "String")],
        "number" => vec![
            ("value", "f64"),
            ("min", "f64"),
            ("max", "f64"),
            ("step", "f64"),
            ("unit", "Option<String>"),
        ],
        "select" => vec![("current", "String"), ("options", "Vec<String>")],
        "text" => vec![("value", "String")],
        "button" => vec![],
        "event" => vec![
            ("event_type", "String"),
            (
                "event_data",
                "std::collections::HashMap<String, serde_json::Value>",
            ),
        ],
        "media_player" => vec![
            ("state", "String"),
            ("volume", "Option<f64>"),
            ("muted", "Option<bool>"),
            ("media_title", "Option<String>"),
        ],
        "alarm_control_panel" => vec![("state", "String"), ("code_format", "Option<String>")],
        "text_sensor" => vec![("value", "String")],
        "update" => vec![
            ("installed_version", "String"),
            ("latest_version", "Option<String>"),
            ("in_progress", "bool"),
        ],
        _ => vec![],
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Convert `snake_case` to `PascalCase`.
fn snake_to_pascal(s: &str) -> String {
    s.split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => {
                    let upper: String = c.to_uppercase().collect();
                    upper + chars.as_str()
                }
            }
        })
        .collect()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rshome_entity::{DomainSpec, DomainSpecRegistry, ServiceSpec};

    #[test]
    fn domain_id_to_type_name_basic() {
        assert_eq!(domain_id_to_type_name("sensor"), "Sensor");
        assert_eq!(domain_id_to_type_name("binary_sensor"), "BinarySensor");
        assert_eq!(
            domain_id_to_type_name("alarm_control_panel"),
            "AlarmControlPanel"
        );
        assert_eq!(domain_id_to_type_name("text"), "Text");
    }

    #[test]
    fn snake_to_pascal_edge_cases() {
        assert_eq!(snake_to_pascal(""), "");
        assert_eq!(snake_to_pascal("a"), "A");
        assert_eq!(snake_to_pascal("hello_world"), "HelloWorld");
    }

    #[test]
    fn state_fields_sensor() {
        let fields = state_fields_for_domain("sensor");
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0], ("value", "f64"));
        assert_eq!(fields[1], ("unit", "Option<String>"));
    }

    #[test]
    fn state_fields_button_is_empty() {
        assert!(state_fields_for_domain("button").is_empty());
    }

    #[test]
    fn state_fields_unknown_is_empty() {
        assert!(state_fields_for_domain("nonexistent").is_empty());
    }

    #[test]
    fn state_fields_covers_all_17_domains() {
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
            // button is intentionally empty, all others should have fields
            if *d != "button" {
                assert!(
                    !state_fields_for_domain(d).is_empty(),
                    "domain {d} should have state fields"
                );
            }
        }
    }

    #[test]
    fn generate_sdk_sensor_contains_state_struct() {
        let reg = DomainSpecRegistry::built_in();
        let spec = reg.get("sensor").unwrap();
        let sdk = generate_guest_sdk(spec);
        assert!(sdk.contains("pub mod sensor {"));
        assert!(sdk.contains("pub struct SensorState {"));
        assert!(sdk.contains("pub value: f64"));
        assert!(sdk.contains("pub unit: Option<String>"));
    }

    #[test]
    fn generate_sdk_sensor_has_device_classes() {
        let reg = DomainSpecRegistry::built_in();
        let spec = reg.get("sensor").unwrap();
        let sdk = generate_guest_sdk(spec);
        assert!(sdk.contains("pub enum SensorDeviceClass {"));
        assert!(sdk.contains("Temperature,"));
        assert!(sdk.contains("Humidity,"));
    }

    #[test]
    fn generate_sdk_switch_has_services() {
        let reg = DomainSpecRegistry::built_in();
        let spec = reg.get("switch").unwrap();
        let sdk = generate_guest_sdk(spec);
        assert!(sdk.contains("pub mod services {"));
        assert!(sdk.contains("TURN_ON"));
        assert!(sdk.contains("TURN_OFF"));
        assert!(sdk.contains("TOGGLE"));
    }

    #[test]
    fn generate_sdk_button_unit_state() {
        let reg = DomainSpecRegistry::built_in();
        let spec = reg.get("button").unwrap();
        let sdk = generate_guest_sdk(spec);
        assert!(sdk.contains("pub struct ButtonState;"));
    }

    #[test]
    fn generate_sdk_has_features() {
        let reg = DomainSpecRegistry::built_in();
        let spec = reg.get("sensor").unwrap();
        let sdk = generate_guest_sdk(spec);
        assert!(sdk.contains("pub mod features {"));
        assert!(sdk.contains("STATE"));
        assert!(sdk.contains("UNIT"));
    }

    #[test]
    fn generate_all_sdks_has_all_domains() {
        let reg = DomainSpecRegistry::built_in();
        let all = generate_all_sdks(&reg);
        assert!(all.contains("pub mod sensor {"));
        assert!(all.contains("pub mod switch {"));
        assert!(all.contains("pub mod light {"));
        assert!(all.contains("pub mod climate {"));
        assert!(all.contains("pub mod button {"));
    }

    #[test]
    fn generate_sdk_extension_domain() {
        let spec = DomainSpec::extension(
            "irrigation",
            vec!["state".into()],
            vec!["schedule".into()],
            vec!["sprinkler".into(), "drip".into()],
            vec![
                ServiceSpec {
                    name: "start_zone".into(),
                    schema: None,
                },
                ServiceSpec {
                    name: "stop_all".into(),
                    schema: None,
                },
            ],
        );
        let sdk = generate_guest_sdk(&spec);
        assert!(sdk.contains("pub mod irrigation {"));
        assert!(sdk.contains("pub enum IrrigationDeviceClass {"));
        assert!(sdk.contains("Sprinkler,"));
        assert!(sdk.contains("Drip,"));
        assert!(sdk.contains("START_ZONE"));
        assert!(sdk.contains("STOP_ALL"));
    }
}
