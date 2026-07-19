//! Stage 5 — Preload (platform + framework detection).
//!
//! Parses the `esphome:` block to determine:
//! - [`ChipTarget`] — ESP32, ESP32-S3, or ESP32-C6
//! - [`FrameworkType`] — ESP-IDF (default) or Arduino
//!
//! The resulting [`PreloadContext`] is passed to later stages.

use rshome_schema::ChipTarget;

use crate::error::{ValidationError, ValidationStage};
use crate::raw::RawConfig;
use crate::validated::FrameworkType;

// ── PreloadContext ─────────────────────────────────────────────────────────────

/// Output of Stage 5: resolved hardware and firmware context.
#[derive(Debug, Clone)]
pub struct PreloadContext {
    /// Resolved chip target.
    pub chip_target: ChipTarget,
    /// Board identifier string (e.g. `"esp32dev"`).
    pub board: String,
    /// Resolved firmware framework.
    pub framework_type: FrameworkType,
    /// Framework version string (optional).
    pub framework_version: Option<String>,
}

// ── Stage entry point ─────────────────────────────────────────────────────────

/// Stage 5: parse `esphome:` block and produce a [`PreloadContext`].
///
/// Returns `Ok(ctx)` on success, or `Err(errors)` if the platform is unknown
/// or the `name` field is empty.
pub fn stage_5_preload_esphome_block(
    config: &RawConfig,
) -> Result<PreloadContext, Vec<ValidationError>> {
    let mut errors = Vec::new();
    let esphome = &config.esphome;

    // Validate device name.
    if esphome.name.trim().is_empty() {
        errors.push(ValidationError::error(
            ValidationStage::Preload,
            "esphome.name",
            "device name must not be empty",
        ));
    }

    // Resolve chip target.
    let chip_target = match esphome.platform.to_lowercase().replace('-', "").as_str() {
        "esp32" => Some(ChipTarget::Esp32),
        "esp32s2" => Some(ChipTarget::Esp32S2),
        "esp32s3" | "esp32s3box" | "esp32s3n8r8" => Some(ChipTarget::Esp32S3),
        "esp32c3" => Some(ChipTarget::Esp32C3),
        "esp32c5" => Some(ChipTarget::Esp32C5),
        "esp32c6" => Some(ChipTarget::Esp32C6),
        "esp32h2" => Some(ChipTarget::Esp32H2),
        "esp32p4" => Some(ChipTarget::Esp32P4),
        other => {
            errors.push(
                ValidationError::error(
                    ValidationStage::Preload,
                    "esphome.platform",
                    format!("unsupported platform '{other}'; supported: esp32, esp32s2, esp32s3, esp32c3, esp32c5, esp32c6, esp32h2, esp32p4"),
                )
                .with_suggestion("rshome supports ESP32, ESP32-S3, and ESP32-C6 targets only"),
            );
            None
        }
    };

    // Validate board.
    if esphome.board.trim().is_empty() {
        errors.push(ValidationError::error(
            ValidationStage::Preload,
            "esphome.board",
            "board identifier must not be empty",
        ));
    }

    // Resolve framework type.
    let (framework_type, framework_version) = match &esphome.framework {
        None => (FrameworkType::EspIdf, None),
        Some(fw) => {
            let ft = match fw.framework_type.to_lowercase().replace('-', "").as_str() {
                "espidf" | "idf" => FrameworkType::EspIdf,
                "arduino" => FrameworkType::Arduino,
                other => {
                    errors.push(ValidationError::error(
                        ValidationStage::Preload,
                        "esphome.framework.type",
                        format!("unknown framework type '{other}'; supported: esp-idf, arduino"),
                    ));
                    FrameworkType::EspIdf // fallback
                }
            };
            (ft, fw.version.clone())
        }
    };

    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(PreloadContext {
        chip_target: chip_target.unwrap(),
        board: esphome.board.clone(),
        framework_type,
        framework_version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::{EsphomeBlock, FrameworkConfig, RawConfig};

    fn base_config(platform: &str, board: &str) -> RawConfig {
        RawConfig {
            esphome: EsphomeBlock {
                name: "mydevice".into(),
                platform: platform.into(),
                board: board.into(),
                friendly_name: None,
                framework: None,
                includes: vec![],
                libraries: vec![],
                project: None,
                area: None,
                min_version: None,
                profile: None,
                solution: None,
                solution_variant: None,
            },
            packages: vec![],
            substitutions: Default::default(),
            components: vec![],
        }
    }

    #[test]
    fn esp32_platform_resolved() {
        let config = base_config("esp32", "esp32dev");
        let ctx = stage_5_preload_esphome_block(&config).unwrap();
        assert_eq!(ctx.chip_target, ChipTarget::Esp32);
        assert_eq!(ctx.board, "esp32dev");
    }

    #[test]
    fn esp32s3_platform_resolved() {
        let config = base_config("esp32s3", "esp32-s3-devkitc-1");
        let ctx = stage_5_preload_esphome_block(&config).unwrap();
        assert_eq!(ctx.chip_target, ChipTarget::Esp32S3);
    }

    #[test]
    fn esp32s3_with_dash_normalized() {
        let config = base_config("esp32-s3", "esp32-s3-devkitc-1");
        let ctx = stage_5_preload_esphome_block(&config).unwrap();
        assert_eq!(ctx.chip_target, ChipTarget::Esp32S3);
    }

    #[test]
    fn esp32c6_platform_resolved() {
        let config = base_config("esp32c6", "esp32-c6-devkitc-1");
        let ctx = stage_5_preload_esphome_block(&config).unwrap();
        assert_eq!(ctx.chip_target, ChipTarget::Esp32C6);
    }

    #[test]
    fn unsupported_platform_produces_error() {
        let config = base_config("esp8266", "nodemcu");
        let result = stage_5_preload_esphome_block(&config);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors
            .iter()
            .any(|e| e.path == "esphome.platform" && e.is_fatal()));
    }

    #[test]
    fn empty_name_produces_error() {
        let config = base_config("esp32", "esp32dev");
        let mut config = config;
        config.esphome.name = "   ".into();
        let result = stage_5_preload_esphome_block(&config);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| e.path == "esphome.name"));
    }

    #[test]
    fn empty_board_produces_error() {
        let config = base_config("esp32", "");
        let result = stage_5_preload_esphome_block(&config);
        assert!(result.is_err());
    }

    #[test]
    fn default_framework_is_esp_idf() {
        let config = base_config("esp32", "esp32dev");
        let ctx = stage_5_preload_esphome_block(&config).unwrap();
        assert_eq!(ctx.framework_type, FrameworkType::EspIdf);
        assert!(ctx.framework_version.is_none());
    }

    #[test]
    fn arduino_framework_resolved() {
        let mut config = base_config("esp32", "esp32dev");
        config.esphome.framework = Some(FrameworkConfig {
            framework_type: "arduino".into(),
            version: Some("2.0.0".into()),
            components: vec![],
            sdkconfig_options: Default::default(),
        });
        let ctx = stage_5_preload_esphome_block(&config).unwrap();
        assert_eq!(ctx.framework_type, FrameworkType::Arduino);
        assert_eq!(ctx.framework_version.as_deref(), Some("2.0.0"));
    }

    #[test]
    fn esp_idf_framework_with_version() {
        let mut config = base_config("esp32s3", "esp32-s3-devkitc-1");
        config.esphome.framework = Some(FrameworkConfig {
            framework_type: "esp-idf".into(),
            version: Some("5.3.1".into()),
            components: vec![],
            sdkconfig_options: Default::default(),
        });
        let ctx = stage_5_preload_esphome_block(&config).unwrap();
        assert_eq!(ctx.framework_type, FrameworkType::EspIdf);
        assert_eq!(ctx.framework_version.as_deref(), Some("5.3.1"));
    }
}
