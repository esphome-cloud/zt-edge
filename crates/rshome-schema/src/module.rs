//! Hardware module definitions and registry.
//!
//! A **module** represents a physical board or hardware configuration — it maps
//! to a specific `ChipTarget` and describes the hardware capabilities available.
//! Users pick a module first, then choose a compatible solution.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::platform::{Capability, ChipTarget, DomainKind};

// ── Types ────────────────────────────────────────────────────────────────────

/// Unique identifier for a hardware module.
pub type ModuleId = String;

/// A hardware module descriptor.
///
/// Each module represents a physical board variant with a fixed chip target and
/// a set of hardware capabilities.  The `compatible_solutions` field lists which
/// solutions can run on this hardware.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModuleDefinition {
    /// Unique identifier, e.g. `"esp32s3_wroom1"`.
    pub id: ModuleId,
    /// Human-readable label for the wizard UI.
    pub label: String,
    /// Optional Chinese (zh-CN) translation of `label`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_zh: Option<String>,
    /// Longer description of the hardware.
    pub description: String,
    /// Optional Chinese (zh-CN) translation of `description`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_zh: Option<String>,
    /// Target chip for this module.
    pub target: ChipTarget,
    /// Hardware capabilities present on this board.
    pub hardware_caps: Vec<Capability>,
    /// Hardware constraints or notes (e.g. "GPIO 0 used for boot button").
    #[serde(default)]
    pub constraints: Vec<String>,
    /// Solution IDs that are compatible with this module.
    #[serde(default)]
    pub compatible_solutions: Vec<String>,
    /// Target domain for wizard scoping. `None` means visible in all domains.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<DomainKind>,
}

// ── Registry ─────────────────────────────────────────────────────────────────

/// Registry of all known hardware modules.
#[derive(Debug, Clone, Default)]
pub struct ModuleRegistry {
    modules: BTreeMap<ModuleId, ModuleDefinition>,
}

impl ModuleRegistry {
    /// Create an empty module registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a module definition.
    pub fn register(&mut self, def: ModuleDefinition) {
        self.modules.insert(def.id.clone(), def);
    }

    /// Look up a module by ID.
    pub fn get(&self, id: &str) -> Option<&ModuleDefinition> {
        self.modules.get(id)
    }

    /// Iterate over all registered modules.
    pub fn all(&self) -> impl Iterator<Item = &ModuleDefinition> {
        self.modules.values()
    }

    /// Return all modules targeting the given chip.
    pub fn for_target(&self, target: ChipTarget) -> Vec<&ModuleDefinition> {
        self.modules
            .values()
            .filter(|m| m.target == target)
            .collect()
    }
}

// ── Default registry ─────────────────────────────────────────────────────────

