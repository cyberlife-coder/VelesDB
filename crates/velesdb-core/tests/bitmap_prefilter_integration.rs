//! End-to-end integration tests for bitmap pre-filter V2 (Issue #487).
//!
//! Validates that filtered searches with quality options correctly use
//! the bitmap pre-filter pipeline at different selectivity levels.

#![cfg(feature = "persistence")]
#![allow(clippy::cast_precision_loss)]

use serde_json::json;
use tempfile::tempdir;
use velesdb_core::{DistanceMetric, Point, StorageMode, VectorCollection};

/// Creates a 1K-vector collection with a secondary index on "category".
///
/// Distribution: 50% "tech", 30% "science", 20% "art".
/// Vectors are 4-dimensional with deterministic values derived from the ID.
fn setup_collection() -> (VectorCollection, tempfile::TempDir) {
    let dir = tempdir().expect("temp dir");
    let col = VectorCollection::create(
        dir.path().join("bitmaptest"),
        "bitmaptest",
        4,
        DistanceMetric::Cosine,
        StorageMode::Full,
    )
    .expect("create collection");

    col.create_index("category")
        .expect("create secondary index on category");

    let points: Vec<Point> = (0u64..1000)
        .map(|id| {
            let category = match id % 10 {
                0..=4 => "tech",    // 50%
                5..=7 => "science", // 30%
                _ => "art",         // 20%
            };
            let payload = json!({ "category": category, "priority": id % 5 });

            // Deterministic vectors with slight variation per ID
            let base = (id as f32) * 0.001;
            let mut vector = vec![0.5 + base, 0.3 - base * 0.5, 0.1 + base * 0.3, 0.2];
            let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
            for x in &mut vector {
                *x /= norm;
            }

            Point::new(id, vector, Some(payload))
        })
        .collect();

    col.upsert(points).expect("upsert 1K points");
    (col, dir)
}

// =========================================================================
// Test: filtered search returns only matching category (high selectivity)
// =========================================================================

#[test]
fn test_bitmap_prefilter_tech_category_50pct() {
    let (col, _dir) = setup_collection();

    let query = vec![0.5_f32, 0.3, 0.1, 0.2];
    let filter = velesdb_core::filter::Filter::new(velesdb_core::filter::Condition::Eq {
        field: "category".to_string(),
        value: json!("tech"),
    });

    let results = col
        .search_with_filter(&query, 10, &filter)
        .expect("filtered search should succeed");

    assert!(!results.is_empty(), "should find tech results");
    for r in &results {
        let cat = r
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get("category"))
            .and_then(|v| v.as_str());
        assert_eq!(
            cat,
            Some("tech"),
            "all results must be tech, got id={}",
            r.point.id
        );
    }
}

// =========================================================================
// Test: filtered search with low selectivity (science = 30%)
// =========================================================================

#[test]
fn test_bitmap_prefilter_science_category_30pct() {
    let (col, _dir) = setup_collection();

    let query = vec![0.4_f32, 0.4, 0.2, 0.1];
    let filter = velesdb_core::filter::Filter::new(velesdb_core::filter::Condition::Eq {
        field: "category".to_string(),
        value: json!("science"),
    });

    let results = col
        .search_with_filter(&query, 10, &filter)
        .expect("filtered search should succeed");

    assert!(!results.is_empty(), "should find science results");
    for r in &results {
        let cat = r
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get("category"))
            .and_then(|v| v.as_str());
        assert_eq!(
            cat,
            Some("science"),
            "all results must be science, got id={}",
            r.point.id
        );
    }
}

// =========================================================================
// Test: filtered search with lowest selectivity (art = 20%)
// =========================================================================

#[test]
fn test_bitmap_prefilter_art_category_20pct() {
    let (col, _dir) = setup_collection();

    let query = vec![0.3_f32, 0.3, 0.3, 0.3];
    let filter = velesdb_core::filter::Filter::new(velesdb_core::filter::Condition::Eq {
        field: "category".to_string(),
        value: json!("art"),
    });

    let results = col
        .search_with_filter(&query, 10, &filter)
        .expect("filtered search should succeed");

    assert!(!results.is_empty(), "should find art results");
    for r in &results {
        let cat = r
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get("category"))
            .and_then(|v| v.as_str());
        assert_eq!(
            cat,
            Some("art"),
            "all results must be art, got id={}",
            r.point.id
        );
    }
}

// =========================================================================
// Test: nonexistent category returns empty
// =========================================================================

#[test]
fn test_bitmap_prefilter_nonexistent_category_returns_empty() {
    let (col, _dir) = setup_collection();

    let query = vec![0.5_f32, 0.3, 0.1, 0.2];
    let filter = velesdb_core::filter::Filter::new(velesdb_core::filter::Condition::Eq {
        field: "category".to_string(),
        value: json!("nonexistent"),
    });

    let results = col
        .search_with_filter(&query, 10, &filter)
        .expect("filtered search should succeed");

    assert!(
        results.is_empty(),
        "nonexistent category should return empty"
    );
}

// =========================================================================
// Test: results are sorted by similarity (cosine = descending)
// =========================================================================

#[test]
fn test_bitmap_prefilter_results_sorted_by_similarity() {
    let (col, _dir) = setup_collection();

    let query = vec![0.5_f32, 0.3, 0.1, 0.2];
    let filter = velesdb_core::filter::Filter::new(velesdb_core::filter::Condition::Eq {
        field: "category".to_string(),
        value: json!("tech"),
    });

    let results = col
        .search_with_filter(&query, 10, &filter)
        .expect("filtered search should succeed");

    // Cosine: higher is better → results should be sorted descending
    for i in 1..results.len() {
        assert!(
            results[i - 1].score >= results[i].score,
            "results should be sorted by similarity descending: {} >= {} at position {}",
            results[i - 1].score,
            results[i].score,
            i
        );
    }
}
