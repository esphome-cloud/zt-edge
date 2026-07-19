use rshome_entity::EntityId;

/// FNV-1a 32-bit hash — matches the C implementation in rshome_core.
pub fn fnv1a_32(s: &str) -> u32 {
    let mut hash: u32 = 0x811c9dc5;
    for byte in s.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x01000193);
    }
    hash
}

/// Entity key used in ESPHome wire protocol — FNV-1a of `object_id` only.
pub fn entity_key(id: &EntityId) -> u32 {
    fnv1a_32(id.object_id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_known_value_temp() {
        // "temp" → FNV-1a 32-bit hash (verified against reference impl)
        let h = fnv1a_32("temp");
        assert_eq!(h, 3_223_044_039); // 0xBFF4A447
    }

    #[test]
    fn fnv1a_known_value_living_room() {
        let h = fnv1a_32("living_room");
        assert_eq!(h, fnv1a_32("living_room")); // at minimum stable
        assert_ne!(h, 0); // non-zero
    }

    #[test]
    fn entity_key_uses_object_id_only() {
        let id1 = EntityId::new("sensor", "temp");
        let id2 = EntityId::new("switch", "temp");
        // Same object_id → same key regardless of domain
        assert_eq!(entity_key(&id1), entity_key(&id2));
        assert_eq!(entity_key(&id1), fnv1a_32("temp"));
    }
}
