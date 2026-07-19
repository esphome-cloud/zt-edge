//! Workspace-level validation: catch cross-profile compatibility issues
//! before flash time.
//!
//! A single wizard session configures **one board**. A real deployment
//! often has multiple boards — a handheld TX + a vehicle RX, or a
//! multi-board vehicle stack (camera board + control board + SBC
//! companion). The wizard can't tell you at configure-time that those
//! boards will actually talk to each other because it only sees one
//! profile at once.
//!
//! This module's `validate_workspace()` takes a set of saved profiles
//! and reports every cross-profile incompatibility: uplink mismatch,
//! telemetry mismatch, missing pair, or GPIO conflict when two profiles
//! live on the same chip.
//!
//! Per va-residuals Phase 8 T8.1-T8.3 + T8.5 / F5.1 + F5.2. UX rendering
//! of these errors is tracked as a follow-on UX PRD (T8.4 deferred).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use rshome_schema::platform::{ChipFamilyKind, ControlUplinkKind, McuRole, TelemetryKind};
use rshome_schema::solution::{SolutionDefinition, SolutionRegistry};

use crate::pin_conflict::detect_conflicts;

/// A single saved wizard configuration — what the user built for one board.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedProfile {
    /// User-visible label (e.g. "My TX", "car-01 control board").
    pub label: String,
    /// Which chip this profile targets. Drives the pin-conflict analysis.
    pub chip_target: ChipFamilyKind,
    /// The registry solution ID the profile was built around.
    pub selected_solution_id: String,
    /// Parameter values the user set. Includes any parameterized chain
    /// hints (e.g. `phone_bridge_solution`'s `vehicle_side_protocol`).
    #[serde(default)]
    pub parameter_values: BTreeMap<String, serde_json::Value>,
}

/// A set of `SavedProfile`s the user wants to deploy together.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Workspace {
    pub profiles: Vec<SavedProfile>,
}

/// The class of incompatibility the validator found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceErrorKind {
    /// A profile references a solution ID that isn't in the registry.
    UnknownSolution,
    /// A TX-role profile exists but no matching RX-role profile agrees on
    /// `control_uplink`.
    UnmatchedTxUplink,
    /// An RX-role profile exists but no matching TX-role profile agrees on
    /// `control_uplink`.
    UnmatchedRxUplink,
    /// A TX+RX pair's `telemetry` values disagree.
    ChainTelemetryMismatch,
    /// Two profiles share a chip target and double-book a GPIO.
    PinConflict,
    /// A parameterized-uplink profile (e.g. `phone_bridge_solution`) has
    /// no `vehicle_side_protocol` parameter set, or the value doesn't
    /// map to a known uplink kind.
    ParameterizedUplinkInvalid,
}

/// A single cross-profile compatibility issue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceError {
    pub kind: WorkspaceErrorKind,
    pub message: String,
    /// Profile labels (1 or 2) implicated in this error.
    pub profiles: Vec<String>,
}

/// Validate a workspace. Returns every cross-profile compatibility issue;
/// an empty result means the workspace is internally consistent.
pub fn validate_workspace(ws: &Workspace, reg: &SolutionRegistry) -> Vec<WorkspaceError> {
    let mut errors = Vec::new();

    // Resolve each profile to its SolutionDefinition. Record unknown-solution
    // errors; don't bail — keep resolving others so we surface as much as
    // possible in one pass.
    let resolved: Vec<(&SavedProfile, Option<&SolutionDefinition>)> = ws
        .profiles
        .iter()
        .map(|p| (p, reg.get(&p.selected_solution_id)))
        .collect();

    for (profile, sol) in &resolved {
        if sol.is_none() {
            errors.push(WorkspaceError {
                kind: WorkspaceErrorKind::UnknownSolution,
                message: format!(
                    "profile '{}' references solution '{}' which is not in the registry",
                    profile.label, profile.selected_solution_id
                ),
                profiles: vec![profile.label.clone()],
            });
        }
    }

    // TX↔RX pair validation.
    errors.extend(validate_pairs(&resolved));

    // Pin-conflict across co-resident profiles on each chip.
    errors.extend(validate_pin_conflicts(&resolved));

    errors
}

