use rshome_entity::{DomainRegistry, EntityCommand};
use serde_json::Value;

/// Map (domain, service, data) to an EntityCommand, if applicable.
pub fn command_for_service(domain: &str, service: &str, data: &Value) -> Option<EntityCommand> {
    // Try the domain registry first
    if let Some(def) = DomainRegistry::built_in().get(domain) {
        return def.encode_command(service, data);
    }

    // HA input_* aliases → canonical domain lookup
    let canonical = match domain {
        "input_number" => "number",
        "input_select" => "select",
        "input_text" => "text",
        _ => {
            return match service {
                "turn_on" => Some(EntityCommand::TurnOn),
                "turn_off" => Some(EntityCommand::TurnOff),
                "toggle" => Some(EntityCommand::Toggle),
                _ => None,
            }
        }
    };
    DomainRegistry::built_in()
        .get(canonical)?
        .encode_command(service, data)
}

/// All registered built-in service (domain, service) pairs.
///
/// Derived from `DomainRegistry` so that domain semantics live in one place.
pub fn builtin_services() -> Vec<(String, String)> {
    let reg = DomainRegistry::built_in();
    reg.all_domains()
        .flat_map(|def| {
            let domain = def.id();
            let (_, features) = reg.resolve_wire_type(domain).unwrap_or((domain, vec![]));
            reg.services_for(domain, &features)
                .into_iter()
                .map(move |svc| (domain.to_string(), svc))
        })
        .collect()
}
