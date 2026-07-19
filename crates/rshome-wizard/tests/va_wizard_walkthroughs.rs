//! End-to-end walk-throughs of the V&A wizard DAG described in the design doc
//! `type-driven-ui/docs/vehicle-aircraft-control-dag.md`.
//!
//! Each test picks a `(domain, form_factor, topology, role)` tuple the doc
//! calls out explicitly and asserts that at least one registered solution
//! satisfies all four constraints at once. This mirrors the filter logic the
//! UI runs in `solution-filter.ts::isSolutionConsistent`, but on the Rust side
//! so regressions in `SolutionRegistry::default` surface as test failures
//! rather than empty wizard lists.

use rshome_schema::platform::{
    ControlUplinkKind, DomainKind, FormFactorKind, McuRole, TopologyKind, VideoDownlinkKind,
};
use rshome_schema::solution::{default_solution_registry, SolutionDefinition};

/// Topology resolution logic used by the wizard's TS filter. Mirrors
/// `va_grid_coverage.rs::topology_of_solution` so the two lints stay
/// consistent. Pulled inline here rather than shared because both are
/// integration tests and Rust integration tests don't share modules.
fn topology_of_solution(s: &SolutionDefinition) -> Option<TopologyKind> {
    s.control_uplink
        .and_then(|u| match u {
            ControlUplinkKind::WifiCrtp
            | ControlUplinkKind::EspNow
            | ControlUplinkKind::WifiMesh
            | ControlUplinkKind::Wifi80211lr
            | ControlUplinkKind::BleMesh
            | ControlUplinkKind::BleGatt
            | ControlUplinkKind::UsbCdc => Some(TopologyKind::DiyLowcost),
            ControlUplinkKind::Crsf | ControlUplinkKind::Sbus => Some(TopologyKind::StandardFpv),
            ControlUplinkKind::WifiMavlink => Some(TopologyKind::ResearchHybrid),
            ControlUplinkKind::None => None,
            _ => None,
        })
        .or_else(|| {
            s.video_downlink.and_then(|v| match v {
                VideoDownlinkKind::MjpegHttp => Some(TopologyKind::DiyLowcost),
                VideoDownlinkKind::MjpegUart
                | VideoDownlinkKind::AnalogVtx
                | VideoDownlinkKind::DjiO4
                | VideoDownlinkKind::Hdzero
                | VideoDownlinkKind::Walksnail => Some(TopologyKind::StandardFpv),
                VideoDownlinkKind::WebrtcSbc => Some(TopologyKind::ResearchHybrid),
                VideoDownlinkKind::None => None,
                _ => None,
            })
        })
}

fn find_matching_solutions(
    reg: &rshome_schema::solution::SolutionRegistry,
    form_factor: FormFactorKind,
    topology: TopologyKind,
    role: McuRole,
) -> Vec<&SolutionDefinition> {
    reg.all()
        .filter(|s| s.domain == Some(DomainKind::VehicleAircraftControl))
        .filter(|s| s.architecture_tier == Some(role))
        .filter(|s| {
            s.form_factor_families
                .as_ref()
                .map(|families| families.contains(&form_factor))
                .unwrap_or(false)
        })
        .filter(|s| topology_of_solution(s) == Some(topology))
        .collect()
}

/// Doc verification step: "V&A → quadcopter_x → standard_fpv → control_board
/// → confirm `quad_stabilizer_solution` appears."
#[test]
fn quadcopter_x_standard_fpv_control_board_resolves_to_quad_stabilizer() {
    let reg = default_solution_registry();
    let matches = find_matching_solutions(
        &reg,
        FormFactorKind::QuadcopterX,
        TopologyKind::StandardFpv,
        McuRole::ControlBoard,
    );
    assert!(
        matches.iter().any(|s| s.id == "quad_stabilizer_solution"),
        "quad_stabilizer_solution missing from ({:?}, {:?}, {:?}); got {:?}",
        FormFactorKind::QuadcopterX,
        TopologyKind::StandardFpv,
        McuRole::ControlBoard,
        matches.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
    );
}

/// Doc verification step: "V&A → wheeled_4wd_ackermann → standard_fpv →
/// control_board → confirm `elrs_crsf_brushless_solution` appears."
#[test]
fn ackermann_standard_fpv_control_board_resolves_to_brushless_crsf() {
    let reg = default_solution_registry();
    let matches = find_matching_solutions(
        &reg,
        FormFactorKind::Wheeled4wdAckermann,
        TopologyKind::StandardFpv,
        McuRole::ControlBoard,
    );
    assert!(
        matches
            .iter()
            .any(|s| s.id == "elrs_crsf_brushless_solution"),
        "elrs_crsf_brushless_solution missing from ({:?}, {:?}, {:?}); got {:?}",
        FormFactorKind::Wheeled4wdAckermann,
        TopologyKind::StandardFpv,
        McuRole::ControlBoard,
        matches.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
    );
}

/// Doc §L1 table (line 234) routes aquatic-surface form factors to
/// `marine_surface_solution` — verify the mapping is concrete in the registry.
#[test]
fn boat_standard_fpv_control_board_resolves_to_marine_surface() {
    let reg = default_solution_registry();
    let matches = find_matching_solutions(
        &reg,
        FormFactorKind::BoatSingleRudder,
        TopologyKind::StandardFpv,
        McuRole::ControlBoard,
    );
    assert!(
        matches.iter().any(|s| s.id == "marine_surface_solution"),
        "marine_surface_solution missing from ({:?}, {:?}, {:?}); got {:?}",
        FormFactorKind::BoatSingleRudder,
        TopologyKind::StandardFpv,
        McuRole::ControlBoard,
        matches.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
    );
}

