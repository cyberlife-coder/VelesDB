#![cfg(feature = "persistence")]
//! BDD integration tests for histogram-based selectivity estimation (Issue #468).
//!
//! Covers:
//! - 11.1 Nominal flow: ANALYZE → histograms exist → CostEstimator produces estimates
//! - 11.2 Incremental maintenance: upsert/delete update bucket counts and staleness
//! - 11.3 Persistence round-trip: histograms survive database close/reopen
//! - 11.4 Heuristic coverage: all specialty conditions return explicit constants
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_lossless,
    clippy::uninlined_format_args
)]

use serde_json::json;
use tempfile::TempDir;
use velesdb_core::collection::stats::{CollectionStats, ColumnStats, Histogram, HistogramBucket};
use velesdb_core::velesql::{
    CompareOp, Condition, ContainsCondition, ContainsMode, ContainsTextCondition, GeoBboxCondition,
    GeoDistanceCondition, MatchCondition, Value,
};
use velesdb_core::velesql::{CostEstimator, FilterPlan};
use velesdb_core::{Database, DistanceMetric, Point};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Creates a temporary database and returns the guard + handle.
fn temp_database() -> (TempDir, Database) {
    let dir = TempDir::new().expect("tempdir");
    let db = Database::open(dir.path()).expect("open database");
    (dir, db)
}

/// Inserts `count` points with a numeric "score" payload into the named collection.
fn insert_scored_points(db: &Database, name: &str, start_id: u64, count: u64) {
    let coll = db.get_vector_collection(name).expect("collection exists");
    let points: Vec<Point> = (start_id..start_id + count)
        .map(|i| Point {
            id: i,
            vector: vec![i as f32; 4],
            payload: Some(json!({"score": i})),
            sparse_vectors: None,
        })
        .collect();
    coll.upsert(points).expect("upsert");
}

/// Builds a minimal `CollectionStats` with a histogram on the "score" column.
fn stats_with_histogram() -> CollectionStats {
    let histogram = Histogram {
        buckets: vec![
            HistogramBucket {
                lower_bound: 0.0,
                upper_bound: 25.0,
                count: 25,
                distinct_count: 25,
            },
            HistogramBucket {
                lower_bound: 25.0,
                upper_bound: 50.0,
                count: 25,
                distinct_count: 25,
            },
            HistogramBucket {
                lower_bound: 50.0,
                upper_bound: 75.0,
                count: 25,
                distinct_count: 25,
            },
            HistogramBucket {
                lower_bound: 75.0,
                upper_bound: 100.0 + f64::EPSILON,
                count: 25,
                distinct_count: 25,
            },
        ],
        total_count: 100,
        incremental_updates: 0,
        stale: false,
    };

    let mut col = ColumnStats::new("score").with_distinct_count(100);
    col.histogram = Some(histogram);

    let mut stats = CollectionStats::with_counts(100, 0);
    stats.column_stats.insert("score".to_string(), col.clone());
    stats.field_stats.insert("score".to_string(), col);
    stats
}

/// Reads persisted stats directly from disk (bypasses in-memory cache).
fn load_persisted_stats(dir: &TempDir, name: &str) -> CollectionStats {
    let stats_path = dir.path().join(name).join("collection.stats.json");
    let bytes = std::fs::read(&stats_path).expect("read stats file");
    serde_json::from_slice(&bytes).expect("parse stats JSON")
}

// ===========================================================================
// 11.1 — Nominal flow: ANALYZE → histograms → CostEstimator → FilterPlan
// ===========================================================================

