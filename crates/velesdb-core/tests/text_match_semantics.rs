#![cfg(feature = "persistence")]
//! A text `MATCH` leaf means one thing wherever a WHERE is evaluated: the
//! row is a BM25 hit for its query — the terms `text_search` scores, read
//! by the text index's tokenizer. The SELECT text path ranked by BM25 while
//! the graph post-filter, aggregates, JOINs, a MATCH statement's WHERE and
//! the row-by-row scans read the same leaf as a case-sensitive substring of
//! the column, so one WHERE selected different rows depending on the path
//! that evaluated it ("pineapple" matched `MATCH 'apple'`, "Apple" did not).

use std::collections::HashMap;

use serde_json::{json, Value};
use tempfile::TempDir;
use velesdb_core::velesql::Parser;
use velesdb_core::{Database, DistanceMetric, GraphEdge, Point, SearchResult};

fn ids(results: &[SearchResult]) -> Vec<u64> {
    results.iter().map(|r| r.point.id).collect()
}

fn sorted(mut ids: Vec<u64>) -> Vec<u64> {
    ids.sort_unstable();
    ids
}

fn select(db: &Database, sql: &str, params: &HashMap<String, Value>) -> Vec<u64> {
    let query = Parser::parse(sql).expect("test: parse");
    ids(&db
        .execute_query(&query, params)
        .unwrap_or_else(|e| panic!("{sql}: {e}")))
}

