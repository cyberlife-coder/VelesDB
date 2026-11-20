//! Tests for `AnyCollection`: each arm writes through its collection's rule.

use serde_json::json;

use super::AnyCollection;
use crate::collection::graph::GraphSchema;
use crate::collection::GraphCollection;
use crate::distance::DistanceMetric;
use crate::point::Point;

/// A graph's upsert writes each row by the graph rule: a vector is kept and
/// its dimension checked, a row without a payload is still written. The arm
/// used to write only the payloads, dropping every vector and every point
/// without one, and to accept a vector of any length.
#[test]
fn a_graph_upsert_writes_rows_by_the_graph_rule() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let graph = GraphCollection::create(
        dir.path().to_path_buf(),
        "g",
        Some(2),
        DistanceMetric::Cosine,
        GraphSchema::schemaless(),
    )
    .expect("test: graph");
    let any = AnyCollection::Graph(graph);

    let refused = any.upsert(vec![
        Point::new(1, vec![1.0, 0.0], Some(json!({"n": 1}))),
        Point::new(3, vec![9.0, 9.0, 9.0], Some(json!({"n": 3}))),
    ]);
    assert_eq!(
        refused.expect_err("test: wrong dimension").code(),
        "VELES-004"
    );
    assert!(
        any.get(&[1])[0].is_none(),
        "a refused batch commits nothing"
    );

    any.upsert(vec![
        Point::new(1, vec![1.0, 0.0], Some(json!({"n": 1}))),
        Point::new(2, vec![0.0, 1.0], None),
    ])
    .expect("test: upsert");
    let vectors: Vec<Option<Vec<f32>>> = any
        .get(&[1, 2])
        .into_iter()
        .map(|point| point.map(|p| p.vector))
        .collect();
    assert_eq!(vectors, vec![Some(vec![1.0, 0.0]), Some(vec![0.0, 1.0])]);
}
