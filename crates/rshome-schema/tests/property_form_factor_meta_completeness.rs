//! Phase-1 Task 1.2 Property row: "for all 64 FormFactorKind variants,
//! `default_form_factor_meta()` emits exactly one entry."
//!
//! Task 1.2 acceptance #3 (`tests/property/form_factor_meta_completeness.rs`).
//! Per the established `tests/` flat-file convention in this crate, the
//! `property/` namespace is encoded as a `property_` filename prefix.
//!
//! The companion test `crates/rshome-wizard/tests/va_metadata_parity.rs`
//! already checks 64-count + uniqueness inline; this file deliberately
//! lives in `rshome-schema` (where the data is authored) and adds:
//!
//! 1. **Exhaustive variant coverage** — every Rust `FormFactorKind` variant
//!    listed in `ALL_FORM_FACTOR_KINDS` is asserted to appear in the meta
//!    set. If a variant is added to `platform.rs` without being added to
//!    the static list below, this test fails fast.
//! 2. **Serde round-trip per entry** — each `FormFactorMeta` serializes
//!    + deserializes to itself (catches private-field drift between Rust
//!    canonical form and the JSON shape consumed by `registry-data.json`).
//! 3. **`label` non-empty per entry** — each variant has a human-readable
//!    label; empty `label` would render as blank cards in the wizard L1
//!    step.
//!
//! When a new `FormFactorKind` variant lands in `platform.rs`, three places
//! need updating in the same PR:
//! - `default_form_factor_meta()` body
//! - `ALL_FORM_FACTOR_KINDS` in this file (mirrors the rshome-wizard
//!   tests/va_form_factor_parity.rs constant of the same shape)
//! - `packages/rshome-web/src/form-factors.ts`
//!
//! `va_form_factor_parity.rs` checks Rust↔TS set parity; this file checks
//! Rust↔meta completeness. Together they triangulate the 64-variant
//! invariant.

use std::collections::BTreeSet;

use rshome_schema::platform::{default_form_factor_meta, FormFactorKind};

/// All 64 Rust `FormFactorKind` variants, listed explicitly. Mirrors the
/// constant of the same name in `crates/rshome-wizard/tests/va_form_factor_parity.rs`.
/// Two places list this set; both fail loudly if either drifts from `platform.rs`.
const ALL_FORM_FACTOR_KINDS: &[FormFactorKind] = &[
    // Ground — wheeled reactive (12)
    FormFactorKind::Wheeled2wdDiff,
    FormFactorKind::Wheeled4wdDiff,
    FormFactorKind::Wheeled4wdAckermann,
    FormFactorKind::Wheeled6wd,
    FormFactorKind::Mecanum4wheel,
    FormFactorKind::Omniwheel3wheel,
    FormFactorKind::Omniwheel4wheel,
    FormFactorKind::BigfootMonsterTruck,
    FormFactorKind::BigfootRockCrawler,
    FormFactorKind::AtvOffroad,
    FormFactorKind::DriftRallyRacer,
    FormFactorKind::TrackedSkidsteer,
    // Ground — balancing (3)
    FormFactorKind::Balance2wheel,
    FormFactorKind::BalanceUnicycle,
    FormFactorKind::Ballbot,
    // Ground — legged (4)
    FormFactorKind::BipedHumanoid,
    FormFactorKind::Quadruped,
    FormFactorKind::Hexapod,
    FormFactorKind::Octopod,
    // Aquatic — surface (5)
    FormFactorKind::BoatSingleRudder,
    FormFactorKind::BoatTwinPropDiff,
    FormFactorKind::Hovercraft,
    FormFactorKind::Hydrofoil,
    FormFactorKind::Sailboat,
    // Aquatic — submerged (3)
    FormFactorKind::Rov4thruster,
    FormFactorKind::Rov6thruster,
    FormFactorKind::AuvTorpedo,
    // Aerial — multirotor (6)
    FormFactorKind::QuadcopterX,
    FormFactorKind::QuadcopterPlus,
    FormFactorKind::Tricopter,
    FormFactorKind::Hexacopter,
    FormFactorKind::OctocopterX,
    FormFactorKind::OctocopterCoax,
    // Aerial — helicopter (3)
    FormFactorKind::HeliSingleRotor,
    FormFactorKind::HeliCoaxial,
    FormFactorKind::HeliTandem,
    // Aerial — fixed-wing (4)
    FormFactorKind::FixedwingStandard,
    FormFactorKind::FixedwingVtail,
    FormFactorKind::FlyingWing,
    FormFactorKind::Glider,
    // Aerial — VTOL / hybrid (4)
    FormFactorKind::VtolTailsitter,
    FormFactorKind::VtolTiltrotor,
    FormFactorKind::VtolQuadplane,
    FormFactorKind::VtolBicopter,
    // Lighter-than-air (2)
    FormFactorKind::LtaBlimp,
    FormFactorKind::LtaAirship,
    // Articulated (3)
    FormFactorKind::SnakeSerpentine,
    FormFactorKind::WormModular,
    FormFactorKind::RollingBall,
    // Agricultural (3)
    FormFactorKind::AutonomousMower,
    FormFactorKind::SprayerSpot,
    FormFactorKind::TractorTowedImplement,
    // Construction (3)
    FormFactorKind::ExcavatorArm,
    FormFactorKind::CraneBoom,
    FormFactorKind::SkidSteerLoader,
    // Climbing (3)
    FormFactorKind::WallClimbingSuction,
    FormFactorKind::CableClimbing,
    FormFactorKind::MagneticClimber,
    // Amphibious (1)
    FormFactorKind::AmphibiousWheelsPlusProp,
    // Soft / continuum (2)
    FormFactorKind::SoftGripper,
    FormFactorKind::TentacleArm,
    // Educational (1)
    FormFactorKind::ModularCubelets,
    // Jumping / hopping (2)
    FormFactorKind::JumpingRobot,
    FormFactorKind::Grasshopper,
];

