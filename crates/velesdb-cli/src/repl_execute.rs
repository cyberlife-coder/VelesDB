//! Query execution logic for the REPL.
//!
//! Extracted from `repl.rs` to keep module size under 500 NLOC.

use anyhow::Result;
use colored::Colorize;
use instant::Instant;
use std::collections::HashMap;
use velesdb_core::Database;

use crate::repl::{QueryKind, QueryResult};

/// Execute a `VelesQL` query and return results.
///
/// Delegates to [`Database::execute_query`] for SELECT/DML/TRAIN queries.
/// MATCH queries are routed to `Collection::execute_query` on the active
/// collection (set via `.use collection_name`).
pub fn execute_query(
    db: &Database,
    query: &str,
    active_collection: Option<&str>,
) -> Result<QueryResult> {
    let start = Instant::now();

    // Parse the query
    let parsed = velesdb_core::velesql::Parser::parse(query)
        .map_err(|e| anyhow::anyhow!("Parse error: {}", e.message))?;

    // Check if there's a vector search requiring parameters (SELECT or MATCH WHERE).
    let has_param_vector = parsed
        .select
        .where_clause
        .as_ref()
        .is_some_and(contains_param_vector)
        || parsed
            .match_clause
            .as_ref()
            .and_then(|m| m.where_clause.as_ref())
            .is_some_and(contains_param_vector);

    if has_param_vector {
        // Vector search with parameter requires external input
        println!(
            "{}",
            "Note: Vector search with $parameter requires REST API. Use literal vectors or metadata-only queries."
                .yellow()
        );
        let duration_ms = start.elapsed().as_secs_f64() * 1000.0;
        return Ok(QueryResult {
            rows: Vec::new(),
            duration_ms,
            kind: QueryKind::Select,
        });
    }

    let kind = query_kind(&parsed);
    let params = HashMap::new();

    let results = if parsed.is_match_query() {
        execute_match_query(db, &parsed, active_collection, &params)?
    } else {
        db.execute_query(&parsed, &params)
            .map_err(|e| anyhow::anyhow!("Query error: {e}"))?
    };

    let rows = results.into_iter().map(result_to_row).collect();
    let duration_ms = start.elapsed().as_secs_f64() * 1000.0;

    Ok(QueryResult {
        rows,
        duration_ms,
        kind,
    })
}

/// Determine the [`QueryKind`] of a parsed query for display purposes.
fn query_kind(parsed: &velesdb_core::velesql::Query) -> QueryKind {
    if parsed.is_ddl_query() {
        QueryKind::Ddl
    } else if parsed.is_introspection_query() {
        QueryKind::Introspection
    } else if parsed.is_admin_query() {
        QueryKind::Admin
    } else if parsed.is_dml_query() {
        QueryKind::Dml
    } else if parsed.is_train() {
        QueryKind::Train
    } else {
        QueryKind::Select
    }
}

/// Route a MATCH query to the active collection (graph first, then vector).
fn execute_match_query(
    db: &Database,
    parsed: &velesdb_core::velesql::Query,
    active_collection: Option<&str>,
    params: &HashMap<String, serde_json::Value>,
) -> Result<Vec<velesdb_core::SearchResult>> {
    let col_name = active_collection.ok_or_else(|| {
        anyhow::anyhow!("MATCH queries require an active collection. Use: .use <collection_name>")
    })?;
    if let Some(graph_col) = db.get_graph_collection(col_name) {
        graph_col
            .execute_query(parsed, params)
            .map_err(|e| anyhow::anyhow!("Query error: {e}"))
    } else if let Some(vec_col) = db.get_vector_collection(col_name) {
        vec_col
            .execute_query(parsed, params)
            .map_err(|e| anyhow::anyhow!("Query error: {e}"))
    } else {
        Err(anyhow::anyhow!("Collection '{}' not found", col_name))
    }
}

/// Convert a single [`velesdb_core::SearchResult`] into a display row.
fn result_to_row(r: velesdb_core::SearchResult) -> HashMap<String, serde_json::Value> {
    let mut row = HashMap::new();
    row.insert("id".to_string(), serde_json::json!(r.point.id));
    row.insert("score".to_string(), serde_json::json!(r.score));

    if let Some(serde_json::Value::Object(map)) = &r.point.payload {
        for (k, v) in map {
            row.insert(k.clone(), v.clone());
        }
    }
    row
}

pub(crate) fn contains_param_vector(condition: &velesdb_core::velesql::Condition) -> bool {
    use velesdb_core::velesql::{Condition, SparseVectorExpr, VectorExpr};
    match condition {
        Condition::VectorSearch(vs) => matches!(vs.vector, VectorExpr::Parameter(_)),
        Condition::VectorFusedSearch(vfs) => vfs
            .vectors
            .iter()
            .any(|v| matches!(v, VectorExpr::Parameter(_))),
        Condition::SparseVectorSearch(svs) => {
            matches!(svs.vector, SparseVectorExpr::Parameter(_))
        }
        Condition::Similarity(sim) => matches!(sim.vector, VectorExpr::Parameter(_)),
        Condition::And(left, right) | Condition::Or(left, right) => {
            contains_param_vector(left) || contains_param_vector(right)
        }
        Condition::Not(inner) | Condition::Group(inner) => contains_param_vector(inner),
        Condition::Comparison(_)
        | Condition::In(_)
        | Condition::Between(_)
        | Condition::Like(_)
        | Condition::IsNull(_)
        | Condition::Match(_)
        | Condition::GraphMatch(_)
        | Condition::Contains(_)
        | Condition::ContainsText(_)
        | Condition::GeoDistance(_)
        | Condition::GeoBbox(_) => false,
        _ => false,
    }
}
