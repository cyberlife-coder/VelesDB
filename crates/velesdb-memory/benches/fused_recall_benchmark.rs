//! Latency of [`MemoryService::recall_fused`] as the fan-out its graph walk
//! reaches grows — the shape issue #1742 diagnosed: `graph_reached`'s
//! per-node `reach_weight` scan used to rescan every edge in the traversal
//! for each reached node, making the whole walk O(reach²). Indexing
//! `mentions` edges by target turned that into O(edges + nodes). This bench
//! is the empirical check: latency should grow ~linearly with the reach,
//! not quadratically — a user accumulating facts about the same topic (the
//! product's nominal use case) must not pay a cost that grows with the
//! square of their history on it.
//!
//! # Why the fixture is a forest of hubs, not one hub
//!
//! The first version of this bench hung every fact off ONE entity hub and
//! asserted the walk reached all of them. Issue #1743 then capped a single
//! node's expansion at [`MAX_WHY_NODE_DEGREE`] (64) and the whole walk at
//! [`MAX_WHY_NODES`] (500) — so that assertion became unsatisfiable for any
//! degree above 64, the bench panicked at its second parameter point, and
//! four of its five measurements were silently never taken. Nothing in CI
//! runs this crate's benches, so nothing noticed.
//!
//! The reach is therefore spread over as many hubs as the per-node cap
//! requires, each with at most `MAX_WHY_NODE_DEGREE` mentions: the total
//! reach still grows into the hundreds while every node stays inside the
//! policy the walk actually runs under. The sweep covers both regimes the
//! caps create — the linear region under [`MAX_WHY_NODES`], and one point
//! past it, where the walk must truncate at exactly the ceiling and the cost
//! must plateau rather than keep climbing.
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
use velesdb_memory::limits::{MAX_WHY_NODES, MAX_WHY_NODE_DEGREE};
use velesdb_memory::{FusionOptions, HashEmbedder, MemoryService};

const DIM: usize = 384;
const SEED: &str = "seed fact anchors the walk";

/// Reach points under the walk's node ceiling. Each stays below
/// [`MAX_WHY_NODES`] once its seed and hubs are counted, so the walk must
/// reach every fact and the liveness gate holds exactly.
const UNDER_CEILING: [usize; 3] = [50, 200, 480];

/// One reach point past the ceiling: the walk truncates at [`MAX_WHY_NODES`]
/// and the measured cost must plateau — the cap is a cost bound, and this is
/// where that claim is checked.
const OVER_CEILING: usize = 1000;

/// How many hubs `reach` facts need when no hub may mention more than
/// [`MAX_WHY_NODE_DEGREE`] of them.
fn hubs_for(reach: usize) -> usize {
    reach.div_ceil(MAX_WHY_NODE_DEGREE)
}

/// A store wired like the autograph's bipartite hubs: a seed fact linked to
/// `hubs_for(reach)` entity hubs, which between them `mention` `reach`
/// facts — no hub over the per-node cap. The exact shape `recall_fused`'s
/// default 2-hop walk exercises (hop 1 reaches the hubs, hop 2 everything
/// they mention).
fn hub_forest(reach: usize) -> (TempDir, MemoryService<HashEmbedder>) {
    let dir = TempDir::new().expect("tempdir");
    let svc = MemoryService::open(dir.path(), HashEmbedder::new(DIM)).expect("open store");
    let seed = svc.remember(SEED, &[], None).expect("remember seed");
    let mut hub = None;
    for i in 0..reach {
        if i % MAX_WHY_NODE_DEGREE == 0 {
            let id = svc
                .remember(
                    &format!("Entity: benchmark-topic-{}", i / MAX_WHY_NODE_DEGREE),
                    &[],
                    None,
                )
                .expect("remember hub");
            svc.relate(seed, id, "about").expect("relate seed->hub");
            hub = Some(id);
        }
        let fact = svc
            .remember(&format!("fact number {i} about the topic"), &[], None)
            .expect("remember fact");
        svc.relate(hub.expect("hub before facts"), fact, "mentions")
            .expect("relate hub->fact");
    }
    (dir, svc)
}

/// The liveness gate: a search regression cannot silently benchmark a no-op.
/// Under the ceiling the walk must reach seed + hubs + every fact; over it,
/// exactly [`MAX_WHY_NODES`] and say so.
fn assert_reach(svc: &MemoryService<HashEmbedder>, reach: usize) {
    let explanation = svc.why(SEED, 2, None).expect("why");
    let expected = 1 + hubs_for(reach) + reach;
    if expected <= MAX_WHY_NODES {
        assert_eq!(
            explanation.nodes.len(),
            expected,
            "traversal did not reach the full hub-forest fan-out"
        );
        assert!(
            !explanation.truncated,
            "an under-ceiling walk reported truncation"
        );
    } else {
        assert_eq!(
            explanation.nodes.len(),
            MAX_WHY_NODES,
            "an over-ceiling walk did not stop at exactly MAX_WHY_NODES"
        );
        assert!(
            explanation.truncated,
            "an over-ceiling walk did not report truncation"
        );
    }
    let preflight = svc
        .recall_fused(SEED, 10, None, FusionOptions::default())
        .expect("recall_fused preflight");
    assert!(
        preflight.iter().any(|memory| memory.content == SEED),
        "fused recall did not retain the exact seed"
    );
}

fn bench_recall_fused_by_reach(c: &mut Criterion) {
    let mut group = c.benchmark_group("recall_fused_reach");
    for reach in UNDER_CEILING.into_iter().chain([OVER_CEILING]) {
        let (_dir, svc) = hub_forest(reach);
        assert_reach(&svc, reach);
        group.throughput(Throughput::Elements(reach as u64));
        group.bench_with_input(BenchmarkId::from_parameter(reach), &reach, |b, _| {
            b.iter(|| {
                svc.recall_fused(SEED, 10, None, FusionOptions::default())
                    .expect("recall_fused")
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_recall_fused_by_reach);
criterion_main!(benches);
