//! V&A chip-coverage matrix presence lint per
//! Task 0.2 acceptance #4: "`chip_coverage` is fully populated
//! (`{ esp32_s3, esp32_c6, esp32_d0wd } ∈ {preferred, caveat,
//! insufficient, unspecified}`) for all 37 rows".
//!
//! Two invariants enforced here, both consumed by
//! `scripts/check-va-table.sh`:
//!
//! 1. **Row-count**: the V&A solution registry contains EXACTLY 37
//!    entries. New V&A solutions land via the per-solution PR review
//!    process (master design §8) and require updating
//!    `EXPECTED_VA_COUNT` here together with the TSV snapshot.
//! 2. **Chip-coverage shape**: every V&A solution declares
//!    `chip_coverage: Some(BTreeMap)` with entries for all 3
//!    `ChipFamilyKind` keys. The Rust type system already restricts
//!    values to `ChipCoverageStatus ∈ {Preferred, Caveat,
//!    Insufficient}`, so the "unspecified" PRD value maps to "key
//!    absent from the map" — which this lint rejects.

use rshome_schema::platform::{ChipFamilyKind, DomainKind};
use rshome_schema::solution::default_solution_registry;

/// Authoritative V&A solution count — matches master design doc and the
/// committed snapshot at `scripts/va-solution-table.tsv`. Update both
/// together when adding or removing a V&A solution.
const EXPECTED_VA_COUNT: usize = 37;

#[test]
fn va_solution_count_is_authoritative() {
    let reg = default_solution_registry();
    let actual = reg
        .all()
        .filter(|s| s.domain == Some(DomainKind::VehicleAircraftControl))
        .count();
    assert_eq!(
        actual, EXPECTED_VA_COUNT,
        "V&A solution count is {} but EXPECTED_VA_COUNT is {}. \
         If this change is intentional, update EXPECTED_VA_COUNT in this file \
         AND refresh the snapshot via `./scripts/check-va-table.sh --update-snapshot` \
         AND commit the new scripts/va-solution-table.tsv in the same PR.",
        actual, EXPECTED_VA_COUNT,
    );
}

#[test]
fn every_va_solution_declares_chip_coverage() {
    let reg = default_solution_registry();
    let missing: Vec<String> = reg
        .all()
        .filter(|s| s.domain == Some(DomainKind::VehicleAircraftControl))
        .filter(|s| s.chip_coverage.is_none())
        .map(|s| s.id.clone())
        .collect();
    assert!(
        missing.is_empty(),
        "V&A solutions missing chip_coverage (must declare at least one ChipFamilyKind → ChipCoverageStatus entry):\n  {}",
        missing.join("\n  "),
    );
}

#[test]
fn every_va_solution_covers_all_three_chip_families() {
    let reg = default_solution_registry();
    let required_keys = [
        ChipFamilyKind::Esp32S3,
        ChipFamilyKind::Esp32C6,
        ChipFamilyKind::Esp32D0wd,
    ];

    let mut failures: Vec<String> = Vec::new();
    for s in reg
        .all()
        .filter(|s| s.domain == Some(DomainKind::VehicleAircraftControl))
    {
        let Some(coverage) = &s.chip_coverage else {
            // Handled by `every_va_solution_declares_chip_coverage`.
            continue;
        };
        let missing: Vec<ChipFamilyKind> = required_keys
            .iter()
            .copied()
            .filter(|k| !coverage.contains_key(k))
            .collect();
        if !missing.is_empty() {
            failures.push(format!(
                "{}: chip_coverage missing keys {:?}",
                s.id, missing,
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "V&A solutions with incomplete chip_coverage matrix:\n  {}",
        failures.join("\n  "),
    );
}
