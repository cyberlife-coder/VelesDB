//! Tests for retrieving named vectors and ordering by a row's named vector.

#![cfg(all(test, feature = "persistence"))]

use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::point::Point;
use std::path::PathBuf;

/// Helper: create a collection with points that have embedded vectors in payload.
fn setup_multi_vector_collection() -> (tempfile::TempDir, Collection) {
    let dir = tempfile::tempdir().expect("temp dir");
    let col = Collection::create(PathBuf::from(dir.path()), 4, DistanceMetric::Cosine)
        .expect("create collection");

    let points = vec![
        Point {
            id: 1,
            vector: vec![1.0, 0.0, 0.0, 0.0],
            payload: Some(serde_json::json!({
                "title_embedding": [0.5, 0.5, 0.0, 0.0],
                "description": "not a vector"
            })),
            sparse_vectors: None,
        },
        Point {
            id: 2,
            vector: vec![0.0, 1.0, 0.0, 0.0],
            payload: Some(serde_json::json!({
                "title_embedding": [0.0, 0.0, 1.0, 0.0]
            })),
            sparse_vectors: None,
        },
        Point {
            id: 3,
            vector: vec![0.0, 0.0, 1.0, 0.0],
            payload: None, // no payload at all
            sparse_vectors: None,
        },
    ];

    col.upsert(points).expect("upsert");
    (dir, col)
}

// -----------------------------------------------------------------------
// Primary "vector" field
// -----------------------------------------------------------------------

#[test]
fn test_get_vector_for_primary_field() {
    let (_dir, col) = setup_multi_vector_collection();

    let vec = col
        .get_vector_for_field(1, "vector")
        .expect("should succeed");
    assert!(vec.is_some(), "primary vector should exist for point 1");
    assert_eq!(vec.unwrap(), vec![1.0, 0.0, 0.0, 0.0]);
}

// -----------------------------------------------------------------------
// Named vector from payload
// -----------------------------------------------------------------------

#[test]
fn test_get_vector_for_named_payload_field() {
    let (_dir, col) = setup_multi_vector_collection();

    let vec = col
        .get_vector_for_field(1, "title_embedding")
        .expect("should succeed");
    assert!(vec.is_some(), "title_embedding should exist for point 1");
    assert_eq!(vec.unwrap(), vec![0.5, 0.5, 0.0, 0.0]);
}

// -----------------------------------------------------------------------
// Missing field -> Ok(None)
// -----------------------------------------------------------------------

#[test]
fn test_get_vector_for_missing_field_returns_none() {
    let (_dir, col) = setup_multi_vector_collection();

    let vec = col
        .get_vector_for_field(1, "nonexistent_field")
        .expect("should succeed, not error");
    assert!(vec.is_none(), "missing field should return None");
}

// -----------------------------------------------------------------------
// Non-numeric array -> Error
// -----------------------------------------------------------------------

#[test]
fn test_get_vector_for_non_array_field_errors() {
    let (_dir, col) = setup_multi_vector_collection();

    // "description" is a string, not an array.
    let result = col.get_vector_for_field(1, "description");
    assert!(result.is_err(), "non-array field should error");
}

// -----------------------------------------------------------------------
// No payload -> Ok(None)
// -----------------------------------------------------------------------

#[test]
fn test_get_vector_for_field_no_payload_returns_none() {
    let (_dir, col) = setup_multi_vector_collection();

    let vec = col
        .get_vector_for_field(3, "title_embedding")
        .expect("should succeed");
    assert!(vec.is_none(), "point with no payload should return None");
}

