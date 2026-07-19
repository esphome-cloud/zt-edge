//! GPIO and peripheral resource allocation model.
//!
//! Tracks which GPIO pins, I²C addresses, SPI CS pins, and UART ports are
//! claimed by which components.  Validates allocations against chip-specific
//! pin capability tables (ESP32 / ESP32-S3 / ESP32-C6).

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::platform::ChipTarget;
use crate::registry::ComponentId;

// ── Pin mode ──────────────────────────────────────────────────────────────────

/// Requested mode / function for a GPIO allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinMode {
    Input,
    Output,
    InputOutput,
    Analog,
    PwmOutput,
    I2cSda,
    I2cScl,
    SpiMosi,
    SpiMiso,
    SpiClk,
    SpiCs,
    Uart,
    Touch,
}

impl PinMode {
    /// Returns `true` if this mode requires the pin to be writable (output capable).
    pub fn requires_output(self) -> bool {
        matches!(
            self,
            Self::Output
                | Self::InputOutput
                | Self::PwmOutput
                | Self::I2cSda
                | Self::I2cScl
                | Self::SpiMosi
                | Self::SpiClk
                | Self::SpiCs
                | Self::Uart
        )
    }
}

// ── Pull mode ─────────────────────────────────────────────────────────────────

/// Internal pull resistor configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PullMode {
    Up,
    Down,
    None,
}

// ── Pin allocation ────────────────────────────────────────────────────────────

/// A single GPIO pin allocation made by a component.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PinAllocation {
    /// GPIO number (0-based).
    pub gpio_num: u8,
    /// Requested pin function.
    pub mode: PinMode,
    /// ID of the component claiming this pin.
    pub component: ComponentId,
    /// Pull resistor mode.
    pub pull_mode: Option<PullMode>,
    /// Whether the signal is inverted before/after the GPIO.
    pub inverted: bool,
}

// ── Errors ────────────────────────────────────────────────────────────────────

/// Two components attempted to allocate the same GPIO.
#[derive(Debug, Clone, Error)]
#[error(
    "GPIO {gpio_num} already allocated by '{existing_owner}' in mode {existing_mode:?}; \
     '{new_owner}' requested mode {new_mode:?}"
)]
pub struct PinConflict {
    pub gpio_num: u8,
    pub existing_owner: ComponentId,
    pub existing_mode: PinMode,
    pub new_owner: ComponentId,
    pub new_mode: PinMode,
}

/// Two components claimed the same I²C address on the same bus.
#[derive(Debug, Clone, Error)]
#[error(
    "I²C address 0x{address:02X} already claimed by '{existing_owner}'; '{new_owner}' requested it"
)]
pub struct BusConflict {
    pub address: u8,
    pub existing_owner: ComponentId,
    pub new_owner: ComponentId,
}

/// A pin allocation violates chip-specific hardware capabilities.
#[derive(Debug, Clone, Error)]
pub enum PinError {
    #[error(
        "GPIO {gpio_num} is input-only on {chip:?} but '{component}' requested output mode {mode:?}"
    )]
    InputOnlyPinUsedAsOutput {
        gpio_num: u8,
        chip: ChipTarget,
        component: ComponentId,
        mode: PinMode,
    },

    #[error("GPIO {gpio_num} does not exist on {chip:?} (max GPIO is {max_gpio})")]
    GpioOutOfRange {
        gpio_num: u8,
        chip: ChipTarget,
        max_gpio: u8,
    },

    #[error(
        "GPIO {gpio_num} is a strapping pin on {chip:?} and claimed by '{component}'; \
         boot reliability may be affected"
    )]
    StrappingPinWarning {
        gpio_num: u8,
        chip: ChipTarget,
        component: ComponentId,
    },

    #[error(
        "GPIO {gpio_num} is reserved for internal flash on {chip:?} and cannot be used by '{component}'"
    )]
    FlashReservedPin {
        gpio_num: u8,
        chip: ChipTarget,
        component: ComponentId,
    },
}

