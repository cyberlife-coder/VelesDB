//! A compound (`UNION`, `INTERSECT`, `EXCEPT`) one of whose operands is an
//! aggregation is refused (`VELES-010`), on every entry point: before, the
//! aggregate path ran the first operand alone and dropped the others.

use std::collections::HashMap;

use super::Database;
use crate::point::Point;
use crate::DistanceMetric;

fn seeded(dir: &std::path::Path) -> Database {
    let db = Database::open(dir).expect("test: open");
    for name in ["a", "b"] {
        db.create_collection(name, 2, DistanceMetric::Cosine)
            .expect("test: collection");
        let points =
            (1..=2).map(|id| Point::new(id, vec![1.0, 0.0], Some(serde_json::json!({"cat": "c"}))));
        db.resolve_collection(name)
            .expect("test: resolve")
            .upsert(points)
            .expect("test: upsert");
    }
    db
}

fn parse(sql: &str) -> crate::velesql::Query {
    crate::velesql::Parser::parse(sql).unwrap_or_else(|e| panic!("{sql}: {e}"))
}

const COMPOUND_AGGREGATES: [&str; 4] = [
    "SELECT COUNT(*) FROM a UNION ALL SELECT COUNT(*) FROM b",
    "SELECT cat, COUNT(*) FROM a GROUP BY cat UNION SELECT cat, COUNT(*) FROM b GROUP BY cat",
    "SELECT COUNT(*) FROM a EXCEPT SELECT COUNT(*) FROM b",
    "SELECT * FROM a UNION SELECT COUNT(*) FROM b",
];

fn assert_refused(sql: &str, refused: crate::Result<impl std::fmt::Debug>) {
    let error = refused.expect_err(sql);
    assert_eq!(error.code(), "VELES-010", "{sql}: {error}");
    assert!(error.to_string().contains("compound"), "{sql}: {error}");
}

#[test]
fn a_compound_of_aggregations_is_refused_by_the_database() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let db = seeded(dir.path());

    for sql in COMPOUND_AGGREGATES {
        assert_refused(sql, db.execute_aggregate(&parse(sql), &HashMap::new()));
        assert_refused(sql, db.execute_query(&parse(sql), &HashMap::new()));
    }
}

#[test]
fn a_compound_of_aggregations_is_refused_by_a_collection() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let db = seeded(dir.path());
    let collection = db.resolve_collection("a").expect("test: resolve");

    for sql in COMPOUND_AGGREGATES {
        assert_refused(
            sql,
            collection.execute_aggregate(&parse(sql), &HashMap::new()),
        );
        assert_refused(sql, collection.execute_query(&parse(sql), &HashMap::new()));
    }
}

/// A compound of plain selects still runs.
#[test]
fn a_compound_of_selects_still_runs() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let db = seeded(dir.path());

    let rows = db
        .execute_query(
            &parse("SELECT * FROM a UNION ALL SELECT * FROM b"),
            &HashMap::new(),
        )
        .expect("test: union");
    assert_eq!(rows.len(), 4);
}
