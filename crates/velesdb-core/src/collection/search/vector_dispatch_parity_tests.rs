#![cfg(all(test, feature = "persistence"))]
//! Parity tests locking the invariant behaviour of the 5 vector-search dispatch
//! methods on `Collection` across the refactoring for issue #452 (Phase 3.4).
//!
//! These tests exercise every code path that shares the prologue
//! (`validate + read metric`) and epilogue
//! (`merge_delta + resolve + tag_vector_component_scores`) that will be
//! extracted into helpers. They MUST pass BEFORE the refactor (contract
//! validation on `develop`) and AFTER (regression protection).
//!
//! Covered dispatch paths:
//! - `Collection::search`                    — vector branch (no metadata_only)
//! - `Collection::search_with_ef`            — prologue+epilogue via ef bracket
//! - `Collection::search_with_quality`       — prologue+epilogue via explicit quality
//! - `Collection::search_with_opts{force_rerank=Some(true)}`  → `search_with_forced_rerank`
//! - `Collection::search_with_opts{force_rerank=Some(false)}` → `search_with_quality_no_rerank`

use crate::collection::search::query::QuerySearchOptions;
use crate::{collection::Collection, distance::DistanceMetric, point::Point};
use std::path::PathBuf;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const DIM: usize = 8;
const METRIC: DistanceMetric = DistanceMetric::Cosine;

/// Deterministic pseudo-random f32 vector.
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

fn create_populated_collection(n: u64) -> (Collection, TempDir) {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let collection =
        Collection::create(PathBuf::from(temp_dir.path()), DIM, METRIC).expect("create collection");
    let points: Vec<Point> = (0..n)
        .map(|i| Point::without_payload(i, make_vector(i, DIM)))
        .collect();
    collection.upsert(points).expect("upsert");
    (collection, temp_dir)
}

fn create_empty_collection() -> (Collection, TempDir) {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let collection =
        Collection::create(PathBuf::from(temp_dir.path()), DIM, METRIC).expect("create collection");
    (collection, temp_dir)
}

fn ids_of(results: &[crate::point::SearchResult]) -> Vec<u64> {
    results.iter().map(|r| r.point.id).collect()
}