// ── Chip capability tables ────────────────────────────────────────────────────

/// Chip-specific GPIO capability metadata.
pub(crate) struct ChipCapabilities {
    /// Highest valid GPIO number (inclusive).
    max_gpio: u8,
    /// GPIO pins that are input-only (cannot drive output).
    input_only: &'static [u8],
    /// GPIO pins reserved for internal flash (QSPI / SPI0/1).
    flash_reserved: &'static [u8],
    /// Strapping pins — can be used but affect boot mode if driven at startup.
    strapping: &'static [u8],
}

pub(crate) fn capabilities_for(chip: ChipTarget) -> ChipCapabilities {
    match chip {
        ChipTarget::Esp32 => ChipCapabilities {
            // GPIO 0-39; 34-39 are input-only; 6-11 reserved for internal flash.
            max_gpio: 39,
            input_only: &[34, 35, 36, 37, 38, 39],
            flash_reserved: &[6, 7, 8, 9, 10, 11],
            strapping: &[0, 2, 5, 12, 15],
        },
        ChipTarget::Esp32S2 => ChipCapabilities {
            // GPIO 0-46; GPIO 46 is input-only; 26-32 reserved for flash/psram.
            max_gpio: 46,
            input_only: &[46],
            flash_reserved: &[26, 27, 28, 29, 30, 31, 32],
            strapping: &[0, 45, 46],
        },
        ChipTarget::Esp32S3 => ChipCapabilities {
            // GPIO 0-48; no input-only pins; 26-32 reserved for octal flash/psram.
            max_gpio: 48,
            input_only: &[],
            flash_reserved: &[26, 27, 28, 29, 30, 31, 32],
            strapping: &[0, 3, 45, 46],
        },
        ChipTarget::Esp32C2 => ChipCapabilities {
            // GPIO 0-20; no input-only; 12-17 reserved for internal SiP flash.
            max_gpio: 20,
            input_only: &[],
            flash_reserved: &[12, 13, 14, 15, 16, 17],
            strapping: &[8, 9],
        },
        ChipTarget::Esp32C3 => ChipCapabilities {
            // GPIO 0-21; no input-only; 11-17 reserved for flash.
            max_gpio: 21,
            input_only: &[],
            flash_reserved: &[11, 12, 13, 14, 15, 16, 17],
            strapping: &[2, 8, 9],
        },
        ChipTarget::Esp32C5 => ChipCapabilities {
            // GPIO 0-28; no input-only; 24-28 reserved for flash.
            max_gpio: 28,
            input_only: &[],
            flash_reserved: &[24, 25, 26, 27, 28],
            strapping: &[7, 8, 27],
        },
        ChipTarget::Esp32C6 => ChipCapabilities {
            // GPIO 0-30; no input-only; 24-30 reserved for internal flash.
            max_gpio: 30,
            input_only: &[],
            flash_reserved: &[24, 25, 26, 27, 28, 29, 30],
            strapping: &[8, 9, 15],
        },
        ChipTarget::Esp32C61 => ChipCapabilities {
            // GPIO 0-24; no input-only; 12-17 typical SiP flash reservation.
            max_gpio: 24,
            input_only: &[],
            flash_reserved: &[12, 13, 14, 15, 16, 17],
            strapping: &[5, 8, 9],
        },
        ChipTarget::Esp32H2 => ChipCapabilities {
            // GPIO 0-27; no input-only; 15-21 reserved for flash.
            max_gpio: 27,
            input_only: &[],
            flash_reserved: &[15, 16, 17, 18, 19, 20, 21],
            strapping: &[8, 9, 25],
        },
        ChipTarget::Esp32P4 => ChipCapabilities {
            // GPIO 0-54; no input-only; no flash-reserved (external flash via SPI).
            max_gpio: 54,
            input_only: &[],
            flash_reserved: &[],
            strapping: &[34, 35],
        },
    }
}

