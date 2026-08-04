//! Latency of [`MemoryService::recall_fused`] as a single entity hub's
//! fan-out grows — the shape issue #1742 diagnosed: `graph_reached`'s
//! per-node `reach_weight` scan used to rescan every edge in the traversal
//! for each reached node, making the whole walk O(hub degree²). Indexing
//! `mentions` edges by target turned that into O(edges + nodes). This bench
//! is the empirical check: latency should grow ~linearly with hub degree,
//! not quadratically — a user accumulating facts about the same topic (the
//! product's nominal use case) must not pay a cost that grows with the
//! square of their history on it.
//!
//! Run with:
//!
//! ```sh
//! cargo bench -p velesdb-memory --bench fused_recall_benchmark
//! ```
//!
//! Results land in `target/criterion/`; store setup happens outside the
//! timed section, so each measured iteration is `recall_fused` alone.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use tempfile::TempDir;
use velesdb_memory::{FusionOptions, HashEmbedder, MemoryService};

const DIM: usize = 384;
const SEED: &str = "seed fact anchors the walk";

/// A store wired like the autograph's bipartite hub: a seed fact linked to
/// one entity hub, which in turn `mentions` `degree` facts — the exact shape
/// `recall_fused`'s default 2-hop walk exercises (hop 1 reaches the hub, hop
/// 2 reaches everything it mentions).
fn hub_store(degree: usize) -> (TempDir, MemoryService<HashEmbedder>) {
    let dir = TempDir::new().expect("tempdir");
    let svc = MemoryService::open(dir.path(), HashEmbedder::new(DIM)).expect("open store");
    let seed = svc.remember(SEED, &[], None).expect("remember seed");
    let hub = svc
        .remember("Entity: benchmark-topic", &[], None)
        .expect("remember hub");
    svc.relate(seed, hub, "about").expect("relate seed->hub");
    for i in 0..degree {
        let fact = svc
            .remember(&format!("fact number {i} about the topic"), &[], None)
            .expect("remember fact");
        svc.relate(hub, fact, "mentions").expect("relate hub->fact");
    }
    (dir, svc)
}

fn bench_recall_fused_by_hub_degree(c: &mut Criterion) {
    let mut group = c.benchmark_group("recall_fused_hub_degree");
    // Capped at 1000: past roughly 1195 total memories in a store, seed
    // lookup by exact vector match stops finding anything at all (tracked
    // separately, not a `recall_fused`/graph-walk issue) — so degrees above
    // this would silently benchmark a no-op walk instead of the real one.
    for degree in [50_usize, 200, 500, 1000] {
        let (_dir, svc) = hub_store(degree);
        // The walk must actually reach every fact under the hub (seed + hub
        // + degree facts) — otherwise a regression elsewhere silently turns
        // this into a no-op benchmark instead of failing loudly.
        let reached = svc.why(SEED, 2, None).expect("why").nodes.len();
        assert_eq!(
            reached,
            degree + 2,
            "traversal did not reach the full hub fan-out"
        );
        group.throughput(Throughput::Elements(degree as u64));
        group.bench_with_input(BenchmarkId::from_parameter(degree), &degree, |b, _| {
            b.iter(|| {
                svc.recall_fused(SEED, 10, None, FusionOptions::default())
                    .expect("recall_fused")
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_recall_fused_by_hub_degree);
criterion_main!(benches);
