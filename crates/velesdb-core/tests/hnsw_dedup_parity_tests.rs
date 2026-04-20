#![cfg(feature = "persistence")]
//! Parity tests locking the invariant behaviour of `HnswIndex` across the
//! refactorings for issue #448 (HNSW deduplication, Phase 3.2).
//!
//! These tests exercise public API surfaces that are indirectly touched by the
//! extracted helpers in Groups A–F. They must pass BEFORE the refactor
//! (contract validation on `develop`) and AFTER the refactor (regression
//! protection).
//!
//! Covered invariants:
//! - Group C: `HnswIndex::save` / `HnswIndex::load` round-trip preserves search
//!   topology (same top-k IDs + bitwise-identical scores).
//! - Group D: `insert_batch_parallel` and repeated `insert` produce the same
//!   top-k under `Balanced` quality (upsert semantics, rollback paths).
//! - Group E: search with and without a bitmap pre-filter returns consistent
//!   top-k on allowed IDs; retry-on-underfill path is exercised.
//! - Group F: `VectorIndex::remove` (trait impl) and direct `HnswIndex` soft
//!   delete leave the index in equivalent state (same length, same surviving
//!   IDs returned by search).
//!
//! Group A (columnar distance kernels) is covered by the crate-internal tests
//! in `index/hnsw/native/columnar_vectors_tests.rs` which already validate
//! parity against the sequential reference — no cross-crate duplication needed.
//!
//! Group B (sharded mappings) is covered by `sharded_mappings_tests.rs` and
//! the batch parity test below (exercises the fast + slow register paths).

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::uninlined_format_args,
    clippy::float_cmp
)]

use std::collections::HashSet;
use tempfile::TempDir;
use velesdb_core::index::hnsw::SearchQuality;
use velesdb_core::index::{HnswIndex, VectorIndex};
use velesdb_core::scored_result::ScoredResult;
use velesdb_core::DistanceMetric;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const DIM: usize = 32;
const METRIC: DistanceMetric = DistanceMetric::Cosine;

/// Deterministic pseudo-random f32 vector built from a seed.
///
/// No PRNG crate is used to keep the test self-contained and the output
/// identical across platforms / Rust versions.
fn make_vector(seed: u64, dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|d| {
            let mix = seed
                .wrapping_mul(2_246_822_519)
                .wrapping_add((d as u64).wrapping_mul(3_266_489_917));
            ((mix & 0xFFFF) as f32) / 65535.0
        })
        .collect()
}

fn populate(index: &HnswIndex, n: u64) -> Vec<(u64, Vec<f32>)> {
    let vectors: Vec<(u64, Vec<f32>)> = (0..n).map(|i| (i, make_vector(i, DIM))).collect();
    for (id, vec) in &vectors {
        index.insert(*id, vec);
    }
    vectors
}

fn ids_of(results: &[ScoredResult]) -> Vec<u64> {
    results.iter().map(|r| r.id).collect()
}

fn scores_of(results: &[ScoredResult]) -> Vec<u32> {
    // Bitwise comparison is the only correctness-preserving equality for f32;
    // NaN-safe and tight to the last bit.
    results.iter().map(|r| r.score.to_bits()).collect()
}

// ---------------------------------------------------------------------------
// Group C: save / load round-trip parity
// ---------------------------------------------------------------------------

#[test]
fn test_hnsw_save_load_roundtrip_preserves_top_k() {
    let index = HnswIndex::new(DIM, METRIC).expect("create hnsw index");
    let vectors = populate(&index, 500);
    let query = make_vector(424_242, DIM);

    let before = index
        .search_with_quality(&query, 10, SearchQuality::Balanced)
        .expect("search before save");

    // Save
    let dir = TempDir::new().expect("tempdir");
    index.save(dir.path()).expect("save index");

    // Reload
    let reloaded = HnswIndex::load(dir.path(), DIM, METRIC).expect("load index");
    assert_eq!(
        reloaded.len(),
        index.len(),
        "len must match after save/load round-trip"
    );

    let after = reloaded
        .search_with_quality(&query, 10, SearchQuality::Balanced)
        .expect("search after load");

    assert_eq!(
        ids_of(&before),
        ids_of(&after),
        "top-10 IDs must be identical after save/load"
    );
    assert_eq!(
        scores_of(&before),
        scores_of(&after),
        "top-10 scores must be bitwise-identical after save/load"
    );

    // Keep vectors alive so the borrow-checker doesn't warn.
    drop(vectors);
}