// ── Resource tracker ──────────────────────────────────────────────────────────

/// Tracks all hardware resource allocations for a device configuration.
///
/// Call `allocate_pin` / `allocate_i2c` / etc. as components are parsed, then
/// call `validate_pin_capabilities` to check the full set against the target chip.
#[derive(Debug, Default)]
pub struct ResourceTracker {
    pin_allocations: Vec<PinAllocation>,
    i2c_addresses: Vec<(u8, ComponentId)>,
    spi_cs_pins: Vec<(u8, ComponentId)>,
    uart_ports: Vec<(u8, ComponentId)>,
}

impl ResourceTracker {
    /// Create an empty resource tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Attempt to allocate a GPIO pin.
    ///
    /// Returns `Err(PinConflict)` if the same GPIO was already allocated by a
    /// different component.  Multiple components may share a pin only when both
    /// request the same mode (this is intentionally strict — relax later if needed).
    pub fn allocate_pin(&mut self, alloc: PinAllocation) -> Result<(), PinConflict> {
        for existing in &self.pin_allocations {
            if existing.gpio_num == alloc.gpio_num {
                return Err(PinConflict {
                    gpio_num: alloc.gpio_num,
                    existing_owner: existing.component.clone(),
                    existing_mode: existing.mode,
                    new_owner: alloc.component.clone(),
                    new_mode: alloc.mode,
                });
            }
        }
        self.pin_allocations.push(alloc);
        Ok(())
    }

    /// Attempt to allocate an I²C address.
    ///
    /// Returns `Err(BusConflict)` if the address is already claimed.
    pub fn allocate_i2c(&mut self, addr: u8, component: ComponentId) -> Result<(), BusConflict> {
        for (existing_addr, existing_owner) in &self.i2c_addresses {
            if *existing_addr == addr {
                return Err(BusConflict {
                    address: addr,
                    existing_owner: existing_owner.clone(),
                    new_owner: component,
                });
            }
        }
        self.i2c_addresses.push((addr, component));
        Ok(())
    }

    /// Attempt to allocate a UART port number.
    ///
    /// Returns `Err(BusConflict)` if the port is already claimed.
    pub fn allocate_uart(&mut self, port: u8, component: ComponentId) -> Result<(), BusConflict> {
        for (existing_port, existing_owner) in &self.uart_ports {
            if *existing_port == port {
                return Err(BusConflict {
                    address: port,
                    existing_owner: existing_owner.clone(),
                    new_owner: component,
                });
            }
        }
        self.uart_ports.push((port, component));
        Ok(())
    }