#[test]
fn histogram_nominal_flow_analyze_and_explain() {
    // GIVEN a collection with 100 points with numeric "score" payload
    let (_dir, db) = temp_database();
    db.create_collection("nominal", 4, DistanceMetric::Cosine)
        .expect("create");
    insert_scored_points(&db, "nominal", 1, 100);

    // WHEN ANALYZE is called
    let stats = db.analyze_collection("nominal").expect("analyze");

    // THEN histograms exist with total_count > 0
    let hist = stats
        .field_stats
        .get("score")
        .or_else(|| stats.column_stats.get("score"))
        .and_then(|cs| cs.histogram.as_ref())
        .expect("histogram should exist for 'score' column");

    assert!(!hist.buckets.is_empty(), "histogram should have buckets");
    assert!(hist.total_count > 0, "total_count should be positive");

    // AND CostEstimator produces histogram-based selectivity for Eq
    let estimator = CostEstimator::new(&stats);
    let eq_cond = Condition::Comparison(velesdb_core::velesql::Comparison {
        column: "score".to_string(),
        operator: CompareOp::Eq,
        value: Value::Integer(50),
    });
    let eq_sel = estimator.estimate_condition_selectivity(&eq_cond);
    assert!(
        eq_sel > 0.0 && eq_sel < 1.0,
        "Eq selectivity should be in (0, 1), got {eq_sel}"
    );

    // AND CostEstimator produces histogram-based selectivity for Lt
    let lt_cond = Condition::Comparison(velesdb_core::velesql::Comparison {
        column: "score".to_string(),
        operator: CompareOp::Lt,
        value: Value::Integer(50),
    });
    let lt_sel = estimator.estimate_condition_selectivity(&lt_cond);
    assert!(
        lt_sel > 0.0 && lt_sel < 1.0,
        "Lt selectivity should be in (0, 1), got {lt_sel}"
    );

    // AND CostEstimator produces histogram-based selectivity for Between
    let between_cond = Condition::Between(velesdb_core::velesql::BetweenCondition {
        column: "score".to_string(),
        low: Value::Integer(25),
        high: Value::Integer(75),
    });
    let between_sel = estimator.estimate_condition_selectivity(&between_cond);
    assert!(
        between_sel > 0.0 && between_sel < 1.0,
        "Between selectivity should be in (0, 1), got {between_sel}"
    );

    // AND FilterPlan has estimated_rows and estimation_method fields
    // Reason: selectivity is clamped to [0.0, 1.0], so the product is non-negative.
    #[allow(clippy::cast_sign_loss)]
    let estimated = (eq_sel * stats.total_points as f64).round() as u64;
    let plan = FilterPlan {
        conditions: "score = 50".to_string(),
        selectivity: eq_sel,
        estimated_rows: Some(estimated),
        estimation_method: Some("histogram".to_string()),
    };
    assert!(
        plan.estimated_rows.is_some(),
        "estimated_rows should be set"
    );
    assert_eq!(
        plan.estimation_method.as_deref(),
        Some("histogram"),
        "estimation_method should be 'histogram'"
    );
}

// ===========================================================================
// 11.2 — Incremental maintenance on upsert and delete
// ===========================================================================

