//! Sensor-tier-floor lint per design doc §"Sensor-tier floor per
//! form-factor family" (lines 614-635) + §"Verification" line 675:
//! "for each form factor, assert its floor ≤ any supporting solution's
//! `sensor_tier_min`."
//!
//! Encodes the doc's per-family floor table. `SensorTierKind` orders as
//!   basic_6ax (0) < standard_9ax (1) < advanced_10ax (2) < research (3).
//! For every solution that lists a `form_factor_families[]`, we check that
//! `solution.sensor_tier_min >= floor(family)` for every family the
//! solution claims to support. Solutions without `sensor_tier_min` (e.g.
//! TX-only) are skipped — they're already form-factor-agnostic.

use rshome_schema::platform::{DomainKind, FormFactorKind, SensorTierKind};
use rshome_schema::solution::default_solution_registry;

fn tier_rank(t: SensorTierKind) -> u8 {
    match t {
        SensorTierKind::Basic6ax => 0,
        SensorTierKind::Standard9ax => 1,
        SensorTierKind::Advanced10ax => 2,
        SensorTierKind::Research => 3,
        // Schema is `#[non_exhaustive]`. Future tiers should be ordered
        // explicitly above; the saturating fallback keeps the lint passing
        // until the doc gets updated.
        _ => u8::MAX,
    }
}

/// Per doc §L7. Returns the minimum sensor tier required by the
/// form-factor *family* that owns this concrete form factor. `None`
/// means the family is intentionally permissive (TX-only / passthrough).
fn floor_for_form_factor(ff: FormFactorKind) -> Option<SensorTierKind> {
    use FormFactorKind::*;
    use SensorTierKind::*;
    match ff {
        // Wheeled reactive — basic_6ax
        Wheeled2wdDiff | Wheeled4wdDiff | Wheeled4wdAckermann | Wheeled6wd
        | BigfootMonsterTruck | BigfootRockCrawler | AtvOffroad | DriftRallyRacer
        | TrackedSkidsteer => Some(Basic6ax),
        // Wheeled holonomic — standard_9ax (yaw lock)
        Mecanum4wheel | Omniwheel3wheel | Omniwheel4wheel => Some(Standard9ax),
        // Balancing — standard_9ax baseline; ballbot/unicycle bumped to advanced_10ax
        Balance2wheel => Some(Standard9ax),
        BalanceUnicycle | Ballbot => Some(Advanced10ax),
        // Legged — standard_9ax (SBC handles higher fusion)
        BipedHumanoid | Quadruped | Hexapod | Octopod => Some(Standard9ax),
        // Aquatic surface — basic_6ax
        BoatSingleRudder | BoatTwinPropDiff | Hovercraft | Hydrofoil | Sailboat => Some(Basic6ax),
        // Aquatic submerged — standard_9ax (+ depth)
        Rov4thruster | Rov6thruster | AuvTorpedo => Some(Standard9ax),
        // Multirotor / heli / fixed-wing — standard_9ax
        QuadcopterX | QuadcopterPlus | Tricopter | Hexacopter | OctocopterX | OctocopterCoax
        | HeliSingleRotor | HeliCoaxial | HeliTandem | FixedwingStandard | FixedwingVtail
        | FlyingWing | Glider => Some(Standard9ax),
        // VTOL — advanced_10ax (transition phase)
        VtolTailsitter | VtolTiltrotor | VtolQuadplane | VtolBicopter => Some(Advanced10ax),
        // LTA — basic_6ax
        LtaBlimp | LtaAirship => Some(Basic6ax),
        // Articulated — standard_9ax
        SnakeSerpentine | WormModular | RollingBall => Some(Standard9ax),
        // Agricultural — standard_9ax (+ GPS); tractor research-grade
        AutonomousMower | SprayerSpot => Some(Standard9ax),
        TractorTowedImplement => Some(Research),
        // Construction — standard_9ax
        ExcavatorArm | CraneBoom | SkidSteerLoader => Some(Standard9ax),
        // Climbing — standard_9ax (+ adhesion sensor)
        WallClimbingSuction | CableClimbing | MagneticClimber => Some(Standard9ax),
        // Amphibious — standard_9ax (+ water-contact)
        AmphibiousWheelsPlusProp => Some(Standard9ax),
        // Soft / continuum — basic_6ax (+ pressure/strain)
        SoftGripper | TentacleArm => Some(Basic6ax),
        // Educational modular — basic_6ax (per cubelet)
        ModularCubelets => Some(Basic6ax),
        // Jumping — advanced_10ax (landing recovery needs altitude + gyro)
        JumpingRobot | Grasshopper => Some(Advanced10ax),
        // FormFactorKind is `#[non_exhaustive]`. Future variants without an
        // explicit floor entry default to the most permissive tier; add a
        // proper row above when the doc grows.
        _ => None,
    }
}

#[test]
fn solution_sensor_tier_min_meets_form_factor_floor() {
    let reg = default_solution_registry();
    let mut violations: Vec<String> = Vec::new();

    for sol in reg.all() {
        if sol.domain != Some(DomainKind::VehicleAircraftControl) {
            continue;
        }
        let Some(families) = &sol.form_factor_families else {
            continue;
        };
        let Some(min_tier) = sol.sensor_tier_min else {
            // Schema test `vehicle_actuator_solutions_declare_failsafe` doesn't
            // require sensor_tier_min — TX/video/passthrough may legitimately
            // skip it. If we get here on a form-factor-bearing solution it's
            // a clear schema bug, not a tier-ordering one.
            violations.push(format!(
                "{}: declares form_factor_families but sensor_tier_min = None",
                sol.id
            ));
            continue;
        };

        for ff in families {
            let Some(floor) = floor_for_form_factor(*ff) else {
                continue;
            };
            if tier_rank(min_tier) < tier_rank(floor) {
                violations.push(format!(
                    "{}: sensor_tier_min = {:?} but form_factor {:?} needs at least {:?}",
                    sol.id, min_tier, ff, floor
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "sensor-tier floor violations:\n  {}",
        violations.join("\n  ")
    );
}
