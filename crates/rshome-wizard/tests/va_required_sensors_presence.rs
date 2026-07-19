//! Non-IMU sensor-requirement lint per va-residuals Phase 3 T3.1 / ADR-06.
//!
//! Every V&A solution whose form-factor family has a known non-IMU sensor
//! dependency must declare `required_sensors` populated with the appropriate
//! enum variant. Catches the case where a new agri / aquatic-submerged /
//! climbing / amphibious / construction solution ships with the default
//! empty `required_sensors: vec![]`.
//!
//! Pairs with schema-side field `SolutionDefinition.required_sensors` and
//! the TS mirror `SolutionInfo.required_sensors`.

use rshome_schema::platform::{DomainKind, FormFactorKind, SensorRequirement};
use rshome_schema::solution::default_solution_registry;

/// Minimum `SensorRequirement` set implied by a form-factor family. If a
/// solution's `form_factor_families` lists a form factor on the left, it
/// must declare the corresponding requirement(s) on the right.
fn implied_requirements(ff: FormFactorKind) -> &'static [SensorRequirement] {
    use FormFactorKind::*;
    use SensorRequirement::*;
    match ff {
        // Agricultural — GPS (RTK for precision, handled via solution-specific override)
        AutonomousMower | SprayerSpot | TractorTowedImplement => &[Gps],
        // Aquatic submerged — depth
        Rov4thruster | Rov6thruster | AuvTorpedo => &[Depth],
        // Climbing — adhesion state
        WallClimbingSuction | CableClimbing | MagneticClimber => &[Adhesion],
        // Amphibious — water-contact
        AmphibiousWheelsPlusProp => &[WaterContact],
        // Construction — joint encoders
        ExcavatorArm | CraneBoom | SkidSteerLoader => &[JointEncoder],
        // Legged — joint encoders
        BipedHumanoid | Quadruped | Hexapod | Octopod => &[JointEncoder],
        // Soft/continuum — pressure/strain per chamber
        SoftGripper | TentacleArm => &[PressureStrain],
        _ => &[],
    }
}

#[test]
fn form_factor_implies_required_sensors() {
    let reg = default_solution_registry();
    let mut missing: Vec<String> = Vec::new();

    for sol in reg.all() {
        if sol.domain != Some(DomainKind::VehicleAircraftControl) {
            continue;
        }
        let Some(ffs) = &sol.form_factor_families else {
            continue;
        };
        for &ff in ffs {
            let required = implied_requirements(ff);
            for &req in required {
                if !sol.required_sensors.contains(&req) {
                    missing.push(format!(
                        "{}: form factor {:?} implies {:?} but required_sensors does not include it",
                        sol.id, ff, req
                    ));
                }
            }
        }
    }

    assert!(
        missing.is_empty(),
        "V&A solutions missing form-factor-implied `required_sensors`:\n  {}",
        missing.join("\n  "),
    );
}