/// Belt-and-suspenders: the static list above must total exactly 64.
/// If `platform.rs` adds a variant without updating this file, the
/// completeness test below fails; if this file's static list miscounts,
/// the assertion here fails first.
const _: () = assert!(
    ALL_FORM_FACTOR_KINDS.len() == 64,
    "ALL_FORM_FACTOR_KINDS must list exactly 64 variants",
);

#[test]
fn meta_covers_every_form_factor_kind_variant() {
    let meta = default_form_factor_meta();
    assert_eq!(
        meta.len(),
        64,
        "default_form_factor_meta() must return exactly 64 entries, got {}",
        meta.len(),
    );

    let meta_ids: BTreeSet<FormFactorKind> = meta.iter().map(|m| m.id).collect();
    assert_eq!(
        meta_ids.len(),
        64,
        "duplicate ids in default_form_factor_meta() — set size {} != 64",
        meta_ids.len(),
    );

    // Exhaustive coverage: every static-list variant appears in the meta.
    for variant in ALL_FORM_FACTOR_KINDS {
        assert!(
            meta_ids.contains(variant),
            "default_form_factor_meta() missing entry for {variant:?}",
        );
    }

    // And the reverse: every meta entry corresponds to a variant in the
    // static list. (If meta has an id NOT in the list, the list is stale.)
    let static_set: BTreeSet<FormFactorKind> = ALL_FORM_FACTOR_KINDS.iter().copied().collect();
    for id in &meta_ids {
        assert!(
            static_set.contains(id),
            "default_form_factor_meta() emits {id:?} which is not in ALL_FORM_FACTOR_KINDS — \
             update the static list (or drop the meta entry if the variant was retired)",
        );
    }
}

#[test]
fn meta_entries_roundtrip_through_serde() {
    let meta = default_form_factor_meta();
    for entry in &meta {
        let json = serde_json::to_value(entry).expect("FormFactorMeta serializes");
        let back: rshome_schema::platform::FormFactorMeta =
            serde_json::from_value(json.clone()).expect("FormFactorMeta deserializes");
        assert_eq!(
            entry,
            &back,
            "FormFactorMeta round-trip mismatch for {:?}; intermediate JSON:\n{}",
            entry.id,
            serde_json::to_string_pretty(&json).unwrap(),
        );
    }
}

#[test]
fn meta_entries_have_nonempty_labels() {
    let meta = default_form_factor_meta();
    for entry in &meta {
        assert!(
            !entry.label.is_empty(),
            "FormFactorMeta for {:?} has empty label; wizard L1 step would render blank",
            entry.id,
        );
    }
}