fn validate_pairs(
    resolved: &[(&SavedProfile, Option<&SolutionDefinition>)],
) -> Vec<WorkspaceError> {
    let mut errors = Vec::new();

    let mut tx: Vec<(&SavedProfile, &SolutionDefinition)> = Vec::new();
    let mut rx: Vec<(&SavedProfile, &SolutionDefinition)> = Vec::new();

    for (profile, sol) in resolved {
        let Some(sol) = sol else { continue };
        match sol.architecture_tier {
            Some(McuRole::RemoteControlTx) => tx.push((profile, sol)),
            Some(McuRole::SmartphoneGateway) => tx.push((profile, sol)),
            Some(McuRole::ControlBoard)
            | Some(McuRole::ControlTelemetryBoard)
            | Some(McuRole::AllInOneCam) => rx.push((profile, sol)),
            // video_board / receiver_direct_drive don't participate in
            // TX/RX pair-matching; skip.
            _ => {}
        }
    }

    for (tx_profile, tx_sol) in &tx {
        let tx_uplink = effective_control_uplink(tx_profile, tx_sol);
        let Some(tx_uplink_str) = tx_uplink else {
            errors.push(WorkspaceError {
                kind: WorkspaceErrorKind::ParameterizedUplinkInvalid,
                message: format!(
                    "profile '{}' (solution '{}') has a parameterized control_uplink but the \
                     required parameter is missing or invalid",
                    tx_profile.label, tx_sol.id,
                ),
                profiles: vec![tx_profile.label.clone()],
            });
            continue;
        };

        let matched = rx.iter().find(|(_, rx_sol)| {
            rx_sol
                .control_uplink
                .map(uplink_to_str)
                .map(|rx_str| rx_str == tx_uplink_str)
                .unwrap_or(false)
        });

        if matched.is_none() {
            errors.push(WorkspaceError {
                kind: WorkspaceErrorKind::UnmatchedTxUplink,
                message: format!(
                    "TX profile '{}' speaks '{}' uplink; no RX profile in this workspace matches",
                    tx_profile.label, tx_uplink_str,
                ),
                profiles: vec![tx_profile.label.clone()],
            });
            continue;
        }

        // Telemetry agreement for the matched pair.
        let (rx_profile, rx_sol) = matched.unwrap();
        if let (Some(tx_t), Some(rx_t)) = (tx_sol.telemetry, rx_sol.telemetry) {
            if telemetry_to_str(tx_t) != telemetry_to_str(rx_t) {
                errors.push(WorkspaceError {
                    kind: WorkspaceErrorKind::ChainTelemetryMismatch,
                    message: format!(
                        "TX '{}' telemetry = {}; RX '{}' telemetry = {} — must agree",
                        tx_profile.label,
                        telemetry_to_str(tx_t),
                        rx_profile.label,
                        telemetry_to_str(rx_t),
                    ),
                    profiles: vec![tx_profile.label.clone(), rx_profile.label.clone()],
                });
            }
        }
    }

    // Also flag RX profiles with no TX peer — but only when an uplink
    // is declared. Video-only / passthrough RX profiles with
    // `control_uplink = None` are legitimately unpaired.
    for (rx_profile, rx_sol) in &rx {
        let Some(rx_uplink) = rx_sol.control_uplink else {
            continue;
        };
        let rx_str = uplink_to_str(rx_uplink);
        let matched = tx.iter().any(|(tx_profile, tx_sol)| {
            effective_control_uplink(tx_profile, tx_sol)
                .map(|s| s == rx_str)
                .unwrap_or(false)
        });
        if !matched {
            errors.push(WorkspaceError {
                kind: WorkspaceErrorKind::UnmatchedRxUplink,
                message: format!(
                    "RX profile '{}' expects '{}' uplink; no TX profile in this workspace provides it",
                    rx_profile.label, rx_str,
                ),
                profiles: vec![rx_profile.label.clone()],
            });
        }
    }

    errors
}