/// Two collections share ids 1 and 2. Only the second one's id 2 has a
/// named vector parallel to the query. The first one's id 3 has no match.
fn named_vector_sides() -> (tempfile::TempDir, crate::Database) {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let db = crate::Database::open(dir.path()).expect("test: database");
    for (name, field) in [("docs", "emb"), ("tags", "tag_emb")] {
        db.create_collection(name, 2, DistanceMetric::Cosine)
            .expect("test: collection");
        let rows = if name == "docs" { 3 } else { 2 };
        let points = (1..=rows).map(|id| {
            let embedding = if name == "tags" && id == 2 {
                [1.0, 0.0]
            } else {
                [0.0, 1.0]
            };
            Point::new(
                id,
                vec![1.0, 0.0],
                Some(serde_json::json!({field: embedding, "tag_id": id})),
            )
        });
        db.get_vector_collection(name)
            .expect("test: collection")
            .upsert(points)
            .expect("test: rows");
    }
    (dir, db)
}

fn winning_named_vector(sql: &str) -> u64 {
    let (_dir, db) = named_vector_sides();
    let query = crate::velesql::Parser::parse(sql).expect("test: parse");
    let params =
        std::collections::HashMap::from([("q".to_string(), serde_json::json!([1.0, 0.0]))]);
    let rows = db.execute_query(&query, &params).expect("test: SELECT");
    assert_eq!(rows.len(), 1);
    rows[0].point.id
}

#[test]
fn named_vector_order_reads_the_compound_rows_aligned_field() {
    let sql = "SELECT emb FROM docs UNION ALL SELECT tag_emb FROM tags \
               ORDER BY similarity(emb, $q) DESC LIMIT 1";
    assert_eq!(winning_named_vector(sql), 2);
}

#[test]
fn named_vector_order_reads_the_joined_sides_alias() {
    let sql = "SELECT * FROM docs AS d JOIN tags AS t ON d.id = t.id \
               ORDER BY similarity(t.tag_emb, $q) DESC LIMIT 1";
    assert_eq!(winning_named_vector(sql), 2);
}

#[test]
fn named_vector_order_sorts_a_null_extended_join_side() {
    let (_dir, db) = named_vector_sides();
    let params =
        std::collections::HashMap::from([("q".to_string(), serde_json::json!([1.0, 0.0]))]);
    for (direction, expected) in [("ASC", [1, 2, 3]), ("DESC", [3, 2, 1])] {
        let sql = format!(
            "SELECT * FROM docs AS d LEFT JOIN tags AS t ON d.tag_id = t.id \
                          ORDER BY similarity(t.tag_emb, $q) {direction}"
        );
        let query = crate::velesql::Parser::parse(&sql).expect("test: parse");
        let rows = db.execute_query(&query, &params).expect("test: LEFT JOIN");
        let unmatched = rows
            .iter()
            .find(|row| row.point.id == 3)
            .expect("test: unmatched");
        let payload = unmatched.point.payload.as_ref().expect("test: payload");
        assert_eq!(
            crate::velesql::row_field(payload, "t.tag_emb"),
            Some(&serde_json::Value::Null)
        );
        assert_eq!(
            rows.iter().map(|row| row.point.id).collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn named_vector_order_keeps_the_rows_read_before_an_update() {
    let (_dir, col) = setup_multi_vector_collection();
    let sql = "SELECT * FROM docs WHERE id <= 2 ORDER BY similarity(title_embedding, $q) DESC";
    let query = crate::velesql::Parser::parse(sql).expect("test: parse");
    let params =
        std::collections::HashMap::from([("q".to_string(), serde_json::json!([1, 0, 0, 0]))]);
    let mut earlier = col
        .execute_query(&query, &params)
        .expect("test: initial read");
    assert_eq!(earlier[0].point.id, 1);

    col.store_node_payload(2, &serde_json::json!({"title_embedding": [1, 0, 0, 0]}))
        .expect("test: update");
    let current = col
        .execute_query(&query, &params)
        .expect("test: updated read");
    assert_eq!(current[0].point.id, 2);
    let order = query.select.order_by.as_deref().expect("test: ORDER BY");
    col.apply_order_by(&mut earlier, order, &params)
        .expect("test: order earlier rows");
    assert_eq!(earlier[0].point.id, 1);
}
