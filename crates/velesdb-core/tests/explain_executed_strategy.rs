#![cfg(all(test, feature = "persistence"))]
//! EXPLAIN ANALYZE reports the filter strategy the executor *actually ran*.
//!
//! The value is recorded at the dispatch site itself (lot 3.2a), so these
//! tests pin the ground truth against corpora whose selectivities are exact
//! by construction:
//!
//! - `<= 1%` with quality overrides → exact brute-force scan (`PreFilterExact`);
//! - mid selectivity → bitmap-constrained HNSW (`PreFilter`);
//! - `> 80%` with quality overrides → `PostFilter`;
//! - `> 80%` WITHOUT overrides → still `PreFilter`: the no-override path
//!   always runs the bitmap when one exists — precisely the plan/execution
//!   divergence this field exists to surface;
//! - MATCH and compound queries report nothing.

#![allow(clippy::cast_precision_loss)]

use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;
use velesdb_core::velesql::{FilterStrategy, Parser};
use velesdb_core::{Database, DistanceMetric, Point};

const N: u64 = 1_000;
const DIM: usize = 16;

/// Builds a database with one collection: three indexed tag fields whose
/// `"y"` population is an id prefix (exact selectivities 1%, 10%, 90%).
fn setup() -> (Database, TempDir) {
    let dir = TempDir::new().expect("temp dir");
    let db = Database::open(dir.path()).expect("open database");
    db.create_collection("docs", DIM, DistanceMetric::Cosine)
        .expect("create collection");
    let collection = db.get_vector_collection("docs").expect("get collection");

    for field in ["b_rare", "b_mid", "b_wide"] {
        collection.create_index(field).expect("create index");
    }

    let points: Vec<Point> = (0..N)
        .map(|id| {
            let payload = json!({
                "b_rare": if id < N / 100 { "y" } else { "n" },  // 1%
                "b_mid": if id < N / 10 { "y" } else { "n" },    // 10%
                "b_wide": if id < N * 9 / 10 { "y" } else { "n" }, // 90%
            });
            let mut vector: Vec<f32> = (0..DIM)
                .map(|d| ((id as f32) * 0.13 + (d as f32) * 0.07).cos())
                .collect();
            let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
            for x in &mut vector {
                *x /= norm;
            }
            Point::new(id, vector, Some(payload))
        })
        .collect();
    collection.upsert(points).expect("upsert");
    (db, dir)
}

fn query_params() -> HashMap<String, serde_json::Value> {
    let v: Vec<f32> = (0..DIM).map(|d| (d as f32 * 0.1).sin()).collect();
    let mut params = HashMap::new();
    params.insert("v".to_string(), json!(v));
    params
}

/// Runs EXPLAIN ANALYZE through the Database facade and returns the
/// executed strategy the response carries.
fn analyzed_strategy(db: &Database, sql: &str) -> Option<FilterStrategy> {
    let query = Parser::parse(sql).expect("parse");
    let output = db
        .explain_analyze_query(&query, &query_params())
        .expect("explain analyze");
    let stats = output.actual_stats.expect("ANALYZE carries actual stats");
    stats.executed_filter_strategy
}

#[test]
fn reports_exact_scan_for_rare_band_with_overrides() {
    let (db, _dir) = setup();
    let strategy = analyzed_strategy(
        &db,
        "SELECT * FROM docs WHERE vector NEAR $v AND b_rare = 'y' LIMIT 5 WITH (mode='balanced')",
    );
    assert_eq!(strategy, Some(FilterStrategy::PreFilterExact));
}

#[test]
fn reports_bitmap_prefilter_for_mid_band_with_overrides() {
    let (db, _dir) = setup();
    let strategy = analyzed_strategy(
        &db,
        "SELECT * FROM docs WHERE vector NEAR $v AND b_mid = 'y' LIMIT 5 WITH (mode='balanced')",
    );
    assert_eq!(strategy, Some(FilterStrategy::PreFilter));
}

#[test]
fn reports_post_filter_for_wide_band_with_overrides() {
    let (db, _dir) = setup();
    let strategy = analyzed_strategy(
        &db,
        "SELECT * FROM docs WHERE vector NEAR $v AND b_wide = 'y' LIMIT 5 WITH (mode='balanced')",
    );
    assert_eq!(strategy, Some(FilterStrategy::PostFilter));
}

/// The divergence this field exists to surface: without quality overrides
/// the executor always runs the bitmap when one exists — even at 90%
/// selectivity, where the override path post-filters.
#[test]
fn no_override_path_runs_bitmap_even_on_wide_band() {
    let (db, _dir) = setup();
    let strategy = analyzed_strategy(
        &db,
        "SELECT * FROM docs WHERE vector NEAR $v AND b_wide = 'y' LIMIT 5",
    );
    assert_eq!(strategy, Some(FilterStrategy::PreFilter));
}

#[test]
fn pure_near_reports_nothing() {
    let (db, _dir) = setup();
    let strategy = analyzed_strategy(&db, "SELECT * FROM docs WHERE vector NEAR $v LIMIT 5");
    assert_eq!(strategy, None, "no filter → no pre/post-filter notion");
}

#[test]
fn compound_query_reports_nothing() {
    let (db, _dir) = setup();
    let strategy = analyzed_strategy(
        &db,
        "SELECT * FROM docs WHERE vector NEAR $v AND b_mid = 'y' LIMIT 5 \
         UNION SELECT * FROM docs WHERE vector NEAR $v AND b_rare = 'y' LIMIT 5",
    );
    assert_eq!(
        strategy, None,
        "several selects → a single strategy would be ambiguous"
    );
}

/// Stats persisted by older versions (no such field) must load unchanged,
/// and a `None` strategy must not appear in the serialized JSON.
#[test]
fn actual_stats_serde_is_backward_compatible() {
    let legacy =
        r#"{"actual_rows":3,"actual_time_ms":1.5,"loops":1,"nodes_visited":0,"edges_traversed":0}"#;
    let stats: velesdb_core::velesql::ActualStats =
        serde_json::from_str(legacy).expect("legacy JSON must deserialize");
    assert_eq!(stats.executed_filter_strategy, None);

    let round = serde_json::to_string(&stats).expect("serialize");
    assert!(
        !round.contains("executed_filter_strategy"),
        "None must stay absent from the wire"
    );
}
