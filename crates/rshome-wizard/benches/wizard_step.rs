// PRD iot-device-tooling-v2 Phase 0 Task 0.4 acceptance #3 / M2 /
// SLO-WIZARD-LATENCY anchor — end-to-end step-transition latency.
//
// PRD spec wording references `Wizard::transition()` on the wizard core
// state machine; that literal function is a UI concept that lives in the
// `type-driven-ui` step-component layer, not in this Rust crate. The
// Rust-side equivalent of "what runs per UI step transition" is the
// validation-plus-derivation pipeline the UI calls into via wasm bindings
// after each user selection:
//
//   1. `validate_workspace` — re-runs cross-profile cross-cell validation
//      against the full SolutionRegistry (the heavy lift; M5 already
//      benches this in isolation at the 10-profile / µs scale).
//   2. `effective_control_uplink` — re-derives the per-profile control
//      uplink, which is what feeds the wizard's downstream filter chain
//      (next-step rendering, parameterized-bridge expansion, etc.).
//
// This bench composes (1) + (2) over the 10-profile fixture so each
// iteration measures one full "step transition" cost end-to-end. M5
// remains the narrow validate-only number; this M2 anchor is what the
// PRD's "p95 < 1500 ms over 1000 transitions on M3 Pro" gate
// (RC-1 binding criterion) measures against.
//
// Methodology contract (governance/performance-engineering.md §2):
//   - 1000 samples per bench (matches PRD "1000 transitions" acceptance)
//   - 30 s warm-up before measurement starts
//   - report p50, p95, p99, max
//   - stddev < 30 % of p50 (otherwise rerun)
//
// === Run + percentile extraction ===
//   cargo bench -p rshome-wizard --bench wizard_step
//
// Per-sample data lands at
// `target/criterion/wizard_step/bench_wizard_step_pipeline/new/sample.json`
// (parallel `iters[]` + `times[]` arrays, ns). Recover the contract
// percentiles with:
//
//   jq -r '
//     [range(0; (.iters|length)) as $i | .times[$i] / .iters[$i]]
//       | sort_by(.) as $s
//       | (length) as $n
//       | {
//           p50_ms: ($s[($n*0.5|floor)] / 1000000),
//           p95_ms: ($s[($n*0.95|floor)] / 1000000),
//           p99_ms: ($s[($n*0.99|floor)] / 1000000),
//           max_ms: ($s[-1]               / 1000000),
//           min_ms: ($s[0]                / 1000000)
//         }
//   ' target/criterion/wizard_step/bench_wizard_step_pipeline/new/sample.json
//
// RC-1 binding criterion: `p95_ms < 1500` on the M3 Pro reference HW
// (per ADR-003). Stddev → criterion's `estimates.json`
// (`std_dev.point_estimate / median.point_estimate < 0.30`).
//
// Drop the ms values into `governance/perf-snapshots/phase-0.md §M2`
// after the M3 Pro reference run.

use std::hint::black_box;
use std::time::{Duration, Instant};

use criterion::{criterion_group, criterion_main, Criterion};

use rshome_schema::solution::default_solution_registry;
use rshome_wizard::workspace::{effective_control_uplink, validate_workspace, Workspace};

const WORKSPACE_10PROFILES_JSON: &str = include_str!("../tests/fixtures/workspace_10profiles.json");

fn bench_wizard_step_pipeline(c: &mut Criterion) {
    let registry = default_solution_registry();
    let workspace: Workspace = serde_json::from_str(WORKSPACE_10PROFILES_JSON)
        .expect("workspace_10profiles.json must deserialize into Workspace");
    assert_eq!(
        workspace.profiles.len(),
        10,
        "fixture must contain exactly 10 profiles (RG-1 A4 / M5 contract reused for M2)"
    );

    let mut group = c.benchmark_group("wizard_step");
    group
        .sample_size(1000)
        .warm_up_time(Duration::from_secs(30))
        .measurement_time(Duration::from_secs(20));

    group.bench_function("bench_wizard_step_pipeline", |b| {
        // iter_custom with an explicit inner loop: criterion picks
        // `iters` per sample to fit `measurement_time / sample_size`
        // (~20 ms per sample at the current call cost). Per-call time =
        // total / iters; sample.json preserves both arrays so the jq
        // command above can recover per-call percentiles directly.
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _ in 0..iters {
                // (1) Cross-profile validation — the heavy lift on every
                //     step. M5 already benches this in isolation; here
                //     it's the dominant component of the M2 composite.
                let errors = validate_workspace(black_box(&workspace), black_box(&registry));
                black_box(errors);

                // (2) Per-profile derivation — what the wizard re-runs
                //     after each step to refresh the downstream filter
                //     chain (next-step rendering, parameterized-bridge
                //     hints, etc.). Skips profiles whose solution_id
                //     isn't in the registry (validation would have
                //     surfaced that already).
                for profile in &workspace.profiles {
                    if let Some(sol) = registry.get(&profile.selected_solution_id) {
                        let uplink = effective_control_uplink(black_box(profile), black_box(sol));
                        black_box(uplink);
                    }
                }
            }
            start.elapsed()
        });
    });

    group.finish();
}

criterion_group!(wizard_step_benches, bench_wizard_step_pipeline);
criterion_main!(wizard_step_benches);