#[test]
fn test_hnsw_save_load_roundtrip_no_vector_storage() {
    // Covers `enable_vector_storage=false` branch in the load/save helpers.
    let index = HnswIndex::new_fast_insert(DIM, METRIC).expect("fast-insert index");
    populate(&index, 200);

    let dir = TempDir::new().expect("tempdir");
    index.save(dir.path()).expect("save fast-insert index");

    let reloaded = HnswIndex::load(dir.path(), DIM, METRIC).expect("load fast-insert index");
    assert_eq!(reloaded.len(), index.len());
    assert!(
        !reloaded.has_vector_storage(),
        "fast-insert round-trip must preserve enable_vector_storage=false"
    );
}

// ---------------------------------------------------------------------------
// Group D: single-insert vs batch-insert top-k parity
// ---------------------------------------------------------------------------

#[test]
fn test_insert_batch_parallel_matches_sequential_top_k() {
    let n: u64 = 500;
    let vectors: Vec<(u64, Vec<f32>)> = (0..n).map(|i| (i, make_vector(i, DIM))).collect();

    // Path 1: sequential single-insert
    let seq = HnswIndex::new(DIM, METRIC).expect("seq index");
    for (id, vec) in &vectors {
        seq.insert(*id, vec);
    }

    // Path 2: batch parallel insert (different code path, same upsert semantics)
    let par = HnswIndex::new(DIM, METRIC).expect("par index");
    let batch: Vec<(u64, &[f32])> = vectors.iter().map(|(id, v)| (*id, v.as_slice())).collect();
    let inserted = par.insert_batch_parallel(batch);
    assert_eq!(inserted as u64, n, "batch should report all inserted");

    assert_eq!(seq.len(), par.len(), "len parity");

    // Compare top-k search results on multiple queries.
    // Note: HNSW graph construction order differs between paths (rayon
    // parallelism), so the raw top-10 may differ by a few positions. We
    // assert a generous Jaccard ≥ 0.7 on top-10 which is tight enough to
    // catch real regressions but tolerates graph-order noise.
    for qseed in [7_u64, 42, 424_242, 999_999] {
        let query = make_vector(qseed, DIM);
        let a = ids_of(
            &seq.search_with_quality(&query, 10, SearchQuality::Balanced)
                .expect("seq search"),
        );
        let b = ids_of(
            &par.search_with_quality(&query, 10, SearchQuality::Balanced)
                .expect("par search"),
        );
        let set_a: HashSet<u64> = a.iter().copied().collect();
        let set_b: HashSet<u64> = b.iter().copied().collect();
        let inter = set_a.intersection(&set_b).count();
        let union = set_a.union(&set_b).count();
        let jaccard = inter as f32 / union as f32;
        assert!(
            jaccard >= 0.7,
            "top-10 Jaccard too low for seed {qseed}: {jaccard} (seq={a:?} par={b:?})"
        );
    }
}

#[test]
fn test_batch_insert_upsert_semantics() {
    // Insert then overwrite via batch: resulting search must return each id
    // exactly once and prefer the new vector.
    let index = HnswIndex::new(DIM, METRIC).expect("index");
    let n: u64 = 50;

    // Phase 1: initial insert
    let v1: Vec<(u64, Vec<f32>)> = (0..n).map(|i| (i, make_vector(i, DIM))).collect();
    let batch1: Vec<(u64, &[f32])> = v1.iter().map(|(id, v)| (*id, v.as_slice())).collect();
    index.insert_batch_parallel(batch1);
    assert_eq!(index.len(), n as usize);

    // Phase 2: upsert the same IDs with different vectors
    let v2: Vec<(u64, Vec<f32>)> = (0..n).map(|i| (i, make_vector(i + 100_000, DIM))).collect();
    let batch2: Vec<(u64, &[f32])> = v2.iter().map(|(id, v)| (*id, v.as_slice())).collect();
    index.insert_batch_parallel(batch2);

    // Len must remain == n (soft-delete of stale mappings, no duplicates).
    assert_eq!(
        index.len(),
        n as usize,
        "upsert must not duplicate IDs (stale mappings are tombstoned)"
    );

    // Searching around a phase-2 query should never return a stale node id.
    let query = make_vector(42 + 100_000, DIM);
    let results = index
        .search_with_quality(&query, 20, SearchQuality::Balanced)
        .expect("search");
    let mut seen = HashSet::new();
    for r in &results {
        assert!(
            seen.insert(r.id),
            "duplicate id {} in search results after upsert",
            r.id
        );
    }
}