/// Build a pre-populated module registry with real Espressif development boards.
///
/// Each module corresponds to an official Espressif module from
/// <https://www.espressif.com/en/products/modules>.
/// The `hardware_caps` reflect the SoC's peripheral resources.
pub fn default_module_registry() -> ModuleRegistry {
    let mut r = ModuleRegistry::new();

    // ── ESP32-S3 Modules ──────────────────────────────────────────────────
    // Datasheet: ESP32-S3 TRM — 3 UART, 4 SPI, 2 I2C, 2 I2S, 2 ADC (20ch),
    // 8 LEDC, 2 MCPWM, 14 Touch, USB OTG, Camera DVP, 4 RMT, 45 GPIO

    r.register(ModuleDefinition {
        id: "esp32s3_wroom1".into(),
        label: "ESP32-S3-WROOM-1".into(),
        label_zh: Some("ESP32-S3-WROOM-1 模组".into()),
        description: "ESP32-S3 module with Wi-Fi 802.11 b/g/n, BLE 5 LE. Flash 4/8/16MB, PSRAM 2/8MB. PCB antenna. 18×25.5mm.".into(),
        description_zh: Some("ESP32-S3 模组，Wi-Fi 802.11 b/g/n、BLE 5 LE。Flash 4/8/16MB，PSRAM 2/8MB。PCB 天线。18×25.5mm。".into()),
        target: ChipTarget::Esp32S3,
        hardware_caps: vec![
            Capability::DualCoreCpu, Capability::Wifi, Capability::Ble,
            Capability::Gpio, Capability::I2c, Capability::Spi, Capability::Uart,
            Capability::I2s, Capability::Adc, Capability::Ledc, Capability::Mcpwm,
            Capability::Touch, Capability::UsbOtg, Capability::Camera,
            Capability::Psram, Capability::Rmt,
            // Vehicle / gateway capabilities available via GPIO wiring
            Capability::MotorControl, Capability::Imu, Capability::FailsafeStop,
            Capability::ApSta, Capability::Bridge, Capability::LongRange, Capability::Csi,
            Capability::EspNow, Capability::MeshLite, Capability::AudioI2s,
        ],
        constraints: vec![
            "GPIO 26-32 used by flash (Quad SPI)".into(),
            "GPIO 33-37 used by PSRAM (Octal SPI) when 8MB PSRAM variant".into(),
        ],
        compatible_solutions: vec![],  // Solutions reference modules, not the other way
        domain: None,
    });

    r.register(ModuleDefinition {
        id: "esp32s3_mini1".into(),
        label: "ESP32-S3-MINI-1".into(),
        label_zh: Some("ESP32-S3-MINI-1 模组".into()),
        description: "Compact ESP32-S3 module. Wi-Fi + BLE 5 LE. 8MB embedded flash, 2MB PSRAM. PCB antenna. 15.4×20.5mm.".into(),
        description_zh: Some("紧凑型 ESP32-S3 模组。Wi-Fi + BLE 5 LE。8MB 内置 Flash，2MB PSRAM。PCB 天线。15.4×20.5mm。".into()),
        target: ChipTarget::Esp32S3,
        hardware_caps: vec![
            Capability::DualCoreCpu, Capability::Wifi, Capability::Ble,
            Capability::Gpio, Capability::I2c, Capability::Spi, Capability::Uart,
            Capability::I2s, Capability::Adc, Capability::Ledc, Capability::Mcpwm,
            Capability::Touch, Capability::UsbOtg, Capability::Camera,
            Capability::Psram, Capability::Rmt,
            Capability::MotorControl, Capability::Imu, Capability::FailsafeStop,
            Capability::ApSta, Capability::Bridge, Capability::LongRange, Capability::Csi,
            Capability::EspNow, Capability::MeshLite, Capability::AudioI2s,
        ],
        constraints: vec![
            "Fewer exposed GPIOs than WROOM-1 due to smaller package".into(),
            "8MB flash + 2MB PSRAM (fixed, not configurable)".into(),
        ],
        compatible_solutions: vec![],
        domain: None,
    });

    // ── ESP32-C6 Modules ──────────────────────────────────────────────────
    // Datasheet: ESP32-C6 TRM — 2 UART + LP_UART, 1 SPI, 1 I2C + LP_I2C,
    // 1 I2S, 1 ADC (7ch), 6 LEDC, 2 TWAI, USB Serial/JTAG, 4 RMT, 31 GPIO

    r.register(ModuleDefinition {
        id: "esp32c6_wroom1".into(),
        label: "ESP32-C6-WROOM-1".into(),
        label_zh: Some("ESP32-C6-WROOM-1 模组".into()),
        description: "ESP32-C6 module with Wi-Fi 6 (802.11ax), BLE 5, Zigbee 3.0, Thread. Flash 4/8/16MB. No PSRAM. 18×25.5mm.".into(),
        description_zh: Some("ESP32-C6 模组，Wi-Fi 6 (802.11ax)、BLE 5、Zigbee 3.0、Thread。Flash 4/8/16MB。无 PSRAM。18×25.5mm。".into()),
        target: ChipTarget::Esp32C6,
        hardware_caps: vec![
            Capability::SingleCoreCpu, Capability::Wifi, Capability::Ble,
            Capability::Thread, Capability::Zigbee,
            Capability::Gpio, Capability::I2c, Capability::Spi, Capability::Uart,
            Capability::I2s, Capability::Adc, Capability::Ledc, Capability::Rmt,
            Capability::EspNow, Capability::MeshLite,
        ],
        constraints: vec![
            "No PSRAM".into(),
            "No MCPWM (use LEDC for PWM)".into(),
            "No camera DVP interface".into(),
        ],
        compatible_solutions: vec![],
        domain: None,
    });

    r.register(ModuleDefinition {
        id: "esp32c6_mini1".into(),
        label: "ESP32-C6-MINI-1".into(),
        label_zh: Some("ESP32-C6-MINI-1 模组".into()),
        description: "Compact ESP32-C6 module. Wi-Fi 6, BLE 5, Zigbee, Thread. 4/8MB embedded flash. 13.2×16.6mm.".into(),
        description_zh: Some("紧凑型 ESP32-C6 模组。Wi-Fi 6、BLE 5、Zigbee、Thread。4/8MB 内置 Flash。13.2×16.6mm。".into()),
        target: ChipTarget::Esp32C6,
        hardware_caps: vec![
            Capability::SingleCoreCpu, Capability::Wifi, Capability::Ble,
            Capability::Thread, Capability::Zigbee,
            Capability::Gpio, Capability::I2c, Capability::Spi, Capability::Uart,
            Capability::I2s, Capability::Adc, Capability::Ledc, Capability::Rmt,
            Capability::EspNow, Capability::MeshLite,
        ],
        constraints: vec![
            "Fewer exposed GPIOs than WROOM-1".into(),
            "No PSRAM".into(),
        ],
        compatible_solutions: vec![],
        domain: None,
    });

    // ── ESP32 Modules ────────────────────────────────────────────────────
    // Datasheet: ESP32 TRM — 3 UART, 4 SPI, 2 I2C, 2 I2S, 2 ADC (18ch),
    // 8 LEDC, 2 MCPWM, 10 Touch, 1 TWAI, 4 RMT, 34 GPIO

    r.register(ModuleDefinition {
        id: "esp32_d0wd_v3".into(),
        label: "ESP32-D0WD-V3".into(),
        label_zh: Some("ESP32-D0WD-V3 模组".into()),
        description: "ESP32 dual-core module. Wi-Fi 802.11 b/g/n, BLE 4.2. 8MB flash. TWAI (CAN), 3 UART. Integrated CAN + RS485 transceiver board.".into(),
        description_zh: Some("ESP32 双核模组。Wi-Fi 802.11 b/g/n、BLE 4.2。8MB Flash。TWAI (CAN)、3 UART。集成 CAN + RS485 收发器开发板。".into()),
        target: ChipTarget::Esp32,
        hardware_caps: vec![
            Capability::DualCoreCpu, Capability::Wifi, Capability::Ble,
            Capability::Gpio, Capability::I2c, Capability::Spi, Capability::Uart,
            Capability::I2s, Capability::Adc, Capability::Ledc, Capability::Mcpwm,
            Capability::Touch, Capability::Rmt,
            Capability::CanBus, Capability::Rs485,
            Capability::EspNow,
        ],
        constraints: vec![
            "GPIO 6-11 used by internal flash (do not use)".into(),
            "GPIO 34-39 are input-only".into(),
            "No USB-OTG (uses CH343P USB-UART bridge)".into(),
            "No PSRAM on D0WD-V3 variant".into(),
        ],
        compatible_solutions: vec![],
        domain: None,
    });

    r
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_definition_serde_roundtrip() {
        let def = ModuleDefinition {
            id: "test_module".into(),
            label: "Test Module".into(),
            label_zh: None,
            description: "A test module.".into(),
            description_zh: None,
            target: ChipTarget::Esp32S3,
            hardware_caps: vec![Capability::Wifi, Capability::Ble, Capability::Gpio],
            constraints: vec!["GPIO 0 reserved".into()],
            compatible_solutions: vec!["sol_a".into(), "sol_b".into()],
            domain: None,
        };
        let json = serde_json::to_string(&def).unwrap();
        let back: ModuleDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(def, back);
    }

    #[test]
    fn registry_crud() {
        let mut reg = ModuleRegistry::new();
        assert!(reg.get("foo").is_none());

        reg.register(ModuleDefinition {
            id: "foo".into(),
            label: "Foo".into(),
            label_zh: None,
            description: String::new(),
            description_zh: None,
            target: ChipTarget::Esp32,
            hardware_caps: vec![],
            constraints: vec![],
            compatible_solutions: vec![],
            domain: None,
        });

        assert!(reg.get("foo").is_some());
        assert_eq!(reg.all().count(), 1);
    }

    #[test]
    fn for_target_filters_correctly() {
        let reg = default_module_registry();
        let s3 = reg.for_target(ChipTarget::Esp32S3);
        assert_eq!(s3.len(), 2); // wroom1 + mini1
        let c6 = reg.for_target(ChipTarget::Esp32C6);
        assert_eq!(c6.len(), 2); // wroom1 + mini1
        let esp32 = reg.for_target(ChipTarget::Esp32);
        assert_eq!(esp32.len(), 1); // d0wd_v3
    }

    #[test]
    fn default_registry_module_count() {
        let reg = default_module_registry();
        assert_eq!(reg.all().count(), 5); // 2 S3 + 2 C6 + 1 ESP32
    }

    #[test]
    fn default_modules_have_capabilities() {
        let reg = default_module_registry();
        for m in reg.all() {
            assert!(!m.hardware_caps.is_empty(), "module '{}' has no caps", m.id);
        }
    }

    #[test]
    fn get_missing_returns_none() {
        let reg = default_module_registry();
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn real_modules_registered() {
        let reg = default_module_registry();
        assert!(reg.get("esp32s3_wroom1").is_some(), "S3-WROOM-1 missing");
        assert!(reg.get("esp32s3_mini1").is_some(), "S3-MINI-1 missing");
        assert!(reg.get("esp32c6_wroom1").is_some(), "C6-WROOM-1 missing");
        assert!(reg.get("esp32c6_mini1").is_some(), "C6-MINI-1 missing");
        assert!(reg.get("esp32_d0wd_v3").is_some(), "ESP32-D0WD-V3 missing");
    }

    #[test]
    fn s3_wroom1_has_full_peripheral_set() {
        let reg = default_module_registry();
        let m = reg.get("esp32s3_wroom1").unwrap();
        assert!(m.hardware_caps.contains(&Capability::Mcpwm));
        assert!(m.hardware_caps.contains(&Capability::Camera));
        assert!(m.hardware_caps.contains(&Capability::UsbOtg));
        assert!(m.hardware_caps.contains(&Capability::Touch));
        assert!(m.hardware_caps.contains(&Capability::Psram));
        assert!(m.hardware_caps.contains(&Capability::Rmt));
    }

    #[test]
    fn c6_wroom1_has_wifi6_and_thread() {
        let reg = default_module_registry();
        let m = reg.get("esp32c6_wroom1").unwrap();
        assert!(m.hardware_caps.contains(&Capability::Wifi));
        assert!(m.hardware_caps.contains(&Capability::Thread));
        assert!(m.hardware_caps.contains(&Capability::Zigbee));
        assert!(!m.hardware_caps.contains(&Capability::Mcpwm)); // C6 has no MCPWM
        assert!(!m.hardware_caps.contains(&Capability::Camera)); // C6 has no camera
    }

    #[test]
    fn all_modules_domain_agnostic() {
        let reg = default_module_registry();
        for m in reg.all() {
            assert!(
                m.domain.is_none(),
                "module '{}' should have domain: None",
                m.id
            );
        }
    }

    #[test]
    fn esp32_d0wd_v3_has_can_rs485() {
        let reg = default_module_registry();
        let m = reg.get("esp32_d0wd_v3").unwrap();
        assert!(m.hardware_caps.contains(&Capability::CanBus));
        assert!(m.hardware_caps.contains(&Capability::Rs485));
        assert!(!m.hardware_caps.contains(&Capability::UsbOtg));
        assert!(!m.hardware_caps.contains(&Capability::Camera));
        assert!(!m.hardware_caps.contains(&Capability::Psram));
    }
}
