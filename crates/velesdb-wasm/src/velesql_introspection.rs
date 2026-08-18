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
///
/// Returns `"unknown"` for variants not yet recognised by the WASM
/// introspection surface (Devin Review Finding L). `DistanceMetric` is
/// `#[non_exhaustive]`; a future variant added in core surfaces honestly
/// as `"unknown"` rather than silently masquerading as cosine in
/// `DESCRIBE COLLECTION`.
// TODO(US-S4-13): update when DistanceMetric gains new variants.
fn metric_to_string(m: velesdb_core::DistanceMetric) -> &'static str {
    use velesdb_core::DistanceMetric;
    match m {
        DistanceMetric::Cosine => "cosine",
        DistanceMetric::Euclidean => "euclidean",
        DistanceMetric::DotProduct => "dot",
        DistanceMetric::Hamming => "hamming",
        DistanceMetric::Jaccard => "jaccard",
        _ => "unknown",
    }
}

#[cfg(test)]
#[path = "velesql_introspection_tests.rs"]
mod tests;
