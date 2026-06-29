//! Criterion benchmark for the **filtered semantic recall** path (`recall_where`).
//!
//! `recall_where` issues a `vector NEAR $q AND <field> <op> $v` query, which the
//! planner runs one of two ways:
//! - **Post-filter (no index):** oversample HNSW, then linearly scan candidate
//!   payloads — work grows O(n) with the collection, so a selective filter that
//!   prunes to a handful of rows still pays for the whole collection.
//! - **Bitmap prefilter (indexed):** resolve the predicate to an id bitmap from
//!   the secondary B-tree index first, so search cost tracks the *matching* set,
//!   not the collection size.
//!
//! Until P3, the agent's `_semantic_memory` collection never created the index,
//! so `recall_where` always took the O(n) post-filter path. This benchmark
//! contrasts the two at a fixed, selective range filter as the collection grows:
//! the indexed series should stay roughly flat while the post-filter series
//! climbs with `n`.
//!
//! Run with: `cargo bench --bench recall_where_scaling_benchmark`

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use serde_json::json;
use velesdb_core::filter::{Condition, Filter};
use velesdb_core::{DistanceMetric, Point, StorageMode, VectorCollection};

/// Vector dimensionality (small — this bench is about filter strategy, not ANN).
const DIM: usize = 16;
/// Collection sizes to sweep; the post-filter cost should climb with these.
const SIZES: &[u64] = &[2_000, 10_000, 50_000];
/// Number of rows the range filter keeps, independent of collection size, so
/// selectivity tightens as `n` grows — exactly where O(n) post-filtering hurts.
const MATCHING_ROWS: u64 = 100;

/// Deterministic unit vector for a given id.
fn make_vector(id: u64) -> Vec<f32> {
    let mut v: Vec<f32> = (0..DIM)
        .map(|d| ((id as f32) * 0.13 + (d as f32) * 0.07).cos())
        .collect();
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

/// Deterministic normalized query vector.
fn query_vector() -> Vec<f32> {
    let mut v: Vec<f32> = (0..DIM).map(|d| (d as f32 * 0.1).sin()).collect();
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

/// Builds a collection of `n` points, each carrying a `ts` field equal to its id
/// (`0..n`). When `indexed`, a secondary index on `ts` is created so filtered
/// search takes the bitmap-prefilter path; otherwise it falls back to O(n)
/// post-filtering.
fn setup_collection(dir: &std::path::Path, n: u64, indexed: bool) -> VectorCollection {
    let collection = VectorCollection::create(
        dir.join("bench_col"),
        "bench_col",
        DIM,
        DistanceMetric::Cosine,
        StorageMode::Full,
    )
    .expect("bench: create collection");

    let points: Vec<Point> = (0..n)
        .map(|id| Point::new(id, make_vector(id), Some(json!({ "ts": id }))))
        .collect();
    collection.upsert(points).expect("bench: upsert");

    if indexed {
        // Created after the bulk upsert to exercise the backfill path the agent
        // hits on the first `recall_where` over an already-populated store.
        collection
            .create_index("ts")
            .expect("bench: create secondary index");
    }
    collection
}

/// A range filter keeping the top `MATCHING_ROWS` ids: `ts >= n - MATCHING_ROWS`.
fn selective_range_filter(n: u64) -> Filter {
    Filter::new(Condition::gte("ts", n.saturating_sub(MATCHING_ROWS)))
}

/// Sweeps both strategies across [`SIZES`]; compare the two series' slopes.
fn bench_recall_where_scaling(c: &mut Criterion) {
    let query = query_vector();
    let mut group = c.benchmark_group("recall_where_scaling");

    for &n in SIZES {
        let filter = selective_range_filter(n);

        let dir_idx = tempfile::tempdir().expect("bench: temp dir");
        let indexed = setup_collection(dir_idx.path(), n, true);
        group.bench_with_input(BenchmarkId::new("indexed_prefilter", n), &n, |b, _| {
            b.iter(|| {
                let results = indexed
                    .search_with_filter(black_box(&query), 10, black_box(&filter))
                    .expect("bench: indexed search");
                black_box(results)
            });
        });

        let dir_post = tempfile::tempdir().expect("bench: temp dir");
        let post = setup_collection(dir_post.path(), n, false);
        group.bench_with_input(BenchmarkId::new("post_filter_on", n), &n, |b, _| {
            b.iter(|| {
                let results = post
                    .search_with_filter(black_box(&query), 10, black_box(&filter))
                    .expect("bench: post-filter search");
                black_box(results)
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_recall_where_scaling);
criterion_main!(benches);
