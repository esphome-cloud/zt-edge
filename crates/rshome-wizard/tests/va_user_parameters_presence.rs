//! V&A user_parameters presence lint per
//! Task 1.1.
//!
//! Generalizes the va-residuals Phase 6 single-solution pilot
//! (`va_parameter_schema_pilot.rs` covers `elrs_crsf_brushless_solution`)
//! into a presence lint scoped to every role where the wizard renders
//! the parameter step:
//!
//! - Vehicle-bound roles (`control_board`, `control_telemetry_board`,
//!   `all_in_one_cam`)
//! - GCS-side roles when paired with a SmartphoneGateway / VideoBoard
//!   declaring V&A intent (the 3 solutions closed by §10.1:
//!   `mavlink_groundstation_solution`, `video_board_sbc_companion_solution`,
//!   `web_ui_groundstation_solution`)
//!
//! Two layers:
//! 1. **Golden fixture replay** — every solution listed in
//!    `tests/fixtures/gcs_user_parameters_golden.json` declares
//!    at least the `expected_min_count` params, and contains every id
//!    in `expected_parameter_ids`.
//! 2. **Presence guard** — the 3 GCS solutions never regress to empty
//!    `user_parameters[]`. (Vehicle-bound roles have their own
//!    coverage in `va_parameter_schema_pilot.rs` for the canonical
//!    pilot; future expansion of that pilot scope happens here.)

use std::collections::BTreeSet;
use std::path::PathBuf;

use rshome_schema::platform::McuRole;
use rshome_schema::solution::default_solution_registry;
use serde::Deserialize;

const GCS_SOLUTIONS: &[&str] = &[
    "mavlink_groundstation_solution",
    "video_board_sbc_companion_solution",
    "web_ui_groundstation_solution",
];

#[derive(Deserialize)]
struct GoldenEntry {
    solution_id: String,
    expected_parameter_ids: Vec<String>,
    expected_min_count: usize,
}

fn fixtures_path() -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest).join("tests/fixtures/gcs_user_parameters_golden.json")
}

#[test]
fn gcs_solutions_populated() {
    let reg = default_solution_registry();
    let raw = std::fs::read_to_string(fixtures_path())
        .expect("gcs_user_parameters_golden.json must exist");
    let golden: Vec<GoldenEntry> = serde_json::from_str(&raw).expect("golden fixture must parse");

    let mut failures: Vec<String> = Vec::new();
    for entry in &golden {
        let Some(sol) = reg.get(&entry.solution_id) else {
            failures.push(format!("{}: not in registry", entry.solution_id));
            continue;
        };
        if sol.user_parameters.len() < entry.expected_min_count {
            failures.push(format!(
                "{}: has {} user_parameters, expected ≥ {}",
                entry.solution_id,
                sol.user_parameters.len(),
                entry.expected_min_count,
            ));
        }
        let actual_ids: BTreeSet<&str> =
            sol.user_parameters.iter().map(|p| p.id.as_str()).collect();
        for expected in &entry.expected_parameter_ids {
            if !actual_ids.contains(expected.as_str()) {
                failures.push(format!(
                    "{}: expected parameter `{}` missing; present: {:?}",
                    entry.solution_id, expected, actual_ids,
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "GCS user_parameters golden mismatches:\n  {}",
        failures.join("\n  "),
    );
}

#[test]
fn no_gcs_solution_ships_with_empty_user_parameters() {
    let reg = default_solution_registry();
    let mut empty: Vec<&str> = Vec::new();
    for id in GCS_SOLUTIONS {
        let Some(sol) = reg.get(id) else {
            panic!("GCS solution `{}` missing from registry", id);
        };
        if sol.user_parameters.is_empty() {
            empty.push(id);
        }
    }
    assert!(
        empty.is_empty(),
        "GCS solutions still ship with empty user_parameters[] — Phase 1 Task 1.1 regression: {:?}",
        empty,
    );
}

#[test]
fn every_gcs_parameter_has_non_empty_label_and_description() {
    let reg = default_solution_registry();
    let mut violations: Vec<String> = Vec::new();
    for id in GCS_SOLUTIONS {
        let sol = reg.get(id).expect("GCS solution must exist");
        for p in &sol.user_parameters {
            if p.label.is_empty() {
                violations.push(format!("{}::{}: empty label", id, p.id));
            }
            if p.description.is_empty() {
                violations.push(format!("{}::{}: empty description", id, p.id));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "GCS user_parameters with empty label/description:\n  {}",
        violations.join("\n  "),
    );
}

#[test]
fn every_in_scope_solution_declares_user_parameters() {
    // ADR-021 V2 broadening: every solution whose architecture_tier sits
    // in the in-scope set — vehicle-bound roles (ControlBoard,
    // ControlTelemetryBoard, AllInOneCam per `McuRole::is_vehicle_bound`)
    // OR GCS-side roles that render a parameter step in the wizard
    // (SmartphoneGateway, VideoBoard) — must declare ≥ 1 user parameter.
    //
    // Solutions with `architecture_tier: None` are non-V&A and naturally
    // skipped (e.g. `bus_sampler_solution` lives in `IotDeviceTooling`).
    // Out-of-scope V&A roles (RemoteControlTx, ReceiverDirectDrive) have
    // no wizard parameter step and are exempt — the wizard's TX-side and
    // passthrough flows don't surface a parameter card today.
    //
    // Generalizes the narrow `no_gcs_solution_ships_with_empty_user_parameters`
    // (3-solution allow-list above) + `va_parameter_schema_pilot.rs`
    // (single ControlBoard pilot `elrs_crsf_brushless_solution`) into a
    // registry-wide check. Closes the V2 follow-up flagged in
    // `governance/adr-021-gcs-user-parameters-rollout.md`.
    let reg = default_solution_registry();

    let mut violations: Vec<String> = Vec::new();
    for sol in reg.all() {
        let Some(role) = sol.architecture_tier else {
            continue;
        };
        let in_scope = role.is_vehicle_bound()
            || matches!(role, McuRole::SmartphoneGateway | McuRole::VideoBoard);
        if !in_scope {
            continue;
        }
        if sol.user_parameters.is_empty() {
            violations.push(format!(
                "{} (role: {:?}): empty user_parameters[]",
                sol.id, role,
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "V&A in-scope solutions with empty user_parameters[] (ADR-021 V2 broadening):\n  {}",
        violations.join("\n  "),
    );
}

#[test]
fn webrtc_signaling_url_is_only_on_web_ui_groundstation() {
    // The PRD scopes webrtc_signaling_url to web_ui_groundstation_solution
    // only — the other two GCS solutions don't talk WebRTC. Catches a
    // future copy-paste that drops it on a solution that doesn't host
    // a browser signaling channel.
    let reg = default_solution_registry();
    let with_signaling: Vec<&str> = GCS_SOLUTIONS
        .iter()
        .filter(|id| {
            reg.get(id)
                .map(|s| {
                    s.user_parameters
                        .iter()
                        .any(|p| p.id == "webrtc_signaling_url")
                })
                .unwrap_or(false)
        })
        .copied()
        .collect();
    assert_eq!(
        with_signaling,
        vec!["web_ui_groundstation_solution"],
        "webrtc_signaling_url is scoped to web_ui_groundstation_solution only; \
         found on: {:?}",
        with_signaling,
    );
}
