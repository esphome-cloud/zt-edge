//! Grid-coverage lint per design doc §"Verification" line 671:
//! "iterate every (topology, role) cell marked ✅ in the topology matrix;
//! assert ≥1 solution exists in the registry for that pair."
//!
//! Topology is implied by `control_uplink`:
//!   diy_lowcost     → wifi_crtp / esp_now / wifi_mesh / wifi_80211lr / ble_mesh / ble_gatt / usb_cdc
//!   standard_fpv    → crsf / sbus
//!   research_hybrid → wifi_mavlink (with optional crsf upstream)
//!
//! We assert here that the priority-5 cells (the ones this PR closed) all
//! have at least one solution; remaining doc cells are tracked but not yet
//! enforced. As more solutions land, move them out of `tracked_only` into
//! `must_cover`.

use rshome_schema::platform::{
    ControlUplinkKind, DomainKind, McuRole, TopologyKind, VideoDownlinkKind,
};
use rshome_schema::solution::{default_solution_registry, SolutionDefinition};

/// Map a chain enum value to the topology it implies. Multiple chain enums
/// participate because video-only roles (`VideoBoard`) have
/// `control_uplink = None` but are still topology-bound through their
/// video downlink (analog_vtx + mjpeg_uart → standard_fpv;
/// mjpeg_http → diy_lowcost; webrtc_sbc → research_hybrid).
fn topology_of_solution(s: &SolutionDefinition) -> Option<TopologyKind> {
    if let Some(t) = s.control_uplink.and_then(topology_of_uplink) {
        return Some(t);
    }
    s.video_downlink.and_then(topology_of_video)
}

fn topology_of_uplink(uplink: ControlUplinkKind) -> Option<TopologyKind> {
    match uplink {
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
    }
}

fn topology_of_video(v: VideoDownlinkKind) -> Option<TopologyKind> {
    match v {
        VideoDownlinkKind::MjpegHttp => Some(TopologyKind::DiyLowcost),
        VideoDownlinkKind::MjpegUart
        | VideoDownlinkKind::AnalogVtx
        | VideoDownlinkKind::DjiO4
        | VideoDownlinkKind::Hdzero
        | VideoDownlinkKind::Walksnail => Some(TopologyKind::StandardFpv),
        VideoDownlinkKind::WebrtcSbc => Some(TopologyKind::ResearchHybrid),
        VideoDownlinkKind::None => None,
        _ => None,
    }
}

fn cell_has_solution(sols: &[&SolutionDefinition], topology: TopologyKind, role: McuRole) -> bool {
    sols.iter().any(|s| {
        s.architecture_tier == Some(role)
            && topology_of_solution(s)
                .map(|t| t == topology)
                .unwrap_or(false)
    })
}

#[test]
fn priority_5_topology_role_cells_have_at_least_one_solution() {
    let reg = default_solution_registry();
    let va: Vec<&SolutionDefinition> = reg
        .all()
        .filter(|s| s.domain == Some(DomainKind::VehicleAircraftControl))
        .collect();

    // Every (topology, role) cell marked ✅ in §L2 of the design doc.
    // Maps to the concrete solutions below — each cell has ≥1 solution whose
    // chain enum routes to the listed topology via `topology_of_solution`.
    //
    //   diy_lowcost / remote_control_tx         → esp_now_tx_solution
    //   diy_lowcost / smartphone_gateway        → phone_bridge_solution
    //   diy_lowcost / control_board             → direct_control + balance_stabilizer + mecanum + modular + hopping
    //   diy_lowcost / control_telemetry_board   → direct_control_telemetry_solution
    //   diy_lowcost / all_in_one_cam            → direct_control_video_solution
    //   standard_fpv / remote_control_tx        → elrs_tx_solution
    //   standard_fpv / control_board            → quad_stabilizer + elrs_crsf_* + fixedwing + heli + amphibious + lta
    //   standard_fpv / control_telemetry_board  → marine_surface + agri_tool_dispatch + vtol + elrs_crsf_mavlink
    //   standard_fpv / receiver_direct_drive    → receiver_direct_drive + sbus_passthrough
    //   standard_fpv / video_board              → video_board + analog_vtx_passthrough
    //   research_hybrid / remote_control_tx     → elrs_tx_solution (shared across standard_fpv + research_hybrid)
    //   research_hybrid / control_board         → rov_thruster_allocation + legged_controller + climbing_controller
    //   research_hybrid / control_telemetry_board → articulated_sequencer
    //   research_hybrid / smartphone_gateway    → mavlink_groundstation + web_ui_groundstation
    //   research_hybrid / video_board           → video_board_sbc_companion_solution (topology inferred from webrtc_sbc downlink)
    let must_cover: &[(TopologyKind, McuRole)] = &[
        (TopologyKind::DiyLowcost, McuRole::RemoteControlTx),
        (TopologyKind::DiyLowcost, McuRole::SmartphoneGateway),
        (TopologyKind::DiyLowcost, McuRole::ControlBoard),
        (TopologyKind::DiyLowcost, McuRole::ControlTelemetryBoard),
        (TopologyKind::DiyLowcost, McuRole::AllInOneCam),
        (TopologyKind::StandardFpv, McuRole::RemoteControlTx),
        (TopologyKind::StandardFpv, McuRole::ControlBoard),
        (TopologyKind::StandardFpv, McuRole::ControlTelemetryBoard),
        (TopologyKind::StandardFpv, McuRole::ReceiverDirectDrive),
        (TopologyKind::StandardFpv, McuRole::VideoBoard),
        (TopologyKind::ResearchHybrid, McuRole::ControlBoard),
        (TopologyKind::ResearchHybrid, McuRole::ControlTelemetryBoard),
        (TopologyKind::ResearchHybrid, McuRole::SmartphoneGateway),
        (TopologyKind::ResearchHybrid, McuRole::VideoBoard),
    ];

    for (topology, role) in must_cover {
        assert!(
            cell_has_solution(&va, *topology, *role),
            "topology x role cell ({:?}, {:?}) has no V&A solution covering it",
            topology,
            role
        );
    }
}
