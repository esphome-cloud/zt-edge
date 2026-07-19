//! Chip-specific GPIO pin capability tables for the wizard pin picker.

use rshome_schema::ChipTarget;

use crate::types::PinInfo;

// ── Internal pin descriptor ───────────────────────────────────────────────────

struct ChipPins {
    max_gpio: u8,
    input_only: &'static [u8],
    flash_reserved: &'static [u8],
    strapping: &'static [u8],
    adc_capable: &'static [u8],
    touch_capable: &'static [u8],
}

// ADC and touch capability arrays are sourced from Espressif TRMs as of 2026.
// max_gpio / flash_reserved / strapping mirror rshome_schema::pin::capabilities_for().
// Pre-release silicon (C5, C61, P4) values are subject to TRM revision — verify
// against the latest datasheet before relying on them in production tooling.
fn chip_pins(target: ChipTarget) -> ChipPins {
    match target {
        ChipTarget::Esp32 => ChipPins {
            max_gpio: 39,
            input_only: &[34, 35, 36, 37, 38, 39],
            flash_reserved: &[6, 7, 8, 9, 10, 11],
            strapping: &[0, 2, 5, 12, 15],
            adc_capable: &[32, 33, 34, 35, 36, 37, 38, 39],
            touch_capable: &[0, 2, 4, 12, 13, 14, 15, 27, 32, 33],
        },
        ChipTarget::Esp32S2 => ChipPins {
            max_gpio: 46,
            input_only: &[46],
            flash_reserved: &[26, 27, 28, 29, 30, 31, 32],
            strapping: &[0, 45, 46],
            // ADC1 GPIO 1-10, ADC2 GPIO 11-20.
            adc_capable: &[
                1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
            ],
            // Touch T1-T14 on GPIO 1-14.
            touch_capable: &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14],
        },
        ChipTarget::Esp32S3 => ChipPins {
            max_gpio: 48,
            input_only: &[],
            flash_reserved: &[26, 27, 28, 29, 30, 31, 32],
            strapping: &[0, 3, 45, 46],
            adc_capable: &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
            touch_capable: &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14],
        },
        ChipTarget::Esp32C2 => ChipPins {
            max_gpio: 20,
            input_only: &[],
            flash_reserved: &[12, 13, 14, 15, 16, 17],
            strapping: &[8, 9],
            // ESP8684 ADC1 5 channels.
            adc_capable: &[0, 1, 2, 3, 4],
            // No touch peripheral on C2.
            touch_capable: &[],
        },
        ChipTarget::Esp32C3 => ChipPins {
            max_gpio: 21,
            input_only: &[],
            flash_reserved: &[11, 12, 13, 14, 15, 16, 17],
            strapping: &[2, 8, 9],
            // ADC1 GPIO 0-4, ADC2 GPIO 5.
            adc_capable: &[0, 1, 2, 3, 4, 5],
            // No touch peripheral on C3.
            touch_capable: &[],
        },
        ChipTarget::Esp32C5 => ChipPins {
            max_gpio: 28,
            input_only: &[],
            flash_reserved: &[24, 25, 26, 27, 28],
            strapping: &[7, 8, 27],
            // ADC1 7 channels (preliminary — verify against final TRM).
            adc_capable: &[0, 1, 2, 3, 4, 5, 6],
            touch_capable: &[],
        },
        ChipTarget::Esp32C6 => ChipPins {
            max_gpio: 30,
            input_only: &[],
            flash_reserved: &[24, 25, 26, 27, 28, 29, 30],
            strapping: &[8, 9, 15],
            adc_capable: &[0, 1, 2, 3, 4, 5, 6],
            touch_capable: &[],
        },
        ChipTarget::Esp32C61 => ChipPins {
            max_gpio: 24,
            input_only: &[],
            flash_reserved: &[12, 13, 14, 15, 16, 17],
            strapping: &[5, 8, 9],
            // ADC1 6 channels (preliminary — verify against final TRM).
            adc_capable: &[0, 1, 2, 3, 4, 5],
            touch_capable: &[],
        },
        ChipTarget::Esp32H2 => ChipPins {
            max_gpio: 27,
            input_only: &[],
            flash_reserved: &[15, 16, 17, 18, 19, 20, 21],
            strapping: &[8, 9, 25],
            // ADC1 5 channels GPIO 1-5.
            adc_capable: &[1, 2, 3, 4, 5],
            // No touch peripheral on H2.
            touch_capable: &[],
        },
        ChipTarget::Esp32P4 => ChipPins {
            max_gpio: 54,
            input_only: &[],
            flash_reserved: &[],
            strapping: &[34, 35],
            // ADC1 GPIO 16-23, ADC2 GPIO 49-54 (preliminary — verify TRM v0.4).
            adc_capable: &[16, 17, 18, 19, 20, 21, 22, 23, 49, 50, 51, 52, 53, 54],
            // Touch v3 hardware T0-T13 on GPIO 2-15 (preliminary).
            touch_capable: &[2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
        },
        // ChipTarget is #[non_exhaustive]; future variants must add an arm
        // here. Panic loudly so tests catch missing pin maps instead of
        // silently shipping zero-pin chips.
        _ => panic!("unhandled ChipTarget variant in chip_pins; add an explicit arm"),
    }
}

