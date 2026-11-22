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
fn joined_window_distinguishes_a_literal_sort_key_from_a_projected_path() {
    let (_dir, db) = named_vector_sides();
    db.get_vector_collection("tags")
        .unwrap()
        .upsert([10, 20, 30].map(|id| {
            Point::new(
                id,
                vec![1.0, 0.0],
                Some(serde_json::json!({"parent": 1, "seq": id, "t.seq": 40-id, "d.tag_id": 999, "emb": [1, 0]})),
            )
        }))
        .unwrap();
    let sql =
        "SELECT t.seq AS child, d.tag_id, ROW_NUMBER() OVER (ORDER BY t.seq) AS t FROM docs d \
               JOIN tags t ON d.id=t.parent ORDER BY similarity(t.emb, [1, 0]) DESC, t.seq ASC";
    let query = crate::velesql::Parser::parse(sql).unwrap();
    let rows = db
        .execute_query(&query, &std::collections::HashMap::new())
        .unwrap();
    let projected =
        crate::collection::search::query::projection::project_results(&rows, &query.select.columns);
    assert_eq!(
        projected
            .iter()
            .map(|p| p["child"].as_u64())
            .collect::<Vec<_>>(),
        [Some(30), Some(20), Some(10)]
    );
    for (row, key) in rows.iter().zip([10, 20, 30]) {
        let payload = row.point.payload.as_ref().unwrap();
        assert_eq!(payload["t.seq"], key);
        assert_eq!(payload["child"], 40 - key);
    }
    assert!(projected.iter().all(|row| row["d.tag_id"] == 1));
}

#[test]
fn joined_window_reads_every_sibling_before_publishing_aliases() {
    let (_dir, db) = named_vector_sides();
    db.get_vector_collection("docs")
        .unwrap()
        .upsert((1..=3).map(|id| {
            Point::new(
                id,
                vec![1., 0.],
                Some(serde_json::json!({
                    "tag_id": 10+id, "meta": {"val": 20+id}, "keep": 30+id
                })),
            )
        }))
        .unwrap();
    for order in [
        "ORDER BY similarity(t.tag_emb,[1,0]) DESC",
        "ORDER BY id ASC, t.id ASC",
        "",
    ] {
        let sql = format!(
            "SELECT id, t.id AS child, t.id AS d, d.tag_id, t.id AS meta, meta.val, \
               t.id AS keep, keep AS saved, ROW_NUMBER() OVER (ORDER BY id) AS t \
               FROM docs d LEFT JOIN tags t ON d.id=t.id {order}"
        );
        let query = crate::velesql::Parser::parse(&sql).unwrap();
        let rows = db
            .execute_query(&query, &std::collections::HashMap::new())
            .unwrap();
        let projected = crate::collection::search::query::projection::project_results(
            &rows,
            &query.select.columns,
        );
        assert_eq!(projected.len(), 3);
        for row in projected {
            let id = row["id"].as_u64().unwrap();
            assert_eq!(row["d.tag_id"], 10 + id, "{row}");
            assert_eq!(row["meta.val"], 20 + id, "{row}");
            assert_eq!(row["saved"], 30 + id, "{row}");
            assert_eq!(row["child"].as_u64(), (id < 3).then_some(id));
            assert_eq!(row["d"], row["child"]);
            assert_eq!(row["meta"], row["child"]);
            assert_eq!(row["keep"], row["child"]);
            assert_eq!(row["t"], id);
        }
    }
}

#[test]
fn joined_window_ranks_the_distinct_rows_before_a_scalar_limit() {
    let (_dir, db) = named_vector_sides();
    db.get_vector_collection("tags")
        .unwrap()
        .upsert(
            [10, 20].map(|id| Point::new(id, vec![1., 0.], Some(serde_json::json!({"parent": 1})))),
        )
        .unwrap();
    let run = |sql: &str| {
        let query = crate::velesql::Parser::parse(sql).unwrap();
        db.execute_query(&query, &std::collections::HashMap::new())
            .unwrap()
            .into_iter()
            .map(|row| {
                (
                    row.point.id,
                    row.point.payload.unwrap()["r"].as_u64().unwrap(),
                )
            })
            .collect::<Vec<_>>()
    };
    for (distinct, expected) in [
        ("", vec![(1, 1), (1, 1), (2, 3), (3, 4)]),
        ("DISTINCT", vec![(1, 1), (2, 2), (3, 3)]),
    ] {
        let sql = format!(
            "SELECT {distinct} id, RANK() OVER (ORDER BY id) AS r \
                           FROM docs d LEFT JOIN tags t ON d.id=t.parent"
        );
        let full = run(&sql);
        let mut sorted = full.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, expected);
        for (offset, row) in full.iter().enumerate() {
            assert_eq!(run(&format!("{sql} LIMIT 1 OFFSET {offset}")), [*row]);
        }
    }
}