fn validate_pin_conflicts(
    resolved: &[(&SavedProfile, Option<&SolutionDefinition>)],
) -> Vec<WorkspaceError> {
    let mut errors = Vec::new();

    // Group profiles by chip_target.
    let mut by_chip: BTreeMap<ChipFamilyKind, Vec<(&SavedProfile, &SolutionDefinition)>> =
        BTreeMap::new();
    for (profile, sol) in resolved {
        let Some(sol) = sol else { continue };
        by_chip
            .entry(profile.chip_target)
            .or_default()
            .push((profile, sol));
    }

    for (chip, entries) in by_chip {
        if entries.len() < 2 {
            continue;
        }
        let solutions: Vec<&SolutionDefinition> = entries.iter().map(|(_, s)| *s).collect();
        let conflicts = detect_conflicts(&solutions, chip);

        // Map solution-id back to profile label for user-friendly output.
        let label_by_id: BTreeMap<&str, &str> = entries
            .iter()
            .map(|(p, s)| (s.id.as_str(), p.label.as_str()))
            .collect();

        for c in conflicts {
            let label_a = label_by_id
                .get(c.solutions.0.as_str())
                .copied()
                .unwrap_or(c.solutions.0.as_str());
            let label_b = label_by_id
                .get(c.solutions.1.as_str())
                .copied()
                .unwrap_or(c.solutions.1.as_str());
            errors.push(WorkspaceError {
                kind: WorkspaceErrorKind::PinConflict,
                message: format!(
                    "GPIO {} on {:?}: '{}' uses it as '{}'; '{}' uses it as '{}' — pick distinct \
                     pins or split onto separate boards",
                    c.gpio, chip, label_a, c.functions.0, label_b, c.functions.1,
                ),
                profiles: vec![label_a.to_string(), label_b.to_string()],
            });
        }
    }

    errors
}

/// Resolve a profile's effective control_uplink.
///
/// For most solutions, it's just `sol.control_uplink`. For parameterized
/// bridges (`phone_bridge_solution`), the vehicle-side uplink lives in
/// the user parameter `vehicle_side_protocol` — return that instead.
///
/// Returns `None` if a parameterized uplink is declared but unset /
/// invalid.
///
/// Exposed as `pub` so the V&A chain-presence lint suite can drive it
/// from golden fixtures per PRD `vehicle-aircraft-control-design`
/// Task 0.3 acceptance #2.
pub fn effective_control_uplink(
    profile: &SavedProfile,
    sol: &SolutionDefinition,
) -> Option<String> {
    // Parameterized bridges: return the parameter value if present, else None.
    if sol.id == "phone_bridge_solution" {
        return profile
            .parameter_values
            .get("vehicle_side_protocol")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
    }
    sol.control_uplink.map(uplink_to_str)
}

fn uplink_to_str(u: ControlUplinkKind) -> String {
    serde_json::to_value(u)
        .ok()
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| format!("{:?}", u))
}

