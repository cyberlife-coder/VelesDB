//! Latency of the deterministic context compiler's `compile()` pipeline.
//!
//! Three axes, each isolating one cost driver:
//! - **fragment count** — does the pipeline scale linearly with N?
//! - **budget pressure** — does packing under a tight budget cost more than
//!   a generous one that just emits everything?
//! - **content shape** — duplicates (dedup-heavy) vs one oversized fragment
//!   (chunk-heavy) vs plain prose (the common case).
//!
//! Run with:
//!
//! ```sh
//! cargo bench -p velesdb-memory --no-default-features --features context
//! ```
//!
//! Results land in `target/criterion/`; `--bench` needs no network, no
//! store, no Ollama — the compiler is memoryless and deterministic.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use velesdb_memory::context::{CompilePolicy, CompileRequest, ContextCompiler, ContextFragment};

/// One deterministic pseudo-random prose paragraph, long enough to need
/// chunking under a tight budget but short enough to fit whole under a
/// generous one — representative of a single conversation turn or log
/// excerpt.
fn paragraph(seed: usize) -> String {
    format!(
        "Turn {seed}: the deploy pipeline reported shard-{shard} finished its checksum \
         verification in {ms} ms after the canary rollout reached {pct}% of the fleet, \
         and the on-call engineer confirmed no alerts fired during the rebalance window.",
        seed = seed,
        shard = seed % 37,
        ms = 40 + (seed * 7) % 900,
        pct = (seed * 3) % 100,
    )
}

/// `n` distinct plain-prose fragments — the common case (no dupes, no
/// oversized content).
fn plain_fragments(n: usize) -> Vec<ContextFragment> {
    (0..n)
        .map(|i| ContextFragment {
            path: None,
            id: None,
            content: paragraph(i),
            kind: None,
            priority: None,
            metadata: None,
            media: None,
        })
        .collect()
}

/// `n` fragments where every third one exactly repeats an earlier one —
/// exercises the dedup path at scale.
fn duplicate_heavy_fragments(n: usize) -> Vec<ContextFragment> {
    (0..n)
        .map(|i| {
            let source = if i % 3 == 0 { i } else { i - (i % 3) };
            ContextFragment {
                path: None,
                id: None,
                content: paragraph(source),
                kind: None,
                priority: None,
                metadata: None,
                media: None,
            }
        })
        .collect()
}

fn request(fragments: Vec<ContextFragment>, token_budget: u64) -> CompileRequest {
    CompileRequest {
        query: "what is the current state of the deploy pipeline".to_owned(),
        fragments,
        project: None,
        target_model: None,
        token_budget,
        memory_scope: None,
        policy: None,
    }
}

fn compiler() -> ContextCompiler {
    ContextCompiler::new(CompilePolicy::default())
}

/// Latency vs fragment count, budget generous enough to never externalize —
/// isolates classify/dedup/pack overhead from budget-pressure effects.
fn bench_by_fragment_count(c: &mut Criterion) {
    let mut group = c.benchmark_group("compile_by_fragment_count");
    for &n in &[10_usize, 100, 500, 1_000] {
        let req = request(plain_fragments(n), 1_000_000);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &req, |b, req| {
            let compiler = compiler();
            b.iter(|| compiler.compile(req).expect("compile"));
        });
    }
    group.finish();
}

/// Latency vs budget pressure at a fixed, realistic fragment count — shows
/// the cost of packing decisions and externalization under a tight budget
/// versus a budget so generous packing is a formality.
fn bench_by_budget_pressure(c: &mut Criterion) {
    let mut group = c.benchmark_group("compile_by_budget_pressure");
    let fragments = plain_fragments(200);
    for &budget in &[200_u64, 2_000, 20_000, 1_000_000] {
        let req = request(fragments.clone(), budget);
        group.bench_with_input(BenchmarkId::from_parameter(budget), &req, |b, req| {
            let compiler = compiler();
            b.iter(|| compiler.compile(req).expect("compile"));
        });
    }
    group.finish();
}

/// Duplicate-heavy corpus at scale — isolates the dedup path's cost.
fn bench_duplicate_heavy(c: &mut Criterion) {
    let mut group = c.benchmark_group("compile_duplicate_heavy");
    for &n in &[100_usize, 1_000] {
        let req = request(duplicate_heavy_fragments(n), 1_000_000);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &req, |b, req| {
            let compiler = compiler();
            b.iter(|| compiler.compile(req).expect("compile"));
        });
    }
    group.finish();
}

/// A single oversized fragment (100 KB) under a tight budget — isolates the
/// chunker's cost, the case every other benchmark here avoids by design.
fn bench_oversized_fragment(c: &mut Criterion) {
    let mut group = c.benchmark_group("compile_oversized_fragment");
    let huge = paragraph(0).repeat(1_200); // ~100 KB
    for &budget in &[2_000_u64, 1_000_000] {
        let req = request(
            vec![ContextFragment {
                path: None,
                id: None,
                content: huge.clone(),
                kind: None,
                priority: None,
                metadata: None,
                media: None,
            }],
            budget,
        );
        group.bench_with_input(BenchmarkId::from_parameter(budget), &req, |b, req| {
            let compiler = compiler();
            b.iter(|| compiler.compile(req).expect("compile"));
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_by_fragment_count,
    bench_by_budget_pressure,
    bench_duplicate_heavy,
    bench_oversized_fragment,
);
criterion_main!(benches);