/// Build a full GPIO pin descriptor list for the given chip target.
pub fn chip_pin_info(target: ChipTarget) -> Vec<PinInfo> {
    let caps = chip_pins(target);
    let mut pins = Vec::new();

    for gpio in 0..=caps.max_gpio {
        let input_only = caps.input_only.contains(&gpio);
        let flash_reserved = caps.flash_reserved.contains(&gpio);
        let is_strapping = caps.strapping.contains(&gpio);
        let adc = caps.adc_capable.contains(&gpio);
        let touch = caps.touch_capable.contains(&gpio);

        let mut supported_modes = vec!["input".to_owned()];
        if !input_only && !flash_reserved {
            supported_modes.push("output".to_owned());
            supported_modes.push("pwm".to_owned());
            supported_modes.push("i2c_sda".to_owned());
            supported_modes.push("i2c_scl".to_owned());
            supported_modes.push("spi_mosi".to_owned());
            supported_modes.push("spi_miso".to_owned());
            supported_modes.push("spi_clk".to_owned());
            supported_modes.push("spi_cs".to_owned());
            supported_modes.push("uart".to_owned());
        }
        if adc {
            supported_modes.push("analog".to_owned());
        }
        if touch {
            supported_modes.push("touch".to_owned());
        }

        let mut notes = Vec::<&str>::new();
        if flash_reserved {
            notes.push("flash-reserved");
        }
        if is_strapping {
            notes.push("strapping");
        }
        if input_only {
            notes.push("input-only");
        }
        if adc {
            notes.push("ADC-capable");
        }
        if touch {
            notes.push("touch-capable");
        }

        let description = if notes.is_empty() {
            format!("GPIO {gpio}")
        } else {
            format!("GPIO {gpio} ({})", notes.join(", "))
        };

        pins.push(PinInfo {
            gpio_num: gpio,
            input_only,
            flash_reserved,
            is_strapping,
            supported_modes,
            description,
        });
    }

    pins
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esp32_pin_map_has_40_entries() {
        let pins = chip_pin_info(ChipTarget::Esp32);
        assert_eq!(pins.len(), 40, "ESP32 has GPIO 0-39 = 40 pins");
    }

    #[test]
    fn esp32_gpio34_is_input_only() {
        let pins = chip_pin_info(ChipTarget::Esp32);
        let p = pins.iter().find(|p| p.gpio_num == 34).unwrap();
        assert!(p.input_only);
        assert!(!p.supported_modes.contains(&"output".to_owned()));
    }

    #[test]
    fn esp32_gpio6_is_flash_reserved() {
        let pins = chip_pin_info(ChipTarget::Esp32);
        let p = pins.iter().find(|p| p.gpio_num == 6).unwrap();
        assert!(p.flash_reserved);
    }

    #[test]
    fn esp32_gpio2_is_strapping() {
        let pins = chip_pin_info(ChipTarget::Esp32);
        let p = pins.iter().find(|p| p.gpio_num == 2).unwrap();
        assert!(p.is_strapping);
    }

    #[test]
    fn esp32s3_pin_map_has_49_entries() {
        let pins = chip_pin_info(ChipTarget::Esp32S3);
        assert_eq!(pins.len(), 49, "ESP32-S3 has GPIO 0-48 = 49 pins");
    }

    #[test]
    fn esp32c6_pin_map_has_31_entries() {
        let pins = chip_pin_info(ChipTarget::Esp32C6);
        assert_eq!(pins.len(), 31, "ESP32-C6 has GPIO 0-30 = 31 pins");
    }

    #[test]
    fn esp32_gpio4_supports_output() {
        let pins = chip_pin_info(ChipTarget::Esp32);
        let p = pins.iter().find(|p| p.gpio_num == 4).unwrap();
        assert!(p.supported_modes.contains(&"output".to_owned()));
        assert!(!p.input_only);
        assert!(!p.flash_reserved);
    }

    #[test]
    fn pin_descriptions_non_empty() {
        let pins = chip_pin_info(ChipTarget::Esp32);
        for p in &pins {
            assert!(!p.description.is_empty());
        }
    }

    #[test]
    fn all_chip_targets_have_pin_maps() {
        for target in ChipTarget::all() {
            let pins = chip_pin_info(*target);
            assert!(
                !pins.is_empty(),
                "ChipTarget::{target:?} returned an empty pin map; add an arm in chip_pins()"
            );
        }
    }
}
