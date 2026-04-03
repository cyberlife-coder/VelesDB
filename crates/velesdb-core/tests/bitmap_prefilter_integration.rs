#![cfg(all(test, feature = "persistence"))]
//! End-to-end integration tests for the bitmap pre-filter V2 pipeline.
//!
//! Validates that filtered searches with secondary indexes correctly use
//! the bitmap pre-filter path at various selectivity levels (1%, 10%, 50%).

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;
use velesdb_core::{DistanceMetric, Point, StorageMode, VectorCollection};

/// Number of vectors in the test collection.
/// Using 1K instead of 10K for test speed while still exercising all paths.
const NUM_VECTORS: u64 = 1_000;
/// Vector dimensionality.
const DIM: usize = 16;

/// Creates a test collection with 1K vectors and a secondary index on "category".
///
/// Payload distribution:
/// - 1% have `category = "rare"` (≈10 vectors → full-scan path)
/// - 10% have `category = "uncommon"` (≈100 vectors → HNSW+bitmap path)
/// - 50% have `category = "common"` (≈500 vectors → HNSW+bitmap path)
/// - rest have `category = "default"`
fn setup_collection() -> (VectorCollection, TempDir) {
    let dir = TempDir::new().expect("temp dir");
    let collection = VectorCollection::create(
        dir.path().join("bitmap_test"),
        "bitmap_test",
        DIM,
        DistanceMetric::Cosine,
        StorageMode::Full,
    )
    .expect("create collection");

    collection
        .create_index("category")
        .expect("create secondary index on category");

    let points: Vec<Point> = (0..NUM_VECTORS)
        .map(|id| {
            let category = match id {
                i if i < (NUM_VECTORS / 100) => "rare",    // 1%
                i if i < (NUM_VECTORS / 10) => "uncommon", // next 9% → total 10%
                i if i < (NUM_VECTORS / 2) => "common",    // next 40% → total 50%
                _ => "default",                            // remaining 50%
            };

            let payload = json!({ "category": category, "seq": id });

            // Generate a deterministic vector with some variation per ID
            let mut vector: Vec<f32> = (0..DIM)
                .map(|d| {
                    let seed = (id as f32) * 0.13 + (d as f32) * 0.07;
                    seed.cos()
                })
                .collect();

            // Normalize for cosine metric
            let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm > 0.0 {
                for x in &mut vector {
                    *x /= norm;
                }
            }

            Point::new(id, vector, Some(payload))
        })
        .collect();

    collection.upsert(points).expect("upsert");
    (collection, dir)
}

/// Builds a query vector for searching.
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

// =========================================================================
// Filtered search at various selectivity levels
// =========================================================================

#[test]
fn test_bitmap_prefilter_rare_1pct_selectivity() {
    let (collection, _dir) = setup_collection();
    let query = query_vector();
    let mut params = HashMap::new();
    params.insert("v".to_string(), json!(query));

    // Use quality-aware path (WITH mode='balanced') which triggers the
    // bitmap prefilter → full_scan_with_bitmap path for ≤1% selectivity.
    let results = collection
        .execute_query_str(
            "SELECT * FROM bitmap_test WHERE vector NEAR $v AND category = 'rare' LIMIT 10 WITH (mode='balanced')",
            &params,
        )
        .expect("search rare via quality-aware path");

    assert!(!results.is_empty(), "rare filter should return results");
    for r in &results {
        let cat = r
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get("category"))
            .and_then(|v| v.as_str());
        assert_eq!(cat, Some("rare"), "all results must match category=rare");
    }
}

#[test]
fn test_bitmap_prefilter_uncommon_10pct_selectivity() {
    let (collection, _dir) = setup_collection();
    let query = query_vector();

    let results = collection
        .search_with_filter(
            &query,
            10,
            &velesdb_core::filter::Filter::new(velesdb_core::filter::Condition::eq(
                "category", "uncommon",
            )),
        )
        .expect("search uncommon");

    assert!(!results.is_empty(), "uncommon filter should return results");
    for r in &results {
        let cat = r
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get("category"))
            .and_then(|v| v.as_str());
        assert_eq!(
            cat,
            Some("uncommon"),
            "all results must match category=uncommon"
        );
    }
}

#[test]
fn test_bitmap_prefilter_common_50pct_selectivity() {
    let (collection, _dir) = setup_collection();
    let query = query_vector();

    let results = collection
        .search_with_filter(
            &query,
            10,
            &velesdb_core::filter::Filter::new(velesdb_core::filter::Condition::eq(
                "category", "common",
            )),
        )
        .expect("search common");

    assert!(!results.is_empty(), "common filter should return results");
    for r in &results {
        let cat = r
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get("category"))
            .and_then(|v| v.as_str());
        assert_eq!(
            cat,
            Some("common"),
            "all results must match category=common"
        );
    }
}

#[test]
fn test_bitmap_prefilter_quality_aware_via_velesql() {
    let (collection, _dir) = setup_collection();
    let mut params = HashMap::new();
    let query = query_vector();
    params.insert("v".to_string(), json!(query));

    // Exercise the search_with_filter_and_opts path via VelesQL WITH clause
    let results = collection
        .execute_query_str(
            "SELECT * FROM bitmap_test WHERE vector NEAR $v AND category = 'uncommon' LIMIT 10 WITH (mode='balanced')",
            &params,
        )
        .expect("quality-aware filtered search");

    assert!(
        !results.is_empty(),
        "quality-aware filtered search should return results"
    );
    for r in &results {
        let cat = r
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get("category"))
            .and_then(|v| v.as_str());
        assert_eq!(
            cat,
            Some("uncommon"),
            "quality-aware results must match filter"
        );
    }
}

#[test]
fn test_bitmap_prefilter_nonexistent_returns_empty() {
    let (collection, _dir) = setup_collection();
    let query = query_vector();

    let results = collection
        .search_with_filter(
            &query,
            10,
            &velesdb_core::filter::Filter::new(velesdb_core::filter::Condition::eq(
                "category",
                "nonexistent",
            )),
        )
        .expect("search nonexistent");

    assert!(
        results.is_empty(),
        "nonexistent category should return empty"
    );
}