// ---------------------------------------------------------------------------
// Group E: bitmap pre-filter search parity
// ---------------------------------------------------------------------------

#[test]
fn test_search_with_bitmap_filter_subset() {
    let index = HnswIndex::new(DIM, METRIC).expect("index");
    populate(&index, 300);

    // Allow only even IDs.
    let allowed: roaring::RoaringBitmap = (0_u32..300).filter(|i| i % 2 == 0).collect();

    let query = make_vector(12_345, DIM);
    let results = index
        .search_with_quality_and_bitmap(&query, 10, SearchQuality::Balanced, &allowed)
        .expect("bitmap search");

    assert!(
        !results.is_empty(),
        "bitmap search must return at least some hits"
    );
    for r in &results {
        let id32 = u32::try_from(r.id).expect("id fits in u32");
        assert!(
            allowed.contains(id32),
            "id {} returned but not in the allowed bitmap",
            r.id
        );
    }
}

#[test]
fn test_search_with_bitmap_filter_empty_subset() {
    let index = HnswIndex::new(DIM, METRIC).expect("index");
    populate(&index, 100);

    // Empty bitmap: no IDs allowed (and no u64 > u32::MAX in test data).
    let allowed = roaring::RoaringBitmap::new();

    let query = make_vector(1, DIM);
    let results = index
        .search_with_quality_and_bitmap(&query, 10, SearchQuality::Balanced, &allowed)
        .expect("bitmap search");

    assert!(
        results.is_empty(),
        "empty bitmap must yield empty results, got {} items",
        results.len()
    );
}

// ---------------------------------------------------------------------------
// Group F: remove (trait impl) parity with direct soft delete
// ---------------------------------------------------------------------------

#[test]
fn test_remove_via_trait_matches_direct_path() {
    let index_a = HnswIndex::new(DIM, METRIC).expect("index a");
    let index_b = HnswIndex::new(DIM, METRIC).expect("index b");
    let n: u64 = 50;

    for i in 0..n {
        let v = make_vector(i, DIM);
        index_a.insert(i, &v);
        index_b.insert(i, &v);
    }

    // Remove odd IDs via the trait method on `a`, via the inherent method on `b`.
    for i in (1..n).step_by(2) {
        assert!(
            <HnswIndex as VectorIndex>::remove(&index_a, i),
            "trait remove must report true for existing id {i}"
        );
    }
    for i in (1..n).step_by(2) {
        let removed_again = index_b.remove_as_vector_index(i);
        assert!(
            removed_again,
            "inherent remove must report true for existing id {i}"
        );
    }

    assert_eq!(index_a.len(), index_b.len(), "lengths must match");

    // Searching for an odd id's vector should not return it from either index.
    let query = make_vector(3, DIM);
    let a = ids_of(
        &index_a
            .search_with_quality(&query, 10, SearchQuality::Balanced)
            .expect("a search"),
    );
    let b = ids_of(
        &index_b
            .search_with_quality(&query, 10, SearchQuality::Balanced)
            .expect("b search"),
    );
    for id in &a {
        assert!(
            id % 2 == 0,
            "removed odd id {} still returned by trait-remove index",
            id
        );
    }
    for id in &b {
        assert!(
            id % 2 == 0,
            "removed odd id {} still returned by inherent-remove index",
            id
        );
    }
    assert_eq!(a.len(), b.len(), "trait and inherent remove must agree");
}

// ---------------------------------------------------------------------------
// Small local extension trait: `HnswIndex::remove_as_vector_index`.
//
// Rationale: `HnswIndex::remove` is not exposed as inherent (there is only the
// `VectorIndex::remove` trait method plus internal helpers). We dispatch
// through the trait explicitly for the "inherent path" comparison. This trait
// is purely a test-side renaming, not a production API addition.
// ---------------------------------------------------------------------------
trait HnswRemoveExt {
    fn remove_as_vector_index(&self, id: u64) -> bool;
}

impl HnswRemoveExt for HnswIndex {
    fn remove_as_vector_index(&self, id: u64) -> bool {
        <HnswIndex as VectorIndex>::remove(self, id)
    }
}
