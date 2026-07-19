// RG-1 A4 / M5: validate_workspace 10-profile latency bench.
//
// Methodology contract (governance/performance-engineering.md §2):
//   - 1000 samples per bench
//   - 30 s warm-up before measurement starts
//   - report p50, p95, p99, max
//   - stddev < 30% of p50 (otherwise rerun)
//
// === Run + percentile extraction ===
//   cargo bench -p rshome-wizard --bench validate_workspace
//
// Criterion's default text output reports a 95 % CI on the mean, not
// percentiles. The raw per-sample data lives at
// `target/criterion/bench_validate_workspace_10profiles/new/sample.json`
// (parallel `iters[]` + `times[]` arrays, ns). Compute the contract
// percentiles with:
//
//   jq -r '
//     [range(0; (.iters|length)) as $i | .times[$i] / .iters[$i]]
//       | sort_by(.) as $s
//       | (length) as $n
//       | {
//           p50_us: ($s[($n*0.5|floor)] / 1000),
//           p95_us: ($s[($n*0.95|floor)] / 1000),
//           p99_us: ($s[($n*0.99|floor)] / 1000),
//           max_us: ($s[-1]               / 1000),
//           min_us: ($s[0]                / 1000)
//         }
//   ' target/criterion/validate_workspace/bench_validate_workspace_10profiles/new/sample.json
//
// Note the path: when a bench uses a `benchmark_group`, criterion files
// land under `target/criterion/<group_name>/<bench_name>/`. The smoke
// run before the group existed wrote to `target/criterion/<bench_name>/`
// directly — drop the group segment if running against a pre-group
// build.
//
// Drop the µs values into `governance/perf-snapshots/phase-0.md §1`
// after the m6i.xlarge run. Stddev → see criterion's `estimates.json`
// (`std_dev.point_estimate`), divide by `median.point_estimate` for the
// 30 % check.

use std::hint::black_box;
use std::time::{Duration, Instant};

use criterion::{criterion_group, criterion_main, Criterion};

use rshome_schema::solution::default_solution_registry;
use rshome_wizard::workspace::{validate_workspace, Workspace};

const WORKSPACE_10PROFILES_JSON: &str = include_str!("../tests/fixtures/workspace_10profiles.json");

fn bench_validate_workspace_10profiles(c: &mut Criterion) {
    let registry = default_solution_registry();
    let workspace: Workspace = serde_json::from_str(WORKSPACE_10PROFILES_JSON)
        .expect("workspace_10profiles.json must deserialize into Workspace");
    assert_eq!(
        workspace.profiles.len(),
        10,
        "fixture must contain exactly 10 profiles (RG-1 A4 / M5 contract)"
    );

    let mut group = c.benchmark_group("validate_workspace");
    group
        .sample_size(1000)
        .warm_up_time(Duration::from_secs(30))
        .measurement_time(Duration::from_secs(10));

    group.bench_function("bench_validate_workspace_10profiles", |b| {
        // iter_custom with an explicit inner loop: criterion picks
        // `iters` per sample to fit `measurement_time / sample_size`
        // (~10 ms per sample at the current call cost). Per-call time =
        // total / iters; sample.json preserves both arrays so the jq
        // command above can recover per-call percentiles directly.
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _ in 0..iters {
                let errors = validate_workspace(black_box(&workspace), black_box(&registry));
                black_box(errors);
            }
            start.elapsed()
        });
    });

    group.finish();
}

criterion_group!(workspace_benches, bench_validate_workspace_10profiles);
criterion_main!(workspace_benches);
