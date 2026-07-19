//! Hardware assembly definitions and registry.
//!
//! A **hardware assembly** maps to what `esp_board_manager` calls a "board definition" —
//! the combination of peripherals and devices that a board provides.  It bridges
//! [`ModuleDefinition`](crate::module::ModuleDefinition) selections to the YAML files
//! consumed by `idf.py gen-bmgr-config`.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::module::{ModuleDefinition, ModuleId};
use crate::platform::{Capability, ChipTarget};

// ── Types ────────────────────────────────────────────────────────────────────

/// Unique identifier for a hardware assembly.
pub type AssemblyId = String;

/// A peripheral declared in `board_peripherals.yaml`.
///
/// Maps directly to an `esp_board_manager` peripheral entry — type/role/config
/// are passed through to the Python generator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PeripheralDeclaration {
    /// Instance name, e.g. `"gpio_test_output"`, `"i2c_master_0"`.
    pub name: String,
    /// Peripheral type as defined by `esp_board_manager` parsers
    /// (e.g. `"gpio"`, `"i2c"`, `"spi"`, `"uart"`).
    pub periph_type: String,
    /// Role within the type (e.g. `"output"`, `"input"`, `"master"`).
    pub role: String,
    /// Type-specific configuration passed through to YAML.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub config: BTreeMap<String, serde_json::Value>,
}

/// A device declared in `board_devices.yaml`.
///
/// Maps directly to an `esp_board_manager` device entry — references peripherals
/// by name and may carry its own device-type-specific configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeviceDeclaration {
    /// Instance name, e.g. `"gpio_ctrl_0"`, `"audio_dac"`.
    pub name: String,
    /// Device type matching an `esp_board_manager` device parser
    /// (e.g. `"gpio_ctrl"`, `"display_lcd"`, `"audio_dac"`).
    pub device_type: String,
    /// Names of peripherals this device uses (must reference `PeripheralDeclaration.name`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub peripherals: Vec<String>,
    /// Device-specific configuration passed through to YAML.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub config: BTreeMap<String, serde_json::Value>,
    /// Additional IDF component dependencies this device requires.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
}

/// Board-level assembly combining one or more modules into a deployable hardware config.
///
/// The assembly is the bridge between wizard module selection and `esp_board_manager`
/// YAML generation.  Each assembly declares the peripherals and devices that the
/// board provides, and the HAL contracts it can satisfy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HardwareAssemblyDefinition {
    /// Unique identifier, e.g. `"esp32s3_gpio_relay_assembly"`.
    pub id: AssemblyId,
    /// Human-readable label for the wizard UI.
    pub label: String,
    /// Longer description of the assembly.
    pub description: String,
    /// Target chip for this assembly.
    pub target: ChipTarget,
    /// Module IDs that compose this assembly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<ModuleId>,
    /// Peripheral declarations for `board_peripherals.yaml`.
    pub peripherals: Vec<PeripheralDeclaration>,
    /// Device declarations for `board_devices.yaml`.
    pub devices: Vec<DeviceDeclaration>,
    /// Derived HAL contracts this assembly can provide
    /// (e.g. `"gpio_output"`, `"audio_playback"`, `"display_panel"`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provided_hal_contracts: Vec<String>,
}

// ── Registry ────────────────────────────────────────────────────────────────

/// Registry of all known hardware assemblies.
#[derive(Debug, Clone, Default)]
pub struct AssemblyRegistry {
    assemblies: BTreeMap<AssemblyId, HardwareAssemblyDefinition>,
}

impl AssemblyRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, def: HardwareAssemblyDefinition) {
        self.assemblies.insert(def.id.clone(), def);
    }

    pub fn get(&self, id: &str) -> Option<&HardwareAssemblyDefinition> {
        self.assemblies.get(id)
    }

    pub fn all(&self) -> impl Iterator<Item = &HardwareAssemblyDefinition> {
        self.assemblies.values()
    }

    /// Return all assemblies targeting the given chip.
    pub fn for_target(&self, target: ChipTarget) -> Vec<&HardwareAssemblyDefinition> {
        self.assemblies
            .values()
            .filter(|a| a.target == target)
            .collect()
    }
}

// ── Auto-derivation from module ────────────────────────────────────────────

/// Extract GPIO pin numbers from a constraint string.
///
/// Handles patterns like:
/// - `"FailsafeStop relay on GPIO 25"` → `[25]`
/// - `"USB OTG uses GPIO 19/20"` → `[19, 20]`
/// - `"Camera uses GPIO 4-15"` → `[4, 15]` (range endpoints)
/// - `"IMU on I2C bus 0 (GPIO 21/22)"` → `[21, 22]`
/// - `"MCPWM uses GPIO 16-19 for motor drivers"` → `[16, 19]`
fn extract_gpio_pins(constraint: &str) -> Vec<u32> {
    // Find "GPIO " followed by digits, optionally separated by / or -
    let mut pins = Vec::new();
    if let Some(idx) = constraint.find("GPIO ") {
        let after = &constraint[idx + 5..];
        // Collect the digit sequence (may contain / or - separators)
        let token: String = after
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '/' || *c == '-')
            .collect();
        for part in token.split(['/', '-']) {
            if let Ok(n) = part.parse::<u32>() {
                pins.push(n);
            }
        }
    }
    pins
}

/// Map a `Capability` to a default peripheral declaration, if applicable.
///
/// Returns `None` for capabilities that don't map to hardware peripherals
/// (e.g. WiFi, BLE, CPU type, PSRAM).
fn capability_to_peripheral(cap: &Capability) -> Option<(String, String, String)> {
    // Returns (periph_type, role, name_prefix)
    match cap {
        Capability::Gpio => Some(("gpio".into(), "output".into(), "gpio_0".into())),
        Capability::I2c => Some(("i2c".into(), "master".into(), "i2c_master_0".into())),
        Capability::Spi => Some(("spi".into(), "master".into(), "spi_master_0".into())),
        Capability::Uart => Some(("uart".into(), "bidirectional".into(), "uart_0".into())),
        Capability::Adc => Some(("adc".into(), "input".into(), "adc_0".into())),
        Capability::Camera => Some(("camera".into(), "data_bus".into(), "camera_0".into())),
        Capability::Mcpwm => Some(("mcpwm".into(), "motor_driver".into(), "mcpwm_0".into())),
        Capability::Ledc => Some(("ledc".into(), "pwm".into(), "ledc_0".into())),
        Capability::I2s => Some(("i2s".into(), "master".into(), "i2s_0".into())),
        Capability::Imu => Some(("imu".into(), "sensor".into(), "imu_0".into())),
        Capability::FailsafeStop => {
            Some(("failsafe".into(), "controller".into(), "failsafe_0".into()))
        }
        Capability::MotorControl => Some((
            "vehicle_control".into(),
            "controller".into(),
            "vehicle_ctrl_0".into(),
        )),
        Capability::ApSta => Some(("gateway_bridge".into(), "bridge".into(), "gateway_0".into())),
        Capability::LongRange => Some(("lr_comm".into(), "link".into(), "lr_link_0".into())),
        Capability::Csi => Some(("csi".into(), "sensor".into(), "csi_0".into())),
        _ => None,
    }
}

/// Map a peripheral type to its default device type.
fn periph_to_device_type(periph_type: &str) -> &str {
    match periph_type {
        "gpio" => "gpio_ctrl",
        "i2c" => "i2c_sensor",
        "spi" => "spi_device",
        "uart" => "uart_device",
        "adc" => "adc_reader",
        "camera" => "camera_capture",
        "mcpwm" => "motor_driver",
        "ledc" => "ledc_pwm",
        "i2s" => "i2s_device",
        "imu" => "imu_sensor",
        "failsafe" => "failsafe_ctrl",
        "vehicle_control" => "vehicle_ctrl",
        "gateway_bridge" => "gateway_bridge",
        "lr_comm" => "lr_link",
        "csi" => "csi_sensor",
        _ => "generic_device",
    }
}

/// Map a `Capability` to an HAL contract string, if applicable.
fn capability_to_hal_contract(cap: &Capability) -> Option<&'static str> {
    match cap {
        Capability::Gpio => Some("gpio_output"),
        Capability::I2c => Some("i2c_sensor"),
        Capability::Spi => Some("spi_device"),
        Capability::Uart => Some("uart_port"),
        Capability::Adc => Some("adc_input"),
        Capability::Camera => Some("camera_capture"),
        Capability::MotorControl => Some("motor_control"),
        Capability::Imu => Some("imu_sensor"),
        Capability::FailsafeStop => Some("failsafe_stop"),
        Capability::Ledc => Some("ledc_pwm"),
        _ => None,
    }
}

impl HardwareAssemblyDefinition {
    /// Auto-derive an assembly from a module's capabilities and constraints.
    ///
    /// Maps each hardware capability to a default peripheral + device pair,
    /// then enriches peripheral config from constraint strings that mention
    /// specific GPIO pins.
    pub fn from_module(module: &ModuleDefinition) -> Self {
        let mut peripherals = Vec::new();
        let mut seen_types = std::collections::HashSet::new();

        // 1. Create peripherals from capabilities
        for cap in &module.hardware_caps {
            if let Some((periph_type, role, name)) = capability_to_peripheral(cap) {
                if seen_types.insert(periph_type.clone()) {
                    let mut config = BTreeMap::new();
                    // Add default config for specific types
                    match periph_type.as_str() {
                        "i2c" => {
                            config.insert("freq_hz".into(), serde_json::json!(400_000));
                        }
                        "uart" => {
                            config.insert("baudrate".into(), serde_json::json!(115200));
                        }
                        _ => {}
                    }
                    peripherals.push(PeripheralDeclaration {
                        name,
                        periph_type,
                        role,
                        config,
                    });
                }
            }
        }

        // 2. Enrich peripheral config from constraints
        for constraint in &module.constraints {
            let pins = extract_gpio_pins(constraint);
            if pins.is_empty() {
                continue;
            }

            let lower = constraint.to_lowercase();
            if lower.contains("i2c") && pins.len() >= 2 {
                if let Some(p) = peripherals.iter_mut().find(|p| p.periph_type == "i2c") {
                    p.config.insert("sda".into(), serde_json::json!(pins[0]));
                    p.config.insert("scl".into(), serde_json::json!(pins[1]));
                }
            } else if lower.contains("mcpwm") || lower.contains("motor") {
                if let Some(p) = peripherals.iter_mut().find(|p| p.periph_type == "mcpwm") {
                    if pins.len() >= 2 {
                        p.config
                            .insert("pin_start".into(), serde_json::json!(pins[0]));
                        p.config
                            .insert("pin_end".into(), serde_json::json!(pins[1]));
                    }
                }
            } else if lower.contains("camera") {
                if let Some(p) = peripherals.iter_mut().find(|p| p.periph_type == "camera") {
                    if pins.len() >= 2 {
                        p.config
                            .insert("pin_start".into(), serde_json::json!(pins[0]));
                        p.config
                            .insert("pin_end".into(), serde_json::json!(pins[1]));
                    }
                }
            } else if lower.contains("usb") {
                // USB OTG pins — informational, don't map to a peripheral
            } else if lower.contains("failsafe") || lower.contains("relay") {
                // FailsafeStop on specific GPIO — enrich gpio peripheral
                if let Some(p) = peripherals.iter_mut().find(|p| p.periph_type == "gpio") {
                    if let Some(&pin) = pins.first() {
                        p.config.insert("pin".into(), serde_json::json!(pin));
                    }
                }
            }
        }

        // 3. Create devices from peripherals
        let devices: Vec<DeviceDeclaration> = peripherals
            .iter()
            .map(|p| {
                let device_type = periph_to_device_type(&p.periph_type).to_string();
                let name = format!("{}_0", device_type);
                DeviceDeclaration {
                    name,
                    device_type,
                    peripherals: vec![p.name.clone()],
                    config: BTreeMap::from([("enabled".into(), serde_json::json!(true))]),
                    dependencies: vec![],
                }
            })
            .collect();

        // 4. Derive HAL contracts
        let provided_hal_contracts: Vec<String> = module
            .hardware_caps
            .iter()
            .filter_map(capability_to_hal_contract)
            .map(String::from)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();

        HardwareAssemblyDefinition {
            id: format!("{}_assembly", module.id),
            label: format!("{} Assembly", module.label),
            description: module.description.clone(),
            target: module.target,
            modules: vec![module.id.clone()],
            peripherals,
            devices,
            provided_hal_contracts,
        }
    }
}

// ── Default registry ────────────────────────────────────────────────────────

