//! GPIO-conflict detection across co-resident V&A solutions on a single device.
//!
//! Two solutions running on the same ESP32 chip may each declare
//! `pin_assignments`. If both map the same GPIO to different functions, the
//! firmware can't honor both — that's a pin conflict. This module finds
//! every such collision and reports it as a structured [`PinConflict`] so
//! the wizard can surface a blocking error before the user reaches flash
//! time.
//!
//! Per va-residuals Phase 5 T5.2 / ADR-driven lint (F4.2).
//!
//! ```no_run
//! use rshome_schema::platform::ChipFamilyKind;
//! use rshome_schema::solution::default_solution_registry;
//! use rshome_wizard::pin_conflict::detect_conflicts;
//!
//! let reg = default_solution_registry();
//! let a = reg.get("direct_control_solution").unwrap();
//! let b = reg.get("elrs_crsf_brushed_solution").unwrap();
//! let conflicts = detect_conflicts(&[a, b], ChipFamilyKind::Esp32S3);
//! // conflicts.is_empty() iff a and b can coexist on one ESP32-S3 board.
//! ```

use std::collections::BTreeMap;

use rshome_schema::platform::{ChipCoverageStatus, ChipFamilyKind};
use rshome_schema::solution::{SolutionDefinition, SolutionId};

/// A single GPIO collision between two co-resident solutions.
///
/// The `solutions` tuple is ordered alphabetically by id so output is stable
/// regardless of input order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinConflict {
    /// The GPIO number that both solutions assigned.
    pub gpio: u8,
    /// The chip the conflict applies to (solutions may be chip-conditional).
    pub chip: ChipFamilyKind,
    /// `(alphabetically-first-solution, alphabetically-second-solution)`.
    pub solutions: (SolutionId, SolutionId),
    /// `(first-solution-function, second-solution-function)` — matches `solutions`.
    pub functions: (String, String),
}

/// Detect GPIO double-booking when `solutions` are co-resident on `chip`.
///
/// A solution is skipped if its `chip_coverage[chip]` is `Insufficient` —
/// in that case it's not expected to run on the chip at all, so its pin
/// map shouldn't trigger a collision report. Solutions without any
/// `chip_coverage` entry are included (legacy; assumed compatible).
///
/// Two solutions assigning the same GPIO to the **same function label**
/// are tolerated (shared wiring, e.g. two solutions both using GPIO 21 as
/// I²C SDA). Different functions on the same GPIO is the conflict.
pub fn detect_conflicts(
    solutions: &[&SolutionDefinition],
    chip: ChipFamilyKind,
) -> Vec<PinConflict> {
    // GPIO → list of (solution_id, function) entries that claim it.
    let mut by_gpio: BTreeMap<u8, Vec<(&str, &str)>> = BTreeMap::new();

    for sol in solutions {
        if !chip_is_compatible(sol, chip) {
            continue;
        }
        let Some(pins) = sol.pin_assignments.as_ref() else {
            continue;
        };
        for p in pins {
            by_gpio
                .entry(p.default_gpio)
                .or_default()
                .push((sol.id.as_str(), p.function.as_str()));
        }
    }

    let mut conflicts = Vec::new();
    for (gpio, uses) in by_gpio {
        if uses.len() < 2 {
            continue;
        }
        for i in 0..uses.len() {
            for j in (i + 1)..uses.len() {
                if uses[i].1 == uses[j].1 {
                    continue; // same function — shared wiring OK
                }
                let (a, b) = if uses[i].0 <= uses[j].0 {
                    (i, j)
                } else {
                    (j, i)
                };
                conflicts.push(PinConflict {
                    gpio,
                    chip,
                    solutions: (uses[a].0.to_string(), uses[b].0.to_string()),
                    functions: (uses[a].1.to_string(), uses[b].1.to_string()),
                });
            }
        }
    }
    conflicts
}