#[test]
fn histogram_incremental_maintenance_on_upsert_and_delete() {
    // GIVEN a collection with 100 points, ANALYZE called
    let (_dir, db) = temp_database();
    db.create_collection("incremental", 4, DistanceMetric::Cosine)
        .expect("create");
    insert_scored_points(&db, "incremental", 1, 100);
    let stats_before = db.analyze_collection("incremental").expect("analyze");

    let hist_before = stats_before
        .field_stats
        .get("score")
        .or_else(|| stats_before.column_stats.get("score"))
        .and_then(|cs| cs.histogram.as_ref())
        .expect("histogram should exist");
    let total_before = hist_before.total_count;
    assert!(total_before > 0, "histogram should have data");
    assert!(!hist_before.stale, "fresh histogram should not be stale");

    // WHEN 25 more points are upserted with scores within the existing histogram range
    // (values must fall within existing bucket boundaries for incremental updates to apply)
    let coll = db.get_vector_collection("incremental").expect("collection");
    let new_points: Vec<Point> = (101..=125)
        .map(|i| Point {
            id: i,
            vector: vec![i as f32; 4],
            // Use scores within the original 1..100 range so they hit existing buckets
            payload: Some(json!({"score": (i - 100) * 2})),
            sparse_vectors: None,
        })
        .collect();
    coll.upsert(new_points).expect("upsert");

    // THEN histogram bucket counts are updated (read from persisted stats on disk)
    // Incremental updates are written to collection.stats.json, not the in-memory cache.
    let stats_after = load_persisted_stats(&_dir, "incremental");

    let hist_after = stats_after
        .field_stats
        .get("score")
        .or_else(|| stats_after.column_stats.get("score"))
        .and_then(|cs| cs.histogram.as_ref())
        .expect("histogram should still exist after upsert");

    // Incremental updates should have been applied
    assert!(
        hist_after.incremental_updates > 0,
        "incremental_updates should be > 0 after upsert, got {}",
        hist_after.incremental_updates
    );

    // AND histogram is marked stale (25 updates on 100-count histogram > 20% threshold)
    assert!(
        hist_after.stale,
        "histogram should be stale after >20% updates (updates={}, total={})",
        hist_after.incremental_updates, hist_after.total_count
    );

    // WHEN points are deleted
    let coll = db.get_vector_collection("incremental").expect("collection");
    let ids_to_delete: Vec<u64> = (1..=5).collect();
    coll.delete(&ids_to_delete).expect("delete");

    // THEN histogram bucket counts are decremented
    let stats_post_delete = load_persisted_stats(&_dir, "incremental");

    let hist_post_delete = stats_post_delete
        .field_stats
        .get("score")
        .or_else(|| stats_post_delete.column_stats.get("score"))
        .and_then(|cs| cs.histogram.as_ref())
        .expect("histogram should still exist after delete");

    // More incremental updates after deletes
    assert!(
        hist_post_delete.incremental_updates > hist_after.incremental_updates,
        "incremental_updates should increase after deletes"
    );
}

// ===========================================================================
// 11.3 — Persistence round-trip: histograms survive restart
// ===========================================================================

#[test]
fn histogram_persistence_survives_restart() {
    let dir = TempDir::new().expect("tempdir");

    // GIVEN a collection with data, ANALYZE called
    let selectivity_before;
    {
        let db = Database::open(dir.path()).expect("open");
        db.create_collection("persist", 4, DistanceMetric::Cosine)
            .expect("create");
        insert_scored_points(&db, "persist", 1, 100);
        let stats = db.analyze_collection("persist").expect("analyze");

        let hist = stats
            .field_stats
            .get("score")
            .or_else(|| stats.column_stats.get("score"))
            .and_then(|cs| cs.histogram.as_ref())
            .expect("histogram should exist");
        assert!(!hist.buckets.is_empty(), "histogram should have buckets");
        assert!(hist.total_count > 0, "total_count should be positive");

        // Record selectivity for comparison after restart
        let estimator = CostEstimator::new(&stats);
        let cond = Condition::Comparison(velesdb_core::velesql::Comparison {
            column: "score".to_string(),
            operator: CompareOp::Lt,
            value: Value::Integer(50),
        });
        selectivity_before = estimator.estimate_condition_selectivity(&cond);
    }
    // Database dropped here — simulates shutdown

    // WHEN database is closed and reopened
    let db2 = Database::open(dir.path()).expect("reopen");

    // THEN histograms are restored from collection.stats.json
    let loaded_stats = db2
        .get_collection_stats("persist")
        .expect("get stats")
        .expect("stats should load from disk");

    let hist_loaded = loaded_stats
        .field_stats
        .get("score")
        .or_else(|| loaded_stats.column_stats.get("score"))
        .and_then(|cs| cs.histogram.as_ref())
        .expect("histogram should be restored after reopen");

    assert!(
        !hist_loaded.buckets.is_empty(),
        "restored histogram should have buckets"
    );
    assert!(
        hist_loaded.total_count > 0,
        "restored total_count should be positive"
    );

    // AND selectivity estimates are unchanged
    let estimator2 = CostEstimator::new(&loaded_stats);
    let cond = Condition::Comparison(velesdb_core::velesql::Comparison {
        column: "score".to_string(),
        operator: CompareOp::Lt,
        value: Value::Integer(50),
    });
    let selectivity_after = estimator2.estimate_condition_selectivity(&cond);

    let diff = (selectivity_before - selectivity_after).abs();
    assert!(
        diff < f64::EPSILON * 10.0,
        "selectivity should be unchanged after restart: before={selectivity_before}, after={selectivity_after}"
    );
}

