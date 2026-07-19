//! Typed sigrok logic analyzer component configuration.
//!
//! Provides [`SigrokConfig`] for deserialization from the component's
//! `serde_json::Value` config block, and [`ValidatedSigrokConfig`] as
//! the post-validation output consumed by the codegen.

use serde::{Deserialize, Serialize};

// ── User-facing config ──────────────────────────────────────────────────────

/// Sigrok component configuration as written by the user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SigrokConfig {
    /// GPIO pin numbers for each channel (must be 8 or 16 entries).
    #[serde(default = "default_channels")]
    pub channels: Vec<u8>,

    /// Target sample rate in Hz.
    #[serde(default = "default_sample_rate")]
    pub sample_rate_hz: u32,

    /// Maximum achievable sample rate (hardware ceiling).
    #[serde(default = "default_max_sample_rate")]
    pub max_sample_rate: u32,

    /// Capture buffer depth in samples.
    #[serde(default = "default_buffer_depth")]
    pub buffer_depth: u32,

    /// Trigger mode.
    #[serde(default)]
    pub trigger_mode: SigrokTriggerMode,

    /// USB transport backend.
    #[serde(default)]
    pub transport: SigrokTransport,
}

/// Trigger mode selection.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(tag = "type", content = "params")]
pub enum SigrokTriggerMode {
    /// No trigger — capture starts immediately on arm.
    #[default]
    Immediate,
    /// Trigger on a rising or falling edge on one channel.
    Edge {
        /// Channel index (must be in `channels`).
        channel: u8,
        /// `true` for rising edge, `false` for falling.
        rising: bool,
    },
    /// Trigger when `(sample & mask) == value`.
    Pattern {
        /// Bitmask of channels to check.
        mask: u32,
        /// Expected bit pattern.
        value: u32,
    },
}

/// USB transport backend.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum SigrokTransport {
    /// ESP32-S3 built-in USB Serial/JTAG (zero-config, default).
    #[default]
    UsbSerialJtag,
    /// USB-OTG via TinyUSB CDC (custom branding, better flush control).
    UsbOtgCdc,
}

// ── Post-validation config ──────────────────────────────────────────────────

/// Validated sigrok config — all fields checked and normalized.
#[derive(Debug, Clone, Serialize)]
pub struct ValidatedSigrokConfig {
    pub channels: Vec<u8>,
    pub channel_count: u8,
    pub sample_rate_hz: u32,
    pub max_sample_rate: u32,
    pub buffer_depth: u32,
    pub trigger_mode: SigrokTriggerMode,
    pub transport: SigrokTransport,
}

// ── Defaults ────────────────────────────────────────────────────────────────

fn default_channels() -> Vec<u8> {
    vec![4, 5, 6, 7, 15, 16, 17, 18]
}
fn default_sample_rate() -> u32 {
    1_000_000
}
fn default_max_sample_rate() -> u32 {
    10_000_000
}
fn default_buffer_depth() -> u32 {
    32_768
}

// ── Helpers ─────────────────────────────────────────────────────────────────

impl SigrokConfig {
    /// Try to deserialize from a `serde_json::Value` (the component's config block).
    pub fn from_value(value: &serde_json::Value) -> Result<Self, serde_json::Error> {
        serde_json::from_value(value.clone())
    }

    /// Produce a validated config (assumes upstream validation passed).
    pub fn into_validated(self) -> ValidatedSigrokConfig {
        let channel_count = self.channels.len() as u8;
        ValidatedSigrokConfig {
            channels: self.channels,
            channel_count,
            sample_rate_hz: self.sample_rate_hz,
            max_sample_rate: self.max_sample_rate,
            buffer_depth: self.buffer_depth,
            trigger_mode: self.trigger_mode,
            transport: self.transport,
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deserialize_minimal() {
        let v = json!({});
        let cfg = SigrokConfig::from_value(&v).unwrap();
        assert_eq!(cfg.channels.len(), 8);
        assert_eq!(cfg.sample_rate_hz, 1_000_000);
        assert_eq!(cfg.buffer_depth, 32_768);
        assert!(matches!(cfg.trigger_mode, SigrokTriggerMode::Immediate));
        assert_eq!(cfg.transport, SigrokTransport::UsbSerialJtag);
    }

    #[test]
    fn deserialize_full() {
        let v = json!({
            "channels": [1, 2, 3, 4, 5, 6, 7, 8],
            "sample_rate_hz": 4000000,
            "buffer_depth": 50000,
            "trigger_mode": { "type": "Edge", "params": { "channel": 1, "rising": true } },
            "transport": "UsbOtgCdc"
        });
        let cfg = SigrokConfig::from_value(&v).unwrap();
        assert_eq!(cfg.channels, vec![1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(cfg.sample_rate_hz, 4_000_000);
        assert_eq!(cfg.transport, SigrokTransport::UsbOtgCdc);
        assert!(matches!(
            cfg.trigger_mode,
            SigrokTriggerMode::Edge {
                channel: 1,
                rising: true
            }
        ));
    }

    #[test]
    fn deserialize_pattern_trigger() {
        let v = json!({
            "trigger_mode": { "type": "Pattern", "params": { "mask": 255, "value": 3 } }
        });
        let cfg = SigrokConfig::from_value(&v).unwrap();
        assert!(matches!(
            cfg.trigger_mode,
            SigrokTriggerMode::Pattern {
                mask: 255,
                value: 3
            }
        ));
    }

    #[test]
    fn into_validated_sets_channel_count() {
        let v = json!({"channels": [4, 5, 6, 7, 15, 16, 17, 18]});
        let cfg = SigrokConfig::from_value(&v).unwrap();
        let validated = cfg.into_validated();
        assert_eq!(validated.channel_count, 8);
    }

    #[test]
    fn roundtrip_serialize() {
        let v = json!({});
        let cfg = SigrokConfig::from_value(&v).unwrap();
        let json = serde_json::to_value(&cfg).unwrap();
        let cfg2 = SigrokConfig::from_value(&json).unwrap();
        assert_eq!(cfg.channels, cfg2.channels);
        assert_eq!(cfg.sample_rate_hz, cfg2.sample_rate_hz);
    }
}