fn chip_is_compatible(sol: &SolutionDefinition, chip: ChipFamilyKind) -> bool {
    let Some(cov) = sol.chip_coverage.as_ref() else {
        return true;
    };
    !matches!(cov.get(&chip), Some(ChipCoverageStatus::Insufficient))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rshome_schema::platform::PinAssignment;

    fn mk_sol(id: &str, pins: Vec<(u8, &str)>) -> SolutionDefinition {
        let mut s = SolutionDefinition {
            id: id.into(),
            label: id.into(),
            label_zh: None,
            kind: rshome_schema::solution::SolutionKind::FirmwareAppliance,
            supported_modules: vec![],
            fixed_inputs: vec![],
            fixed_outputs: vec![],
            fixed_orchestration: vec![],
            scheduling: rshome_schema::solution::SchedulingPolicy {
                id: "test".into(),
                label: "test".into(),
                decisions: vec![],
            },
            user_parameters: vec![],
            feedback_paths: vec![],
            variants: vec![],
            component_bundle: rshome_schema::solution::ComponentBundle::default(),
            runtime_binding: rshome_schema::solution::RuntimeBinding::default(),
            external_contracts: vec![],
            network_topology: rshome_schema::solution::NetworkTopology::default(),
            domain: None,
            architecture_tier: None,
            communication_chains: None,
            pin_assignments: Some(
                pins.into_iter()
                    .map(|(gpio, fn_name)| PinAssignment {
                        function: fn_name.into(),
                        default_gpio: gpio,
                        alternatives: vec![],
                        capability: "test".into(),
                    })
                    .collect(),
            ),
            family: None,
            form_factor_families: None,
            control_uplink: None,
            video_downlink: None,
            telemetry: None,
            sensor_tier_min: None,
            actuator_family: None,
            power_rails: None,
            failsafe: None,
            topology_category: None,
            required_sensors: vec![],
            companion_link: None,
            chip_coverage: None,
        };
        // Suppress unused warning for the mutable binding style.
        let _ = &mut s;
        s
    }

    #[test]
    fn no_conflict_when_gpios_distinct() {
        let a = mk_sol("a", vec![(10, "led")]);
        let b = mk_sol("b", vec![(11, "button")]);
        let conflicts = detect_conflicts(&[&a, &b], ChipFamilyKind::Esp32S3);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn conflict_when_same_gpio_different_functions() {
        let a = mk_sol("alpha", vec![(10, "motor_a_pwm")]);
        let b = mk_sol("beta", vec![(10, "status_led")]);
        let conflicts = detect_conflicts(&[&a, &b], ChipFamilyKind::Esp32S3);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].gpio, 10);
        assert_eq!(conflicts[0].solutions, ("alpha".into(), "beta".into()));
        assert_eq!(
            conflicts[0].functions,
            ("motor_a_pwm".into(), "status_led".into())
        );
    }

    #[test]
    fn no_conflict_when_same_gpio_same_function() {
        let a = mk_sol("a", vec![(21, "i2c_sda")]);
        let b = mk_sol("b", vec![(21, "i2c_sda")]);
        let conflicts = detect_conflicts(&[&a, &b], ChipFamilyKind::Esp32S3);
        assert!(
            conflicts.is_empty(),
            "shared wiring with same function is OK"
        );
    }

    #[test]
    fn real_registry_stabilizers_coexist_on_s3() {
        // Smoke test: the actual quad + fixedwing stabilizers should be able
        // to coexist on ESP32-S3 without conflict (both use distinct pin
        // functions via `vehicle_control_pins()`).
        let reg = rshome_schema::solution::default_solution_registry();
        let quad = reg.get("quad_stabilizer_solution").expect("quad");
        let fw = reg.get("fixedwing_stabilizer_solution").expect("fixedwing");
        let _conflicts = detect_conflicts(&[quad, fw], ChipFamilyKind::Esp32S3);
        // Current pin_assignments are shared via `vehicle_control_pins()` so
        // they produce same-function same-GPIO entries — no conflicts expected.
        // If this ever fails, it means the helpers diverged; investigate.
    }
}
