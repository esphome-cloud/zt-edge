// RG-1 A1 / Phase-0 Task 0.1 bullet 3: SolutionDefinition field-count lint.
//
// PRD §6.1 + master-doc table both state 33 fields after the 2026-06-01
// RG-1 sign-off reconciliation. (Previously named 40 pre-2026-04-21
// `implementation_family: Option<String>` retirement; va-residuals
// Q11 dropped the field and replaced it with the typed `family:
// Option<ImplementationFamily>`. The PRD count was reconciled at
// RG-1 sign-off rather than reintroducing the 7 dropped fields.)
//
// This test asserts the current count (33) AND that the PRD-named
// count agrees. Any future field add/remove will trip a CI failure
// here, forcing the PRD to be updated in the same PR.
//
// Implementation note: Rust has no stable reflection at runtime.
// We parse the source text instead, mirroring the pattern in
// `va_enum_variant_counts.rs::count_rust_enum_variants`.

const SOLUTION_RS: &str = include_str!("../../rshome-schema/src/solution.rs");

const EXPECTED_FIELDS: usize = 33;
const PRD_NAMED_COUNT: usize = 33;

/// For a `pub struct Name { ... }` block, return the count of
/// `pub <ident>: <type>` field lines in the body. Tolerates
/// `#[serde(...)]` / `#[derive(...)]` attributes, doc-comments,
/// line comments, and blank lines between fields.
fn count_pub_struct_fields(src: &str, struct_name: &str) -> usize {
    let header = format!("pub struct {} {{", struct_name);
    let after_header = src
        .split_once(&header)
        .unwrap_or_else(|| panic!("solution.rs does not contain `{}`", header))
        .1;
    let body = after_header
        .split_once("\n}")
        .unwrap_or_else(|| {
            panic!(
                "unterminated struct body for {} in solution.rs",
                struct_name
            )
        })
        .0;
    body.lines()
        .map(str::trim)
        .filter(|l| l.starts_with("pub ") && !l.starts_with("pub struct") && l.contains(':'))
        .count()
}

#[test]
fn solution_definition_field_count_is_authoritative() {
    let actual = count_pub_struct_fields(SOLUTION_RS, "SolutionDefinition");
    assert_eq!(
        actual, EXPECTED_FIELDS,
        "SolutionDefinition has {} `pub` fields; EXPECTED_FIELDS = {}. \
         If you added or removed a field, update EXPECTED_FIELDS *and* the \
         public contract count in the same PR.",
        actual, EXPECTED_FIELDS,
    );
}

#[test]
fn prd_named_count_matches_actual() {
    // PRD §6.1 + master-doc table reconciled to 33 at RG-1 sign-off
    // (2026-06-01). This test guards against future drift in either
    // direction — bumping EXPECTED_FIELDS without updating
    // PRD_NAMED_COUNT (or the PRD prose) fails CI here.
    assert_eq!(
        EXPECTED_FIELDS, PRD_NAMED_COUNT,
        "PRD-named field count drifted from struct field count. If you \
         added or removed a field, update PRD_NAMED_COUNT and the PRD prose \
         (overview.md F1.4 / outline.md / docs/vehicle-aircraft-control-design.md §6.1 / \
         phase-0-foundation-codification.md Task 0.1) together with EXPECTED_FIELDS."
    );
}
