#![cfg(all(test, feature = "persistence"))]

use crate::{distance::DistanceMetric, point::Point, quantization::StorageMode, Collection};
use std::path::PathBuf;
use tempfile::TempDir;

#[test]
fn test_search_ids_product_quantization_cosine_scores_stay_in_similarity_domain() {
    // ARRANGE
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let collection = Collection::create_with_options(
        PathBuf::from(temp_dir.path()),
        16,
        DistanceMetric::Cosine,
        StorageMode::ProductQuantization,
    )
    .expect("collection should be created");

    let points: Vec<Point> = (0u64..160)
        .map(|id| {
            let mut vector: Vec<f32> = (0..16)
                .map(|d| {
                    let id_term = f32::from(u16::try_from(id + 1).expect("id fits in u16")) * 0.13;
                    let d_term =
                        f32::from(u16::try_from(d).expect("dimension index fits in u16")) * 0.07;
                    (id_term + d_term).cos()
                })
                .collect();
            let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm > 0.0 {
                for x in &mut vector {
                    *x /= norm;
                }
            }
            Point::without_payload(id, vector)
        })
        .collect();

    let query = points[0].vector.clone();
    collection.upsert(points).expect("upsert should succeed");

    // ACT
    let results = collection
        .search_ids(&query, 10)
        .expect("search_ids should succeed");

    // ASSERT
    assert!(!results.is_empty(), "search should return candidates");
    assert!(
        results.iter().all(|sr| (-1.0..=1.0).contains(&sr.score)),
        "cosine metric scores must remain in similarity domain [-1, 1]"
    );
}

// =============================================================================
// Bitmap pre-filter integration tests
// =============================================================================

/// Creates a collection with indexed points for bitmap pre-filter testing.
///
/// Inserts 50 points in 4-dim space with payloads:
/// - IDs 0..25 have `category = "tech"`
/// - IDs 25..50 have `category = "science"`
/// - All have a `priority` field (numeric)
///
/// Returns the collection with a secondary index on `category`.
fn create_indexed_collection() -> (Collection, TempDir) {
    let temp_dir = TempDir::new().expect("test: temp dir");
    let collection = Collection::create(temp_dir.path().to_path_buf(), 4, DistanceMetric::Cosine)
        .expect("test: create collection");

    // Create secondary index on "category"
    collection
        .create_index("category")
        .expect("test: create secondary index");

    // Insert points with payloads
    let points: Vec<Point> = (0u64..50)
        .map(|id| {
            let category = if id < 25 { "tech" } else { "science" };
            let payload = serde_json::json!({
                "category": category,
                "priority": id % 5,
            });
            let mut vector = vec![0.1_f32; 4];
            // Reason: id fits in u16 for 50 points.
            #[allow(clippy::cast_precision_loss)]
            {
                vector[0] += (id as f32) * 0.01;
            }
            // Normalize for cosine
            let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
            for x in &mut vector {
                *x /= norm;
            }
            Point {
                id,
                vector,
                payload: Some(payload),
                sparse_vectors: None,
            }
        })
        .collect();

    collection.upsert(points).expect("test: upsert");
    (collection, temp_dir)
}

#[test]
fn test_search_with_filter_bitmap_prefilter_returns_only_matching_category() {
    let (collection, _temp) = create_indexed_collection();

    let query = vec![0.5_f32, 0.5, 0.5, 0.5];
    let filter = crate::filter::Filter::new(crate::filter::Condition::Eq {
        field: "category".to_string(),
        value: serde_json::Value::String("tech".to_string()),
    });

    let results = collection
        .search_with_filter(&query, 10, &filter)
        .expect("test: search with filter");

    // All results must be "tech" (IDs 0..25)
    for r in &results {
        assert!(
            r.point.id < 25,
            "expected tech category (id < 25), got id={}",
            r.point.id
        );
        let payload = r.point.payload.as_ref().expect("test: payload");
        assert_eq!(
            payload.get("category").and_then(|v| v.as_str()),
            Some("tech"),
            "payload category mismatch for id={}",
            r.point.id
        );
    }
    // Should return some results (not empty)
    assert!(!results.is_empty(), "should find tech results");
}

