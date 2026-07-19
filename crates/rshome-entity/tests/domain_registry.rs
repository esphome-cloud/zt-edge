use rshome_entity::{DomainRegistry, EntityCategory, EntityDescriptor, EntityId};

#[test]
fn domain_sensor_required_features() {
    assert!(DomainRegistry::built_in().is_legal("sensor", &["state".to_string()]));
}

#[test]
fn domain_sensor_missing_required_feature() {
    assert!(!DomainRegistry::built_in().is_legal("sensor", &[]));
}

#[test]
fn domain_unknown_wire_type() {
    assert!(DomainRegistry::built_in()
        .resolve_wire_type("unknown_xyz")
        .is_none());
}

#[test]
fn domain_light_optional_features_are_optional() {
    // Light requires "state" and "toggle"; brightness is optional — omitting it is still legal.
    assert!(
        DomainRegistry::built_in().is_legal("light", &["state".to_string(), "toggle".to_string()])
    );
}

#[test]
fn entity_descriptor_roundtrip() {
    let descriptor = EntityDescriptor {
        entity_id: EntityId::new("sensor", "temperature"),
        name: "Temperature".to_string(),
        icon: Some("mdi:thermometer".to_string()),
        device_id: None,
        area_id: None,
        entity_category: EntityCategory::None,
        domain_id: "sensor".to_string(),
        feature_set: vec!["state".to_string(), "unit".to_string()],
        device_class: None,
    };
    let json = serde_json::to_string(&descriptor).unwrap();
    let back: EntityDescriptor = serde_json::from_str(&json).unwrap();
    assert_eq!(back.domain_id, "sensor");
    assert_eq!(back.feature_set, vec!["state", "unit"]);
    assert_eq!(back.entity_id, EntityId::new("sensor", "temperature"));
}

#[test]
fn resolve_wire_type_includes_all_required_features() {
    let (domain, features) = DomainRegistry::built_in()
        .resolve_wire_type("switch")
        .unwrap();
    assert_eq!(domain, "switch");
    assert!(features.contains(&"state".to_string()));
    assert!(features.contains(&"toggle".to_string()));
}

#[test]
fn resolve_wire_type_climate() {
    let (domain, features) = DomainRegistry::built_in()
        .resolve_wire_type("climate")
        .unwrap();
    assert_eq!(domain, "climate");
    assert!(features.contains(&"set_mode".to_string()));
}

#[test]
fn is_legal_with_extra_features_still_legal() {
    // Extra features beyond required are allowed.
    assert!(DomainRegistry::built_in().is_legal(
        "light",
        &[
            "state".to_string(),
            "toggle".to_string(),
            "brightness".to_string()
        ]
    ));
}

#[test]
fn services_for_switch_includes_all_three_toggle_services() {
    let services =
        DomainRegistry::built_in().services_for("switch", &["state".into(), "toggle".into()]);
    assert!(services.contains(&"turn_on".to_string()));
    assert!(services.contains(&"turn_off".to_string()));
    assert!(services.contains(&"toggle".to_string()));
}

#[test]
fn services_for_climate_uses_ha_convention() {
    let services =
        DomainRegistry::built_in().services_for("climate", &["state".into(), "set_mode".into()]);
    assert!(services.contains(&"set_hvac_mode".to_string()));
    assert!(!services.contains(&"set_climate_mode".to_string()));
}

#[test]
fn lock_domain_registered() {
    let (domain, features) = DomainRegistry::built_in()
        .resolve_wire_type("lock")
        .unwrap();
    assert_eq!(domain, "lock");
    let services = DomainRegistry::built_in().services_for(domain, &features);
    assert!(services.contains(&"lock".to_string()));
    assert!(services.contains(&"unlock".to_string()));
}

#[test]
fn services_for_cover_includes_position_commands() {
    let (_, features) = DomainRegistry::built_in()
        .resolve_wire_type("cover")
        .unwrap();
    let services = DomainRegistry::built_in().services_for("cover", &features);
    assert!(services.contains(&"open_cover".to_string()));
    assert!(services.contains(&"close_cover".to_string()));
    assert!(services.contains(&"set_cover_position".to_string()));
}