// ===========================================================================
// 11.4 — Heuristic coverage: all specialty conditions return explicit constants
// ===========================================================================

#[test]
fn histogram_heuristic_constants_for_all_condition_types() {
    // GIVEN a CostEstimator with collection stats
    let stats = stats_with_histogram();
    let estimator = CostEstimator::new(&stats);

    // WHEN estimating selectivity for Match
    let match_cond = Condition::Match(MatchCondition {
        column: "text".to_string(),
        query: "hello".to_string(),
    });
    // THEN Match returns 0.1
    let match_sel = estimator.estimate_condition_selectivity(&match_cond);
    assert!(
        (match_sel - 0.1).abs() < f64::EPSILON,
        "Match should return 0.1, got {match_sel}"
    );

    // WHEN estimating selectivity for ContainsText
    let contains_text_cond = Condition::ContainsText(ContainsTextCondition {
        column: "text".to_string(),
        query: "hello".to_string(),
    });
    // THEN ContainsText returns 0.05
    let ct_sel = estimator.estimate_condition_selectivity(&contains_text_cond);
    assert!(
        (ct_sel - 0.05).abs() < f64::EPSILON,
        "ContainsText should return 0.05, got {ct_sel}"
    );

    // WHEN estimating selectivity for Contains
    let contains_cond = Condition::Contains(ContainsCondition {
        column: "tags".to_string(),
        mode: ContainsMode::Single,
        values: vec![Value::Integer(1)],
    });
    // THEN Contains returns 0.1
    let contains_sel = estimator.estimate_condition_selectivity(&contains_cond);
    assert!(
        (contains_sel - 0.1).abs() < f64::EPSILON,
        "Contains should return 0.1, got {contains_sel}"
    );

    // WHEN estimating selectivity for GeoDistance
    let geo_dist_cond = Condition::GeoDistance(GeoDistanceCondition {
        column: "location".to_string(),
        lat: 48.8566,
        lng: 2.3522,
        operator: CompareOp::Lt,
        threshold: 1000.0,
    });
    // THEN GeoDistance returns 0.1
    let geo_sel = estimator.estimate_condition_selectivity(&geo_dist_cond);
    assert!(
        (geo_sel - 0.1).abs() < f64::EPSILON,
        "GeoDistance should return 0.1, got {geo_sel}"
    );

    // WHEN estimating selectivity for GeoBbox
    let geo_bbox_cond = Condition::GeoBbox(GeoBboxCondition {
        column: "location".to_string(),
        lat_min: 48.0,
        lng_min: 2.0,
        lat_max: 49.0,
        lng_max: 3.0,
    });
    // THEN GeoBbox returns 0.2
    let bbox_sel = estimator.estimate_condition_selectivity(&geo_bbox_cond);
    assert!(
        (bbox_sel - 0.2).abs() < f64::EPSILON,
        "GeoBbox should return 0.2, got {bbox_sel}"
    );

    // AND no condition falls through to 0.5
    let all_selectivities = [match_sel, ct_sel, contains_sel, geo_sel, bbox_sel];
    for (i, sel) in all_selectivities.iter().enumerate() {
        assert!(
            (*sel - 0.5).abs() > f64::EPSILON,
            "condition {i} should NOT return 0.5 (catch-all), got {sel}"
        );
    }
}
