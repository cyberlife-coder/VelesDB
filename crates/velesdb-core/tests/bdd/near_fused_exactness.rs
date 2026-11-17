//! BDD scenarios: `NEAR_FUSED` fuses the best passing points of each query
//! vector.
//!
//! Each branch is the `top_k` overfetch of points that pass the WHERE
//! metadata filter, ranked in the metric's direction. Before, a branch was
//! cut to its overfetch window first and filtered after, so a selective
//! filter left fewer rows than the LIMIT though more points passed.

use std::collections::HashMap;

use serde_json::json;
use velesdb_core::{Database, DistanceMetric, Point};

use super::helpers::{create_test_db, execute_sql_with_params};

/// `c` (dim 2, `metric`): 100 points `kind = 'near'` at `[1, 0]` and 5 points
/// `kind = 'far'` at `[0, 1]`.
fn setup(db: &Database, metric: DistanceMetric) {
    db.create_vector_collection("c", 2, metric)
        .expect("test: create c");
    let c = db.get_vector_collection("c").expect("test: c");
    let points: Vec<Point> = (1..=100_u64)
        .map(|id| {
            Point::new(
                id,
                vec![1.0, 0.01 * id as f32],
                Some(json!({"kind": "near"})),
            )
        })
        .chain((101..=105_u64).map(|id| {
            Point::new(
                id,
                vec![0.01 * id as f32, 1.0],
                Some(json!({"kind": "far"})),
            )
        }))
        .collect();
    c.upsert(points).expect("test: upsert");
}

fn params() -> HashMap<String, serde_json::Value> {
    HashMap::from([
        ("a".to_string(), json!([1.0, 0.0])),
        ("b".to_string(), json!([1.0, 0.1])),
    ])
}

/// GIVEN 100 points nearer both query vectors than the 5 that pass the filter
/// WHEN `NEAR_FUSED` asks for 4 of the passing points
/// THEN it returns 4: each branch holds passing points, not the nearest
///      points the filter then discards.
#[test]
fn a_selective_filter_still_fills_the_limit() {
    for metric in [DistanceMetric::Cosine, DistanceMetric::Euclidean] {
        let (_dir, db) = create_test_db();
        setup(&db, metric);
        let rows = execute_sql_with_params(
            &db,
            "SELECT * FROM c WHERE vector NEAR_FUSED [$a, $b] AND kind = 'far' LIMIT 4",
            &params(),
        )
        .expect("test: NEAR_FUSED");
        assert_eq!(rows.len(), 4, "{metric:?}");
        assert!(rows.iter().all(|r| r.point.id > 100), "{metric:?}");
    }
}

/// GIVEN the 100 `near` and 5 `far` points
/// WHEN a vector `GROUP BY kind` sits beside `NEAR_FUSED`
/// THEN the fused hits are grouped, one row per kind with its count, as
///      beside a `NEAR`: they came back ungrouped, cut at the LIMIT.
#[test]
fn a_vector_group_by_groups_the_fused_hits() {
    let (_dir, db) = create_test_db();
    setup(&db, DistanceMetric::Cosine);
    let rows = execute_sql_with_params(
        &db,
        "SELECT kind, COUNT(*) FROM c WHERE vector NEAR_FUSED [$a, $b] GROUP BY kind LIMIT 2",
        &params(),
    )
    .expect("test: NEAR_FUSED GROUP BY");
    let mut groups: Vec<(String, Option<i64>)> = rows
        .iter()
        .map(|r| {
            let payload = r.point.payload.as_ref().expect("test: payload");
            (
                payload["kind"].as_str().expect("test: kind").to_string(),
                payload.get("count").and_then(serde_json::Value::as_i64),
            )
        })
        .collect();
    groups.sort();
    assert_eq!(groups.len(), 2, "{groups:?}");
    assert_eq!(groups[0].0, "far");
    assert_eq!(groups[1].0, "near");
    assert!(
        groups.iter().all(|(_, count)| count.is_some_and(|n| n > 0)),
        "{groups:?}"
    );
}