/// Doc §L1: balancing form factors require `balance_stabilizer_solution`,
/// which lives in diy_lowcost topology (esp_now uplink per §L5).
#[test]
fn balance_2wheel_diy_control_board_resolves_to_balance_stabilizer() {
    let reg = default_solution_registry();
    let matches = find_matching_solutions(
        &reg,
        FormFactorKind::Balance2wheel,
        TopologyKind::DiyLowcost,
        McuRole::ControlBoard,
    );
    assert!(
        matches
            .iter()
            .any(|s| s.id == "balance_stabilizer_solution"),
        "balance_stabilizer_solution missing from ({:?}, {:?}, {:?}); got {:?}",
        FormFactorKind::Balance2wheel,
        TopologyKind::DiyLowcost,
        McuRole::ControlBoard,
        matches.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
    );
}

/// Doc §L1: legged form factors restricted to research_hybrid topology with
/// SBC companion (`legged_controller_solution`).
#[test]
fn quadruped_research_hybrid_control_board_resolves_to_legged_controller() {
    let reg = default_solution_registry();
    let matches = find_matching_solutions(
        &reg,
        FormFactorKind::Quadruped,
        TopologyKind::ResearchHybrid,
        McuRole::ControlBoard,
    );
    assert!(
        matches.iter().any(|s| s.id == "legged_controller_solution"),
        "legged_controller_solution missing from ({:?}, {:?}, {:?}); got {:?}",
        FormFactorKind::Quadruped,
        TopologyKind::ResearchHybrid,
        McuRole::ControlBoard,
        matches.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
    );
}

/// Doc §L1: submerged-aquatic (ROV/AUV) → research_hybrid topology →
/// `rov_thruster_allocation_solution`.
#[test]
fn rov_research_hybrid_control_board_resolves_to_thruster_allocation() {
    let reg = default_solution_registry();
    let matches = find_matching_solutions(
        &reg,
        FormFactorKind::Rov4thruster,
        TopologyKind::ResearchHybrid,
        McuRole::ControlBoard,
    );
    assert!(
        matches
            .iter()
            .any(|s| s.id == "rov_thruster_allocation_solution"),
        "rov_thruster_allocation_solution missing from ({:?}, {:?}, {:?}); got {:?}",
        FormFactorKind::Rov4thruster,
        TopologyKind::ResearchHybrid,
        McuRole::ControlBoard,
        matches.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
    );
}

/// Doc §L1: fixed-wing → standard_fpv → `fixedwing_stabilizer_solution`
/// (ArduPlane lineage, glide_trim on rx_loss).
#[test]
fn flying_wing_standard_fpv_control_board_resolves_to_fixedwing_stabilizer() {
    let reg = default_solution_registry();
    let matches = find_matching_solutions(
        &reg,
        FormFactorKind::FlyingWing,
        TopologyKind::StandardFpv,
        McuRole::ControlBoard,
    );
    assert!(
        matches
            .iter()
            .any(|s| s.id == "fixedwing_stabilizer_solution"),
        "fixedwing_stabilizer_solution missing from ({:?}, {:?}, {:?}); got {:?}",
        FormFactorKind::FlyingWing,
        TopologyKind::StandardFpv,
        McuRole::ControlBoard,
        matches.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
    );
}

/// Doc §L1: VTOL → `vtol_transition_solution` with hover_hold failsafe.
#[test]
fn vtol_tiltrotor_standard_fpv_control_board_resolves_to_vtol_transition() {
    let reg = default_solution_registry();
    let matches = find_matching_solutions(
        &reg,
        FormFactorKind::VtolTiltrotor,
        TopologyKind::StandardFpv,
        McuRole::ControlBoard,
    );
    assert!(
        matches.iter().any(|s| s.id == "vtol_transition_solution"),
        "vtol_transition_solution missing from ({:?}, {:?}, {:?}); got {:?}",
        FormFactorKind::VtolTiltrotor,
        TopologyKind::StandardFpv,
        McuRole::ControlBoard,
        matches.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
    );
}

/// Doc §"Form-factor-to-solution validation rule" (line 605): a
/// `(solution, form_factor)` pair must be rejected when the form factor
/// is not in the solution's `form_factor_families[]`. Spot-check by
/// confirming `quad_stabilizer_solution` does NOT list a wheeled form
/// factor — that would indicate cross-contamination.
#[test]
fn quad_stabilizer_rejects_wheeled_form_factor() {
    let reg = default_solution_registry();
    let quad = reg
        .get("quad_stabilizer_solution")
        .expect("quad_stabilizer_solution must be registered");
    let families = quad
        .form_factor_families
        .as_ref()
        .expect("quad_stabilizer must declare form_factor_families");
    assert!(
        !families.contains(&FormFactorKind::Wheeled2wdDiff),
        "quad_stabilizer must not accept wheeled_2wd_diff"
    );
    assert!(
        !families.contains(&FormFactorKind::Wheeled4wdAckermann),
        "quad_stabilizer must not accept wheeled_4wd_ackermann"
    );
    assert!(
        !families.contains(&FormFactorKind::BoatSingleRudder),
        "quad_stabilizer must not accept boat_single_rudder"
    );
    // Positive control: the canonical multirotor.
    assert!(
        families.contains(&FormFactorKind::QuadcopterX),
        "quad_stabilizer must accept quadcopter_x (positive control)"
    );
}
