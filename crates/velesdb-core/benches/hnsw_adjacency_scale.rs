//! Search latency as a function of graph size — the regime `hnsw_benchmark`
//! cannot see, measured in a way that can actually resolve it.
//!
//! Run with: `cargo bench --bench hnsw_adjacency_scale`
//!
//! # Why this exists
//!
//! Issue #2075 proposes changing how layer-0 adjacency is laid out (flat CSR,
//! narrower ids, no per-node lock) and argues from "1M locks + 1M scattered
//! allocations, zero adjacency locality". Every claim in that sentence is
//! about a graph large enough that its adjacency does not fit in cache — and
//! nothing else in this repository benchmarks one.
//!
//! `hnsw_search_latency` builds **10 000 nodes at 768 dimensions**. Its
//! adjacency is `10_000 * M0 * size_of::<NodeId>()`, on the order of 2.5 MB:
//! resident in L2/L3 on any machine this runs on, whatever its layout. The
//! vectors, at ~30 MB, are what actually evicts cache there. A layout change
//! to adjacency is therefore measured at 10K as added instructions with no
//! locality to win back.
//!
//! # Two traps this file exists to avoid, both hit in practice
//!
//! **1. A nondeterministic graph.** `insert_batch_parallel` inserts in
//! whatever order the threads reach the index, so it builds a *different graph
//! on every run*. Two runs of one unchanged binary measured 144.85 µs and
//! 171.03 µs — an 18 % spread with nothing different but the graph. Criterion
//! cannot see this: it builds the index once and then varies only the
//! searches, so its confidence intervals are tight around the wrong variance
//! component, and an A/B built that way reports `p = 0.00` on a difference
//! that is mostly luck.
//!
//! This benchmark therefore builds **sequentially** and prints a checksum of
//! a fixed set of query results. Two configurations are comparable only if
//! their checksums match: equal checksums mean the same graph was traversed
//! the same way, so a latency difference is the layout and nothing else.
//!
//! **2. Machine noise larger than the effect.** Even on a byte-identical
//! graph, process-to-process spread reached 22–25 % on a shared cloud
//! container under concurrent load. A single-digit effect is unmeasurable
//! there no matter how many samples are taken *inside* one process. Before
//! trusting a difference from this benchmark, run the identical configuration
//! twice and check that the two agree more closely than the difference being
//! claimed. If they do not, the machine cannot arbitrate the question and a
//! quieter one is needed — that is a fact about the hardware, not a reason to
//! average harder.
//!
//! Two calibrations measured on one idle 4-core Xeon, 33 MiB L3, to show how
//! sharply this varies — and that it is worth re-measuring rather than
//! assumed:
//!
//! | nodes | spread between two runs of the *same* binary |
//! |---|---|
//! | 200 000 | 0.35 % |
//! | 1 000 000 | ~5 % |
//!
//! The same container had shown 22–25 % while busy compiling. Idling it was
//! what made 200K resolvable; nothing about the hardware changed. At 1M the
//! working set leaves cache entirely and per-process page placement dominates,
//! so the noise floor rises with size — an effect worth a few percent is
//! decidable at 200K and *not* decidable at 1M on this machine, which is the
//! opposite of the intuition that bigger inputs average noise away.
//!
//! # Reading the output
//!
//! Each size prints the adjacency footprint alongside the latency. Compare it
//! against this machine's last-level cache (`lscpu | grep L3`): below it, a
//! layout change can only cost; above it, the locality argument starts to be
//! testable. With `M0 = 32` and 8-byte ids, adjacency crosses a 33 MiB L3
//! somewhere around 130K nodes.
//!
//! Compare the **sum** of adjacency and vectors, not adjacency alone. A search
//! touches a vector at every hop, so both compete for the same cache: at 200K
//! and 64d the printed figures are ~53 MB of adjacency next to ~48 MB of
//! vectors, and halving the adjacency still leaves the pair far above a 33 MiB
//! L3. Reading the adjacency column on its own suggests a narrower layout
//! would become cache-resident there. It does not, and the measurement says so
//! — narrowing ids to 4 bytes cost 2.6 % at 200K on the machine tabulated
//! above, and was lost in the noise at 1M.
//!
//! # Sizing
//!
//! Defaults stay small enough to finish unattended. Push it as far as the
//! machine allows — the regime the issue argues about starts around 1M:
//!
//! ```text
//! VELESDB_SCALE_NODES=10000,100000,500000,1000000 \
//! VELESDB_SCALE_DIM=64 \
//! cargo bench --bench hnsw_adjacency_scale
//! ```
//!
//! Dimension is deliberately low. High dimensions make the vector working set
//! dominate every cache level and mask the adjacency effect entirely — that is
//! what `hnsw_search_latency` runs into at 768d, and raising the node count
//! without lowering the dimension would reproduce it.
//!
//! Sequential insertion is the price of a comparable graph: expect roughly a
//! minute of build per 200K nodes before any measurement starts.