/// Build a pre-populated assembly registry with known board assemblies.
pub fn default_assembly_registry() -> AssemblyRegistry {
    let mut r = AssemblyRegistry::new();

    // ── GPIO relay assembly ─────────────────────────────────────────────
    r.register(HardwareAssemblyDefinition {
        id: "esp32s3_gpio_relay_assembly".into(),
        label: "ESP32-S3 GPIO Relay Assembly".into(),
        description: "Single-channel GPIO output for relay/LED control.".into(),
        target: ChipTarget::Esp32S3,
        modules: vec!["esp32s3_wroom1".into()],
        peripherals: vec![PeripheralDeclaration {
            name: "gpio_relay_output".into(),
            periph_type: "gpio".into(),
            // esp_board_manager uses `io` for GPIO peripherals (the
            // `esp_board_periph_role_t` enum has NONE/MASTER/SLAVE/IO/TX/
            // RX/CONTINUOUS/ONESHOT/COSINE — no OUTPUT variant). The
            // mode field below carries the input/output direction.
            role: "io".into(),
            config: BTreeMap::from([
                ("pin".into(), serde_json::json!(6)),
                ("mode".into(), serde_json::json!("GPIO_MODE_OUTPUT")),
                ("default_level".into(), serde_json::json!(0)),
            ]),
        }],
        devices: vec![DeviceDeclaration {
            name: "gpio_ctrl_0".into(),
            device_type: "gpio_ctrl".into(),
            peripherals: vec!["gpio_relay_output".into()],
            config: BTreeMap::from([
                ("enabled".into(), serde_json::json!(true)),
                ("active_level".into(), serde_json::json!(1)),
            ]),
            dependencies: vec![],
        }],
        provided_hal_contracts: vec!["gpio_output".into()],
    });

    // ── I2C sensor assembly ─────────────────────────────────────────────
    r.register(HardwareAssemblyDefinition {
        id: "esp32s3_i2c_sensor_assembly".into(),
        label: "ESP32-S3 I2C Sensor Assembly".into(),
        description: "I2C bus with BME280 environmental sensor.".into(),
        target: ChipTarget::Esp32S3,
        modules: vec!["esp32s3_wroom1".into()],
        peripherals: vec![PeripheralDeclaration {
            name: "i2c_master_0".into(),
            periph_type: "i2c".into(),
            role: "master".into(),
            config: BTreeMap::from([
                ("sda".into(), serde_json::json!(8)),
                ("scl".into(), serde_json::json!(9)),
                ("freq_hz".into(), serde_json::json!(400_000)),
            ]),
        }],
        devices: vec![DeviceDeclaration {
            name: "bme280_0".into(),
            device_type: "i2c_sensor".into(),
            peripherals: vec!["i2c_master_0".into()],
            config: BTreeMap::from([
                ("address".into(), serde_json::json!("0x76")),
                ("sensor_type".into(), serde_json::json!("bme280")),
            ]),
            dependencies: vec![],
        }],
        provided_hal_contracts: vec!["i2c_sensor".into()],
    });

    // ── CAN485 DevBoard assembly ─────────────────────────────────────────
    r.register(HardwareAssemblyDefinition {
        id: "esp32_can485_devboard".into(),
        label: "WeActStudio CAN485DevBoard V1".into(),
        description: "ESP32 board with isolated CAN (CA-IS2062A) and RS485 (CA-IS2092A) transceivers, MicroSD, WS2812 LED, VIN monitoring. 5-36V input.".into(),
        target: ChipTarget::Esp32,
        modules: vec!["esp32_d0wd_v3".into()],
        peripherals: vec![
            PeripheralDeclaration {
                name: "twai_0".into(),
                periph_type: "twai".into(),
                role: "controller".into(),
                config: BTreeMap::from([
                    ("rx_pin".into(), serde_json::json!(26)),
                    ("tx_pin".into(), serde_json::json!(27)),
                ]),
            },
            PeripheralDeclaration {
                name: "uart1_rs485".into(),
                periph_type: "uart".into(),
                role: "rs485_half_duplex".into(),
                config: BTreeMap::from([
                    ("rx_pin".into(), serde_json::json!(21)),
                    ("tx_pin".into(), serde_json::json!(22)),
                    ("de_pin".into(), serde_json::json!(17)),
                ]),
            },
            PeripheralDeclaration {
                name: "spi_sd".into(),
                periph_type: "spi".into(),
                role: "master".into(),
                config: BTreeMap::from([
                    ("cs_pin".into(), serde_json::json!(13)),
                    ("sck_pin".into(), serde_json::json!(14)),
                    ("mosi_pin".into(), serde_json::json!(15)),
                    ("miso_pin".into(), serde_json::json!(2)),
                ]),
            },
            PeripheralDeclaration {
                name: "adc_vin".into(),
                periph_type: "adc".into(),
                role: "input".into(),
                config: BTreeMap::from([
                    ("pin".into(), serde_json::json!(36)),
                    ("channel".into(), serde_json::json!("ADC1_CH0")),
                ]),
            },
            PeripheralDeclaration {
                name: "rmt_led".into(),
                periph_type: "rmt".into(),
                role: "output".into(),
                config: BTreeMap::from([
                    ("pin".into(), serde_json::json!(4)),
                ]),
            },
            PeripheralDeclaration {
                name: "gpio_key".into(),
                periph_type: "gpio".into(),
                role: "input".into(),
                config: BTreeMap::from([
                    ("pin".into(), serde_json::json!(0)),
                    ("mode".into(), serde_json::json!("GPIO_MODE_INPUT")),
                ]),
            },
        ],
        devices: vec![
            DeviceDeclaration {
                name: "can_0".into(),
                device_type: "can_transceiver".into(),
                peripherals: vec!["twai_0".into()],
                config: BTreeMap::from([
                    ("baud_rate".into(), serde_json::json!(500_000)),
                    ("mode".into(), serde_json::json!("listen_only")),
                    ("transceiver".into(), serde_json::json!("CA-IS2062A")),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "rs485_0".into(),
                device_type: "rs485_transceiver".into(),
                peripherals: vec!["uart1_rs485".into()],
                config: BTreeMap::from([
                    ("baud_rate".into(), serde_json::json!(115_200)),
                    ("data_bits".into(), serde_json::json!(8)),
                    ("parity".into(), serde_json::json!("none")),
                    ("stop_bits".into(), serde_json::json!(1)),
                    ("transceiver".into(), serde_json::json!("CA-IS2092A")),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "sd_0".into(),
                device_type: "sd_logger".into(),
                peripherals: vec!["spi_sd".into()],
                config: BTreeMap::from([
                    ("mount_point".into(), serde_json::json!("/sd")),
                    ("max_file_size_mb".into(), serde_json::json!(10)),
                    ("flush_interval_s".into(), serde_json::json!(5)),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "led_0".into(),
                device_type: "ws2812_led".into(),
                peripherals: vec!["rmt_led".into()],
                config: BTreeMap::from([
                    ("led_count".into(), serde_json::json!(1)),
                ]),
                dependencies: vec!["led_strip".into()],
            },
            DeviceDeclaration {
                name: "vin_0".into(),
                device_type: "vin_monitor".into(),
                peripherals: vec!["adc_vin".into()],
                config: BTreeMap::from([
                    ("divider_ratio".into(), serde_json::json!(12.0)),
                    ("low_voltage_threshold_mv".into(), serde_json::json!(6000)),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "key_0".into(),
                device_type: "boot_key".into(),
                peripherals: vec!["gpio_key".into()],
                config: BTreeMap::from([
                    ("long_press_ms".into(), serde_json::json!(1000)),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "wifi_tel_0".into(),
                device_type: "wifi_telemetry".into(),
                peripherals: vec![],
                config: BTreeMap::from([
                    ("port".into(), serde_json::json!(80)),
                    ("path".into(), serde_json::json!("/status")),
                ]),
                dependencies: vec![],
            },
        ],
        provided_hal_contracts: vec![
            "can_bus".into(),
            "rs485_bus".into(),
            "sd_card".into(),
            "status_led".into(),
            "voltage_monitor".into(),
            "boot_key".into(),
            "wifi_telemetry".into(),
        ],
    });

    // ── V&A Wheeled-Diff Dev Assembly ─────────────────────────────────────
    // First physical-verification target for the V&A solution catalog.
    // Pin layout targets a bare ESP32-S3-DevKitC-1 (no shields attached):
    //   MCPWM L/R on GPIO 4/5            — motor PWM output is observable
    //                                       on logic analyser even with no
    //                                       motor attached.
    //   I²C SDA/SCL on GPIO 8/9          — IMU probe at 0x68; NACKs benign,
    //                                       backend logs each transaction.
    //   SPI on GPIO 10-13 (CS/MOSI/SCK/MISO) — aux bus; exercises bus init.
    //   Status LED on GPIO 2, Failsafe on GPIO 21 — plain pulled-down GPIOs.
    // All pins are safe on ESP32-S3-DevKitC-1: no strap pins, no flash/PSRAM
    // (26-32), no UART0 console (43/44), no USB (19/20).
    r.register(HardwareAssemblyDefinition {
        id: "esp32s3_va_wheeled_diff_assembly".into(),
        label: "ESP32-S3 V&A Wheeled-Diff Dev Assembly".into(),
        description: "Dev-bench assembly for V&A wheeled-diff solutions (direct_control, mecanum, balance_stabilizer). 2x MCPWM motor drivers, IMU on I2C, SPI aux bus, status LED, failsafe relay. No physical actuators required -- peripherals exercise themselves via backend init.".into(),
        target: ChipTarget::Esp32S3,
        modules: vec!["esp32s3_wroom1".into()],
        // esp_board_manager v0.5.8 ships peripheral parsers for gpio,
        // i2c, spi, mcpwm, ledc, pcnt, rmt, sdm, uart, i2s, anacmpr,
        // adc, dac, dsi, and ldo (see upstream `peripherals/*/*.yml` for
        // canonical schemas). We expose MCPWM × 2 (left/right motors),
        // I²C × 1 (IMU bus), SPI × 1 (aux bus), and 2× plain GPIO
        // (status LED + failsafe relay). Nested configs match the
        // upstream YAML reference files exactly — `crates/rshome-
        // codegen/src/board_yaml.rs::render_yaml_value` turns the
        // `serde_json::json!` trees below into block-style YAML.
        //
        // Phase 3d: motor_driver / imu_sensor / failsafe_ctrl are
        // registered as logical devices that reference their underlying
        // peripherals. The YAML emitter
        // (`crates/rshome-codegen/src/board_yaml.rs::resolve_device_type_yaml`)
        // coerces any device_type that esp_board_manager v0.5.x does not
        // ship a parser for to `type: custom` — the built-in no-op
        // device that registers the device without running a driver
        // init. Backends resolve the associated peripheral directly via
        // `esp_board_manager_get_periph_handle(periph_name, ...)` using
        // the `DeviceDeclaration.peripherals[0]` passed from codegen.
        peripherals: vec![
            // Left motor — MCPWM group 0, GPIO 4, 20 kHz
            // (period_ticks=500 at resolution 10 MHz).
            PeripheralDeclaration {
                name: "mcpwm_left".into(),
                periph_type: "mcpwm".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    (
                        "timer_config".into(),
                        serde_json::json!({
                            "group_id": 0,
                            "clk_src": "MCPWM_TIMER_CLK_SRC_DEFAULT",
                            "resolution_hz": 10_000_000,
                            "count_mode": "MCPWM_TIMER_COUNT_MODE_UP",
                            "period_ticks": 500,
                        }),
                    ),
                    (
                        "operator_config".into(),
                        serde_json::json!({"group_id": 0}),
                    ),
                    (
                        "comparator_configs".into(),
                        serde_json::json!([{"comparator": 0}]),
                    ),
                    (
                        "generator_config".into(),
                        serde_json::json!({"gpio_num": 4}),
                    ),
                ]),
            },
            // Right motor — same shape, GPIO 5.
            PeripheralDeclaration {
                name: "mcpwm_right".into(),
                periph_type: "mcpwm".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    (
                        "timer_config".into(),
                        serde_json::json!({
                            "group_id": 0,
                            "clk_src": "MCPWM_TIMER_CLK_SRC_DEFAULT",
                            "resolution_hz": 10_000_000,
                            "count_mode": "MCPWM_TIMER_COUNT_MODE_UP",
                            "period_ticks": 500,
                        }),
                    ),
                    (
                        "operator_config".into(),
                        serde_json::json!({"group_id": 0}),
                    ),
                    (
                        "comparator_configs".into(),
                        serde_json::json!([{"comparator": 0}]),
                    ),
                    (
                        "generator_config".into(),
                        serde_json::json!({"gpio_num": 5}),
                    ),
                ]),
            },
            // I²C master — IMU at 0x68 (MPU-6050 default). `pins` is a
            // nested map per periph_i2c.yml. Port 0 (HP I2C) with
            // internal pullups enabled.
            PeripheralDeclaration {
                name: "i2c_imu".into(),
                periph_type: "i2c".into(),
                role: "master".into(),
                config: BTreeMap::from([
                    ("port".into(), serde_json::json!(0)),
                    (
                        "clk_source".into(),
                        serde_json::json!("I2C_CLK_SRC_DEFAULT"),
                    ),
                    ("pins".into(), serde_json::json!({"sda": 8, "scl": 9})),
                    ("enable_internal_pullup".into(), serde_json::json!(true)),
                    ("glitch_count".into(), serde_json::json!(7)),
                    ("intr_priority".into(), serde_json::json!(1)),
                ]),
            },
            // SPI master on SPI2_HOST. `spi_bus_config` is the nested
            // pin block per periph_spi.yml. No slave attached in the
            // dev assembly; bus init alone exercises the IDF SPI path.
            PeripheralDeclaration {
                name: "spi_aux".into(),
                periph_type: "spi".into(),
                role: "master".into(),
                config: BTreeMap::from([
                    ("spi_port".into(), serde_json::json!("SPI2_HOST")),
                    (
                        "spi_bus_config".into(),
                        serde_json::json!({
                            "mosi_io_num": 11,
                            "miso_io_num": 13,
                            "sclk_io_num": 12,
                            "quadwp_io_num": -1,
                            "quadhd_io_num": -1,
                            "max_transfer_sz": 4092,
                        }),
                    ),
                ]),
            },
            // Status LED on a plain GPIO — drives high/low so a
            // multimeter or scope confirms the output path.
            PeripheralDeclaration {
                name: "gpio_status_led".into(),
                periph_type: "gpio".into(),
                // esp_board_manager uses `io` for GPIO peripherals (the
                // `esp_board_periph_role_t` enum has NONE/MASTER/SLAVE/
                // IO/TX/RX/CONTINUOUS/ONESHOT/COSINE — no OUTPUT
                // variant). The `mode` field below carries direction.
                role: "io".into(),
                config: BTreeMap::from([
                    ("pin".into(), serde_json::json!(2)),
                    ("mode".into(), serde_json::json!("GPIO_MODE_OUTPUT")),
                    ("default_level".into(), serde_json::json!(0)),
                ]),
            },
            // Failsafe relay GPIO.
            PeripheralDeclaration {
                name: "gpio_failsafe".into(),
                periph_type: "gpio".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("pin".into(), serde_json::json!(21)),
                    ("mode".into(), serde_json::json!("GPIO_MODE_OUTPUT")),
                    ("default_level".into(), serde_json::json!(0)),
                ]),
            },
        ],
        devices: vec![
            DeviceDeclaration {
                name: "status_led_0".into(),
                device_type: "gpio_ctrl".into(),
                peripherals: vec!["gpio_status_led".into()],
                config: BTreeMap::from([
                    ("enabled".into(), serde_json::json!(true)),
                    ("active_level".into(), serde_json::json!(1)),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "failsafe_gpio_0".into(),
                device_type: "gpio_ctrl".into(),
                peripherals: vec!["gpio_failsafe".into()],
                config: BTreeMap::from([
                    ("enabled".into(), serde_json::json!(true)),
                    ("active_level".into(), serde_json::json!(1)),
                ]),
                dependencies: vec![],
            },
            // Left differential-drive motor driven by MCPWM on GPIO 4.
            // Coerced to `type: custom` in YAML — backend resolves the
            // `mcpwm_left` peripheral directly.
            DeviceDeclaration {
                name: "motor_left".into(),
                device_type: "motor_driver".into(),
                peripherals: vec!["mcpwm_left".into()],
                config: BTreeMap::from([
                    ("max_duty".into(), serde_json::json!(0.8)),
                    ("direction".into(), serde_json::json!("forward")),
                ]),
                dependencies: vec![],
            },
            // Right differential-drive motor (MCPWM GPIO 5).
            DeviceDeclaration {
                name: "motor_right".into(),
                device_type: "motor_driver".into(),
                peripherals: vec!["mcpwm_right".into()],
                config: BTreeMap::from([
                    ("max_duty".into(), serde_json::json!(0.8)),
                    ("direction".into(), serde_json::json!("forward")),
                ]),
                dependencies: vec![],
            },
            // MPU-6050 IMU on I²C bus (0x68 default address).
            DeviceDeclaration {
                name: "imu_mpu6050".into(),
                device_type: "imu_sensor".into(),
                peripherals: vec!["i2c_imu".into()],
                config: BTreeMap::from([
                    ("i2c_address".into(), serde_json::json!("0x68")),
                    ("poll_hz".into(), serde_json::json!(50)),
                ]),
                dependencies: vec![],
            },
            // Software failsafe: 500 ms watchdog, drives brake GPIO high
            // on trip. Shares the gpio_failsafe peripheral with
            // `failsafe_gpio_0` above — they publish different contracts
            // (`gpio_output` vs `failsafe_stop`).
            DeviceDeclaration {
                name: "failsafe_0".into(),
                device_type: "failsafe_ctrl".into(),
                peripherals: vec!["gpio_failsafe".into()],
                config: BTreeMap::from([
                    ("timeout_ms".into(), serde_json::json!(500)),
                    ("active_level".into(), serde_json::json!(1)),
                ]),
                dependencies: vec![],
            },
            // Phase 5b — diff_drive vehicle_control orchestrator. Two-
            // motor skid-steer mixer: caller does set_target_throttle
            // + set_yaw_rate, motor_left = clamp(throttle - yaw, 0..100)
            // and motor_right = clamp(throttle + yaw, 0..100). No IMU
            // PID — open-loop. Disarmed by default; set_armed(true)
            // gates the wire output.
            DeviceDeclaration {
                name: "vehicle_ctrl_0".into(),
                device_type: "vehicle_ctrl".into(),
                peripherals: vec![],
                config: BTreeMap::from([
                    ("mixer".into(), serde_json::json!("diff_drive")),
                    ("failsafe_ref".into(), serde_json::json!("failsafe_0")),
                    (
                        "motor_refs".into(),
                        serde_json::json!(["motor_left", "motor_right"]),
                    ),
                    ("target_throttle".into(), serde_json::json!(0.0)),
                ]),
                dependencies: vec![],
            },
        ],
        provided_hal_contracts: vec![
            "gpio_output".into(),
            "motor_control".into(),
            "imu_sensor".into(),
            "failsafe_stop".into(),
            "vehicle_control".into(),
        ],
    });

    // ── V&A assembly #2: ELRS / CRSF receiver + ESC + steering servo ────────
    //
    // Covers `elrs_crsf_brushed`, `elrs_crsf_brushless`, `elrs_crsf_dshot`,
    // `elrs_crsf_mavlink`. UART2 carries the inverted CRSF stream from an
    // ELRS receiver; LEDC drives an ESC + a steering servo. Status LED on
    // GPIO 2, failsafe relay on GPIO 21.
    //
    // Peripheral configs follow the upstream `esp_board_manager v0.5.8`
    // schema literally (see `peripherals/periph_<type>/periph_<type>.yml`
    // in the managed component): UART fields are flat
    // (`uart_num`/`tx_io_num`/...), LEDC fields are flat, GPIO uses
    // `pull_up`/`pull_down` booleans.
    //
    // Phase 4 (2026-04-20): typed devices added matching wheeled_diff
    // pattern. motor_driver + servo_control + crsf_link + failsafe_ctrl
    // entries reference their underlying peripherals; YAML emitter
    // coerces unknown types to `type: custom`. motor_backend dispatches
    // to the LEDC code path based on dev.periph_type.
    r.register(HardwareAssemblyDefinition {
        id: "esp32s3_va_elrs_crsf_assembly".into(),
        label: "ESP32-S3 V&A ELRS/CRSF Dev Assembly".into(),
        description: "Dev-bench assembly for V&A ELRS/CRSF solutions (elrs_crsf_brushed, elrs_crsf_brushless, elrs_crsf_dshot, elrs_crsf_mavlink). UART2 (inverted) for CRSF, LEDC ESC + servo, status LED, failsafe relay.".into(),
        target: ChipTarget::Esp32S3,
        modules: vec!["esp32s3_wroom1".into()],
        peripherals: vec![
            // CRSF receiver UART (inverted at the wire). UART2, RX=18, TX=17.
            // Upstream `periph_uart.yml` wants flat `uart_num`, `*_io_num`
            // fields plus a nested `uart_config` block.
            PeripheralDeclaration {
                name: "uart_crsf".into(),
                periph_type: "uart".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("uart_num".into(), serde_json::json!(2)),
                    ("tx_io_num".into(), serde_json::json!(17)),
                    ("rx_io_num".into(), serde_json::json!(18)),
                    ("rts_io_num".into(), serde_json::json!(-1)),
                    ("cts_io_num".into(), serde_json::json!(-1)),
                    (
                        "uart_config".into(),
                        serde_json::json!({
                            "baud_rate": 420000,
                            "data_bits": "UART_DATA_8_BITS",
                            "parity": "UART_PARITY_DISABLE",
                            "stop_bits": "UART_STOP_BITS_1",
                            "flow_ctrl": "UART_HW_FLOWCTRL_DISABLE",
                            "source_clk": "UART_SCLK_DEFAULT",
                        }),
                    ),
                ]),
            },
            // ESC PWM via LEDC, GPIO 4, 50 Hz servo signal. Flat schema
            // per upstream `periph_ledc.yml`.
            PeripheralDeclaration {
                name: "ledc_esc".into(),
                periph_type: "ledc".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("gpio_num".into(), serde_json::json!(4)),
                    ("channel".into(), serde_json::json!("LEDC_CHANNEL_0")),
                    ("timer_sel".into(), serde_json::json!("LEDC_TIMER_0")),
                    ("freq_hz".into(), serde_json::json!(50)),
                    ("duty".into(), serde_json::json!(0)),
                    (
                        "duty_resolution".into(),
                        serde_json::json!("LEDC_TIMER_14_BIT"),
                    ),
                    (
                        "speed_mode".into(),
                        serde_json::json!("LEDC_LOW_SPEED_MODE"),
                    ),
                ]),
            },
            // Steering servo PWM, GPIO 5, same 50 Hz timer.
            PeripheralDeclaration {
                name: "ledc_servo".into(),
                periph_type: "ledc".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("gpio_num".into(), serde_json::json!(5)),
                    ("channel".into(), serde_json::json!("LEDC_CHANNEL_1")),
                    ("timer_sel".into(), serde_json::json!("LEDC_TIMER_0")),
                    ("freq_hz".into(), serde_json::json!(50)),
                    ("duty".into(), serde_json::json!(0)),
                    (
                        "duty_resolution".into(),
                        serde_json::json!("LEDC_TIMER_14_BIT"),
                    ),
                    (
                        "speed_mode".into(),
                        serde_json::json!("LEDC_LOW_SPEED_MODE"),
                    ),
                ]),
            },
            PeripheralDeclaration {
                name: "gpio_status_led".into(),
                periph_type: "gpio".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("pin".into(), serde_json::json!(2)),
                    ("mode".into(), serde_json::json!("GPIO_MODE_OUTPUT")),
                    ("default_level".into(), serde_json::json!(0)),
                ]),
            },
            PeripheralDeclaration {
                name: "gpio_failsafe".into(),
                periph_type: "gpio".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("pin".into(), serde_json::json!(21)),
                    ("mode".into(), serde_json::json!("GPIO_MODE_OUTPUT")),
                    ("default_level".into(), serde_json::json!(0)),
                ]),
            },
        ],
        devices: vec![
            DeviceDeclaration {
                name: "status_led_0".into(),
                device_type: "gpio_ctrl".into(),
                peripherals: vec!["gpio_status_led".into()],
                config: BTreeMap::from([
                    ("enabled".into(), serde_json::json!(true)),
                    ("active_level".into(), serde_json::json!(1)),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "failsafe_gpio_0".into(),
                device_type: "gpio_ctrl".into(),
                peripherals: vec!["gpio_failsafe".into()],
                config: BTreeMap::from([
                    ("enabled".into(), serde_json::json!(true)),
                    ("active_level".into(), serde_json::json!(1)),
                ]),
                dependencies: vec![],
            },
            // ESC on GPIO 4 via LEDC (50 Hz servo signal). motor_backend
            // dispatches to its LEDC path because dev.periph_type=ledc.
            DeviceDeclaration {
                name: "motor_esc_0".into(),
                device_type: "motor_driver".into(),
                peripherals: vec!["ledc_esc".into()],
                config: BTreeMap::from([
                    ("max_duty".into(), serde_json::json!(0.8)),
                    ("direction".into(), serde_json::json!("forward")),
                ]),
                dependencies: vec![],
            },
            // Steering servo on GPIO 5 via LEDC. servo_control type has no
            // backend yet — typed for codegen completeness, no driver init.
            DeviceDeclaration {
                name: "servo_steer_0".into(),
                device_type: "servo_control".into(),
                peripherals: vec!["ledc_servo".into()],
                config: BTreeMap::from([
                    ("min_pulse_us".into(), serde_json::json!(1000)),
                    ("max_pulse_us".into(), serde_json::json!(2000)),
                    ("center_pulse_us".into(), serde_json::json!(1500)),
                ]),
                dependencies: vec![],
            },
            // CRSF receiver on UART2. crsf_link type has no backend yet.
            DeviceDeclaration {
                name: "crsf_rx_0".into(),
                device_type: "crsf_link".into(),
                peripherals: vec!["uart_crsf".into()],
                config: BTreeMap::from([
                    ("baud_rate".into(), serde_json::json!(420000)),
                    ("inverted".into(), serde_json::json!(true)),
                ]),
                dependencies: vec![],
            },
            // Software failsafe — shares the gpio_failsafe peripheral with
            // failsafe_gpio_0 above (gpio_ctrl is the LED-style contract,
            // failsafe_ctrl is the SAFE_STOP watchdog contract).
            DeviceDeclaration {
                name: "failsafe_0".into(),
                device_type: "failsafe_ctrl".into(),
                peripherals: vec!["gpio_failsafe".into()],
                config: BTreeMap::from([
                    ("timeout_ms".into(), serde_json::json!(500)),
                    ("active_level".into(), serde_json::json!(1)),
                ]),
                dependencies: vec![],
            },
            // Phase 5b — single_motor_servo vehicle_control orchestrator.
            // RC car shape: 1 ESC + 1 steering servo. Direct passthrough
            // — set_target_throttle drives the motor's set_duty,
            // set_steering(angle_deg) drives the servo's set_angle.
            // No PID, no IMU coupling.
            DeviceDeclaration {
                name: "vehicle_ctrl_0".into(),
                device_type: "vehicle_ctrl".into(),
                peripherals: vec![],
                config: BTreeMap::from([
                    ("mixer".into(), serde_json::json!("single_motor_servo")),
                    ("failsafe_ref".into(), serde_json::json!("failsafe_0")),
                    ("motor_refs".into(), serde_json::json!(["motor_esc_0"])),
                    ("servo_ref".into(), serde_json::json!("servo_steer_0")),
                    ("target_throttle".into(), serde_json::json!(0.0)),
                ]),
                dependencies: vec![],
            },
        ],
        provided_hal_contracts: vec![
            "gpio_output".into(),
            "motor_control".into(),
            "servo_control".into(),
            "crsf_link".into(),
            "failsafe_stop".into(),
            "vehicle_control".into(),
        ],
    });

    // ── V&A assembly #3: multirotor (4× ESC + IMU + VBAT ADC) ──────────────
    //
    // Covers `quad_stabilizer`, `heli_stabilizer`, `vtol_transition`,
    // `hopping_ballistic`. 4 ESCs on LEDC (50 Hz analog PWM) + IMU on I2C
    // + VBAT divider on ADC1 + status LED + failsafe. DShot RMT is a v1.x
    // follow-up once esp_board_manager's RMT YAML emitter matches IDF v5.5.
    r.register(HardwareAssemblyDefinition {
        id: "esp32s3_va_multirotor_assembly".into(),
        label: "ESP32-S3 V&A Multirotor Dev Assembly".into(),
        description: "Dev-bench assembly for V&A multirotor solutions (quad_stabilizer, heli_stabilizer, vtol_transition, hopping_ballistic). 4× RMT-DShot ESC, IMU on I2C, VBAT on ADC1, status LED, failsafe relay.".into(),
        target: ChipTarget::Esp32S3,
        modules: vec!["esp32s3_wroom1".into()],
        peripherals: vec![
            // 4× LEDC ESC outputs on GPIO 4/5/6/7. LEDC gives 50 Hz PWM
            // (standard analog ESC protocol). Proper DShot telemetry needs
            // RMT, but `esp_board_manager` v0.5.8's `periph_rmt.py` emits
            // a `flags.init_level` bit that only exists on IDF v6.0's
            // `rmt_tx_channel_config_t`, not v5.5 — so we stay on LEDC
            // until the v6.0 Brookesia/esp_board_manager pipeline stabilises.
            // Tracked as a v1.x follow-up in
            // `docs/va-board-assembly-gap-2026-04-18.md`.
            PeripheralDeclaration {
                name: "ledc_esc_0".into(),
                periph_type: "ledc".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("gpio_num".into(), serde_json::json!(4)),
                    ("channel".into(), serde_json::json!("LEDC_CHANNEL_0")),
                    ("timer_sel".into(), serde_json::json!("LEDC_TIMER_0")),
                    ("freq_hz".into(), serde_json::json!(50)),
                    ("duty".into(), serde_json::json!(0)),
                    (
                        "duty_resolution".into(),
                        serde_json::json!("LEDC_TIMER_14_BIT"),
                    ),
                    (
                        "speed_mode".into(),
                        serde_json::json!("LEDC_LOW_SPEED_MODE"),
                    ),
                ]),
            },
            PeripheralDeclaration {
                name: "ledc_esc_1".into(),
                periph_type: "ledc".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("gpio_num".into(), serde_json::json!(5)),
                    ("channel".into(), serde_json::json!("LEDC_CHANNEL_1")),
                    ("timer_sel".into(), serde_json::json!("LEDC_TIMER_0")),
                    ("freq_hz".into(), serde_json::json!(50)),
                    ("duty".into(), serde_json::json!(0)),
                    (
                        "duty_resolution".into(),
                        serde_json::json!("LEDC_TIMER_14_BIT"),
                    ),
                    (
                        "speed_mode".into(),
                        serde_json::json!("LEDC_LOW_SPEED_MODE"),
                    ),
                ]),
            },
            PeripheralDeclaration {
                name: "ledc_esc_2".into(),
                periph_type: "ledc".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("gpio_num".into(), serde_json::json!(6)),
                    ("channel".into(), serde_json::json!("LEDC_CHANNEL_2")),
                    ("timer_sel".into(), serde_json::json!("LEDC_TIMER_0")),
                    ("freq_hz".into(), serde_json::json!(50)),
                    ("duty".into(), serde_json::json!(0)),
                    (
                        "duty_resolution".into(),
                        serde_json::json!("LEDC_TIMER_14_BIT"),
                    ),
                    (
                        "speed_mode".into(),
                        serde_json::json!("LEDC_LOW_SPEED_MODE"),
                    ),
                ]),
            },
            PeripheralDeclaration {
                name: "ledc_esc_3".into(),
                periph_type: "ledc".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("gpio_num".into(), serde_json::json!(7)),
                    ("channel".into(), serde_json::json!("LEDC_CHANNEL_3")),
                    ("timer_sel".into(), serde_json::json!("LEDC_TIMER_0")),
                    ("freq_hz".into(), serde_json::json!(50)),
                    ("duty".into(), serde_json::json!(0)),
                    (
                        "duty_resolution".into(),
                        serde_json::json!("LEDC_TIMER_14_BIT"),
                    ),
                    (
                        "speed_mode".into(),
                        serde_json::json!("LEDC_LOW_SPEED_MODE"),
                    ),
                ]),
            },
            // IMU bus — I2C0 master, SDA=8 SCL=9 (matches wheeled-diff).
            PeripheralDeclaration {
                name: "i2c_imu".into(),
                periph_type: "i2c".into(),
                role: "master".into(),
                config: BTreeMap::from([
                    ("port".into(), serde_json::json!(0)),
                    (
                        "clk_source".into(),
                        serde_json::json!("I2C_CLK_SRC_DEFAULT"),
                    ),
                    ("pins".into(), serde_json::json!({"sda": 8, "scl": 9})),
                    ("enable_internal_pullup".into(), serde_json::json!(true)),
                    ("glitch_count".into(), serde_json::json!(7)),
                    ("intr_priority".into(), serde_json::json!(1)),
                ]),
            },
            // VBAT monitor — ADC1 channel 0 (GPIO 1 on S3 — confirmed safe).
            // Flat shape per upstream `periph_adc.yml` oneshot example.
            PeripheralDeclaration {
                name: "adc_vbat".into(),
                periph_type: "adc".into(),
                role: "oneshot".into(),
                config: BTreeMap::from([
                    ("unit_id".into(), serde_json::json!("ADC_UNIT_1")),
                    ("atten".into(), serde_json::json!("ADC_ATTEN_DB_12")),
                    ("bit_width".into(), serde_json::json!("ADC_BITWIDTH_DEFAULT")),
                    ("channel_id".into(), serde_json::json!(0)),
                ]),
            },
            PeripheralDeclaration {
                name: "gpio_status_led".into(),
                periph_type: "gpio".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("pin".into(), serde_json::json!(2)),
                    ("mode".into(), serde_json::json!("GPIO_MODE_OUTPUT")),
                    ("default_level".into(), serde_json::json!(0)),
                ]),
            },
            PeripheralDeclaration {
                name: "gpio_failsafe".into(),
                periph_type: "gpio".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("pin".into(), serde_json::json!(21)),
                    ("mode".into(), serde_json::json!("GPIO_MODE_OUTPUT")),
                    ("default_level".into(), serde_json::json!(0)),
                ]),
            },
        ],
        devices: vec![
            DeviceDeclaration {
                name: "status_led_0".into(),
                device_type: "gpio_ctrl".into(),
                peripherals: vec!["gpio_status_led".into()],
                config: BTreeMap::from([
                    ("enabled".into(), serde_json::json!(true)),
                    ("active_level".into(), serde_json::json!(1)),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "failsafe_gpio_0".into(),
                device_type: "gpio_ctrl".into(),
                peripherals: vec!["gpio_failsafe".into()],
                config: BTreeMap::from([
                    ("enabled".into(), serde_json::json!(true)),
                    ("active_level".into(), serde_json::json!(1)),
                ]),
                dependencies: vec![],
            },
            // 4× ESC on GPIO 4/5/6/7 via LEDC channels 0..3, all sharing
            // the same 50 Hz timer. motor_backend's LEDC path writes each
            // channel's duty independently.
            DeviceDeclaration {
                name: "motor_esc_0".into(),
                device_type: "motor_driver".into(),
                peripherals: vec!["ledc_esc_0".into()],
                config: BTreeMap::from([
                    ("max_duty".into(), serde_json::json!(0.8)),
                    ("position".into(), serde_json::json!("front_right")),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "motor_esc_1".into(),
                device_type: "motor_driver".into(),
                peripherals: vec!["ledc_esc_1".into()],
                config: BTreeMap::from([
                    ("max_duty".into(), serde_json::json!(0.8)),
                    ("position".into(), serde_json::json!("rear_right")),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "motor_esc_2".into(),
                device_type: "motor_driver".into(),
                peripherals: vec!["ledc_esc_2".into()],
                config: BTreeMap::from([
                    ("max_duty".into(), serde_json::json!(0.8)),
                    ("position".into(), serde_json::json!("rear_left")),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "motor_esc_3".into(),
                device_type: "motor_driver".into(),
                peripherals: vec!["ledc_esc_3".into()],
                config: BTreeMap::from([
                    ("max_duty".into(), serde_json::json!(0.8)),
                    ("position".into(), serde_json::json!("front_left")),
                ]),
                dependencies: vec![],
            },
            // MPU-6050 IMU on I²C bus — same imu_backend code as wheeled_diff.
            DeviceDeclaration {
                name: "imu_mpu6050".into(),
                device_type: "imu_sensor".into(),
                peripherals: vec!["i2c_imu".into()],
                config: BTreeMap::from([
                    ("i2c_address".into(), serde_json::json!("0x68")),
                    ("poll_hz".into(), serde_json::json!(50)),
                ]),
                dependencies: vec![],
            },
            // VBAT divider on ADC1. vbat_monitor type has no backend yet.
            DeviceDeclaration {
                name: "vbat_monitor_0".into(),
                device_type: "vbat_monitor".into(),
                peripherals: vec!["adc_vbat".into()],
                config: BTreeMap::from([
                    ("divider_ratio".into(), serde_json::json!(11.0)),
                    ("low_voltage_v".into(), serde_json::json!(10.5)),
                    ("poll_hz".into(), serde_json::json!(2)),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "failsafe_0".into(),
                device_type: "failsafe_ctrl".into(),
                peripherals: vec!["gpio_failsafe".into()],
                config: BTreeMap::from([
                    ("timeout_ms".into(), serde_json::json!(500)),
                    ("active_level".into(), serde_json::json!(1)),
                ]),
                dependencies: vec![],
            },
            // Phase 5b — closed-loop stabilizer. References the imu, all 4
            // motors, and the failsafe by name; the vehicle_control backend
            // subscribes to imu sensor_data, runs a P-controller per axis,
            // mixes via quad-X, and drives motor.set_duty via CustomService.
            // Disarmed by default — caller must invoke
            // `vehicle_ctrl_0.set_armed(true)` to start outputting (this
            // works externally over RPC since Phase 5c).
            DeviceDeclaration {
                name: "vehicle_ctrl_0".into(),
                device_type: "vehicle_ctrl".into(),
                peripherals: vec![],
                config: BTreeMap::from([
                    ("mixer".into(), serde_json::json!("quad_x")),
                    ("imu_ref".into(), serde_json::json!("imu_mpu6050")),
                    ("failsafe_ref".into(), serde_json::json!("failsafe_0")),
                    (
                        "motor_refs".into(),
                        serde_json::json!([
                            "motor_esc_0",   // front_right
                            "motor_esc_1",   // rear_right
                            "motor_esc_2",   // rear_left
                            "motor_esc_3",   // front_left
                        ]),
                    ),
                    // Conservative P gains — tune on bench. 0.5 % duty per
                    // deg/s is enough to feel control authority without
                    // saturating mid-tilt.
                    ("kp_roll".into(), serde_json::json!(0.5)),
                    ("kp_pitch".into(), serde_json::json!(0.5)),
                    ("kp_yaw".into(), serde_json::json!(0.3)),
                    // Hover throttle baseline. Real value depends on
                    // payload + battery voltage; this is a safe init value
                    // for first arming.
                    ("target_throttle".into(), serde_json::json!(35.0)),
                ]),
                // NOTE: cross-device references live in `config` (imu_ref,
                // failsafe_ref, motor_refs) — NOT in `dependencies`, which
                // is reserved for IDF managed component names.
                dependencies: vec![],
            },
        ],
        provided_hal_contracts: vec![
            "gpio_output".into(),
            "motor_control".into(),
            "imu_sensor".into(),
            "vbat_monitor".into(),
            "failsafe_stop".into(),
            "vehicle_control".into(),
        ],
    });

    // ── V&A assembly #3b: Multirotor DShot variant (Phase 5d) ──────────────
    //
    // Same airframe as #3 (multirotor) but the 4 ESCs speak DShot600
    // over RMT instead of analog PWM over LEDC. Each motor's GPIO is
    // declared as a regular periph_gpio (output mode) so esp_board_manager
    // configures it; the motor_backend then attaches its own RMT TX
    // channel to that pin and drives it with DShot symbols (600 kbps,
    // 16-bit frames at 1 kHz). Distinguished from the LEDC multirotor
    // by the per-device `signaling: "dshot600"` config field — codegen
    // dispatches accordingly.
    //
    // IMU + vbat + failsafe + status LED + vehicle_ctrl are unchanged
    // from the LEDC variant.
    r.register(HardwareAssemblyDefinition {
        id: "esp32s3_va_multirotor_dshot_assembly".into(),
        label: "ESP32-S3 V&A Multirotor DShot Dev Assembly".into(),
        description: "Phase 5d variant of the multirotor assembly: 4× DShot600 ESCs over RMT instead of LEDC PWM. Same IMU + vbat + failsafe + vehicle_ctrl as the LEDC variant.".into(),
        target: ChipTarget::Esp32S3,
        modules: vec!["esp32s3_wroom1".into()],
        peripherals: vec![
            // 4× DShot ESC GPIOs on the same pins (4/5/6/7) as the LEDC
            // variant — only the signaling protocol changes. Each is a
            // straight gpio output; the motor_backend attaches RMT.
            PeripheralDeclaration {
                name: "gpio_dshot_esc_0".into(),
                periph_type: "gpio".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("pin".into(), serde_json::json!(4)),
                    ("mode".into(), serde_json::json!("GPIO_MODE_OUTPUT")),
                    ("default_level".into(), serde_json::json!(0)),
                ]),
            },
            PeripheralDeclaration {
                name: "gpio_dshot_esc_1".into(),
                periph_type: "gpio".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("pin".into(), serde_json::json!(5)),
                    ("mode".into(), serde_json::json!("GPIO_MODE_OUTPUT")),
                    ("default_level".into(), serde_json::json!(0)),
                ]),
            },
            PeripheralDeclaration {
                name: "gpio_dshot_esc_2".into(),
                periph_type: "gpio".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("pin".into(), serde_json::json!(6)),
                    ("mode".into(), serde_json::json!("GPIO_MODE_OUTPUT")),
                    ("default_level".into(), serde_json::json!(0)),
                ]),
            },
            PeripheralDeclaration {
                name: "gpio_dshot_esc_3".into(),
                periph_type: "gpio".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("pin".into(), serde_json::json!(7)),
                    ("mode".into(), serde_json::json!("GPIO_MODE_OUTPUT")),
                    ("default_level".into(), serde_json::json!(0)),
                ]),
            },
            // IMU + status_led + failsafe identical to LEDC variant.
            PeripheralDeclaration {
                name: "i2c_imu".into(),
                periph_type: "i2c".into(),
                role: "master".into(),
                config: BTreeMap::from([
                    ("port".into(), serde_json::json!(0)),
                    (
                        "clk_source".into(),
                        serde_json::json!("I2C_CLK_SRC_DEFAULT"),
                    ),
                    ("pins".into(), serde_json::json!({"sda": 8, "scl": 9})),
                    ("enable_internal_pullup".into(), serde_json::json!(true)),
                    ("glitch_count".into(), serde_json::json!(7)),
                    ("intr_priority".into(), serde_json::json!(1)),
                ]),
            },
            PeripheralDeclaration {
                name: "gpio_status_led".into(),
                periph_type: "gpio".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("pin".into(), serde_json::json!(2)),
                    ("mode".into(), serde_json::json!("GPIO_MODE_OUTPUT")),
                    ("default_level".into(), serde_json::json!(0)),
                ]),
            },
            PeripheralDeclaration {
                name: "gpio_failsafe".into(),
                periph_type: "gpio".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("pin".into(), serde_json::json!(21)),
                    ("mode".into(), serde_json::json!("GPIO_MODE_OUTPUT")),
                    ("default_level".into(), serde_json::json!(0)),
                ]),
            },
        ],
        devices: vec![
            DeviceDeclaration {
                name: "status_led_0".into(),
                device_type: "gpio_ctrl".into(),
                peripherals: vec!["gpio_status_led".into()],
                config: BTreeMap::from([
                    ("enabled".into(), serde_json::json!(true)),
                    ("active_level".into(), serde_json::json!(1)),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "failsafe_gpio_0".into(),
                device_type: "gpio_ctrl".into(),
                peripherals: vec!["gpio_failsafe".into()],
                config: BTreeMap::from([
                    ("enabled".into(), serde_json::json!(true)),
                    ("active_level".into(), serde_json::json!(1)),
                ]),
                dependencies: vec![],
            },
            // 4× DShot motors. The `signaling` config field is what the
            // codegen reads to dispatch to motor_backend_init_dshot
            // instead of the periph_type-based mcpwm/ledc paths.
            DeviceDeclaration {
                name: "motor_esc_0".into(),
                device_type: "motor_driver".into(),
                peripherals: vec!["gpio_dshot_esc_0".into()],
                config: BTreeMap::from([
                    ("signaling".into(), serde_json::json!("dshot600")),
                    ("max_duty".into(), serde_json::json!(0.8)),
                    ("position".into(), serde_json::json!("front_right")),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "motor_esc_1".into(),
                device_type: "motor_driver".into(),
                peripherals: vec!["gpio_dshot_esc_1".into()],
                config: BTreeMap::from([
                    ("signaling".into(), serde_json::json!("dshot600")),
                    ("max_duty".into(), serde_json::json!(0.8)),
                    ("position".into(), serde_json::json!("rear_right")),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "motor_esc_2".into(),
                device_type: "motor_driver".into(),
                peripherals: vec!["gpio_dshot_esc_2".into()],
                config: BTreeMap::from([
                    ("signaling".into(), serde_json::json!("dshot600")),
                    ("max_duty".into(), serde_json::json!(0.8)),
                    ("position".into(), serde_json::json!("rear_left")),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "motor_esc_3".into(),
                device_type: "motor_driver".into(),
                peripherals: vec!["gpio_dshot_esc_3".into()],
                config: BTreeMap::from([
                    ("signaling".into(), serde_json::json!("dshot600")),
                    ("max_duty".into(), serde_json::json!(0.8)),
                    ("position".into(), serde_json::json!("front_left")),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "imu_mpu6050".into(),
                device_type: "imu_sensor".into(),
                peripherals: vec!["i2c_imu".into()],
                config: BTreeMap::from([
                    ("i2c_address".into(), serde_json::json!("0x68")),
                    ("poll_hz".into(), serde_json::json!(50)),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "failsafe_0".into(),
                device_type: "failsafe_ctrl".into(),
                peripherals: vec!["gpio_failsafe".into()],
                config: BTreeMap::from([
                    ("timeout_ms".into(), serde_json::json!(500)),
                    ("active_level".into(), serde_json::json!(1)),
                ]),
                dependencies: vec![],
            },
        ],
        provided_hal_contracts: vec![
            "gpio_output".into(),
            "motor_control".into(),
            "imu_sensor".into(),
            "failsafe_stop".into(),
        ],
    });

    // ── V&A assembly #3c: Multirotor bdshot variant (Phase 5d.2) ───────────
    //
    // This assembly ships the scaffolding; bench-validated firmware
    // enables the RX path. Keeping this separate from the dshot
    // variant isolates snapshot diffs.
    r.register(HardwareAssemblyDefinition {
        id: "esp32s3_va_multirotor_bdshot_assembly".into(),
        label: "ESP32-S3 V&A Multirotor bdshot Dev Assembly".into(),
        description: "Phase 5d.2 variant: 4× bidirectional DShot600 ESCs — TX identical to #3b, but each motor's RMT RX channel decodes eRPM telemetry from BLHeli_32/AM32 ESCs. Requires open-drain wiring (external 2 kΩ pull-up to 3V3).".into(),
        target: ChipTarget::Esp32S3,
        modules: vec!["esp32s3_wroom1".into()],
        peripherals: vec![
            // 4× ESC GPIOs on 4/5/6/7 — identical declarations to
            // #3b. The motor_backend still attaches RMT TX; the
            // bidirectional config adds a paired RX channel.
            PeripheralDeclaration {
                name: "gpio_dshot_esc_0".into(),
                periph_type: "gpio".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("pin".into(), serde_json::json!(4)),
                    ("mode".into(), serde_json::json!("GPIO_MODE_OUTPUT")),
                    ("default_level".into(), serde_json::json!(0)),
                ]),
            },
            PeripheralDeclaration {
                name: "gpio_dshot_esc_1".into(),
                periph_type: "gpio".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("pin".into(), serde_json::json!(5)),
                    ("mode".into(), serde_json::json!("GPIO_MODE_OUTPUT")),
                    ("default_level".into(), serde_json::json!(0)),
                ]),
            },
            PeripheralDeclaration {
                name: "gpio_dshot_esc_2".into(),
                periph_type: "gpio".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("pin".into(), serde_json::json!(6)),
                    ("mode".into(), serde_json::json!("GPIO_MODE_OUTPUT")),
                    ("default_level".into(), serde_json::json!(0)),
                ]),
            },
            PeripheralDeclaration {
                name: "gpio_dshot_esc_3".into(),
                periph_type: "gpio".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("pin".into(), serde_json::json!(7)),
                    ("mode".into(), serde_json::json!("GPIO_MODE_OUTPUT")),
                    ("default_level".into(), serde_json::json!(0)),
                ]),
            },
            PeripheralDeclaration {
                name: "i2c_imu".into(),
                periph_type: "i2c".into(),
                role: "master".into(),
                config: BTreeMap::from([
                    ("port".into(), serde_json::json!(0)),
                    (
                        "clk_source".into(),
                        serde_json::json!("I2C_CLK_SRC_DEFAULT"),
                    ),
                    ("pins".into(), serde_json::json!({"sda": 8, "scl": 9})),
                    ("enable_internal_pullup".into(), serde_json::json!(true)),
                    ("glitch_count".into(), serde_json::json!(7)),
                    ("intr_priority".into(), serde_json::json!(1)),
                ]),
            },
            PeripheralDeclaration {
                name: "gpio_status_led".into(),
                periph_type: "gpio".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("pin".into(), serde_json::json!(2)),
                    ("mode".into(), serde_json::json!("GPIO_MODE_OUTPUT")),
                    ("default_level".into(), serde_json::json!(0)),
                ]),
            },
            PeripheralDeclaration {
                name: "gpio_failsafe".into(),
                periph_type: "gpio".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("pin".into(), serde_json::json!(21)),
                    ("mode".into(), serde_json::json!("GPIO_MODE_OUTPUT")),
                    ("default_level".into(), serde_json::json!(0)),
                ]),
            },
        ],
        devices: vec![
            DeviceDeclaration {
                name: "status_led_0".into(),
                device_type: "gpio_ctrl".into(),
                peripherals: vec!["gpio_status_led".into()],
                config: BTreeMap::from([
                    ("enabled".into(), serde_json::json!(true)),
                    ("active_level".into(), serde_json::json!(1)),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "failsafe_gpio_0".into(),
                device_type: "gpio_ctrl".into(),
                peripherals: vec!["gpio_failsafe".into()],
                config: BTreeMap::from([
                    ("enabled".into(), serde_json::json!(true)),
                    ("active_level".into(), serde_json::json!(1)),
                ]),
                dependencies: vec![],
            },
            // 4× bdshot motors. `bidirectional: true` flips the
            // codegen so the motor_backend allocates an RX channel
            // on the same GPIO + uses the inverted open-drain TX
            // config; the C++ decoder then consumes ESC replies.
            DeviceDeclaration {
                name: "motor_esc_0".into(),
                device_type: "motor_driver".into(),
                peripherals: vec!["gpio_dshot_esc_0".into()],
                config: BTreeMap::from([
                    ("signaling".into(), serde_json::json!("dshot600")),
                    ("bidirectional".into(), serde_json::json!(true)),
                    ("max_duty".into(), serde_json::json!(0.8)),
                    ("position".into(), serde_json::json!("front_right")),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "motor_esc_1".into(),
                device_type: "motor_driver".into(),
                peripherals: vec!["gpio_dshot_esc_1".into()],
                config: BTreeMap::from([
                    ("signaling".into(), serde_json::json!("dshot600")),
                    ("bidirectional".into(), serde_json::json!(true)),
                    ("max_duty".into(), serde_json::json!(0.8)),
                    ("position".into(), serde_json::json!("rear_right")),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "motor_esc_2".into(),
                device_type: "motor_driver".into(),
                peripherals: vec!["gpio_dshot_esc_2".into()],
                config: BTreeMap::from([
                    ("signaling".into(), serde_json::json!("dshot600")),
                    ("bidirectional".into(), serde_json::json!(true)),
                    ("max_duty".into(), serde_json::json!(0.8)),
                    ("position".into(), serde_json::json!("rear_left")),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "motor_esc_3".into(),
                device_type: "motor_driver".into(),
                peripherals: vec!["gpio_dshot_esc_3".into()],
                config: BTreeMap::from([
                    ("signaling".into(), serde_json::json!("dshot600")),
                    ("bidirectional".into(), serde_json::json!(true)),
                    ("max_duty".into(), serde_json::json!(0.8)),
                    ("position".into(), serde_json::json!("front_left")),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "imu_mpu6050".into(),
                device_type: "imu_sensor".into(),
                peripherals: vec!["i2c_imu".into()],
                config: BTreeMap::from([
                    ("i2c_address".into(), serde_json::json!("0x68")),
                    ("poll_hz".into(), serde_json::json!(50)),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "failsafe_0".into(),
                device_type: "failsafe_ctrl".into(),
                peripherals: vec!["gpio_failsafe".into()],
                config: BTreeMap::from([
                    ("timeout_ms".into(), serde_json::json!(500)),
                    ("active_level".into(), serde_json::json!(1)),
                ]),
                dependencies: vec![],
            },
        ],
        provided_hal_contracts: vec![
            "gpio_output".into(),
            "motor_control".into(),
            "imu_sensor".into(),
            "failsafe_stop".into(),
        ],
    });

    // ── V&A assembly #4: SBC bridge (dual-MCU) ──────────────────────────────
    //
    // Covers `mcu_sbc_bridge`, `legged_controller`,
    // `rov_thruster_allocation`, `articulated_sequencer`. 2× UARTs (one
    // to the SBC, one CRSF uplink) + failsafe relay + status LED.
    r.register(HardwareAssemblyDefinition {
        id: "esp32s3_va_sbc_bridge_assembly".into(),
        label: "ESP32-S3 V&A SBC Bridge Dev Assembly".into(),
        description: "Dev-bench assembly for V&A SBC-bridge solutions (mcu_sbc_bridge, legged_controller, rov_thruster_allocation, articulated_sequencer). UART1 SBC link, UART2 CRSF uplink, status LED, failsafe relay.".into(),
        target: ChipTarget::Esp32S3,
        modules: vec!["esp32s3_wroom1".into()],
        peripherals: vec![
            // SBC link — UART1, RX=GPIO16 TX=GPIO15, 1 Mbaud. Flat shape
            // per upstream `periph_uart.yml`.
            PeripheralDeclaration {
                name: "uart_sbc".into(),
                periph_type: "uart".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("uart_num".into(), serde_json::json!(1)),
                    ("tx_io_num".into(), serde_json::json!(15)),
                    ("rx_io_num".into(), serde_json::json!(16)),
                    ("rts_io_num".into(), serde_json::json!(-1)),
                    ("cts_io_num".into(), serde_json::json!(-1)),
                    (
                        "uart_config".into(),
                        serde_json::json!({
                            "baud_rate": 1000000,
                            "data_bits": "UART_DATA_8_BITS",
                            "parity": "UART_PARITY_DISABLE",
                            "stop_bits": "UART_STOP_BITS_1",
                            "flow_ctrl": "UART_HW_FLOWCTRL_DISABLE",
                            "source_clk": "UART_SCLK_DEFAULT",
                        }),
                    ),
                ]),
            },
            // CRSF uplink — UART2, RX=18 TX=17, 420 kbaud (same as elrs_crsf).
            PeripheralDeclaration {
                name: "uart_crsf".into(),
                periph_type: "uart".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("uart_num".into(), serde_json::json!(2)),
                    ("tx_io_num".into(), serde_json::json!(17)),
                    ("rx_io_num".into(), serde_json::json!(18)),
                    ("rts_io_num".into(), serde_json::json!(-1)),
                    ("cts_io_num".into(), serde_json::json!(-1)),
                    (
                        "uart_config".into(),
                        serde_json::json!({
                            "baud_rate": 420000,
                            "data_bits": "UART_DATA_8_BITS",
                            "parity": "UART_PARITY_DISABLE",
                            "stop_bits": "UART_STOP_BITS_1",
                            "flow_ctrl": "UART_HW_FLOWCTRL_DISABLE",
                            "source_clk": "UART_SCLK_DEFAULT",
                        }),
                    ),
                ]),
            },
            PeripheralDeclaration {
                name: "gpio_status_led".into(),
                periph_type: "gpio".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("pin".into(), serde_json::json!(2)),
                    ("mode".into(), serde_json::json!("GPIO_MODE_OUTPUT")),
                    ("default_level".into(), serde_json::json!(0)),
                ]),
            },
            PeripheralDeclaration {
                name: "gpio_failsafe".into(),
                periph_type: "gpio".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("pin".into(), serde_json::json!(21)),
                    ("mode".into(), serde_json::json!("GPIO_MODE_OUTPUT")),
                    ("default_level".into(), serde_json::json!(0)),
                ]),
            },
        ],
        devices: vec![
            DeviceDeclaration {
                name: "status_led_0".into(),
                device_type: "gpio_ctrl".into(),
                peripherals: vec!["gpio_status_led".into()],
                config: BTreeMap::from([
                    ("enabled".into(), serde_json::json!(true)),
                    ("active_level".into(), serde_json::json!(1)),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "failsafe_gpio_0".into(),
                device_type: "gpio_ctrl".into(),
                peripherals: vec!["gpio_failsafe".into()],
                config: BTreeMap::from([
                    ("enabled".into(), serde_json::json!(true)),
                    ("active_level".into(), serde_json::json!(1)),
                ]),
                dependencies: vec![],
            },
            // SBC link on UART1. sbc_link type has no backend yet —
            // typed for codegen completeness.
            DeviceDeclaration {
                name: "sbc_link_0".into(),
                device_type: "sbc_link".into(),
                peripherals: vec!["uart_sbc".into()],
                config: BTreeMap::from([
                    ("baud_rate".into(), serde_json::json!(1000000)),
                    ("protocol".into(), serde_json::json!("mavlink")),
                ]),
                dependencies: vec![],
            },
            // CRSF uplink on UART2.
            DeviceDeclaration {
                name: "crsf_rx_0".into(),
                device_type: "crsf_link".into(),
                peripherals: vec!["uart_crsf".into()],
                config: BTreeMap::from([
                    ("baud_rate".into(), serde_json::json!(420000)),
                    ("inverted".into(), serde_json::json!(true)),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "failsafe_0".into(),
                device_type: "failsafe_ctrl".into(),
                peripherals: vec!["gpio_failsafe".into()],
                config: BTreeMap::from([
                    ("timeout_ms".into(), serde_json::json!(500)),
                    ("active_level".into(), serde_json::json!(1)),
                ]),
                dependencies: vec![],
            },
        ],
        provided_hal_contracts: vec![
            "gpio_output".into(),
            "sbc_link".into(),
            "crsf_link".into(),
            "failsafe_stop".into(),
        ],
    });

    // ── V&A assembly #5: marine surface ────────────────────────────────────
    //
    // Covers `marine_surface`, `amphibious_transition`. ESC PWM (LEDC) +
    // rudder servo (LEDC) + water-contact GPIO input + status LED +
    // failsafe relay.
    r.register(HardwareAssemblyDefinition {
        id: "esp32s3_va_marine_assembly".into(),
        label: "ESP32-S3 V&A Marine Dev Assembly".into(),
        description: "Dev-bench assembly for V&A marine solutions (marine_surface, amphibious_transition). LEDC ESC, LEDC rudder servo, water-contact GPIO input, status LED, failsafe relay.".into(),
        target: ChipTarget::Esp32S3,
        modules: vec!["esp32s3_wroom1".into()],
        peripherals: vec![
            // ESC on LEDC channel 0, 50 Hz, GPIO 4. Flat shape per upstream.
            PeripheralDeclaration {
                name: "ledc_esc".into(),
                periph_type: "ledc".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("gpio_num".into(), serde_json::json!(4)),
                    ("channel".into(), serde_json::json!("LEDC_CHANNEL_0")),
                    ("timer_sel".into(), serde_json::json!("LEDC_TIMER_0")),
                    ("freq_hz".into(), serde_json::json!(50)),
                    ("duty".into(), serde_json::json!(0)),
                    (
                        "duty_resolution".into(),
                        serde_json::json!("LEDC_TIMER_14_BIT"),
                    ),
                    (
                        "speed_mode".into(),
                        serde_json::json!("LEDC_LOW_SPEED_MODE"),
                    ),
                ]),
            },
            // Rudder servo on LEDC channel 1, GPIO 5.
            PeripheralDeclaration {
                name: "ledc_rudder".into(),
                periph_type: "ledc".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("gpio_num".into(), serde_json::json!(5)),
                    ("channel".into(), serde_json::json!("LEDC_CHANNEL_1")),
                    ("timer_sel".into(), serde_json::json!("LEDC_TIMER_0")),
                    ("freq_hz".into(), serde_json::json!(50)),
                    ("duty".into(), serde_json::json!(0)),
                    (
                        "duty_resolution".into(),
                        serde_json::json!("LEDC_TIMER_14_BIT"),
                    ),
                    (
                        "speed_mode".into(),
                        serde_json::json!("LEDC_LOW_SPEED_MODE"),
                    ),
                ]),
            },
            // Water-contact sensor — GPIO 6 input, internal pull-up. Field
            // names are `pull_up` / `pull_down` per upstream `periph_gpio.yml`.
            PeripheralDeclaration {
                name: "gpio_water_sensor".into(),
                periph_type: "gpio".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("pin".into(), serde_json::json!(6)),
                    ("mode".into(), serde_json::json!("GPIO_MODE_INPUT")),
                    ("pull_up".into(), serde_json::json!(true)),
                    ("pull_down".into(), serde_json::json!(false)),
                ]),
            },
            PeripheralDeclaration {
                name: "gpio_status_led".into(),
                periph_type: "gpio".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("pin".into(), serde_json::json!(2)),
                    ("mode".into(), serde_json::json!("GPIO_MODE_OUTPUT")),
                    ("default_level".into(), serde_json::json!(0)),
                ]),
            },
            PeripheralDeclaration {
                name: "gpio_failsafe".into(),
                periph_type: "gpio".into(),
                role: "io".into(),
                config: BTreeMap::from([
                    ("pin".into(), serde_json::json!(21)),
                    ("mode".into(), serde_json::json!("GPIO_MODE_OUTPUT")),
                    ("default_level".into(), serde_json::json!(0)),
                ]),
            },
        ],
        devices: vec![
            DeviceDeclaration {
                name: "status_led_0".into(),
                device_type: "gpio_ctrl".into(),
                peripherals: vec!["gpio_status_led".into()],
                config: BTreeMap::from([
                    ("enabled".into(), serde_json::json!(true)),
                    ("active_level".into(), serde_json::json!(1)),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "failsafe_gpio_0".into(),
                device_type: "gpio_ctrl".into(),
                peripherals: vec!["gpio_failsafe".into()],
                config: BTreeMap::from([
                    ("enabled".into(), serde_json::json!(true)),
                    ("active_level".into(), serde_json::json!(1)),
                ]),
                dependencies: vec![],
            },
            // ESC on GPIO 4 via LEDC. motor_backend's LEDC path drives it.
            DeviceDeclaration {
                name: "motor_esc_0".into(),
                device_type: "motor_driver".into(),
                peripherals: vec!["ledc_esc".into()],
                config: BTreeMap::from([
                    ("max_duty".into(), serde_json::json!(0.7)),
                    ("waterproof".into(), serde_json::json!(true)),
                ]),
                dependencies: vec![],
            },
            // Rudder servo on GPIO 5 via LEDC. servo_control type, no
            // backend yet.
            DeviceDeclaration {
                name: "servo_rudder_0".into(),
                device_type: "servo_control".into(),
                peripherals: vec!["ledc_rudder".into()],
                config: BTreeMap::from([
                    ("min_pulse_us".into(), serde_json::json!(1000)),
                    ("max_pulse_us".into(), serde_json::json!(2000)),
                    ("center_pulse_us".into(), serde_json::json!(1500)),
                    ("range_deg".into(), serde_json::json!(60)),
                ]),
                dependencies: vec![],
            },
            // Water-contact GPIO input — wet detection. gpio_input type
            // (distinct from gpio_ctrl which is OUTPUT-oriented), no
            // backend yet.
            DeviceDeclaration {
                name: "water_sensor_0".into(),
                device_type: "gpio_input".into(),
                peripherals: vec!["gpio_water_sensor".into()],
                config: BTreeMap::from([
                    ("active_level".into(), serde_json::json!(0)),
                    ("debounce_ms".into(), serde_json::json!(50)),
                ]),
                dependencies: vec![],
            },
            DeviceDeclaration {
                name: "failsafe_0".into(),
                device_type: "failsafe_ctrl".into(),
                peripherals: vec!["gpio_failsafe".into()],
                config: BTreeMap::from([
                    ("timeout_ms".into(), serde_json::json!(500)),
                    ("active_level".into(), serde_json::json!(1)),
                ]),
                dependencies: vec![],
            },
            // Phase 5b — single_motor_servo vehicle_control orchestrator.
            // Same shape as elrs_crsf: 1 ESC + 1 rudder servo. Direct
            // passthrough — set_target_throttle drives the motor's
            // set_duty, set_steering(angle_deg) drives the rudder
            // servo's set_angle. No PID, no IMU coupling.
            DeviceDeclaration {
                name: "vehicle_ctrl_0".into(),
                device_type: "vehicle_ctrl".into(),
                peripherals: vec![],
                config: BTreeMap::from([
                    ("mixer".into(), serde_json::json!("single_motor_servo")),
                    ("failsafe_ref".into(), serde_json::json!("failsafe_0")),
                    ("motor_refs".into(), serde_json::json!(["motor_esc_0"])),
                    ("servo_ref".into(), serde_json::json!("servo_rudder_0")),
                    ("target_throttle".into(), serde_json::json!(0.0)),
                ]),
                dependencies: vec![],
            },
        ],
        provided_hal_contracts: vec![
            "gpio_output".into(),
            "motor_control".into(),
            "servo_control".into(),
            "gpio_input".into(),
            "failsafe_stop".into(),
            "vehicle_control".into(),
        ],
    });

    r
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registry_has_entries() {
        let reg = default_assembly_registry();
        assert!(reg.get("esp32s3_gpio_relay_assembly").is_some());
        assert!(reg.get("esp32s3_i2c_sensor_assembly").is_some());
        assert!(reg.get("esp32_can485_devboard").is_some());
        assert!(reg.get("esp32s3_va_wheeled_diff_assembly").is_some());
    }

    #[test]
    fn for_target_filters() {
        let reg = default_assembly_registry();
        let s3 = reg.for_target(ChipTarget::Esp32S3);
        // 2 generic dev assemblies (gpio_relay, i2c_sensor) +
        // 5 V&A assemblies (wheeled_diff, elrs_crsf, multirotor,
        // sbc_bridge, marine) +
        // 2 V&A multirotor variants (multirotor_dshot — Phase 5d,
        // multirotor_bdshot — Phase 5d.2).
        assert_eq!(s3.len(), 9);
        let c6 = reg.for_target(ChipTarget::Esp32C6);
        assert!(c6.is_empty());
        let esp32 = reg.for_target(ChipTarget::Esp32);
        assert_eq!(esp32.len(), 1);
    }

    #[test]
    fn wheeled_diff_has_gpio_and_vehicle_devices() {
        // Phase 3d added motor_driver / imu_sensor / failsafe_ctrl atop
        // the original gpio_ctrl pair. The YAML emitter coerces those
        // non-built-in types to `type: custom` downstream.
        let reg = default_assembly_registry();
        let asm = reg.get("esp32s3_va_wheeled_diff_assembly").unwrap();

        let device_types: Vec<&str> = asm.devices.iter().map(|d| d.device_type.as_str()).collect();

        assert!(
            device_types.iter().filter(|t| **t == "gpio_ctrl").count() >= 2,
            "status LED + failsafe_gpio must stay as gpio_ctrl, got {device_types:?}"
        );
        assert!(
            device_types.contains(&"motor_driver"),
            "motor_driver device required for has_motor codegen flag, got {device_types:?}"
        );
        assert!(
            device_types.contains(&"imu_sensor"),
            "imu_sensor device required for has_imu codegen flag, got {device_types:?}"
        );
        assert!(
            device_types.contains(&"failsafe_ctrl"),
            "failsafe_ctrl device required for has_failsafe codegen flag, got {device_types:?}"
        );
    }

    #[test]
    fn wheeled_diff_motor_devices_reference_mcpwm_peripherals() {
        let reg = default_assembly_registry();
        let asm = reg.get("esp32s3_va_wheeled_diff_assembly").unwrap();
        for motor in asm
            .devices
            .iter()
            .filter(|d| d.device_type == "motor_driver")
        {
            assert_eq!(
                motor.peripherals.len(),
                1,
                "motor_driver '{}' must reference exactly one MCPWM peripheral",
                motor.name
            );
            let periph_name = &motor.peripherals[0];
            let periph = asm
                .peripherals
                .iter()
                .find(|p| &p.name == periph_name)
                .unwrap_or_else(|| {
                    panic!(
                        "motor '{}' references unknown peripheral '{}'",
                        motor.name, periph_name
                    )
                });
            assert_eq!(
                periph.periph_type, "mcpwm",
                "motor '{}' must drive an MCPWM peripheral, got {}",
                motor.name, periph.periph_type
            );
        }
    }

    #[test]
    fn wheeled_diff_has_gpio_peripherals() {
        let reg = default_assembly_registry();
        let asm = reg.get("esp32s3_va_wheeled_diff_assembly").unwrap();
        let types: Vec<&str> = asm
            .peripherals
            .iter()
            .map(|p| p.periph_type.as_str())
            .collect();
        assert!(types.contains(&"gpio"), "gpio required for LED + failsafe");
    }

    #[test]
    fn wheeled_diff_has_mcpwm_i2c_spi_peripherals() {
        // Re-enabled per the offline-cache / nested-YAML follow-up.
        // MCPWM × 2 (motor left + right), I²C (IMU), SPI (aux bus).
        // The YAML renderer handles nested maps/arrays in the config
        // values — see `crates/rshome-codegen/src/board_yaml.rs`.
        let reg = default_assembly_registry();
        let asm = reg.get("esp32s3_va_wheeled_diff_assembly").unwrap();

        let mcpwm: Vec<&str> = asm
            .peripherals
            .iter()
            .filter(|p| p.periph_type == "mcpwm")
            .map(|p| p.name.as_str())
            .collect();
        assert!(
            mcpwm.contains(&"mcpwm_left") && mcpwm.contains(&"mcpwm_right"),
            "wheeled_diff needs MCPWM left + right for differential drive, got {mcpwm:?}"
        );

        // MCPWM config must be nested per esp_board_manager v0.5.8 schema.
        let mcpwm_left = asm
            .peripherals
            .iter()
            .find(|p| p.name == "mcpwm_left")
            .unwrap();
        assert!(
            mcpwm_left
                .config
                .get("timer_config")
                .is_some_and(|v| v.is_object()),
            "mcpwm_left.timer_config must be a nested map"
        );
        assert!(
            mcpwm_left
                .config
                .get("comparator_configs")
                .is_some_and(|v| v.is_array()),
            "mcpwm_left.comparator_configs must be an array of maps"
        );
        let gen_cfg = mcpwm_left.config.get("generator_config").unwrap();
        assert_eq!(
            gen_cfg.get("gpio_num").and_then(|v| v.as_i64()),
            Some(4),
            "mcpwm_left generator_config.gpio_num must be GPIO 4"
        );

        // I²C — pins nested under `pins`, port = 0 (HP I2C).
        let i2c = asm
            .peripherals
            .iter()
            .find(|p| p.periph_type == "i2c")
            .expect("wheeled_diff needs I²C for IMU bus");
        let pins = i2c.config.get("pins").expect("i2c.pins is required");
        assert_eq!(pins.get("sda").and_then(|v| v.as_i64()), Some(8));
        assert_eq!(pins.get("scl").and_then(|v| v.as_i64()), Some(9));

        // SPI — spi_bus_config nested, MOSI/MISO/SCK populated.
        let spi = asm
            .peripherals
            .iter()
            .find(|p| p.periph_type == "spi")
            .expect("wheeled_diff needs SPI aux bus");
        let bus = spi
            .config
            .get("spi_bus_config")
            .expect("spi.spi_bus_config is required");
        assert_eq!(bus.get("mosi_io_num").and_then(|v| v.as_i64()), Some(11));
        assert_eq!(bus.get("miso_io_num").and_then(|v| v.as_i64()), Some(13));
        assert_eq!(bus.get("sclk_io_num").and_then(|v| v.as_i64()), Some(12));
    }

    #[test]
    fn wheeled_diff_pins_are_safe_for_devkitc_1() {
        // Strapping / flash / USB / UART0 pins on ESP32-S3 that must NOT be
        // used as general-purpose I/O on DevKitC-1:
        //   0, 3, 45, 46 — strap
        //   19, 20       — USB D-/D+
        //   26..=32      — octal SPI flash/PSRAM
        //   43, 44       — UART0 console
        let reserved: &[u32] = &[0, 3, 19, 20, 26, 27, 28, 29, 30, 31, 32, 43, 44, 45, 46];

        // Recursive helper — walks nested maps + arrays in the config
        // tree. Required now that MCPWM uses `generator_config.gpio_num`,
        // SPI uses `spi_bus_config.mosi_io_num`, etc.
        fn check_pins(v: &serde_json::Value, periph: &str, key_path: &str, reserved: &[u32]) {
            match v {
                serde_json::Value::Object(map) => {
                    for (k, child) in map {
                        let next = if key_path.is_empty() {
                            k.clone()
                        } else {
                            format!("{key_path}.{k}")
                        };
                        let looks_like_pin = k == "pin"
                            || k == "sda"
                            || k == "scl"
                            || k.ends_with("_pin")
                            || k.ends_with("_io_num")
                            || k == "gpio_num";
                        if looks_like_pin {
                            if let Some(n) = child.as_i64() {
                                if n >= 0 {
                                    assert!(
                                        !reserved.contains(&(n as u32)),
                                        "peripheral '{periph}' uses reserved pin {n} ({next})"
                                    );
                                }
                            }
                        }
                        check_pins(child, periph, &next, reserved);
                    }
                }
                serde_json::Value::Array(arr) => {
                    for (idx, child) in arr.iter().enumerate() {
                        check_pins(child, periph, &format!("{key_path}[{idx}]"), reserved);
                    }
                }
                _ => {}
            }
        }

        let reg = default_assembly_registry();
        let asm = reg.get("esp32s3_va_wheeled_diff_assembly").unwrap();
        for p in &asm.peripherals {
            for (k, v) in &p.config {
                check_pins(v, &p.name, k, reserved);
            }
        }
    }

    #[test]
    fn peripheral_device_reference_integrity() {
        let reg = default_assembly_registry();
        for asm in reg.all() {
            let periph_names: Vec<&str> = asm.peripherals.iter().map(|p| p.name.as_str()).collect();
            for dev in &asm.devices {
                for pref in &dev.peripherals {
                    assert!(
                        periph_names.contains(&pref.as_str()),
                        "Device '{}' references unknown peripheral '{}'",
                        dev.name,
                        pref
                    );
                }
            }
        }
    }

    #[test]
    fn serialization_round_trip() {
        let reg = default_assembly_registry();
        let asm = reg.get("esp32s3_gpio_relay_assembly").unwrap();
        let json = serde_json::to_string(asm).unwrap();
        let back: HardwareAssemblyDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, asm);
    }

    // ── from_module tests ──────────────────────────────────────────────────

    #[test]
    fn extract_gpio_pins_single() {
        assert_eq!(
            super::extract_gpio_pins("FailsafeStop relay on GPIO 25"),
            vec![25]
        );
    }

    #[test]
    fn extract_gpio_pins_pair_slash() {
        assert_eq!(
            super::extract_gpio_pins("USB OTG uses GPIO 19/20"),
            vec![19, 20]
        );
    }

    #[test]
    fn extract_gpio_pins_range() {
        assert_eq!(
            super::extract_gpio_pins("Camera uses GPIO 4-15 (I2S data bus)"),
            vec![4, 15]
        );
    }

    #[test]
    fn extract_gpio_pins_parenthesized() {
        assert_eq!(
            super::extract_gpio_pins("IMU on I2C bus 0 (GPIO 21/22)"),
            vec![21, 22]
        );
    }

    #[test]
    fn extract_gpio_pins_no_match() {
        assert!(super::extract_gpio_pins("PSRAM required for frame buffers").is_empty());
        assert!(super::extract_gpio_pins("AP+STA shares single home channel").is_empty());
    }

    #[test]
    fn from_module_generic_wifi_board() {
        let mod_reg = crate::module::default_module_registry();
        let module = mod_reg.get("esp32s3_wroom1").unwrap();
        let asm = HardwareAssemblyDefinition::from_module(module);

        assert_eq!(asm.target, ChipTarget::Esp32S3);
        assert_eq!(asm.modules, vec!["esp32s3_wroom1"]);

        let types: Vec<&str> = asm
            .peripherals
            .iter()
            .map(|p| p.periph_type.as_str())
            .collect();
        assert!(types.contains(&"gpio"), "missing gpio peripheral");
        assert!(types.contains(&"i2c"), "missing i2c peripheral");
        assert!(types.contains(&"spi"), "missing spi peripheral");
        assert!(types.contains(&"uart"), "missing uart peripheral");
        assert!(types.contains(&"adc"), "missing adc peripheral");
        assert!(types.contains(&"ledc"), "missing ledc peripheral");
    }

    #[test]
    fn from_module_control_board() {
        let mod_reg = crate::module::default_module_registry();
        let module = mod_reg.get("esp32s3_wroom1").unwrap();
        let asm = HardwareAssemblyDefinition::from_module(module);

        let types: Vec<&str> = asm
            .peripherals
            .iter()
            .map(|p| p.periph_type.as_str())
            .collect();
        assert!(types.contains(&"mcpwm"), "missing mcpwm peripheral");

        assert!(asm
            .provided_hal_contracts
            .contains(&"motor_control".to_string()));
        assert!(asm
            .provided_hal_contracts
            .contains(&"imu_sensor".to_string()));
        assert!(asm
            .provided_hal_contracts
            .contains(&"failsafe_stop".to_string()));
    }

    #[test]
    fn from_module_preserves_target() {
        let mod_reg = crate::module::default_module_registry();
        for module in mod_reg.all() {
            let asm = HardwareAssemblyDefinition::from_module(module);
            assert_eq!(
                asm.target, module.target,
                "target mismatch for module {}",
                module.id
            );
        }
    }

    #[test]
    fn from_module_device_peripheral_refs() {
        let mod_reg = crate::module::default_module_registry();
        for module in mod_reg.all() {
            let asm = HardwareAssemblyDefinition::from_module(module);
            let periph_names: Vec<&str> = asm.peripherals.iter().map(|p| p.name.as_str()).collect();
            for dev in &asm.devices {
                for pref in &dev.peripherals {
                    assert!(
                        periph_names.contains(&pref.as_str()),
                        "Device '{}' in module '{}' references unknown peripheral '{}'",
                        dev.name,
                        module.id,
                        pref
                    );
                }
            }
        }
    }

    #[test]
    fn from_module_hal_contracts() {
        let mod_reg = crate::module::default_module_registry();
        let module = mod_reg.get("esp32s3_wroom1").unwrap();
        let asm = HardwareAssemblyDefinition::from_module(module);

        assert!(asm
            .provided_hal_contracts
            .contains(&"gpio_output".to_string()));
        assert!(asm
            .provided_hal_contracts
            .contains(&"i2c_sensor".to_string()));
    }

    #[test]
    fn from_module_camera_board_has_camera() {
        let mod_reg = crate::module::default_module_registry();
        let module = mod_reg.get("esp32s3_wroom1").unwrap();
        let asm = HardwareAssemblyDefinition::from_module(module);
        let types: Vec<&str> = asm
            .peripherals
            .iter()
            .map(|p| p.periph_type.as_str())
            .collect();
        assert!(
            types.contains(&"camera"),
            "camera_board should have camera peripheral"
        );
    }

    #[test]
    fn from_module_wroom1_has_i2c() {
        let mod_reg = crate::module::default_module_registry();
        let module = mod_reg.get("esp32s3_wroom1").unwrap();
        let asm = HardwareAssemblyDefinition::from_module(module);
        let types: Vec<&str> = asm
            .peripherals
            .iter()
            .map(|p| p.periph_type.as_str())
            .collect();
        assert!(types.contains(&"i2c"), "WROOM-1 should have i2c peripheral");
    }

    // ── Phase 4 (2026-04-20): typed-device coverage on the 4 other V&A
    // ── assemblies. Each one mirrors wheeled_diff's pattern: gpio_ctrl
    // ── for status/failsafe LED-style entries + typed devices for the
    // ── motor/sensor/link contracts; YAML emitter coerces unknown
    // ── device_types to `type: custom`.

    fn device_types_of(asm_id: &str) -> Vec<String> {
        let reg = default_assembly_registry();
        let asm = reg
            .get(asm_id)
            .unwrap_or_else(|| panic!("missing {asm_id}"));
        asm.devices.iter().map(|d| d.device_type.clone()).collect()
    }

    fn periph_type_for(asm_id: &str, periph_name: &str) -> String {
        let reg = default_assembly_registry();
        let asm = reg.get(asm_id).unwrap();
        asm.peripherals
            .iter()
            .find(|p| p.name == periph_name)
            .unwrap_or_else(|| panic!("missing peripheral {periph_name} in {asm_id}"))
            .periph_type
            .clone()
    }

    /// motor_driver devices in any V&A assembly must reference a peripheral
    /// of type `mcpwm` or `ledc` — those are the only periph types
    /// motor_backend knows how to drive.
    #[test]
    fn va_motor_devices_reference_mcpwm_or_ledc_or_dshot_gpio() {
        let reg = default_assembly_registry();
        for asm in reg.all() {
            let asm_id = &asm.id;
            for motor in asm
                .devices
                .iter()
                .filter(|d| d.device_type == "motor_driver")
            {
                assert_eq!(
                    motor.peripherals.len(),
                    1,
                    "motor_driver '{}' in '{asm_id}' must reference exactly one peripheral",
                    motor.name
                );
                let pt = periph_type_for(asm_id, &motor.peripherals[0]);
                let signaling = motor
                    .config
                    .get("signaling")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let is_dshot_gpio = pt == "gpio" && signaling.starts_with("dshot");
                assert!(
                    pt == "mcpwm" || pt == "ledc" || is_dshot_gpio,
                    "motor_driver '{}' in '{asm_id}' references '{}' (type {pt}, signaling {signaling}); supported: mcpwm, ledc, or gpio + signaling=dshot*",
                    motor.name, motor.peripherals[0]
                );
            }
        }
    }

    #[test]
    fn elrs_crsf_has_typed_devices() {
        let types = device_types_of("esp32s3_va_elrs_crsf_assembly");
        assert!(types.iter().filter(|t| *t == "gpio_ctrl").count() >= 2);
        assert!(types.contains(&"motor_driver".into()));
        assert!(types.contains(&"servo_control".into()));
        assert!(types.contains(&"crsf_link".into()));
        assert!(types.contains(&"failsafe_ctrl".into()));
        // motor on LEDC, not MCPWM (this assembly has no MCPWM peripheral).
        let pt = periph_type_for("esp32s3_va_elrs_crsf_assembly", "ledc_esc");
        assert_eq!(pt, "ledc");
    }

    #[test]
    fn multirotor_has_typed_devices_with_4_motors() {
        let types = device_types_of("esp32s3_va_multirotor_assembly");
        let motor_count = types.iter().filter(|t| *t == "motor_driver").count();
        assert_eq!(
            motor_count, 4,
            "multirotor needs exactly 4 motor_driver devices (one per ESC), got {motor_count}"
        );
        assert!(types.contains(&"imu_sensor".into()));
        assert!(types.contains(&"vbat_monitor".into()));
        assert!(types.contains(&"failsafe_ctrl".into()));
        // All 4 ESCs on LEDC.
        for i in 0..4 {
            let pt = periph_type_for("esp32s3_va_multirotor_assembly", &format!("ledc_esc_{i}"));
            assert_eq!(pt, "ledc", "ledc_esc_{i} must be LEDC");
        }
    }

    #[test]
    fn sbc_bridge_has_typed_devices_no_motors() {
        let types = device_types_of("esp32s3_va_sbc_bridge_assembly");
        assert!(types.contains(&"sbc_link".into()));
        assert!(types.contains(&"crsf_link".into()));
        assert!(types.contains(&"failsafe_ctrl".into()));
        // sbc_bridge is a pure bridge — no actuators, no motor_driver.
        assert!(
            !types.contains(&"motor_driver".into()),
            "sbc_bridge must NOT declare motor_driver, got {types:?}"
        );
    }

    #[test]
    fn marine_has_typed_devices() {
        let types = device_types_of("esp32s3_va_marine_assembly");
        assert!(types.contains(&"motor_driver".into()));
        assert!(types.contains(&"servo_control".into()));
        assert!(types.contains(&"gpio_input".into()));
        assert!(types.contains(&"failsafe_ctrl".into()));
        let pt = periph_type_for("esp32s3_va_marine_assembly", "ledc_esc");
        assert_eq!(pt, "ledc");
    }

    /// failsafe_ctrl device must exist on every V&A assembly that declares
    /// any motor_driver — without it, the SAFE_STOP contract is unmet.
    #[test]
    fn va_assemblies_with_motors_have_failsafe() {
        let reg = default_assembly_registry();
        for asm in reg.all() {
            if !asm.id.starts_with("esp32s3_va_") {
                continue;
            }
            let has_motor = asm.devices.iter().any(|d| d.device_type == "motor_driver");
            let has_failsafe = asm.devices.iter().any(|d| d.device_type == "failsafe_ctrl");
            if has_motor {
                assert!(
                    has_failsafe,
                    "{} declares motor_driver but no failsafe_ctrl",
                    asm.id
                );
            }
        }
    }
}