    /// Validate all pin allocations against the capabilities of the target chip.
    ///
    /// Returns `Err(Vec<PinError>)` with all detected capability violations.
    /// Strapping pin warnings are included in the error list (as non-fatal diagnostics).
    pub fn validate_pin_capabilities(&self, target: ChipTarget) -> Result<(), Vec<PinError>> {
        let caps = capabilities_for(target);
        let mut errors: Vec<PinError> = Vec::new();

        for alloc in &self.pin_allocations {
            let gpio = alloc.gpio_num;

            // Range check.
            if gpio > caps.max_gpio {
                errors.push(PinError::GpioOutOfRange {
                    gpio_num: gpio,
                    chip: target,
                    max_gpio: caps.max_gpio,
                });
                continue; // Skip further checks for out-of-range pins.
            }

            // Flash-reserved.
            if caps.flash_reserved.contains(&gpio) {
                errors.push(PinError::FlashReservedPin {
                    gpio_num: gpio,
                    chip: target,
                    component: alloc.component.clone(),
                });
                continue;
            }

            // Input-only vs output mode.
            if caps.input_only.contains(&gpio) && alloc.mode.requires_output() {
                errors.push(PinError::InputOnlyPinUsedAsOutput {
                    gpio_num: gpio,
                    chip: target,
                    component: alloc.component.clone(),
                    mode: alloc.mode,
                });
            }

            // Strapping pin warning (non-fatal — still add to list for visibility).
            if caps.strapping.contains(&gpio) {
                errors.push(PinError::StrappingPinWarning {
                    gpio_num: gpio,
                    chip: target,
                    component: alloc.component.clone(),
                });
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Read-only view of all current GPIO allocations.
    pub fn pin_allocations(&self) -> &[PinAllocation] {
        &self.pin_allocations
    }

    /// Read-only view of all current I²C address claims.
    pub fn i2c_addresses(&self) -> &[(u8, ComponentId)] {
        &self.i2c_addresses
    }

    /// Read-only view of all current SPI CS pin claims.
    pub fn spi_cs_pins(&self) -> &[(u8, ComponentId)] {
        &self.spi_cs_pins
    }

    /// Read-only view of all current UART port claims.
    pub fn uart_ports(&self) -> &[(u8, ComponentId)] {
        &self.uart_ports
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn alloc(gpio: u8, mode: PinMode, comp: &str) -> PinAllocation {
        PinAllocation {
            gpio_num: gpio,
            mode,
            component: comp.into(),
            pull_mode: None,
            inverted: false,
        }
    }

    // ── PinAllocation round-trips ─────────────────────────────────────────────

    #[test]
    fn pin_allocation_roundtrip() {
        let a = PinAllocation {
            gpio_num: 4,
            mode: PinMode::I2cSda,
            component: "bme280".into(),
            pull_mode: Some(PullMode::Up),
            inverted: false,
        };
        let json = serde_json::to_string(&a).unwrap();
        let back: PinAllocation = serde_json::from_str(&json).unwrap();
        assert_eq!(a, back);
    }

    // ── allocate_pin ──────────────────────────────────────────────────────────

    #[test]
    fn allocate_pin_succeeds_for_unused_gpio() {
        let mut rt = ResourceTracker::new();
        let result = rt.allocate_pin(alloc(4, PinMode::I2cSda, "bme280"));
        assert!(result.is_ok());
    }

    #[test]
    fn allocate_pin_detects_duplicate_gpio() {
        let mut rt = ResourceTracker::new();
        rt.allocate_pin(alloc(4, PinMode::I2cSda, "bme280"))
            .unwrap();
        let result = rt.allocate_pin(alloc(4, PinMode::I2cScl, "sht3x"));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.gpio_num, 4);
        assert_eq!(err.existing_owner, "bme280");
        assert_eq!(err.new_owner, "sht3x");
    }

    #[test]
    fn allocate_pin_different_gpios_allowed() {
        let mut rt = ResourceTracker::new();
        rt.allocate_pin(alloc(21, PinMode::I2cSda, "bme280"))
            .unwrap();
        rt.allocate_pin(alloc(22, PinMode::I2cScl, "bme280"))
            .unwrap();
        assert_eq!(rt.pin_allocations().len(), 2);
    }

    // ── allocate_i2c ─────────────────────────────────────────────────────────

    #[test]
    fn allocate_i2c_succeeds_for_unused_address() {
        let mut rt = ResourceTracker::new();
        assert!(rt.allocate_i2c(0x76, "bme280".into()).is_ok());
    }

    #[test]
    fn allocate_i2c_detects_address_collision() {
        let mut rt = ResourceTracker::new();
        rt.allocate_i2c(0x76, "bme280".into()).unwrap();
        let result = rt.allocate_i2c(0x76, "bme680".into());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.address, 0x76);
        assert_eq!(err.existing_owner, "bme280");
        assert_eq!(err.new_owner, "bme680");
    }

    #[test]
    fn allocate_i2c_different_addresses_allowed() {
        let mut rt = ResourceTracker::new();
        rt.allocate_i2c(0x76, "bme280".into()).unwrap();
        rt.allocate_i2c(0x44, "sht3x".into()).unwrap();
        assert_eq!(rt.i2c_addresses().len(), 2);
    }

    // ── validate_pin_capabilities — ESP32 ─────────────────────────────────────

    #[test]
    fn esp32_input_only_pin_as_output_rejected() {
        let mut rt = ResourceTracker::new();
        rt.allocate_pin(alloc(36, PinMode::Output, "my_comp"))
            .unwrap();
        let result = rt.validate_pin_capabilities(ChipTarget::Esp32);
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, PinError::InputOnlyPinUsedAsOutput { gpio_num: 36, .. })));
    }

    #[test]
    fn esp32_input_only_pin_as_input_allowed() {
        let mut rt = ResourceTracker::new();
        rt.allocate_pin(alloc(36, PinMode::Input, "my_comp"))
            .unwrap();
        // Should produce only a strapping-pin warning (36 is not strapping on ESP32)
        // or no errors at all.
        let result = rt.validate_pin_capabilities(ChipTarget::Esp32);
        // GPIO 36 is input-only but input mode is fine — only strapping check matters.
        // GPIO 36 is not in the strapping list for ESP32, so result should be Ok.
        assert!(result.is_ok(), "errors: {:?}", result.err());
    }

    #[test]
    fn esp32_flash_reserved_pin_rejected() {
        let mut rt = ResourceTracker::new();
        rt.allocate_pin(alloc(6, PinMode::Input, "my_comp"))
            .unwrap();
        let result = rt.validate_pin_capabilities(ChipTarget::Esp32);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .iter()
            .any(|e| matches!(e, PinError::FlashReservedPin { gpio_num: 6, .. })));
    }

    #[test]
    fn esp32_strapping_pin_produces_warning() {
        let mut rt = ResourceTracker::new();
        rt.allocate_pin(alloc(2, PinMode::Output, "led")).unwrap();
        let result = rt.validate_pin_capabilities(ChipTarget::Esp32);
        // GPIO 2 is a strapping pin; should get a warning.
        let errs = result.unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, PinError::StrappingPinWarning { gpio_num: 2, .. })));
    }

    #[test]
    fn esp32_valid_output_pin_passes() {
        let mut rt = ResourceTracker::new();
        // GPIO 4 is a safe output pin on ESP32.
        rt.allocate_pin(alloc(4, PinMode::Output, "relay")).unwrap();
        let result = rt.validate_pin_capabilities(ChipTarget::Esp32);
        assert!(result.is_ok(), "errors: {:?}", result.err());
    }

    #[test]
    fn esp32_gpio_out_of_range_rejected() {
        let mut rt = ResourceTracker::new();
        rt.allocate_pin(alloc(40, PinMode::Input, "my_comp"))
            .unwrap();
        let result = rt.validate_pin_capabilities(ChipTarget::Esp32);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .iter()
            .any(|e| matches!(e, PinError::GpioOutOfRange { gpio_num: 40, .. })));
    }

    // ── validate_pin_capabilities — ESP32-S3 ─────────────────────────────────

    #[test]
    fn esp32s3_flash_reserved_rejected() {
        let mut rt = ResourceTracker::new();
        rt.allocate_pin(alloc(27, PinMode::Output, "my_comp"))
            .unwrap();
        let result = rt.validate_pin_capabilities(ChipTarget::Esp32S3);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .iter()
            .any(|e| matches!(e, PinError::FlashReservedPin { gpio_num: 27, .. })));
    }

    #[test]
    fn esp32s3_gpio_48_valid() {
        let mut rt = ResourceTracker::new();
        rt.allocate_pin(alloc(48, PinMode::Output, "my_comp"))
            .unwrap();
        let result = rt.validate_pin_capabilities(ChipTarget::Esp32S3);
        // 48 is the max; strapping pin check: 48 is not in strapping list → Ok
        assert!(result.is_ok(), "errors: {:?}", result.err());
    }

    // ── validate_pin_capabilities — ESP32-C6 ─────────────────────────────────

    #[test]
    fn esp32c6_gpio_out_of_range_rejected() {
        let mut rt = ResourceTracker::new();
        rt.allocate_pin(alloc(31, PinMode::Input, "my_comp"))
            .unwrap();
        let result = rt.validate_pin_capabilities(ChipTarget::Esp32C6);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .iter()
            .any(|e| matches!(e, PinError::GpioOutOfRange { gpio_num: 31, .. })));
    }

    // ── PinMode helpers ───────────────────────────────────────────────────────

    #[test]
    fn pin_mode_output_requires_output() {
        assert!(PinMode::Output.requires_output());
        assert!(PinMode::I2cSda.requires_output());
        assert!(PinMode::SpiMosi.requires_output());
    }

    #[test]
    fn pin_mode_input_does_not_require_output() {
        assert!(!PinMode::Input.requires_output());
        assert!(!PinMode::Analog.requires_output());
        assert!(!PinMode::Touch.requires_output());
    }

    // ── Error display ─────────────────────────────────────────────────────────

    #[test]
    fn pin_conflict_error_display() {
        let err = PinConflict {
            gpio_num: 4,
            existing_owner: "bme280".into(),
            existing_mode: PinMode::I2cSda,
            new_owner: "sht3x".into(),
            new_mode: PinMode::I2cScl,
        };
        let msg = err.to_string();
        assert!(msg.contains("4"));
        assert!(msg.contains("bme280"));
        assert!(msg.contains("sht3x"));
    }

    #[test]
    fn bus_conflict_error_display() {
        let err = BusConflict {
            address: 0x76,
            existing_owner: "bme280".into(),
            new_owner: "bme680".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("76"));
        assert!(msg.contains("bme280"));
        assert!(msg.contains("bme680"));
    }

    #[test]
    fn allocate_uart_detects_port_collision() {
        let mut rt = ResourceTracker::new();
        rt.allocate_uart(0, "gps".into()).unwrap();
        let result = rt.allocate_uart(0, "modbus".into());
        assert!(result.is_err());
    }

    // ── New target variants ──────────────────────────────────────────────────

    #[test]
    fn esp32p4_validate_does_not_panic() {
        let rt = ResourceTracker::new();
        let result = rt.validate_pin_capabilities(ChipTarget::Esp32P4);
        assert!(result.is_ok());
    }

    #[test]
    fn esp32s2_input_only_pin_46() {
        let mut rt = ResourceTracker::new();
        rt.allocate_pin(alloc(46, PinMode::Output, "led")).unwrap();
        let result = rt.validate_pin_capabilities(ChipTarget::Esp32S2);
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, PinError::InputOnlyPinUsedAsOutput { gpio_num: 46, .. })));
    }

    #[test]
    fn esp32c3_flash_reserved_rejected() {
        let mut rt = ResourceTracker::new();
        rt.allocate_pin(alloc(12, PinMode::Output, "my_comp"))
            .unwrap();
        let result = rt.validate_pin_capabilities(ChipTarget::Esp32C3);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .iter()
            .any(|e| matches!(e, PinError::FlashReservedPin { gpio_num: 12, .. })));
    }

    #[test]
    fn esp32h2_gpio_out_of_range() {
        let mut rt = ResourceTracker::new();
        rt.allocate_pin(alloc(28, PinMode::Input, "my_comp"))
            .unwrap();
        let result = rt.validate_pin_capabilities(ChipTarget::Esp32H2);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .iter()
            .any(|e| matches!(e, PinError::GpioOutOfRange { gpio_num: 28, .. })));
    }

    #[test]
    fn chip_target_serde_roundtrip_new_variants() {
        for target in [
            ChipTarget::Esp32S2,
            ChipTarget::Esp32C3,
            ChipTarget::Esp32C5,
            ChipTarget::Esp32H2,
            ChipTarget::Esp32P4,
        ] {
            let json = serde_json::to_string(&target).unwrap();
            let back: ChipTarget = serde_json::from_str(&json).unwrap();
            assert_eq!(target, back);
        }
    }
}
