//! V&A chip-floor lint per
//! Task 0.5 acceptances #3 + #4.
//!
//! Two registry-side invariants enforced here:
//!
//! 1. **Video-downlink → S3-only chip floor (ADR-013)**: every solution
//!    with `video_downlink ∈ {mjpeg_http, mjpeg_uart, dji_o4, hdzero,
//!    walksnail}` has both `chip_coverage.esp32_c6 == insufficient` AND
//!    `chip_coverage.esp32_d0wd == insufficient`. These video paths
//!    require LCD_CAM peripheral + PSRAM, which only the S3 family
//!    provides.
//!
//! 2. **Stabilizer S3-only (ADR-013)**: the 4 attitude-stabilizing
//!    solutions (`quad_stabilizer`, `fixedwing_stabilizer`,
//!    `heli_stabilizer`, `vtol_transition`) have
//!    `chip_coverage.esp32_c6 != preferred`. These run an IMU fusion
//!    loop ≥1 kHz on Tier-S hardware (M1 of governance/perf SLOs); the
//!    single-core C6 cannot sustain the loop budget.
//!
//! **Out-of-scope for this PRD task:** PRD Task 0.5 acceptances #1 + #2
//! describe `rshome-config::Stage 8` emitting `RadioMutualExclusionViolation`
//! when a config enables both `wifi_80211lr` and `mjpeg_http`. The
//! current 13-stage pipeline has no such stage and no such error
//! variant — implementing them is new infrastructure beyond
//! "codification of already-shipping invariants." Tracked as a follow-up.

use rshome_schema::platform::{ChipCoverageStatus, ChipFamilyKind, DomainKind, VideoDownlinkKind};
use rshome_schema::solution::default_solution_registry;

/// Video downlinks that need LCD_CAM + PSRAM (S3-only). Matches ADR-013
/// §"Video downlink chip floor" verbatim.
const VIDEO_DL_REQUIRING_S3: &[VideoDownlinkKind] = &[
    VideoDownlinkKind::MjpegHttp,
    VideoDownlinkKind::MjpegUart,
    VideoDownlinkKind::DjiO4,
    VideoDownlinkKind::Hdzero,
    VideoDownlinkKind::Walksnail,
];

/// TRANSITIONAL: solutions exempt from the chip-floor invariant. Each
/// entry is a known asymmetry awaiting resolution; the comment names
/// the resolution path.
///
/// - `dual_mcu_car_solution`: declares `video_downlink: mjpeg_uart` but
///   ships with `esp32_c6: preferred` + `esp32_d0wd: caveat`. The
///   architectural rationale is dual-MCU — a C6 control-MCU paired
///   with an S3 video-MCU; the `chip_coverage` field describes the
///   *control* side. ADR-014 review needs to either (a) split the
///   coverage matrix into per-MCU columns, or (b) tighten the
///   contract so the video-downlink check looks at the video-side
///   chip only. Until then, this allow-list is the documented
///   exception.
const CHIP_FLOOR_VIDEO_KNOWN_DRIFT: &[&str] = &["dual_mcu_car_solution"];

/// Attitude-stabilizing solutions per ADR-013. Forbidden behavior:
/// `chip_coverage.esp32_c6 == Preferred` on any of these.
const STABILIZER_SOLUTIONS: &[&str] = &[
    "quad_stabilizer_solution",
    "fixedwing_stabilizer_solution",
    "heli_stabilizer_solution",
    "vtol_transition_solution",
];

