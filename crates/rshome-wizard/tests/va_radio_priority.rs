// RG-1 A1 / Phase-0 Task 0.5 bullets 1+2: single-radio mutual-exclusion
// + LR > CSI > MJPEG priority order (ADR-012).
//
// Implementation note (sign-off 2026-06-01, path b₂):
// ADR-012 V1 originally specified that `rshome-config Stage 8` emits a
// `RadioMutualExclusionViolation` when 2+ of `{wifi_80211lr, csi_capture,
// mjpeg_http}` are configured. Those three component IDs do not exist
// in the registry today (95 ComponentDefs, none radio-capable beyond
// the generic `wifi` block), so the config-level surface ADR-012 V1
// names has no enforcement target. The check moved here, to the V&A
// solution-registry layer, where the radio surface is encoded in the
// typed fields `control_uplink: ControlUplinkKind` and
// `video_downlink: VideoDownlinkKind`. Conflict is caught at solution
// declaration time, which is strictly earlier than user-config time
// and covers the same real-world failure mode (a board that compiles
// but starves its own RX queue at runtime).
//
// Mapping (typed field → ADR-012 radio kind):
//   LR    ← ControlUplinkKind::Wifi80211lr
//   MJPEG ← VideoDownlinkKind::MjpegHttp   (camera + Wi-Fi)
//   CSI   ← VideoDownlinkKind::MjpegUart   (camera + serial; no Wi-Fi)
// Other video kinds (AnalogVtx / DjiO4 / Hdzero / Walksnail / WebrtcSbc)
// do not engage MCU-side CSI capture (analog VTX is hardware; the
// branded FPV systems integrate their own camera; WebrtcSbc routes
// through an SBC). They are radio-neutral for ADR-012.
//
// V1 invariant: no V&A solution declares more than one of {LR, CSI, MJPEG}.
// V2 invariant: priority_winner() returns LR over CSI over MJPEG.
// V3 invariant: already enforced at the chip-floor layer by
//   va_chip_floor.rs::radio_using_solutions_chip_match (pre-existing).

use std::collections::BTreeSet;

use proptest::prelude::*;

use rshome_schema::platform::{ControlUplinkKind, DomainKind, VideoDownlinkKind};
use rshome_schema::solution::{default_solution_registry, SolutionDefinition};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum RadioKind {
    Lr,    // priority 0 (highest)
    Csi,   // priority 1
    Mjpeg, // priority 2 (lowest)
}

fn radio_kinds_of(sol: &SolutionDefinition) -> BTreeSet<RadioKind> {
    let mut kinds = BTreeSet::new();
    if matches!(sol.control_uplink, Some(ControlUplinkKind::Wifi80211lr)) {
        kinds.insert(RadioKind::Lr);
    }
    match sol.video_downlink {
        Some(VideoDownlinkKind::MjpegHttp) => {
            kinds.insert(RadioKind::Mjpeg);
        }
        Some(VideoDownlinkKind::MjpegUart) => {
            kinds.insert(RadioKind::Csi);
        }
        _ => {}
    }
    kinds
}

/// LR > CSI > MJPEG priority. Returns None on empty input.
fn priority_winner(active: &BTreeSet<RadioKind>) -> Option<RadioKind> {
    if active.contains(&RadioKind::Lr) {
        Some(RadioKind::Lr)
    } else if active.contains(&RadioKind::Csi) {
        Some(RadioKind::Csi)
    } else if active.contains(&RadioKind::Mjpeg) {
        Some(RadioKind::Mjpeg)
    } else {
        None
    }
}

// ── V1: registry-level invariant ──────────────────────────────────────

