//! A column qualified by the FROM side's name — its alias, or its table's
//! when it has none — is the base row's own column, with or without a JOIN,
//! for a SELECT and an aggregate alike; a qualifier naming no side stays a
//! payload path.

use std::collections::HashMap;

use super::Database;
use crate::point::Point;
use crate::DistanceMetric;

/// `docs` 1..=6, the even ones in category `y`, "apple" in 2 and 6 only
/// ("pineapple" is another word); 7 holds a nested `meta.lang`. `tags`
/// holds one row per doc, its `doc_id`.
fn seeded(dir: &std::path::Path) -> Database {
    let db = Database::open(dir).expect("test: open");
    db.create_collection("docs", 2, DistanceMetric::Cosine)
        .expect("test: docs");
    db.create_collection("tags", 2, DistanceMetric::Cosine)
        .expect("test: tags");
    let texts = [
        (1, "apple pie", "x"),
        (2, "apple bread", "y"),
        (3, "cherry tart", "x"),
        (4, "pineapple juice", "y"),
        (5, "apple crumble", "x"),
        (6, "apple apple apple", "y"),
    ];
    #[allow(clippy::cast_precision_loss)]
    let docs = texts.iter().map(|&(id, content, cat)| {
        Point::new(
            id,
            vec![1.0, id as f32],
            Some(serde_json::json!({"content": content, "cat": cat})),
        )
    });
    let docs = docs.chain([Point::new(
        7,
        vec![1.0, 7.0],
        Some(serde_json::json!({"content": "plum", "meta": {"lang": "en"}})),
    )]);
    let collection = db.resolve_collection("docs").expect("test: docs");
    collection.upsert(docs).expect("test: upsert docs");
    let tags = (1..=6).map(|id| {
        Point::new(
            100 + id,
            vec![1.0, 0.0],
            Some(serde_json::json!({"doc_id": id})),
        )
    });
    db.resolve_collection("tags")
        .expect("test: tags")
        .upsert(tags)
        .expect("test: upsert tags");
    db
}

fn parse(sql: &str) -> crate::velesql::Query {
    crate::velesql::Parser::parse(sql).unwrap_or_else(|e| panic!("{sql}: {e}"))
}

fn sorted_ids(db: &Database, sql: &str) -> Vec<u64> {
    let rows = db
        .execute_query(&parse(sql), &HashMap::new())
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
    let mut ids: Vec<u64> = rows.iter().map(|r| r.point.id).collect();
    ids.sort_unstable();
    ids
}

fn count(db: &Database, sql: &str) -> serde_json::Value {
    let aggregate = db
        .execute_aggregate(&parse(sql), &HashMap::new())
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
    aggregate["count"].clone()
}

#[test]
fn a_base_qualified_condition_reads_the_base_rows_columns_in_a_join() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let db = seeded(dir.path());

    for sql in [
        "SELECT * FROM docs d JOIN tags t ON d.id = t.doc_id WHERE d.cat = 'y'",
        "SELECT * FROM docs AS d JOIN tags AS t ON d.id = t.doc_id WHERE d.cat IN ('y')",
        "SELECT * FROM docs JOIN tags t ON docs.id = t.doc_id WHERE docs.cat = 'y'",
    ] {
        assert_eq!(sorted_ids(&db, sql), [2, 4, 6], "{sql}");
    }
    let matched = "SELECT * FROM docs d JOIN tags t ON d.id = t.doc_id \
                   WHERE d.content MATCH 'apple' AND d.cat = 'y'";
    assert_eq!(sorted_ids(&db, matched), [2, 6]);
    let counted = "SELECT COUNT(*) FROM docs d JOIN tags t ON d.id = t.doc_id \
                   WHERE d.content MATCH 'apple' AND d.cat = 'y'";
    assert_eq!(count(&db, counted), 2);
}

#[test]
fn a_base_qualified_condition_reads_the_rows_columns_without_a_join() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let db = seeded(dir.path());

    for sql in [
        "SELECT * FROM docs d WHERE d.cat = 'y'",
        "SELECT * FROM docs AS d WHERE d.cat = 'y' AND d.content LIKE '%a%'",
        "SELECT * FROM docs WHERE docs.cat = 'y'",
    ] {
        assert_eq!(sorted_ids(&db, sql), [2, 4, 6], "{sql}");
    }
    assert_eq!(
        sorted_ids(
            &db,
            "SELECT * FROM docs d WHERE d.content MATCH 'apple' AND d.cat = 'y'"
        ),
        [2, 6]
    );
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM docs d WHERE d.cat = 'y'"),
        3
    );
    let collection = db.resolve_collection("docs").expect("test: docs");
    let rows = collection
        .execute_query(
            &parse("SELECT * FROM docs d WHERE d.cat = 'x' LIMIT 10"),
            &HashMap::new(),
        )
        .expect("test: collection query");
    let mut ids: Vec<u64> = rows.iter().map(|r| r.point.id).collect();
    ids.sort_unstable();
    assert_eq!(ids, [1, 3, 5]);
}

/// A qualifier that names no side reads the payload path it spells: `meta`
/// is no side of `FROM docs d`, so `meta.lang` is 7's nested field — and
/// `docs` is no side of it either, the alias naming the table there.
#[test]
fn a_qualifier_naming_no_side_stays_a_payload_path() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let db = seeded(dir.path());

    assert_eq!(
        sorted_ids(&db, "SELECT * FROM docs d WHERE meta.lang = 'en'"),
        [7]
    );
    assert!(sorted_ids(&db, "SELECT * FROM docs d WHERE docs.cat = 'y'").is_empty());
}
