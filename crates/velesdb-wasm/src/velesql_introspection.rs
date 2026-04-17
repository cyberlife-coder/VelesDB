//! Introspection dispatch for the WASM VelesQL executor (S4-13).
//!
//! Supports:
//! - `SHOW COLLECTIONS` → one row per collection `(name, type, dimension)`
//! - `DESCRIBE COLLECTION <name>` → `(name, type, dimension, metric)`
//! - `EXPLAIN <query>` → synthetic plan emitted as one row per logical step.

use velesdb_core::velesql::{DescribeCollectionStatement, IntrospectionStatement};

use crate::database::DatabaseInner;
use crate::velesql_explain;
use crate::velesql_result::QueryResultRow;

/// Executes an introspection statement and returns its row set.
pub(crate) fn execute(
    db: &DatabaseInner,
    stmt: &IntrospectionStatement,
) -> Result<Vec<QueryResultRow>, String> {
    match stmt {
        IntrospectionStatement::ShowCollections => show_collections(db),
        IntrospectionStatement::DescribeCollection(s) => describe_collection(db, s),
        IntrospectionStatement::Explain(q) => velesql_explain::explain(db, q),
        // Defensive: `IntrospectionStatement` is `#[non_exhaustive]`.
        _ => Err(format!(
            "Unsupported introspection variant in WASM: {stmt:?}"
        )),
    }
}

/// `SHOW COLLECTIONS` — one synthetic row per registered collection.
fn show_collections(db: &DatabaseInner) -> Result<Vec<QueryResultRow>, String> {
    let mut rows = Vec::new();
    for (name, dim, is_metadata) in db.collection_summaries() {
        let kind = if is_metadata { "metadata" } else { "vector" };
        let payload = serde_json::json!({
            "name": name,
            "type": kind,
            "dimension": dim,
        });
        rows.push(QueryResultRow::synthetic(payload)?);
    }
    Ok(rows)
}

/// `DESCRIBE COLLECTION <name>` — a single synthetic row with metadata.
fn describe_collection(
    db: &DatabaseInner,
    stmt: &DescribeCollectionStatement,
) -> Result<Vec<QueryResultRow>, String> {
    let store = db.get_shared_store(&stmt.name)?;
    let borrowed = store.borrow();
    let kind = if borrowed.dimension() == 0 {
        "metadata"
    } else {
        "vector"
    };
    let metric = metric_to_string(borrowed.metric);
    let payload = serde_json::json!({
        "name": stmt.name,
        "type": kind,
        "dimension": borrowed.dimension(),
        "metric": metric,
        "count": borrowed.len(),
    });
    Ok(vec![QueryResultRow::synthetic(payload)?])
}

/// Canonical string form of a distance metric.
fn metric_to_string(m: velesdb_core::DistanceMetric) -> &'static str {
    use velesdb_core::DistanceMetric;
    match m {
        DistanceMetric::Cosine => "cosine",
        DistanceMetric::Euclidean => "euclidean",
        DistanceMetric::DotProduct => "dot",
        DistanceMetric::Hamming => "hamming",
        DistanceMetric::Jaccard => "jaccard",
        // `DistanceMetric` is `#[non_exhaustive]`; unknown variants fall back
        // to "cosine" for display stability.
        _ => "cosine",
    }
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn test_explain_returns_plan_rows() {
        let mut db = DatabaseInner::new();
        db.create_metadata_collection("docs").expect("test: meta");
        let rows = execute(&db, &parse_intro("EXPLAIN SELECT * FROM docs LIMIT 10"))
            .expect("test: explain");
        assert!(!rows.is_empty());
        assert!(rows[0].data_json_ref().contains("Scan"));
    }
}
