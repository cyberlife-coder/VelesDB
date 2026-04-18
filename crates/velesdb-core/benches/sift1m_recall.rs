//! SIFT1M recall + latency Criterion benchmark.
//!
//! Runs the canonical ANN benchmark on the INRIA TEXMEX SIFT1M
//! corpus (1M × 128D, L2 metric). Sweeps `ef_search` ∈ {64, 128, 256, 512}
//! and reports:
//!   - p50 search latency (via Criterion sampling) for each ef
//!   - Recall@10 measured across the full 10,000-query set (printed to
//!     stdout as `RECALL_REPORT\tef=<E>\trecall@10=<R>` — grep-friendly)
//!
//! Build with the gating feature, otherwise the bench is not discovered:
//! ```text
//! cargo bench -p velesdb-core --bench sift1m_recall --features bench-sift1m
//! VELESDB_SIFT1M_DIR=/data/sift1m cargo bench ...  # pre-populated data
//! ```
//!
//! First run downloads ≈168 MB from the INRIA mirror and extracts to
//! `target/bench-data/sift1m/`. Subsequent runs read from cache.

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use velesdb_core::distance::DistanceMetric;
use velesdb_core::{HnswIndex, HnswParams, ScoredResult, SearchQuality, VectorIndex};

#[path = "datasets/mod.rs"]
mod datasets;

use datasets::sift1m::{load_sift1m, load_sift1m_subset, DatasetError, Sift1M};

const K: usize = 10;
const EF_VALUES: &[usize] = &[64, 128, 256, 512];
/// Env override: set `VELESDB_SIFT1M_SUBSET_BASE=10000` and
/// `VELESDB_SIFT1M_SUBSET_QUERY=100` for a smoke run that exercises the
/// loader+index path without the full 1M build (useful in agent sessions).
const ENV_SUBSET_BASE: &str = "VELESDB_SIFT1M_SUBSET_BASE";
const ENV_SUBSET_QUERY: &str = "VELESDB_SIFT1M_SUBSET_QUERY";

fn bench_sift1m_recall_at_10(c: &mut Criterion) {
    let data = match try_load() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[sift1m_recall] skipping: {e}");
            return;
        }
    };

    eprintln!(
        "[sift1m_recall] dataset: base={} queries={} gt_k={}",
        data.base.len(),
        data.query.len(),
        data.groundtruth.first().map_or(0, Vec::len)
    );

    let index = build_hnsw_from_base(&data.base);

    run_latency_sweep(c, &index, &data);
    report_recall(&index, &data);
}

/// Dispatches to the subset loader when either env var is set (smoke),
/// otherwise loads the full 1M corpus.
fn try_load() -> Result<Sift1M, DatasetError> {
    if let (Some(nb), Some(nq)) = (env_usize(ENV_SUBSET_BASE), env_usize(ENV_SUBSET_QUERY)) {
        eprintln!("[sift1m_recall] subset mode: base={nb} query={nq}");
        return load_sift1m_subset(nb, nq);
    }
    load_sift1m()
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name).ok().and_then(|v| v.parse().ok())
}

// ---------------------------------------------------------------------------
// Index construction
// ---------------------------------------------------------------------------

fn build_hnsw_from_base(base: &[Vec<f32>]) -> HnswIndex {
    let dim = base.first().map_or(128, Vec::len);
    // Start from `auto(dim)` to fill forward-compatible fields (`alpha`,
    // `storage_mode`), then override the knobs the methodology fixes
    // (M=16, efConstruction=200) — matches the canonical HNSWlib SIFT1M
    // numbers in the literature.
    let mut params = HnswParams::auto(dim);
    params.max_connections = 16;
    params.ef_construction = 200;
    params.max_elements = base.len().max(1);
    let index = HnswIndex::with_params(dim, DistanceMetric::Euclidean, params)
        .expect("sift1m: HNSW construction must succeed with valid params");

    for (idx, vec) in base.iter().enumerate() {
        // VectorIndex::insert is fire-and-forget (returns ()); the trait
        // logs and drops dimension errors. We've validated dim shape in
        // the loader so this is always well-formed.
        index.insert(idx as u64, vec);
    }
    index
}

// ---------------------------------------------------------------------------
// Criterion sweep
// ---------------------------------------------------------------------------

fn run_latency_sweep(c: &mut Criterion, index: &HnswIndex, data: &Sift1M) {
    let mut group = c.benchmark_group("sift1m_recall_at_10");
    group.sample_size(20);
    group.throughput(Throughput::Elements(1));

    for &ef in EF_VALUES {
        group.bench_with_input(BenchmarkId::from_parameter(ef), &ef, |b, &ef| {
            let quality = SearchQuality::Custom(ef);
            let mut cursor = 0_usize;
            b.iter(|| {
                let q = &data.query[cursor % data.query.len()];
                cursor = cursor.wrapping_add(1);
                index
                    .search_with_quality(q, K, quality)
                    .expect("sift1m: search must succeed for valid query")
            });
        });
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// Recall reporting (separate pass, not a Criterion measurement)
// ---------------------------------------------------------------------------

fn report_recall(index: &HnswIndex, data: &Sift1M) {
    for &ef in EF_VALUES {
        let recall = measure_recall_at_10(index, &data.query, &data.groundtruth, ef);
        println!("RECALL_REPORT\tef={ef}\trecall@10={recall:.4}");
    }
}

fn measure_recall_at_10(
    index: &HnswIndex,
    queries: &[Vec<f32>],
    groundtruth: &[Vec<u32>],
    ef: usize,
) -> f64 {
    let quality = SearchQuality::Custom(ef);
    let mut sum = 0.0_f64;
    let mut counted = 0_usize;
    for (q, gt) in queries.iter().zip(groundtruth.iter()) {
        // Subset-mode may leave a groundtruth row empty after filtering
        // out-of-range IDs — nothing to score, skip rather than bias the mean.
        if gt.is_empty() {
            continue;
        }
        let Ok(results) = index.search_with_quality(q, K, quality) else {
            continue;
        };
        sum += intersection_ratio(&results, gt, K);
        counted += 1;
    }
    if counted == 0 {
        0.0
    } else {
        sum / counted as f64
    }
}

/// Intersection ratio between a search result set and the top-k of groundtruth.
///
/// Denominator is `min(k, groundtruth.len())` so that subset-mode runs
/// (where `load_sift1m_subset` filters out-of-range IDs per row) still
/// report meaningful recall. Returns `0.0` when the groundtruth row is
/// empty after filtering (no valid neighbours remain in the truncated base).
fn intersection_ratio(results: &[ScoredResult], groundtruth: &[u32], k: usize) -> f64 {
    let denom = k.min(groundtruth.len());
    if denom == 0 {
        return 0.0;
    }
    let gt_top: std::collections::HashSet<u64> = groundtruth
        .iter()
        .take(k)
        .map(|&id| u64::from(id))
        .collect();
    let hits = results.iter().filter(|r| gt_top.contains(&r.id)).count();
    hits as f64 / denom as f64
}

criterion_group!(benches, bench_sift1m_recall_at_10);
criterion_main!(benches);
