// RG-2 B5 / Task 1.3 capacity: `bench_export_registry` p95 ≤ 5 s.
//
// Measures the wall-clock cost of producing the full `registry-data.json`
// payload via `rshome_wizard::export::build_registry_export()`. The binary
// `bin/export-registry` is now a thin pretty-print wrapper over the same
// function, so this bench is the canonical measurement of "the export
// binary completes within 5 s" per the Phase 1 acceptance row.
//
// Methodology contract (governance/performance-engineering.md §2):
//   - 100 samples per bench (acceptance row floors at "10 runs"; we
//     sample 10× higher for stable percentile estimates)
//   - 30 s warm-up before measurement starts
//   - report p50, p95, p99, max
//   - stddev < 30 % of p50 (otherwise rerun)
//
// === Run + percentile extraction ===
//   cargo bench -p rshome-wizard --bench export_registry
//
// Criterion writes per-sample data at
// `target/criterion/export_registry/bench_export_registry/new/sample.json`
// (parallel `iters[]` + `times[]` arrays, ns). Compute the contract
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
//   ' target/criterion/export_registry/bench_export_registry/new/sample.json
//
// Drop the ms values into `governance/perf-snapshots/phase-1.md §1`
// after the first measurement run.

use std::hint::black_box;
use std::time::{Duration, Instant};

use criterion::{criterion_group, criterion_main, Criterion};

use rshome_wizard::export::build_registry_export;

fn bench_export_registry(c: &mut Criterion) {
    let mut group = c.benchmark_group("export_registry");
    group
        .sample_size(100)
        .warm_up_time(Duration::from_secs(30))
        // Per-sample target is < 2 s today (per Task 1.3 capacity row);
        // measurement_time = 200 s gives criterion headroom to pick
        // `iters = 1` per sample when the per-call cost is high.
        .measurement_time(Duration::from_secs(200));

    group.bench_function("bench_export_registry", |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _ in 0..iters {
                let payload = build_registry_export();
                black_box(payload);
            }
            start.elapsed()
        });
    });

    group.finish();
}

criterion_group!(export_benches, bench_export_registry);
criterion_main!(export_benches);