fn telemetry_to_str(t: TelemetryKind) -> String {
    serde_json::to_value(t)
        .ok()
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| format!("{:?}", t))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rshome_schema::solution::default_solution_registry;

    fn profile(label: &str, solution_id: &str, chip: ChipFamilyKind) -> SavedProfile {
        SavedProfile {
            label: label.into(),
            chip_target: chip,
            selected_solution_id: solution_id.into(),
            parameter_values: BTreeMap::new(),
        }
    }

    #[test]
    fn unknown_solution_is_reported() {
        let reg = default_solution_registry();
        let ws = Workspace {
            profiles: vec![profile(
                "bad",
                "nonexistent_solution",
                ChipFamilyKind::Esp32S3,
            )],
        };
        let errs = validate_workspace(&ws, &reg);
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].kind, WorkspaceErrorKind::UnknownSolution);
    }

    #[test]
    fn matching_elrs_tx_rx_validates() {
        let reg = default_solution_registry();
        let ws = Workspace {
            profiles: vec![
                profile("tx", "elrs_tx_solution", ChipFamilyKind::Esp32S3),
                profile("rx", "elrs_crsf_brushed_solution", ChipFamilyKind::Esp32S3),
            ],
        };
        let errs = validate_workspace(&ws, &reg);
        // Both speak crsf + crsf_telemetry. Zero pair-match errors expected.
        let pair_errs: Vec<_> = errs
            .iter()
            .filter(|e| {
                matches!(
                    e.kind,
                    WorkspaceErrorKind::UnmatchedTxUplink
                        | WorkspaceErrorKind::UnmatchedRxUplink
                        | WorkspaceErrorKind::ChainTelemetryMismatch
                )
            })
            .collect();
        assert!(
            pair_errs.is_empty(),
            "unexpected pair errors: {:?}",
            pair_errs
        );
    }

    #[test]
    fn mismatched_uplink_is_flagged() {
        let reg = default_solution_registry();
        // elrs_tx speaks crsf; direct_control_solution speaks wifi_crtp.
        let ws = Workspace {
            profiles: vec![
                profile("tx", "elrs_tx_solution", ChipFamilyKind::Esp32S3),
                profile("rx", "direct_control_solution", ChipFamilyKind::Esp32S3),
            ],
        };
        let errs = validate_workspace(&ws, &reg);
        let has_unmatched = errs.iter().any(|e| {
            matches!(
                e.kind,
                WorkspaceErrorKind::UnmatchedTxUplink | WorkspaceErrorKind::UnmatchedRxUplink
            )
        });
        assert!(
            has_unmatched,
            "expected an Unmatched*Uplink error for crsf/wifi_crtp mismatch. Got: {:?}",
            errs
        );
    }

    #[test]
    fn phone_bridge_with_vehicle_side_protocol_matches() {
        let reg = default_solution_registry();

        let mut bridge = profile(
            "phone_bridge",
            "phone_bridge_solution",
            ChipFamilyKind::Esp32S3,
        );
        bridge.parameter_values.insert(
            "vehicle_side_protocol".into(),
            serde_json::Value::String("esp_now".into()),
        );

        // Need an RX that speaks esp_now. balance_stabilizer_solution does.
        let ws = Workspace {
            profiles: vec![
                bridge,
                profile(
                    "balance_rx",
                    "balance_stabilizer_solution",
                    ChipFamilyKind::Esp32S3,
                ),
            ],
        };
        let errs = validate_workspace(&ws, &reg);
        let has_unmatched = errs.iter().any(|e| {
            matches!(
                e.kind,
                WorkspaceErrorKind::UnmatchedTxUplink | WorkspaceErrorKind::UnmatchedRxUplink
            )
        });
        assert!(
            !has_unmatched,
            "parameterized phone_bridge → balance_stabilizer pair should validate. Got: {:?}",
            errs
        );
    }

    #[test]
    fn phone_bridge_without_parameter_flagged() {
        let reg = default_solution_registry();
        // Bridge profile with no vehicle_side_protocol param set.
        let bridge = profile(
            "phone_bridge",
            "phone_bridge_solution",
            ChipFamilyKind::Esp32S3,
        );
        let ws = Workspace {
            profiles: vec![bridge],
        };
        let errs = validate_workspace(&ws, &reg);
        assert!(
            errs.iter()
                .any(|e| e.kind == WorkspaceErrorKind::ParameterizedUplinkInvalid),
            "expected ParameterizedUplinkInvalid; got {:?}",
            errs
        );
    }

    #[test]
    fn workspace_serde_roundtrip() {
        let ws = Workspace {
            profiles: vec![profile("tx", "elrs_tx_solution", ChipFamilyKind::Esp32S3)],
        };
        let json = serde_json::to_string(&ws).expect("serialize");
        let back: Workspace = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(ws, back);
    }
}