#[test]
fn joined_window_projects_the_qualified_side_field() {
    let (_dir, db) = named_vector_sides();
    let sql = "SELECT t.id AS child, ROW_NUMBER() OVER (ORDER BY id) AS t \
               FROM docs d LEFT JOIN tags t ON d.id = t.id ORDER BY similarity(t.tag_emb, [1, 0]) DESC";
    let query = crate::velesql::Parser::parse(sql).expect("test: parse");
    let rows = db
        .execute_query(&query, &std::collections::HashMap::new())
        .expect("test: joined rows");
    let projected =
        crate::collection::search::query::projection::project_results(&rows, &query.select.columns);
    assert_eq!(rows.len(), 3);
    for (row, output) in rows.iter().zip(projected) {
        let expected = (row.point.id < 3).then_some(row.point.id);
        assert_eq!(output["child"].as_u64(), expected);
        assert_eq!(output["t"], row.point.id);
        assert_eq!(output.as_object().expect("test: projection").len(), 2);
        let payload = row.point.payload.as_ref().expect("test: hydrated row");
        assert!(
            payload.get("t.tag_emb").is_none(),
            "ORDER-only field in {payload}"
        );
    }
}

#[test]
fn joined_window_orders_by_the_qualified_side_field() {
    let (_dir, db) = named_vector_sides();
    let tags = db.get_vector_collection("tags").expect("test: tags");
    tags.upsert([20, 10, 30].map(|id| {
        Point::new(
            id,
            vec![0.0, 1.0],
            Some(serde_json::json!({"parent": 1, "seq": id, "emb": [1, 0]})),
        )
    }))
    .expect("test: equal embeddings on three joined rows");
    for (direction, expected) in [("ASC", [10, 20, 30]), ("DESC", [30, 20, 10])] {
        for alias in ["rn", "t"] {
            let sql = format!(
                "LET k = {alias} * 2 SELECT seq, ROW_NUMBER() OVER (ORDER BY t.seq) AS {alias} \
                 FROM docs d JOIN tags t ON d.id = t.parent ORDER BY \
                 similarity(t.emb, [1, 0]) DESC, t.seq {direction}, k DESC"
            );
            let query = crate::velesql::Parser::parse(&sql).expect("test: parse");
            let rows = db
                .execute_query(&query, &std::collections::HashMap::new())
                .expect("test: order");
            let values: Vec<_> = rows
                .into_iter()
                .map(|r| {
                    let p = r.point.payload.expect("test: payload");
                    assert!(p.get("t.seq").is_none(), "sort key leaked into {p}");
                    p["seq"].as_u64().expect("test: seq")
                })
                .collect();
            assert_eq!(values, expected, "{sql}");
        }
    }
}

