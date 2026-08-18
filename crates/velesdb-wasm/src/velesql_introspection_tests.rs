use super::*;
use crate::database::DatabaseInner;
use velesdb_core::velesql::Parser;

fn parse_intro(sql: &str) -> IntrospectionStatement {
    let q = Parser::parse(sql).expect("test: parse");
    q.introspection.expect("test: has intro")
}

#[test]
fn test_show_collections_empty() {
    let db = DatabaseInner::new();
    let rows = execute(&db, &parse_intro("SHOW COLLECTIONS")).expect("test: show");
    assert!(rows.is_empty());
}

#[test]
fn test_show_collections_lists_types() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("meta").expect("test: meta");
    db.create_collection("vecs", 4, "cosine")
        .expect("test: vecs");
    let rows = execute(&db, &parse_intro("SHOW COLLECTIONS")).expect("test: show");
    assert_eq!(rows.len(), 2);
    let json: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| serde_json::from_str(r.data_json_ref()).expect("test: parse"))
        .collect();
    let kinds: Vec<String> = json
        .iter()
        .map(|j| j["type"].as_str().expect("test: type").to_string())
        .collect();
    assert!(kinds.contains(&"metadata".to_string()));
    assert!(kinds.contains(&"vector".to_string()));
}

#[test]
fn test_describe_collection_vector() {
    let mut db = DatabaseInner::new();
    db.create_collection("vecs", 8, "euclidean")
        .expect("test: create");
    let rows = execute(&db, &parse_intro("DESCRIBE COLLECTION vecs")).expect("test: describe");
    assert_eq!(rows.len(), 1);
    let json: serde_json::Value =
        serde_json::from_str(rows[0].data_json_ref()).expect("test: parse");
    assert_eq!(json["name"], "vecs");
    assert_eq!(json["type"], "vector");
    assert_eq!(json["dimension"], 8);
    assert_eq!(json["metric"], "euclidean");
}

#[test]
fn test_describe_missing_collection_errors() {
    let db = DatabaseInner::new();
    let err = execute(&db, &parse_intro("DESCRIBE COLLECTION ghost"));
    assert!(err.is_err());
}

// --- Finding L: metric_to_string coverage (introspection side) ------
//
// Exhaustive coverage of currently-supported DistanceMetric variants.
// A future variant added in core without updating the match arms
// surfaces honestly as "unknown" in DESCRIBE COLLECTION (never as a
// silent "cosine" masquerade).

#[test]
fn test_introspection_metric_to_string_all_supported_variants() {
    use velesdb_core::DistanceMetric;
    assert_eq!(metric_to_string(DistanceMetric::Cosine), "cosine");
    assert_eq!(metric_to_string(DistanceMetric::Euclidean), "euclidean");
    assert_eq!(metric_to_string(DistanceMetric::DotProduct), "dot");
    assert_eq!(metric_to_string(DistanceMetric::Hamming), "hamming");
    assert_eq!(metric_to_string(DistanceMetric::Jaccard), "jaccard");
}

#[test]
fn test_explain_returns_plan_rows() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("docs").expect("test: meta");
    let rows =
        execute(&db, &parse_intro("EXPLAIN SELECT * FROM docs LIMIT 10")).expect("test: explain");
    assert!(!rows.is_empty());
    assert!(rows[0].data_json_ref().contains("Scan"));
}
