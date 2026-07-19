use rshome_schema::ha_export::{HaEntityExportDefinition, HaEntityKind};

pub fn fnv1a_32(value: &str) -> u32 {
    value.bytes().fold(0x811c_9dc5, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    })
}

pub fn generate_ha_adapter_source(
    entities: &[HaEntityExportDefinition],
    device_name: &str,
) -> String {
    let mut source = String::from("#include \"ha_export_adapter.h\"\n#include \"esp_log.h\"\n\n");
    source.push_str("void ha_export_adapter_init(void) {\n");
    source.push_str(&format!(
        "    ESP_LOGI(\"ha_export\", \"Registering {} entities for {}\");\n",
        entities.len(),
        device_name
    ));
    for entity in entities {
        source.push_str(&format!(
            "    /* {}:{} key=0x{:08X} */\n",
            entity_kind_name(entity.kind),
            entity.object_id,
            fnv1a_32(&entity.object_id)
        ));
    }
    source.push_str("}\n");
    source
}

fn entity_kind_name(kind: HaEntityKind) -> &'static str {
    match kind {
        HaEntityKind::BinarySensor => "binary_sensor",
        HaEntityKind::Sensor => "sensor",
        HaEntityKind::Switch => "switch",
        HaEntityKind::Light => "light",
        HaEntityKind::Climate => "climate",
        HaEntityKind::Button => "button",
        HaEntityKind::TextSensor => "text_sensor",
        HaEntityKind::Number => "number",
        HaEntityKind::Select => "select",
        _ => "unknown",
    }
}
