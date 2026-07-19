use base64::Engine as _;

/// Per-device security configuration for the Native API connection.
///
/// Each `DeviceSessionActor` is given a copy of this at construction time.
/// A device with `noise_psk: None` uses the cleartext baseline.  A device
/// with `noise_psk: Some(key)` will initiate the ESPHome Noise_NNpsk0
/// handshake before exchanging any application-level messages.
#[derive(Debug, Clone, Default)]
pub struct DeviceSecurityConfig {
    /// Optional Noise PSK.  When `Some`, the session uses the ESPHome Native API
    /// Noise encryption path.  When `None`, the cleartext path is used.
    ///
    /// The PSK must be a base64-encoded 32-byte key, matching the value produced
    /// by ESPHome's `api_encryption.key` configuration field.
    pub noise_psk: Option<String>,
}

impl DeviceSecurityConfig {
    /// Returns `true` if this device should use Noise encryption.
    pub fn uses_noise(&self) -> bool {
        self.noise_psk.is_some()
    }

    /// Decode the Noise PSK from base64 to a raw 32-byte array.
    ///
    /// Returns `None` when no PSK is configured.
    /// Returns `Some(Err(_))` when the PSK is present but cannot be decoded.
    /// Returns `Some(Ok([u8; 32]))` on success.
    pub fn decode_psk(&self) -> Option<Result<[u8; 32], PskError>> {
        let b64 = self.noise_psk.as_ref()?;
        let bytes = match base64::engine::general_purpose::STANDARD.decode(b64) {
            Ok(b) => b,
            Err(e) => return Some(Err(PskError::Base64(e.to_string()))),
        };
        if bytes.len() != 32 {
            return Some(Err(PskError::WrongLength(bytes.len())));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Some(Ok(arr))
    }
}

/// Errors that can occur when decoding a Noise PSK.
#[derive(Debug, thiserror::Error)]
pub enum PskError {
    #[error("base64 decode failed: {0}")]
    Base64(String),
    #[error("PSK must be exactly 32 bytes; got {0}")]
    WrongLength(usize),
}

/// Session-level errors specific to Native API connection security or hardening.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// Server requires Noise encryption but no PSK is configured for this device.
    #[error("server requires Noise encryption but no PSK is configured")]
    NoisePskRequired,
    /// PSK is configured but server is using the cleartext protocol.
    #[error("Noise PSK is configured but server uses cleartext")]
    UnexpectedCleartext,
    /// Noise handshake failed, typically due to incorrect PSK.
    #[error("Noise handshake failed (incorrect PSK or protocol mismatch)")]
    NoiseHandshakeFailed,
    /// Frame payload exceeded the maximum allowed size.
    #[error("frame payload too large: {0} bytes")]
    FrameTooLarge(u64),
}

/// State of a discovered-device mDNS record within the link manager.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum DiscoveryRecordState {
    /// Record was recently announced via mDNS and refreshed within 90 seconds.
    Fresh,
    /// Record has not been re-announced for ≥ 90 seconds and may no longer be active.
    Stale,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(psk: Option<&str>) -> DeviceSecurityConfig {
        DeviceSecurityConfig {
            noise_psk: psk.map(|s| s.to_string()),
        }
    }

    #[test]
    fn no_noise_psk_means_cleartext() {
        let cfg = make_config(None);
        assert!(!cfg.uses_noise());
        assert!(cfg.decode_psk().is_none());
    }

    #[test]
    fn noise_psk_some_means_noise() {
        // 32 zero bytes, base64 encoded
        let psk_b64 = base64::engine::general_purpose::STANDARD.encode([0u8; 32]);
        let cfg = make_config(Some(&psk_b64));
        assert!(cfg.uses_noise());
    }

    #[test]
    fn decode_psk_correct_length_succeeds() {
        let key = [0xABu8; 32];
        let b64 = base64::engine::general_purpose::STANDARD.encode(key);
        let cfg = make_config(Some(&b64));
        let result = cfg.decode_psk().unwrap().unwrap();
        assert_eq!(result, key);
    }

    #[test]
    fn decode_psk_wrong_length_returns_error() {
        let b64 = base64::engine::general_purpose::STANDARD.encode([0u8; 16]);
        let cfg = make_config(Some(&b64));
        let result = cfg.decode_psk().unwrap();
        assert!(matches!(result, Err(PskError::WrongLength(16))));
    }

    #[test]
    fn decode_psk_invalid_base64_returns_error() {
        let cfg = make_config(Some("not!valid!base64!!!"));
        let result = cfg.decode_psk().unwrap();
        assert!(matches!(result, Err(PskError::Base64(_))));
    }

    #[test]
    fn discovery_record_state_is_serializable() {
        let s = DiscoveryRecordState::Fresh;
        let j = serde_json::to_string(&s).unwrap();
        assert_eq!(j, "\"Fresh\"");

        let s2 = DiscoveryRecordState::Stale;
        let j2 = serde_json::to_string(&s2).unwrap();
        assert_eq!(j2, "\"Stale\"");
    }
}