fn aggregate(db: &Database, sql: &str) -> Value {
    let query = Parser::parse(sql).expect("test: parse");
    db.execute_aggregate(&query, &HashMap::new())
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

fn near() -> HashMap<String, Value> {
    HashMap::from([("v".to_string(), json!([1.0, 0.0]))])
}

/// `docs`: 1 "Apple pie recipe", 2 "apple banana smoothie", 3 "pineapple
/// juice", 4 "banana bread", 5 "green apple tart"; 1, 2, 3 and 5 each have
/// a `REL` edge to 4. `meta`: rows 1..=5 pointing at the doc of their id,
/// the odd ones tagged `odd`, 1 and 2 noting "kiwi", the others "plum".
fn fixture() -> (TempDir, Database) {
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open(dir.path()).expect("test: open");
    db.create_vector_collection("docs", 2, DistanceMetric::Cosine)
        .expect("test: create docs");
    let docs = db.get_vector_collection("docs").expect("test: docs");
    let rows = [
        (1, "Apple pie recipe", "a", [1.0, 0.0]),
        (2, "apple banana smoothie", "a", [0.9, 0.1]),
        (3, "pineapple juice", "a", [0.8, 0.2]),
        (4, "banana bread", "b", [0.7, 0.3]),
        (5, "green apple tart", "b", [0.6, 0.4]),
    ];
    docs.upsert(rows.iter().map(|(id, text, cat, vector)| {
        Point::new(
            *id,
            vector.to_vec(),
            Some(json!({"content": text, "cat": cat})),
        )
    }))
    .expect("test: upsert docs");
    for (edge, source) in [1u64, 2, 3, 5].iter().enumerate() {
        let edge = 100 + u64::try_from(edge).expect("test: edge id");
        docs.add_edge(GraphEdge::new(edge, *source, 4, "REL").expect("test: edge"))
            .expect("test: add edge");
    }
    db.create_metadata_collection("meta")
        .expect("test: create meta");
    let meta = db.get_metadata_collection("meta").expect("test: meta");
    meta.upsert_metadata((1..=5u64).map(|id| {
        let tag = if id % 2 == 0 { "even" } else { "odd" };
        let note = if id <= 2 { "kiwi note" } else { "plum note" };
        Point::new(
            id,
            vec![],
            Some(json!({"doc_id": id, "tag": tag, "note": note})),
        )
    }))
    .expect("test: upsert meta");
    (dir, db)
}

#[test]
fn a_match_beside_a_graph_predicate_selects_the_bm25_hits_in_bm25_order() {
    let (_dir, db) = fixture();
    let none = HashMap::new();
    let docs = db.get_vector_collection("docs").expect("test: docs");

    for text in ["apple", "apple banana"] {
        // Every hit of the text search with an outgoing REL edge (4 has none).
        let expected: Vec<u64> = ids(&docs.text_search(text, 10).expect("test: text search"))
            .into_iter()
            .filter(|&id| id != 4)
            .collect();
        let sql = format!(
            "SELECT * FROM docs WHERE content MATCH '{text}' AND MATCH (d)-[:REL]->(x) LIMIT 10"
        );
        assert_eq!(select(&db, &sql, &none), expected, "{sql}");
    }
    assert_eq!(
        sorted(select(
            &db,
            "SELECT * FROM docs WHERE content MATCH 'apple' AND MATCH (d)-[:REL]->(x) LIMIT 10",
            &none
        )),
        [1, 2, 5]
    );
}

#[test]
fn a_match_fused_with_near_beside_a_graph_predicate_boosts_and_filters_nothing() {
    let (_dir, db) = fixture();

    for text in ["apple", "Apple"] {
        let sql = format!(
            "SELECT * FROM docs WHERE vector NEAR $v AND content MATCH '{text}' \
             AND MATCH (d)-[:REL]->(x) LIMIT 10"
        );
        // The hybrid ranks every anchor; its MATCH filters none of them.
        assert_eq!(sorted(select(&db, &sql, &near())), [1, 2, 3, 5], "{sql}");
    }
}

#[test]
fn an_aggregate_counts_the_rows_the_select_returns() {
    let (_dir, db) = fixture();
    let none = HashMap::new();

    for text in ["apple", "apple banana", "pineapple"] {
        let rows = select(
            &db,
            &format!("SELECT * FROM docs WHERE content MATCH '{text}' LIMIT 100"),
            &none,
        );
        let count = aggregate(
            &db,
            &format!("SELECT COUNT(*) FROM docs WHERE content MATCH '{text}'"),
        );
        assert_eq!(count["count"], json!(rows.len()), "{text}: {rows:?}");
    }
    assert_eq!(
        aggregate(
            &db,
            "SELECT cat, COUNT(*) FROM docs WHERE content MATCH 'apple' GROUP BY cat ORDER BY cat"
        ),
        json!([{"cat": "a", "count": 2}, {"cat": "b", "count": 1}])
    );
    assert_eq!(
        aggregate(
            &db,
            "SELECT COUNT(*) FROM docs WHERE content MATCH 'apple' OR content MATCH 'banana'"
        )["count"],
        json!(4)
    );
}

#[test]
fn a_join_reads_a_match_on_the_base_collection_as_its_bm25_hits() {
    let (_dir, db) = fixture();

    let rows = select(
        &db,
        "SELECT * FROM docs JOIN meta ON meta.doc_id = docs.id \
         WHERE docs.content MATCH 'apple' AND meta.tag = 'odd' LIMIT 10",
        &HashMap::new(),
    );

    assert_eq!(sorted(rows), [1, 5]);
}

#[test]
fn a_join_reads_a_match_on_a_joined_table_from_that_tables_text_index() {
    let (_dir, db) = fixture();

    let rows = select(
        &db,
        "SELECT * FROM docs JOIN meta ON meta.doc_id = docs.id \
         WHERE meta.note MATCH 'kiwi' LIMIT 10",
        &HashMap::new(),
    );

    assert_eq!(sorted(rows), [1, 2]);
}

/// Under an `OR` with a base column, the `MATCH` on a joined table's column
/// is evaluated on the joined rows, against that table's text index.
#[test]
fn a_join_reads_a_match_on_each_side_after_the_join() {
    let (_dir, db) = fixture();

    let rows = select(
        &db,
        "SELECT * FROM docs JOIN meta ON meta.doc_id = docs.id \
         WHERE docs.content MATCH 'tart' OR meta.note MATCH 'kiwi' LIMIT 10",
        &HashMap::new(),
    );

    assert_eq!(sorted(rows), [1, 2, 5]);
}

#[test]
fn a_match_statement_where_reads_a_node_match_as_its_bm25_hits() {
    let (_dir, db) = fixture();
    let params = HashMap::from([("_collection".to_string(), json!("docs"))]);
    let query = Parser::parse(
        "MATCH (d)-[:REL]->(x) WHERE d.content MATCH 'apple' RETURN d.content LIMIT 10",
    )
    .expect("test: parse");

    let rows = db.execute_query(&query, &params).expect("test: match");

    let mut texts: Vec<String> = rows
        .iter()
        .filter_map(|r| r.point.payload.as_ref()?.get("d.content")?.as_str())
        .map(str::to_string)
        .collect();
    texts.sort();
    assert_eq!(
        texts,
        [
            "Apple pie recipe",
            "apple banana smoothie",
            "green apple tart"
        ]
    );
}

#[test]
fn a_match_statement_refuses_a_match_on_a_relationship() {
    let (_dir, db) = fixture();
    let params = HashMap::from([("_collection".to_string(), json!("docs"))]);
    let query = Parser::parse(
        "MATCH (d)-[r:REL]->(x) WHERE r.note MATCH 'apple' RETURN d.content LIMIT 10",
    )
    .expect("test: parse");

    let err = db
        .execute_query(&query, &params)
        .expect_err("test: refused");

    assert_eq!(err.code(), "VELES-010", "{err}");
    assert!(err.to_string().contains("relationship"), "{err}");
}

/// 1..=3 "banana split" at `[1, 0]`, 4..=6 "cherry pie" at `[0, 1]`; the
/// even ids are category `a`.
fn shapes() -> (TempDir, Database) {
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open(dir.path()).expect("test: open");
    db.create_vector_collection("v", 2, DistanceMetric::Cosine)
        .expect("test: create");
    let collection = db.get_vector_collection("v").expect("test: v");
    collection
        .upsert((1..=6u64).map(|id| {
            let (content, vector) = if id <= 3 {
                ("banana split", vec![1.0, 0.0])
            } else {
                ("cherry pie", vec![0.0, 1.0])
            };
            let cat = if id % 2 == 0 { "a" } else { "b" };
            Point::new(id, vector, Some(json!({"content": content, "cat": cat})))
        }))
        .expect("test: upsert");
    (dir, db)
}

#[test]
fn a_match_under_or_or_not_or_beside_a_threshold_is_its_hit_set() {
    let (_dir, db) = shapes();

    for (sql, expected) in [
        (
            "SELECT * FROM v WHERE similarity(vector, $v) < 0.5 AND content MATCH 'cherry' LIMIT 10",
            vec![4, 5, 6],
        ),
        (
            "SELECT * FROM v WHERE similarity(vector, $v) > 0.5 OR content MATCH 'cherry' LIMIT 10",
            vec![1, 2, 3, 4, 5, 6],
        ),
        (
            "SELECT * FROM v WHERE NOT (similarity(vector, $v) > 0.5) AND NOT (content MATCH 'pie') \
             LIMIT 10",
            vec![],
        ),
        (
            "SELECT * FROM v WHERE NOT (similarity(vector, $v) > 0.5) \
             AND NOT (content MATCH 'banana') LIMIT 10",
            vec![4, 5, 6],
        ),
        // A NEAR under OR reads TRUE: the disjunction admits every row it ranks.
        (
            "SELECT * FROM v WHERE vector NEAR $v OR content MATCH 'cherry' LIMIT 10",
            vec![1, 2, 3, 4, 5, 6],
        ),
        (
            "SELECT * FROM v WHERE content MATCH 'banana' OR cat = 'a' LIMIT 10",
            vec![1, 2, 3, 4, 6],
        ),
        ("SELECT * FROM v WHERE NOT (content MATCH 'banana') LIMIT 10", vec![4, 5, 6]),
        (
            "SELECT * FROM v WHERE vector NEAR $v AND (content MATCH 'cherry' OR cat = 'b') LIMIT 10",
            vec![1, 3, 4, 5, 6],
        ),
    ] {
        assert_eq!(sorted(select(&db, sql, &near())), expected, "{sql}");
    }
}

#[test]
fn a_match_the_ranking_cannot_fuse_is_refused() {
    let (_dir, db) = shapes();

    for sql in [
        "SELECT * FROM v WHERE content MATCH 'banana' AND content MATCH 'split' LIMIT 10",
        "SELECT * FROM v WHERE vector NEAR $v AND content MATCH 'banana' AND content MATCH 'pie' \
         LIMIT 10",
    ] {
        let query = Parser::parse(sql).expect("test: parse");
        let err = db.execute_query(&query, &near()).expect_err(sql);
        assert_eq!(err.code(), "VELES-010", "{sql}: {err}");
    }
}

/// A `SPARSE_NEAR` ranks the rows; a `MATCH` under `NOT` or `OR` beside it
/// is a membership test of the sparse hits.
#[test]
fn a_match_beside_a_sparse_ranking_tests_membership() {
    use std::collections::BTreeMap;
    use velesdb_core::sparse_index::SparseVector;

    let (_dir, db) = shapes();
    let collection = db.get_vector_collection("v").expect("test: v");
    collection
        .upsert((1..=6u64).map(|id| {
            let (content, vector) = if id <= 3 {
                ("banana split", vec![1.0, 0.0])
            } else {
                ("cherry pie", vec![0.0, 1.0])
            };
            let cat = if id % 2 == 0 { "a" } else { "b" };
            #[allow(clippy::cast_precision_loss)]
            let weight = id as f32;
            let sparse = BTreeMap::from([(String::new(), SparseVector::new(vec![(7, weight)]))]);
            Point::with_sparse(
                id,
                vector,
                Some(json!({"content": content, "cat": cat})),
                Some(sparse),
            )
        }))
        .expect("test: upsert");
    let params = HashMap::from([("s".to_string(), json!({"7": 1.0}))]);

    for (sql, expected) in [
        (
            "SELECT * FROM v WHERE vector SPARSE_NEAR $s AND NOT (content MATCH 'banana') LIMIT 10",
            vec![6, 5, 4],
        ),
        (
            "SELECT * FROM v WHERE vector SPARSE_NEAR $s AND (content MATCH 'banana' OR cat = 'a') \
             LIMIT 10",
            vec![6, 4, 3, 2, 1],
        ),
    ] {
        assert_eq!(select(&db, sql, &params), expected, "{sql}");
    }
    let query = Parser::parse(
        "SELECT * FROM v WHERE vector SPARSE_NEAR $s AND content MATCH 'banana' LIMIT 10",
    )
    .expect("test: parse");
    let err = db
        .execute_query(&query, &params)
        .expect_err("test: refused");
    assert_eq!(err.code(), "VELES-010", "{err}");
}