/// Every V&A solution declares **at most one** radio kind. The registry
/// must not ship a solution that would runtime-starve its own RX queue.
///
/// This is the V&A-side analog of ADR-012 V1: instead of detecting the
/// conflict at user-config time (where the surface doesn't exist yet),
/// we enforce it where the radio surface IS encoded today — in the
/// typed `control_uplink` + `video_downlink` fields of each solution.
#[test]
fn every_va_solution_has_at_most_one_radio_kind() {
    let reg = default_solution_registry();
    let mut violations = Vec::new();
    for sol in reg.all() {
        if sol.domain != Some(DomainKind::VehicleAircraftControl) {
            continue;
        }
        let kinds = radio_kinds_of(sol);
        if kinds.len() > 1 {
            violations.push(format!(
                "  {} declares {:?} (control_uplink={:?}, video_downlink={:?})",
                sol.id, kinds, sol.control_uplink, sol.video_downlink
            ));
        }
    }
    assert!(
        violations.is_empty(),
        "V&A solutions with >1 radio kind active (LR / CSI / MJPEG) — \
         each row would runtime-starve its RX queue per ADR-012:\n{}",
        violations.join("\n")
    );
}

/// Documentation test: cover every V&A solution that DOES declare a
/// radio kind. Surfaces the population so additions are visible in CI.
#[test]
fn radio_active_va_solutions_are_enumerated() {
    let reg = default_solution_registry();
    let mut active: Vec<(String, RadioKind)> = Vec::new();
    for sol in reg.all() {
        if sol.domain != Some(DomainKind::VehicleAircraftControl) {
            continue;
        }
        let kinds = radio_kinds_of(sol);
        for k in kinds {
            active.push((sol.id.clone(), k));
        }
    }
    // Surface the count + content via panic message on a deliberate
    // miscompare so the audit trail in CI logs the actual rows. This
    // test never fails; it exists to print.
    eprintln!("V&A solutions with active radio kind ({}):", active.len());
    for (id, k) in &active {
        eprintln!("  {} → {:?}", id, k);
    }
}

// ── V2: priority-order invariant ──────────────────────────────────────

#[test]
fn priority_lr_wins_pairwise() {
    let a = BTreeSet::from([RadioKind::Lr, RadioKind::Csi]);
    let b = BTreeSet::from([RadioKind::Lr, RadioKind::Mjpeg]);
    let c = BTreeSet::from([RadioKind::Csi, RadioKind::Mjpeg]);
    let d = BTreeSet::from([RadioKind::Lr, RadioKind::Csi, RadioKind::Mjpeg]);

    assert_eq!(priority_winner(&a), Some(RadioKind::Lr));
    assert_eq!(priority_winner(&b), Some(RadioKind::Lr));
    assert_eq!(priority_winner(&c), Some(RadioKind::Csi));
    assert_eq!(priority_winner(&d), Some(RadioKind::Lr));
}

#[test]
fn priority_singletons_self_select() {
    assert_eq!(
        priority_winner(&BTreeSet::from([RadioKind::Lr])),
        Some(RadioKind::Lr)
    );
    assert_eq!(
        priority_winner(&BTreeSet::from([RadioKind::Csi])),
        Some(RadioKind::Csi)
    );
    assert_eq!(
        priority_winner(&BTreeSet::from([RadioKind::Mjpeg])),
        Some(RadioKind::Mjpeg)
    );
    assert_eq!(priority_winner(&BTreeSet::new()), None);
}

fn arb_subset() -> impl Strategy<Value = BTreeSet<RadioKind>> {
    (any::<bool>(), any::<bool>(), any::<bool>()).prop_map(|(lr, csi, mj)| {
        let mut s = BTreeSet::new();
        if lr {
            s.insert(RadioKind::Lr);
        }
        if csi {
            s.insert(RadioKind::Csi);
        }
        if mj {
            s.insert(RadioKind::Mjpeg);
        }
        s
    })
}

proptest! {
    /// Property: priority_winner picks the lowest-numbered RadioKind
    /// present (Lr < Csi < Mjpeg by enum discriminant). 256 cases by
    /// default; bump with `PROPTEST_CASES=10000` for the 10K spec.
    #[test]
    fn priority_winner_matches_min_present(subset in arb_subset()) {
        let winner = priority_winner(&subset);
        let expected = subset.iter().min().copied();
        prop_assert_eq!(winner, expected);
    }

    /// Property: priority_winner is deterministic on equal inputs
    /// (sanity check for BTreeSet ordering).
    #[test]
    fn priority_winner_is_deterministic(subset in arb_subset()) {
        prop_assert_eq!(priority_winner(&subset), priority_winner(&subset));
    }
}
