//! Companion-link presence lint per va-residuals Phase 3 T3.2 / ADR-07.
//!
//! Every V&A solution that signals SBC dependency (via chain values like
//! `wifi_mavlink` + `webrtc_sbc`, or via id ending in `_sbc_*`) must
//! declare a non-null `companion_link` (`uart` / `can` / `i2c`). This
//! guards against the implication-only style (where the chain values
//! imply an SBC but the schema doesn't say so).

use rshome_schema::platform::{
    CompanionLinkKind, ControlUplinkKind, DomainKind, VideoDownlinkKind,
};
use rshome_schema::solution::default_solution_registry;

#[test]
fn sbc_dependent_solutions_declare_companion_link() {
    let reg = default_solution_registry();
    let mut missing: Vec<String> = Vec::new();

    for sol in reg.all() {
        if sol.domain != Some(DomainKind::VehicleAircraftControl) {
            continue;
        }
        // Heuristic: if the solution uses wifi_mavlink as uplink OR webrtc_sbc
        // as video, it's SBC-dependent.
        let uplink_implies_sbc = matches!(sol.control_uplink, Some(ControlUplinkKind::WifiMavlink));
        let video_implies_sbc = matches!(sol.video_downlink, Some(VideoDownlinkKind::WebrtcSbc));
        let id_implies_sbc = sol.id.contains("sbc") || sol.id == "mcu_sbc_bridge_solution";

        if !(uplink_implies_sbc || video_implies_sbc || id_implies_sbc) {
            continue;
        }

        // Exceptions: groundstation solutions live on the SBC side (the SBC IS
        // the station) so they don't need an MCU↔SBC link of their own.
        if matches!(
            sol.id.as_str(),
            "mavlink_groundstation_solution" | "web_ui_groundstation_solution"
        ) {
            continue;
        }

        if sol.companion_link.is_none() {
            missing.push(sol.id.clone());
        }
    }

    assert!(
        missing.is_empty(),
        "V&A solutions imply SBC dependency but don't declare `companion_link`:\n  {}",
        missing.join("\n  "),
    );
}

#[test]
fn companion_link_values_are_reasonable() {
    // Per ADR-07 + per-solution populations.
    let reg = default_solution_registry();
    let cases = [
        ("mcu_sbc_bridge_solution", CompanionLinkKind::Uart),
        ("rov_thruster_allocation_solution", CompanionLinkKind::Uart),
        ("legged_controller_solution", CompanionLinkKind::Uart),
        (
            "video_board_sbc_companion_solution",
            CompanionLinkKind::Uart,
        ),
        ("articulated_sequencer_solution", CompanionLinkKind::Can),
    ];
    for (id, expected) in cases {
        let sol = reg.get(id).unwrap_or_else(|| panic!("{} missing", id));
        assert_eq!(
            sol.companion_link,
            Some(expected),
            "{} should declare companion_link = {:?}",
            id,
            expected
        );
    }
}