#[test]
fn mjpeg_and_proprietary_video_require_s3_chip_floor() {
    let reg = default_solution_registry();
    let mut violations: Vec<String> = Vec::new();

    for sol in reg.all() {
        if sol.domain != Some(DomainKind::VehicleAircraftControl) {
            continue;
        }
        if CHIP_FLOOR_VIDEO_KNOWN_DRIFT.contains(&sol.id.as_str()) {
            continue;
        }
        let Some(vd) = sol.video_downlink else {
            continue;
        };
        if !VIDEO_DL_REQUIRING_S3.contains(&vd) {
            continue;
        }
        let Some(coverage) = &sol.chip_coverage else {
            violations.push(format!(
                "{}: chip_coverage = None on S3-floor video",
                sol.id
            ));
            continue;
        };
        for required_key in [ChipFamilyKind::Esp32C6, ChipFamilyKind::Esp32D0wd] {
            match coverage.get(&required_key) {
                Some(ChipCoverageStatus::Insufficient) => {}
                Some(other) => violations.push(format!(
                    "{}: video_downlink={:?} requires {:?} = Insufficient, found {:?} (ADR-013)",
                    sol.id, vd, required_key, other,
                )),
                None => violations.push(format!(
                    "{}: video_downlink={:?} requires {:?} entry in chip_coverage; missing",
                    sol.id, vd, required_key,
                )),
            }
        }
    }

    assert!(
        violations.is_empty(),
        "chip-floor violations ({}):\n  {}",
        violations.len(),
        violations.join("\n  "),
    );
}

#[test]
fn stabilizer_s3_only() {
    let reg = default_solution_registry();
    let mut violations: Vec<String> = Vec::new();

    for id in STABILIZER_SOLUTIONS {
        let Some(sol) = reg.get(id) else {
            violations.push(format!("{}: not in registry", id));
            continue;
        };
        let Some(coverage) = &sol.chip_coverage else {
            violations.push(format!("{}: chip_coverage = None", id));
            continue;
        };
        // C6 must NOT be Preferred — Insufficient (or Caveat as a transitional
        // value) are the documented states for a stabilizer.
        if let Some(ChipCoverageStatus::Preferred) = coverage.get(&ChipFamilyKind::Esp32C6) {
            violations.push(format!(
                "{}: chip_coverage.esp32_c6 = Preferred — stabilizers are S3-only (ADR-013)",
                id,
            ));
        }
        // S3 must be Preferred — that's the whole point.
        match coverage.get(&ChipFamilyKind::Esp32S3) {
            Some(ChipCoverageStatus::Preferred) => {}
            other => violations.push(format!(
                "{}: chip_coverage.esp32_s3 = {:?} — stabilizers require Preferred",
                id, other,
            )),
        }
    }

    assert!(
        violations.is_empty(),
        "stabilizer chip-floor violations ({}):\n  {}",
        violations.len(),
        violations.join("\n  "),
    );
}

#[test]
fn known_drift_exception_list_actually_contains_drifters() {
    // Defensive check: the CHIP_FLOOR_VIDEO_KNOWN_DRIFT list should
    // contain ONLY solutions that actually need the carve-out. If a
    // solution gets fixed (its chip_coverage is updated to match the
    // contract), this test fires to nudge removing it from the
    // allow-list — keeping the exception surface honest.
    let reg = default_solution_registry();
    let mut stale: Vec<String> = Vec::new();

    for id in CHIP_FLOOR_VIDEO_KNOWN_DRIFT {
        let Some(sol) = reg.get(id) else {
            stale.push(format!("{}: not in registry — drop from allow-list", id));
            continue;
        };
        let Some(vd) = sol.video_downlink else {
            stale.push(format!("{}: no video_downlink — drop from allow-list", id));
            continue;
        };
        if !VIDEO_DL_REQUIRING_S3.contains(&vd) {
            stale.push(format!(
                "{}: video_downlink={:?} is not in the S3-floor set — drop from allow-list",
                id, vd,
            ));
            continue;
        }
        let Some(coverage) = &sol.chip_coverage else {
            stale.push(format!("{}: no chip_coverage — drop from allow-list", id));
            continue;
        };
        let c6 = coverage.get(&ChipFamilyKind::Esp32C6);
        let d0 = coverage.get(&ChipFamilyKind::Esp32D0wd);
        // If both are now Insufficient, the carve-out is no longer needed.
        if c6 == Some(&ChipCoverageStatus::Insufficient)
            && d0 == Some(&ChipCoverageStatus::Insufficient)
        {
            stale.push(format!(
                "{}: chip_coverage now satisfies the contract — drop from allow-list",
                id,
            ));
        }
    }

    assert!(
        stale.is_empty(),
        "CHIP_FLOOR_VIDEO_KNOWN_DRIFT entries that no longer need an exception:\n  {}",
        stale.join("\n  "),
    );
}
