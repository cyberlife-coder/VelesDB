use std::collections::HashMap;

use serde_json::json;
use tempfile::TempDir;
use velesdb_core::{velesql::Parser, Collection, Database, DistanceMetric, Point};

fn setup_collection() -> (TempDir, Collection) {
    let dir = TempDir::new().expect("tempdir");
    let db = Database::open(dir.path()).expect("open db");
    db.create_collection("people", 4, DistanceMetric::Cosine)
        .expect("create collection");
    let collection = db.get_collection("people").expect("get collection");

    collection
        .upsert(vec![
            Point::new(
                1,
                vec![1.0, 0.0, 0.0, 0.0],
                Some(json!({"_labels": ["Person"], "name": "Alice", "age": 30})),
            ),
            Point::new(
                2,
                vec![0.9, 0.1, 0.0, 0.0],
                Some(json!({"_labels": ["Person"], "name": "Bob", "age": 15})),
            ),
        ])
        .expect("upsert");

    (dir, collection)
}

#[test]
fn test_match_where_alias_property_filters_results() {
    let (_dir, collection) = setup_collection();
    let query =
        Parser::parse("MATCH (n:Person) WHERE n.age > 18 RETURN n LIMIT 10").expect("parse match");
    let match_clause = query.match_clause.as_ref().expect("match clause");

    let results = collection
        .execute_match(match_clause, &HashMap::new())
        .expect("execute match");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].bindings.get("n"), Some(&1));
}