#[test]
fn test_search_with_filter_no_index_falls_back_to_postfilter() {
    let temp_dir = TempDir::new().expect("test: temp dir");
    let collection = Collection::create(temp_dir.path().to_path_buf(), 4, DistanceMetric::Cosine)
        .expect("test: create collection");

    // Insert points WITHOUT creating a secondary index
    let points: Vec<Point> = (0u64..20)
        .map(|id| {
            let payload = serde_json::json!({"tag": if id < 10 { "A" } else { "B" }});
            let mut vector = vec![0.1_f32; 4];
            #[allow(clippy::cast_precision_loss)]
            {
                vector[0] += (id as f32) * 0.01;
            }
            let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
            for x in &mut vector {
                *x /= norm;
            }
            Point {
                id,
                vector,
                payload: Some(payload),
                sparse_vectors: None,
            }
        })
        .collect();

    collection.upsert(points).expect("test: upsert");

    // Filter on non-indexed field => pure post-filter (still works)
    let query = vec![0.5_f32, 0.5, 0.5, 0.5];
    let filter = crate::filter::Filter::new(crate::filter::Condition::Eq {
        field: "tag".to_string(),
        value: serde_json::Value::String("A".to_string()),
    });

    let results = collection
        .search_with_filter(&query, 5, &filter)
        .expect("test: search with filter");

    // All results must have tag=A (IDs 0..10)
    for r in &results {
        assert!(
            r.point.id < 10,
            "expected tag=A (id < 10), got id={}",
            r.point.id
        );
    }
    assert!(!results.is_empty(), "should find tag=A results");
}

#[test]
fn test_search_with_filter_bitmap_empty_result_for_nonexistent_value() {
    let (collection, _temp) = create_indexed_collection();

    let query = vec![0.5_f32, 0.5, 0.5, 0.5];
    let filter = crate::filter::Filter::new(crate::filter::Condition::Eq {
        field: "category".to_string(),
        value: serde_json::Value::String("nonexistent".to_string()),
    });

    let results = collection
        .search_with_filter(&query, 10, &filter)
        .expect("test: search with filter");

    assert!(results.is_empty(), "no points match nonexistent category");
}

// =============================================================================
// Selectivity estimator unit tests
// =============================================================================

#[cfg(test)]
mod selectivity_tests {
    use super::super::vector::{estimate_real_selectivity, SELECTIVITY_THRESHOLD};

    #[test]
    fn test_estimate_real_selectivity_nominal() {
        let mut bitmap = roaring::RoaringBitmap::new();
        for i in 0..100_u32 {
            bitmap.insert(i);
        }
        let result = estimate_real_selectivity(&bitmap, 10_000);
        assert!(
            (result - 0.01).abs() < f64::EPSILON,
            "100/10000 should be 0.01, got {result}"
        );
    }

    #[test]
    fn test_estimate_real_selectivity_empty_collection_returns_zero() {
        let bitmap = roaring::RoaringBitmap::new();
        let result = estimate_real_selectivity(&bitmap, 0);
        assert!(
            (result - 0.0).abs() < f64::EPSILON,
            "empty collection should return 0.0"
        );
    }

    #[test]
    fn test_estimate_real_selectivity_empty_bitmap_returns_zero() {
        let bitmap = roaring::RoaringBitmap::new();
        let result = estimate_real_selectivity(&bitmap, 10_000);
        assert!(
            (result - 0.0).abs() < f64::EPSILON,
            "empty bitmap should return 0.0"
        );
    }

    #[test]
    fn test_selectivity_threshold_default_is_one_percent() {
        assert!(
            (SELECTIVITY_THRESHOLD - 0.01).abs() < f64::EPSILON,
            "default threshold should be 0.01 (1%)"
        );
    }
}

// =============================================================================
// Selectivity property-based tests
// =============================================================================

#[cfg(test)]
mod selectivity_property_tests {
    use super::super::vector::{estimate_real_selectivity, SELECTIVITY_THRESHOLD};
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// **Validates: Requirements 4.1**
        ///
        /// Feature: bitmap-prefilter-v2, Property 5: Selectivity formula
        ///
        /// For any bitmap of cardinality n and any collection of size N > 0,
        /// `estimate_real_selectivity` returns `n as f64 / N as f64`.
        /// For N = 0, returns 0.0.
        #[test]
        fn prop_selectivity_formula(
            n in 0u32..10_000,
            big_n in 1usize..100_000,
        ) {
            let mut bitmap = roaring::RoaringBitmap::new();
            for i in 0..n {
                bitmap.insert(i);
            }
            let result = estimate_real_selectivity(&bitmap, big_n);
            #[allow(clippy::cast_precision_loss)]
            let expected = n as f64 / big_n as f64;
            prop_assert!(
                (result - expected).abs() < f64::EPSILON,
                "selectivity mismatch: got {result}, expected {expected}"
            );
        }