#[test]
fn joined_window_alias_keeps_the_input_embedding() {
    use crate::point::Point;
    use serde_json::json;

    let (_dir, db) = named_vector_sides();
    let tags = db.get_vector_collection("tags").expect("test: tags");
    let children = [(10, [0.0, 1.0]), (20, [1.0, 0.0]), (30, [0.6, 0.8])];
    tags.upsert(children.map(|(id, emb)| {
        let payload = json!({"parent": 1, "seq": id, "emb": emb});
        Point::new(id, vec![0.0, 1.0], Some(payload))
    }))
    .expect("test: one parent, three children");
    let params = std::collections::HashMap::from([("q".to_owned(), json!([1.0, 0.0]))]);
    for alias in ["rn", "t"] {
        for (offset, child, rank) in [(0, 20, 2), (1, 30, 3), (2, 10, 1)] {
            let sql = format!(
                "LET twice = {alias} * 2 SELECT id, ROW_NUMBER() OVER (ORDER BY t.seq) AS {alias} \
             FROM docs d JOIN tags t ON d.id = t.parent \
             ORDER BY similarity(t.emb, $q) DESC LIMIT 1 OFFSET {offset}"
            );
            let query = crate::velesql::Parser::parse(&sql).expect("test: parse");
            let rows = db
                .execute_query(&query, &params)
                .expect("test: window on joined rows");
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].point.id, 1);
            let payload = rows[0]
                .point
                .payload
                .as_ref()
                .expect("test: joined payload");
            assert_eq!(payload["seq"], child, "{sql}");
            assert_eq!(payload[alias], rank, "{sql}");
            assert_eq!(
                payload["twice"].as_f64(),
                Some(f64::from(2 * rank)),
                "{sql}"
            );
        }
    }
}

#[test]
fn joined_window_pages_preserve_all_children_with_equal_sort_keys() {
    use serde_json::json;
    use std::collections::HashMap;

    let (_dir, db) = named_vector_sides();
    let tags = db.get_vector_collection("tags").expect("test: tags");
    let vectors = [[1.0, 0.0], [0.6, 0.8], [0.0, 1.0]];
    tags.upsert(
        (0_u64..24)
            .zip(vectors.into_iter().cycle())
            .map(|(n, emb)| {
                Point::new(
                    10 + n,
                    vec![0.0, 1.0],
                    Some(json!({"parent": 1, "n": n, "emb": emb})),
                )
            }),
    )
    .expect("test: 24 children for one parent");
    let params = HashMap::from([("q".to_owned(), json!([1.0, 0.0]))]);
    for alias in ["rn", "t"] {
        let page = |offset: usize, limit: usize| {
            let sql = format!(
                "SELECT id, n, ROW_NUMBER() OVER (ORDER BY t.n) AS {alias} FROM docs d \
                 JOIN tags t ON d.id = t.parent ORDER BY similarity(t.emb, $q) DESC \
                 LIMIT {limit} OFFSET {offset}"
            );
            let stmt = crate::velesql::Parser::parse(&sql).expect("test: parse");
            db.execute_query(&stmt, &params)
                .expect("test: joined page")
                .into_iter()
                .map(|row| {
                    assert_eq!(row.point.id, 1);
                    let payload = row.point.payload.expect("test: child");
                    let n = payload["n"].as_u64().expect("test: child number");
                    assert_eq!(payload[alias], n + 1);
                    n
                })
                .collect::<Vec<_>>()
        };
        let full = page(0, 24);
        assert_joined_vector_buckets(&full);
        let pages: Vec<_> = (0..24)
            .step_by(3)
            .flat_map(|offset| page(offset, 3))
            .collect();
        assert_eq!(pages, full, "{alias}");
    }
}

fn assert_joined_vector_buckets(full: &[u64]) {
    let mut children = full.to_vec();
    children.sort_unstable();
    assert_eq!(children, (0..24).collect::<Vec<_>>());
    for (group, rows) in (0_u64..).zip(full.chunks_exact(8)) {
        assert!(rows.iter().all(|n| n % 3 == group));
    }
}

#[test]
fn joined_window_rejects_an_unsupported_let_search_shape() {
    let (_dir, db) = named_vector_sides();
    let sql = "LET k = 1 SELECT id, ROW_NUMBER() OVER (ORDER BY id) AS rn \
               FROM docs d JOIN tags t ON d.id = t.id WHERE vector SPARSE_NEAR {1: 1.0} \
               ORDER BY similarity(t.tag_emb, $q) DESC";
    let query = crate::velesql::Parser::parse(sql).expect("test: parse");
    let params = std::collections::HashMap::from([("q".to_owned(), serde_json::json!([1.0, 0.0]))]);
    let err = db
        .execute_query(&query, &params)
        .expect_err("test: unsupported LET");
    assert!(
        err.to_string()
            .contains("LET bindings are not supported with SPARSE_NEAR"),
        "{err}"
    );
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