fn opts_with_rerank(force_rerank: Option<bool>) -> QuerySearchOptions {
    QuerySearchOptions {
        quality: Some(crate::SearchQuality::Balanced),
        force_rerank,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Test 1: search() ≡ search_with_quality(Balanced) on same top-k IDs
// ---------------------------------------------------------------------------

#[test]
fn test_search_and_search_with_quality_balanced_match_top_k() {
    let (collection, _temp) = create_populated_collection(200);
    let query = make_vector(42, DIM);

    let a = collection.search(&query, 10).expect("search ok");
    let b = collection
        .search_with_quality(&query, 10, crate::SearchQuality::Balanced)
        .expect("search_with_quality ok");

    assert_eq!(a.len(), 10, "search returns k=10");
    assert_eq!(b.len(), 10, "search_with_quality returns k=10");
    assert_eq!(
        ids_of(&a),
        ids_of(&b),
        "search and search_with_quality(Balanced) must return identical top-k IDs"
    );
}

// ---------------------------------------------------------------------------
// Test 2: search_with_ef(128) ≡ search_with_quality(Balanced)
// ---------------------------------------------------------------------------

#[test]
fn test_search_with_ef_matches_search_with_quality_balanced() {
    let (collection, _temp) = create_populated_collection(200);
    let query = make_vector(7, DIM);

    let ef_path = collection
        .search_with_ef(&query, 10, 128)
        .expect("search_with_ef ok");
    let quality_path = collection
        .search_with_quality(&query, 10, crate::SearchQuality::Balanced)
        .expect("search_with_quality ok");

    assert_eq!(
        ids_of(&ef_path),
        ids_of(&quality_path),
        "search_with_ef(128) must match search_with_quality(Balanced) on IDs"
    );
}

// ---------------------------------------------------------------------------
// Test 3: search_with_opts{force_rerank=Some(true)} returns valid sorted top-k
// ---------------------------------------------------------------------------

#[test]
fn test_search_with_forced_rerank_returns_valid_top_k() {
    let (collection, _temp) = create_populated_collection(200);
    let query = make_vector(13, DIM);

    let opts = opts_with_rerank(Some(true));
    let results = collection
        .search_with_opts(&query, 10, &opts)
        .expect("forced rerank ok");

    assert_eq!(results.len(), 10, "returns exactly k=10");

    // Cosine is higher_is_better=true → scores monotone decreasing
    for window in results.windows(2) {
        assert!(
            window[0].score >= window[1].score,
            "forced-rerank results must be sorted desc on cosine: {} < {}",
            window[0].score,
            window[1].score
        );
    }
}

// ---------------------------------------------------------------------------
// Test 4: search_with_opts{force_rerank=Some(false)} bypasses rerank
// ---------------------------------------------------------------------------

#[test]
fn test_search_with_quality_no_rerank_returns_valid_top_k() {
    let (collection, _temp) = create_populated_collection(200);
    let query = make_vector(99, DIM);

    let opts_no_rerank = opts_with_rerank(Some(false));
    let results = collection
        .search_with_opts(&query, 10, &opts_no_rerank)
        .expect("no-rerank ok");

    assert_eq!(results.len(), 10, "returns exactly k=10");
    for window in results.windows(2) {
        assert!(
            window[0].score >= window[1].score,
            "no-rerank results must be sorted desc on cosine"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 5: every dispatch path tags component_scores with ("vector_score", score)
// ---------------------------------------------------------------------------

#[test]
fn test_all_search_paths_tag_vector_component_score() {
    let (collection, _temp) = create_populated_collection(50);
    let query = make_vector(1, DIM);

    let paths: Vec<(&str, Vec<crate::point::SearchResult>)> = vec![
        ("search", collection.search(&query, 5).expect("search")),
        (
            "search_with_ef",
            collection
                .search_with_ef(&query, 5, 64)
                .expect("search_with_ef"),
        ),
        (
            "search_with_quality",
            collection
                .search_with_quality(&query, 5, crate::SearchQuality::Balanced)
                .expect("search_with_quality"),
        ),
        (
            "search_with_opts(force_rerank=true)",
            collection
                .search_with_opts(&query, 5, &opts_with_rerank(Some(true)))
                .expect("forced rerank"),
        ),
        (
            "search_with_opts(force_rerank=false)",
            collection
                .search_with_opts(&query, 5, &opts_with_rerank(Some(false)))
                .expect("no rerank"),
        ),
    ];

    for (label, results) in &paths {
        assert!(
            !results.is_empty(),
            "{label}: must return at least one result"
        );
        for sr in results {
            let cs = sr.component_scores.as_ref().unwrap_or_else(|| {
                panic!(
                    "{label}: component_scores must be Some for id={}",
                    sr.point.id
                )
            });
            assert_eq!(cs.len(), 1, "{label}: exactly one component entry expected");
            assert_eq!(
                cs[0].0, "vector_score",
                "{label}: component key must be 'vector_score'"
            );
            assert_eq!(
                cs[0].1, sr.score,
                "{label}: component value must equal result score"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Test 6: dimension mismatch errors on every dispatch path
// ---------------------------------------------------------------------------

#[test]
fn test_dimension_mismatch_error_all_paths() {
    let (collection, _temp) = create_populated_collection(20);
    let wrong_dim_query = vec![0.5_f32; DIM + 8]; // 16-dim query on 8-dim collection

    assert!(
        matches!(
            collection.search(&wrong_dim_query, 5),
            Err(crate::error::Error::DimensionMismatch { .. })
        ),
        "search must reject dimension mismatch"
    );
    assert!(
        matches!(
            collection.search_with_ef(&wrong_dim_query, 5, 64),
            Err(crate::error::Error::DimensionMismatch { .. })
        ),
        "search_with_ef must reject dimension mismatch"
    );
    assert!(
        matches!(
            collection.search_with_quality(&wrong_dim_query, 5, crate::SearchQuality::Balanced),
            Err(crate::error::Error::DimensionMismatch { .. })
        ),
        "search_with_quality must reject dimension mismatch"
    );
    assert!(
        matches!(
            collection.search_with_opts(&wrong_dim_query, 5, &opts_with_rerank(Some(true))),
            Err(crate::error::Error::DimensionMismatch { .. })
        ),
        "search_with_opts(force_rerank=true) must reject dimension mismatch"
    );
    assert!(
        matches!(
            collection.search_with_opts(&wrong_dim_query, 5, &opts_with_rerank(Some(false))),
            Err(crate::error::Error::DimensionMismatch { .. })
        ),
        "search_with_opts(force_rerank=false) must reject dimension mismatch"
    );
}

// ---------------------------------------------------------------------------
// Test 7: empty collection → every path returns Ok(empty vec)
// ---------------------------------------------------------------------------

#[test]
fn test_empty_collection_all_paths() {
    let (collection, _temp) = create_empty_collection();
    let query = make_vector(0, DIM);

    assert!(
        collection
            .search(&query, 10)
            .expect("search empty")
            .is_empty(),
        "search on empty collection must return empty vec"
    );
    assert!(
        collection
            .search_with_ef(&query, 10, 64)
            .expect("search_with_ef empty")
            .is_empty(),
        "search_with_ef on empty must return empty"
    );
    assert!(
        collection
            .search_with_quality(&query, 10, crate::SearchQuality::Balanced)
            .expect("search_with_quality empty")
            .is_empty(),
        "search_with_quality on empty must return empty"
    );
    assert!(
        collection
            .search_with_opts(&query, 10, &opts_with_rerank(Some(true)))
            .expect("forced rerank empty")
            .is_empty(),
        "forced rerank on empty must return empty"
    );
    assert!(
        collection
            .search_with_opts(&query, 10, &opts_with_rerank(Some(false)))
            .expect("no rerank empty")
            .is_empty(),
        "no rerank on empty must return empty"
    );
}

// ---------------------------------------------------------------------------
// Test 8: metadata-only collection rejects search with SearchNotSupported
// ---------------------------------------------------------------------------

#[test]
fn test_metadata_only_collection_search_errors() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let collection = Collection::create_metadata_only(PathBuf::from(temp_dir.path()), "md_only")
        .expect("metadata-only");
    let query = vec![0.1_f32; DIM];

    assert!(
        matches!(
            collection.search(&query, 5),
            Err(crate::error::Error::SearchNotSupported(_))
        ),
        "metadata-only collections must reject vector search"
    );
}

// ---------------------------------------------------------------------------
// Test 9: search_with_opts with no options set falls through to search()
// ---------------------------------------------------------------------------

#[test]
fn test_search_with_opts_no_options_falls_back_to_search() {
    let (collection, _temp) = create_populated_collection(50);
    let query = make_vector(5, DIM);

    let via_default = collection.search(&query, 10).expect("search");
    let via_opts = collection
        .search_with_opts(&query, 10, &QuerySearchOptions::default())
        .expect("search_with_opts empty opts");

    assert_eq!(
        ids_of(&via_default),
        ids_of(&via_opts),
        "search_with_opts with no options must match plain search()"
    );
}