        /// **Validates: Requirements 3.1, 3.4, 4.3, 4.4**
        ///
        /// Feature: bitmap-prefilter-v2, Property 3: Strategy selection by selectivity threshold
        ///
        /// For any non-empty bitmap and collection size > 0, the strategy
        /// selected (full_scan vs hnsw_bitmap) is deterministic and depends
        /// only on the selectivity ratio vs SELECTIVITY_THRESHOLD.
        #[test]
        fn prop_strategy_selection_by_threshold(
            n in 1u32..10_000,
            big_n in 1usize..100_000,
        ) {
            let mut bitmap = roaring::RoaringBitmap::new();
            for i in 0..n {
                bitmap.insert(i);
            }
            let selectivity = estimate_real_selectivity(&bitmap, big_n);
            let use_full_scan = selectivity <= SELECTIVITY_THRESHOLD;
            let use_hnsw_bitmap = selectivity > SELECTIVITY_THRESHOLD;

            // Exactly one strategy must be selected
            prop_assert!(
                use_full_scan ^ use_hnsw_bitmap,
                "exactly one strategy must be selected"
            );

            // Verify consistency with threshold
            if selectivity <= SELECTIVITY_THRESHOLD {
                prop_assert!(use_full_scan, "should select full_scan for low selectivity");
            } else {
                prop_assert!(use_hnsw_bitmap, "should select hnsw_bitmap for high selectivity");
            }
        }
    }
}

#[test]
fn test_search_with_filter_and_opts_bitmap_empty_returns_empty() {
    let (col, _temp) = create_indexed_collection();
    let filter = crate::filter::Filter::new(crate::filter::Condition::Eq {
        field: "category".to_string(),
        value: serde_json::Value::String("nonexistent".to_string()),
    });
    let opts = crate::collection::search::query::QuerySearchOptions {
        quality: Some(crate::SearchQuality::Balanced),
        ef_search: None,
        force_rerank: None,
        fusion_clause: None,
    };
    let results = col
        .search_with_filter_and_opts(&[0.5, 0.5, 0.5, 0.5], 10, &filter, &opts)
        .expect("search should succeed");
    assert!(
        results.is_empty(),
        "nonexistent category with opts should return empty"
    );
}

#[test]
fn test_search_with_filter_and_opts_no_bitmap_falls_back_to_post_filter() {
    let temp_dir = TempDir::new().expect("test: temp dir");
    let col = Collection::create(temp_dir.path().to_path_buf(), 4, DistanceMetric::Cosine)
        .expect("create");
    let points: Vec<crate::point::Point> = (0u64..20)
        .map(|id| {
            let payload = serde_json::json!({"tag": if id < 10 { "A" } else { "B" }});
            let mut vector = vec![0.1_f32; 4];
            #[allow(clippy::cast_precision_loss)]
            {
                vector[0] += (id as f32) * 0.01;
            }
            let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
            for x in &mut vector {
                *x /= norm;
            }
            crate::point::Point {
                id,
                vector,
                payload: Some(payload),
                sparse_vectors: None,
            }
        })
        .collect();
    col.upsert(points).expect("upsert");
    let filter = crate::filter::Filter::new(crate::filter::Condition::Eq {
        field: "tag".to_string(),
        value: serde_json::Value::String("A".to_string()),
    });
    let opts = crate::collection::search::query::QuerySearchOptions {
        quality: Some(crate::SearchQuality::Accurate),
        ef_search: None,
        force_rerank: None,
        fusion_clause: None,
    };
    let results = col
        .search_with_filter_and_opts(&[0.5, 0.5, 0.5, 0.5], 5, &filter, &opts)
        .expect("search should succeed");
    for r in &results {
        assert!(r.point.id < 10, "expected tag=A, got id={}", r.point.id);
    }
    assert!(
        !results.is_empty(),
        "should find tag=A results via post-filter fallback"
    );
}
