//! Benchmark OTL tier sweeps: compile-once vs per-sweep compile, sequential vs parallel signals.

use std::collections::HashMap;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use petgraph::stable_graph::NodeIndex;
use pulsar_marketlab_core::{
    compile_object_program, compile_object_tier, evaluate_compiled_tier, run_parallel_signal_batch,
    CompiledProgramTier, ExecutionContext, GraphSeriesMatrix, ObjectCodegenRegistry,
    ParallelSignalJob, ParallelSweepContext,
};

const BARS: usize = 500;

fn synthetic_prices() -> Vec<f64> {
    (0..BARS)
        .map(|bar| 100.0 + (bar as f64) * 0.05 + ((bar as f64) * 0.02).sin())
        .collect()
}

fn signal_source() -> &'static str {
    r#"
signal bench_alpha(input closure raw, output closure gated) {
    gated = sma(data, 20);
}
"#
}

fn compile_signal_template() -> CompiledProgramTier {
    let program = compile_object_program(signal_source()).expect("parse OTL");
    compile_object_tier(&program, &ObjectCodegenRegistry::default()).expect("compile tier")
}

fn sweep_cached_tier(tier: &mut CompiledProgramTier, upstream: &[f64]) {
    let mut ctx = ExecutionContext::new(BARS, 1_000_000.0, "Allocation::EqualWeight", HashMap::new(), HashMap::new());
    ctx.signal_upstream = upstream.to_vec();
    ctx.signal_output_column = 0;
    let mut matrix = GraphSeriesMatrix::with_capacity(BARS, 1, 1);
    evaluate_compiled_tier(&mut ctx, tier, &mut matrix).expect("sweep");
    black_box(matrix.read_signal(0, BARS - 1));
}

fn sweep_compile_each_bar_batch(upstream: &[f64]) {
    let program = compile_object_program(signal_source()).expect("parse");
    let registry = ObjectCodegenRegistry {
        upstream_series: vec![upstream.to_vec()],
        ..ObjectCodegenRegistry::default()
    };
    let mut tier = compile_object_tier(&program, &registry).expect("compile");
    sweep_cached_tier(&mut tier, upstream);
}

fn bench_signal_sweep(c: &mut Criterion) {
    let upstream = synthetic_prices();
    let mut group = c.benchmark_group("signal_tier_sweep");
    let mut cached = compile_signal_template();

    group.bench_function("cached_tier_reuse", |b| {
        b.iter(|| sweep_cached_tier(&mut cached, black_box(&upstream)));
    });

    group.bench_function("compile_tier_each_sweep", |b| {
        b.iter(|| sweep_compile_each_bar_batch(black_box(&upstream)));
    });

    group.finish();
}

fn bench_parallel_batch_only(c: &mut Criterion) {
    let template = compile_signal_template();
    let ctx = ParallelSweepContext {
        timeline_len: BARS,
        initial_capital: 1_000_000.0,
        allocation_method: "Allocation::EqualWeight".into(),
        asset_quotes: HashMap::new(),
    };
    let upstream_a = synthetic_prices();
    let upstream_b = upstream_a.iter().map(|v| v + 40.0).collect::<Vec<_>>();
    let jobs = vec![
        ParallelSignalJob {
            node_index: NodeIndex::new(0),
            prim_path: "/sig/a".into(),
            column: 0,
            upstream: upstream_a,
            expression: String::new(),
        },
        ParallelSignalJob {
            node_index: NodeIndex::new(1),
            prim_path: "/sig/b".into(),
            column: 1,
            upstream: upstream_b,
            expression: String::new(),
        },
    ];

    c.bench_with_input(
        BenchmarkId::new("parallel_rayon_batch", BARS),
        &jobs,
        |b, jobs| {
            b.iter(|| {
                let mut local_tiers = vec![template.fork_for_sweep(), template.fork_for_sweep()];
                black_box(run_parallel_signal_batch(jobs, &mut local_tiers, &ctx));
            });
        },
    );
}

criterion_group!(benches, bench_signal_sweep, bench_parallel_batch_only);
criterion_main!(benches);