#![allow(clippy::cast_precision_loss)]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, SamplingMode};
use std::time::Duration;
use velesdb_core::{DistanceMetric, HnswIndex, VectorIndex};

/// Node counts to sweep, overridable with `VELESDB_SCALE_NODES`.
const DEFAULT_NODES: &str = "10000,50000,200000";
/// Vector dimension, overridable with `VELESDB_SCALE_DIM`.
const DEFAULT_DIM: usize = 64;
/// Queries whose results are folded into the comparability checksum.
const CHECKSUM_QUERIES: u64 = 64;

/// Generates a random-ish vector, matching `hnsw_benchmark`'s generator so
/// numbers from the two files describe the same kind of data.
fn generate_vector(dim: usize, seed: u64) -> Vec<f32> {
    (0..dim)
        .map(|i| (seed as f32 * 0.1 + i as f32 * 0.01).sin().midpoint(1.0))
        .collect()
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn node_counts() -> Vec<usize> {
    std::env::var("VELESDB_SCALE_NODES")
        .unwrap_or_else(|_| DEFAULT_NODES.to_string())
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect()
}

/// Approximate resident layer-0 adjacency, in bytes.
///
/// Reported rather than measured: the point is the order of magnitude next to
/// last-level cache, not exact allocator accounting. Assumes the default `M0`
/// and a full adjacency list per node, so it is an upper bound.
///
/// `id_bytes` is a parameter and not `size_of::<NodeId>()` because a bench
/// binary **cannot observe the width the build actually uses** — the neighbour
/// id type is `pub(crate)`. The caller therefore prints both plausible widths
/// rather than asserting one. An earlier version passed `size_of::<usize>()`
/// and printed a single figure, which reported "adjacency ~267 MB" for a build
/// storing 4-byte ids that in truth held ~150 MB. This line exists to be
/// compared against L3; a confidently wrong number is worse than none.
fn adjacency_bytes(nodes: usize, m0: usize, id_bytes: usize) -> usize {
    nodes * (m0 * id_bytes + std::mem::size_of::<usize>() * 3)
}

/// Folds a fixed set of query results into one number.
///
/// Two runs printing the same checksum searched the same graph and reached the
/// same answers, which is the precondition for comparing their latencies at
/// all. A changed checksum means the comparison is meaningless — whatever else
/// moved, the graph moved too.
fn results_checksum(index: &HnswIndex, dim: usize) -> u64 {
    let mut checksum: u64 = 0;
    for s in 0..CHECKSUM_QUERIES {
        let q = generate_vector(dim, 7_000_000 + s);
        for (rank, r) in index.search(&q, 10).iter().enumerate() {
            checksum = checksum
                .wrapping_mul(1_000_003)
                .wrapping_add(r.id.wrapping_add(rank as u64));
        }
    }
    checksum
}

fn bench_search_by_graph_size(c: &mut Criterion) {
    let dim = env_usize("VELESDB_SCALE_DIM", DEFAULT_DIM);
    let mut group = c.benchmark_group("hnsw_search_by_graph_size");
    // Large builds make per-sample time uneven; flat sampling keeps criterion
    // from extrapolating an iteration count from an unrepresentative warm-up.
    group.sampling_mode(SamplingMode::Flat);
    group.measurement_time(Duration::from_secs(10));
    group.sample_size(30);

    for nodes in node_counts() {
        // Built once, outside the measured closure: this benchmark is about
        // search, and an index rebuilt per iteration would measure insert.
        //
        // Sequential rather than `insert_batch_parallel`, deliberately and at
        // the cost of build time: the parallel path builds a different graph
        // every run, which makes any A/B compare two graphs instead of two
        // layouts. See the "traps" section in the module docs.
        let index = HnswIndex::new(dim, DistanceMetric::Cosine).expect("bench: index");
        for i in 0..nodes as u64 {
            index.insert(i, &generate_vector(dim, i));
        }
        index.set_searching_mode();

        let adj8_mb = adjacency_bytes(nodes, 32, 8) / (1024 * 1024);
        let adj4_mb = adjacency_bytes(nodes, 32, 4) / (1024 * 1024);
        let vec_mb = nodes * dim * 4 / (1024 * 1024);
        let checksum = results_checksum(&index, dim);
        println!(
            "  [{nodes} nodes x {dim}d] adjacency ~{adj8_mb} MB @8-byte ids / ~{adj4_mb} MB \
             @4-byte ids (this bench cannot see which the build stores), vectors ~{vec_mb} MB, \
             checksum {checksum} — compare the working set against this machine's L3, and only \
             compare runs whose checksums match"
        );

        let query = generate_vector(dim, u64::MAX / 2);
        group.bench_with_input(BenchmarkId::new("top_k_10", nodes), &nodes, |b, _| {
            b.iter(|| black_box(index.search(&query, 10)));
        });
    }

    group.finish();
}

criterion_group!(benches, bench_search_by_graph_size);
criterion_main!(benches);
