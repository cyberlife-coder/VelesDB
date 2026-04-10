//! VelesQL query execution handlers.

pub mod aggregation;
pub mod explain;
pub(crate) mod velesql_helpers;

pub use aggregation::__path_aggregate;
pub use aggregation::aggregate;
pub use explain::{__path_explain, explain};

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use std::sync::Arc;
use velesdb_core::collection::search::query::projection;
#[cfg(test)]
use velesdb_core::velesql;
use velesdb_core::velesql::{
    DdlStatement, DmlStatement, IntrospectionStatement, Query, SelectColumns,
};

use crate::handlers::helpers::notify_query_timing;
use crate::types::{
    QueryRequest, QueryResponse, QueryResponseMeta, QueryType, VELESQL_CONTRACT_VERSION,
};
use crate::AppState;

use aggregation::{execute_aggregation_query, is_aggregation_query};
use explain::condition_has_vector_search;
use velesql_helpers::{parse_and_validate, velesql_collection_not_found, velesql_error};

/// Returns `true` when the query should bypass collection resolution and go
/// directly through `Database::execute_query` — DDL, introspection, admin,
/// TRAIN, or graph/edge/delete DML that resolves its own collection from the AST.
fn requires_mutation_dispatch(parsed: &Query) -> bool {
    parsed.is_ddl_query()
        || parsed.is_introspection_query()
        || parsed.is_admin_query()
        || parsed.is_train()
        || is_ast_routed_dml(parsed)
}

/// Returns `true` for DML statements that resolve their collection name from
/// the AST rather than from the request body's `FROM` clause:
/// `INSERT EDGE`, `DELETE`, `DELETE EDGE`, `SELECT EDGES`, `INSERT NODE`.
///
/// `INSERT INTO`, `UPSERT`, and `UPDATE` return result rows and must flow
/// through the standard query path (they use `stmt.table` which maps to
/// the SELECT `FROM`).
fn is_ast_routed_dml(parsed: &Query) -> bool {
    matches!(
        parsed.dml,
        Some(
            DmlStatement::InsertEdge(_)
                | DmlStatement::Delete(_)
                | DmlStatement::DeleteEdge(_)
                | DmlStatement::SelectEdges(_)
                | DmlStatement::InsertNode(_)
        )
    )
}

/// Execute a VelesQL query.
///
/// BUG-1 FIX: Automatically detects aggregation queries (GROUP BY, COUNT, SUM, etc.)
/// and routes them to execute_aggregate for proper handling.
///
/// DDL statements (CREATE/DROP COLLECTION) are intercepted before collection
/// resolution and dispatched directly through `Database::execute_query`.
#[utoipa::path(
    post,
    path = "/query",
    tag = "query",
    request_body = QueryRequest,
    responses(
        (status = 200, description = "Query results", body = QueryResponse),
        (status = 400, description = "Query syntax error", body = crate::types::QueryErrorResponse),
        (status = 422, description = "Query validation/execution error", body = crate::types::VelesqlErrorResponse),
        (status = 404, description = "Collection not found", body = crate::types::VelesqlErrorResponse)
    )
)]
#[allow(clippy::unused_async)]
pub async fn query(
    State(state): State<Arc<AppState>>,
    Json(req): Json<QueryRequest>,
) -> impl IntoResponse {
    let start = std::time::Instant::now();

    let parsed = match parse_and_validate(&req.query) {
        Ok(q) => q,
        Err(resp) => return resp,
    };

    // DDL/Introspection/Admin/graph-mutation bypass: these extract collection from
    // the SQL AST, not from the request body.  INSERT INTO, UPSERT, and UPDATE flow
    // through the standard path because they return meaningful result rows.
    if requires_mutation_dispatch(&parsed) {
        return execute_mutation_query(&state, &parsed, &req.params, start);
    }

    let collection_name = match resolve_collection_name(&parsed, &req) {
        Ok(name) => name,
        Err(resp) => return resp,
    };

    // BUG-1 FIX: Detect aggregation queries and route to execute_aggregate
    if is_aggregation_query(&parsed.select) {
        return execute_aggregation_query(&state, &collection_name, &parsed, &req.params, start);
    }

    let results = match execute_standard_query(&state, &parsed, &collection_name, &req) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    build_query_response(
        &state,
        &collection_name,
        start,
        results,
        &parsed.select.columns,
    )
}

/// Extract the collection name targeted by a mutation/DDL/introspection query.
///
/// Returns `"_system"` for global operations that do not target a specific
/// collection (SHOW COLLECTIONS, EXPLAIN, FLUSH without collection). This
/// function is used exclusively for telemetry tagging — it is NOT on the
/// HTTP routing path. The actual query execution goes through
/// [`Database::execute_query`] which enforces its own dispatch rules.
///
/// # Forward compatibility
///
/// The four VelesQL statement enums (`IntrospectionStatement`,
/// `AdminStatement`, `DdlStatement`, `DmlStatement`) are annotated
/// `#[non_exhaustive]` so a wildcard `_` arm is mandatory here. To prevent
/// silent telemetry degradation when a new core variant is added without
/// an accompanying handler update, every wildcard arm now logs a
/// `tracing::warn!` with a stable target so it is visible in CI logs and
/// production observability.
fn extract_mutation_collection_name(parsed: &Query) -> String {
    if let Some(name) = extract_ddl_collection(parsed) {
        return name;
    }
    if let Some(name) = extract_dml_collection(parsed) {
        return name;
    }
    if let Some(ref intro) = parsed.introspection {
        return match intro {
            IntrospectionStatement::DescribeCollection(d) => d.name.clone(),
            IntrospectionStatement::ShowCollections | IntrospectionStatement::Explain(_) => {
                "_system".to_string()
            }
            other => warn_unknown_velesql_variant("IntrospectionStatement", other),
        };
    }
    if let Some(ref admin) = parsed.admin {
        return match admin {
            velesdb_core::velesql::AdminStatement::Flush(f) => f
                .collection
                .clone()
                .unwrap_or_else(|| "_system".to_string()),
            other => warn_unknown_velesql_variant("AdminStatement", other),
        };
    }
    if let Some(ref train) = parsed.train {
        return train.collection.clone();
    }
    "_system".to_string()
}

/// Extract collection name from a DDL statement.
fn extract_ddl_collection(parsed: &Query) -> Option<String> {
    parsed.ddl.as_ref().and_then(|ddl| match ddl {
        DdlStatement::CreateCollection(s) => Some(s.name.clone()),
        DdlStatement::DropCollection(s) => Some(s.name.clone()),
        DdlStatement::CreateIndex(s) => Some(s.collection.clone()),
        DdlStatement::DropIndex(s) => Some(s.collection.clone()),
        DdlStatement::Analyze(s) => Some(s.collection.clone()),
        DdlStatement::Truncate(s) => Some(s.collection.clone()),
        DdlStatement::AlterCollection(s) => Some(s.collection.clone()),
        other => {
            warn_unknown_velesql_variant("DdlStatement", other);
            None
        }
    })
}

/// Extract collection name from a DML statement.
fn extract_dml_collection(parsed: &Query) -> Option<String> {
    parsed.dml.as_ref().and_then(|dml| match dml {
        DmlStatement::Insert(s) | DmlStatement::Upsert(s) => Some(s.table.clone()),
        DmlStatement::Update(s) => Some(s.table.clone()),
        DmlStatement::InsertEdge(s) => Some(s.collection.clone()),
        DmlStatement::Delete(s) => Some(s.table.clone()),
        DmlStatement::DeleteEdge(s) => Some(s.collection.clone()),
        DmlStatement::SelectEdges(s) => Some(s.collection.clone()),
        DmlStatement::InsertNode(s) => Some(s.collection.clone()),
        other => {
            warn_unknown_velesql_variant("DmlStatement", other);
            None
        }
    })
}

/// Logs a structured warning when an unknown VelesQL statement variant is
/// encountered on the telemetry extraction path and returns the
/// `"_system"` sentinel string. The structured `target` `velesql.dispatch`
/// is stable so CI log aggregators and production observability can alert
/// when a new core variant slips past the handler mapper without an
/// accompanying update.
///
/// Note: this fallback only affects the collection-name tag attached to
/// request telemetry. The HTTP response is still produced by the
/// downstream `Database::execute_query` call which rejects truly
/// unsupported statements with a proper error.
fn warn_unknown_velesql_variant<T: std::fmt::Debug>(kind: &'static str, variant: &T) -> String {
    tracing::warn!(
        target: "velesql.dispatch",
        enum_kind = kind,
        variant = ?variant,
        "unknown VelesQL statement variant on telemetry extraction path; \
         routing collection tag to _system — add the new variant to \
         extract_mutation_collection_name in handlers/query/mod.rs"
    );
    "_system".to_string()
}

/// Execute a DDL, graph/delete DML, introspection, admin, or TRAIN query.
///
/// DDL (CREATE/DROP/ALTER/ANALYZE/TRUNCATE), graph/delete DML mutations
/// (INSERT EDGE, DELETE, DELETE EDGE, SELECT EDGES, INSERT NODE),
/// introspection (SHOW/DESCRIBE/EXPLAIN), admin (FLUSH), and TRAIN
/// statements extract collection names from the SQL AST — no FROM clause
/// needed.
///
/// Results from `Database::execute_query` are propagated into the response
/// so that introspection (SHOW, DESCRIBE, EXPLAIN), ANALYZE, and SELECT
/// EDGES return their data to the caller.
fn execute_mutation_query(
    state: &Arc<AppState>,
    parsed: &Query,
    params: &std::collections::HashMap<String, serde_json::Value>,
    start: std::time::Instant,
) -> axum::response::Response {
    match state.db.execute_query(parsed, params) {
        Ok(results) => {
            let coll_name = extract_mutation_collection_name(parsed);
            notify_query_timing(state, &coll_name, start);
            let timing_ms = start.elapsed().as_secs_f64() * 1000.0;
            #[allow(clippy::cast_possible_truncation)]
            // Reason: timing_ms is always < u64::MAX (query durations < 585 millennia)
            let took_ms = timing_ms.round() as u64;
            let projected = projection::project_results(&results, &parsed.select.columns);
            let rows_returned = projected.len();
            Json(QueryResponse {
                results: projected,
                timing_ms,
                took_ms,
                rows_returned,
                meta: QueryResponseMeta {
                    velesql_contract_version: VELESQL_CONTRACT_VERSION.to_string(),
                    count: rows_returned,
                },
            })
            .into_response()
        }
        Err(e) => velesql_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "VELESQL_MUTATION_ERROR",
            &e.to_string(),
            "Check collection name, statement syntax, and target existence",
            None,
        ),
    }
}

/// Determine the target collection from the parsed query and request body.
#[allow(clippy::result_large_err)]
fn resolve_collection_name(
    parsed: &Query,
    req: &QueryRequest,
) -> Result<String, axum::response::Response> {
    if parsed.is_match_query() {
        req.collection
            .as_ref()
            .filter(|name| !name.is_empty())
            .cloned()
            .ok_or_else(|| {
                velesql_error(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "VELESQL_MISSING_COLLECTION",
                    "MATCH query via /query requires `collection` in request body",
                    "Add `collection` to the /query JSON body or use /collections/{name}/match",
                    Some(serde_json::json!({
                        "field": "collection",
                        "endpoint": "/query",
                        "query_type": "MATCH"
                    })),
                )
            })
    } else {
        Ok(parsed.select.from.clone())
    }
}

/// Execute a standard (non-aggregation) query, dispatching MATCH vs SELECT.
#[allow(clippy::result_large_err)]
fn execute_standard_query(
    state: &Arc<AppState>,
    parsed: &Query,
    collection_name: &str,
    req: &QueryRequest,
) -> Result<Vec<velesdb_core::SearchResult>, axum::response::Response> {
    let execute_result = if parsed.is_match_query() {
        // MATCH queries need a collection instance for execute_query.
        // Route through typed registries: vector → graph → metadata.
        if let Some(vc) = state.db.get_vector_collection(collection_name) {
            vc.execute_query(parsed, &req.params)
        } else if let Some(gc) = state.db.get_graph_collection(collection_name) {
            gc.execute_query(parsed, &req.params)
        } else if let Some(mc) = state.db.get_metadata_collection(collection_name) {
            mc.execute_query(parsed, &req.params)
        } else {
            Err(velesdb_core::Error::CollectionNotFound(
                collection_name.to_string(),
            ))
        }
    } else {
        state.db.execute_query(parsed, &req.params)
    };

    execute_result.map_err(|e| match e {
        velesdb_core::Error::CollectionNotFound(name) => velesql_collection_not_found(&name),
        other => velesql_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "VELESQL_EXECUTION_ERROR",
            &other.to_string(),
            "Validate query semantics and parameter types against the target collection",
            None,
        ),
    })
}

/// Build the final query response with timing metrics and SQL projection.
fn build_query_response(
    state: &Arc<AppState>,
    collection_name: &str,
    start: std::time::Instant,
    results: Vec<velesdb_core::SearchResult>,
    select_columns: &SelectColumns,
) -> axum::response::Response {
    let timing_ms = start.elapsed().as_secs_f64() * 1000.0;
    #[allow(clippy::cast_possible_truncation)]
    // Reason: timing_ms is always < u64::MAX (query durations < 585 millennia)
    let took_ms = timing_ms.round() as u64;
    notify_query_timing(state, collection_name, start);
    let projected = projection::project_results(&results, select_columns);
    let rows_returned = projected.len();

    Json(QueryResponse {
        results: projected,
        timing_ms,
        took_ms,
        rows_returned,
        meta: QueryResponseMeta {
            velesql_contract_version: VELESQL_CONTRACT_VERSION.to_string(),
            count: rows_returned,
        },
    })
    .into_response()
}

/// Detect query type from parsed AST (EPIC-052 US-006).
///
/// Priority order:
/// 1. DDL (CREATE/DROP COLLECTION) -> Ddl
/// 2. DML (INSERT/UPDATE/DELETE) -> Dml
/// 3. MATCH clause -> Graph
/// 4. GROUP BY or aggregates -> Aggregation
/// 5. Vector search -> Search
/// 6. Default -> Rows
#[allow(dead_code)] // Used in tests, will be used in unified handler
pub fn detect_query_type(query: &Query) -> QueryType {
    if query.is_ddl_query() {
        return QueryType::Ddl;
    }

    if query.is_dml_query() {
        return QueryType::Dml;
    }

    if query.is_match_query() {
        return QueryType::Graph;
    }

    if is_aggregation_query(&query.select) {
        return QueryType::Aggregation;
    }

    let has_vector = query
        .select
        .where_clause
        .as_ref()
        .map(condition_has_vector_search)
        .unwrap_or(false);

    if has_vector {
        return QueryType::Search;
    }

    QueryType::Rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_query_type_search() {
        let parsed = velesql::Parser::parse(
            "SELECT * FROM docs WHERE similarity(embedding, $v) > 0.8 LIMIT 10",
        )
        .unwrap();
        assert_eq!(detect_query_type(&parsed), QueryType::Search);
    }

    #[test]
    fn test_detect_query_type_aggregation() {
        let parsed =
            velesql::Parser::parse("SELECT category, COUNT(*) FROM products GROUP BY category")
                .unwrap();
        assert_eq!(detect_query_type(&parsed), QueryType::Aggregation);
    }

    #[test]
    fn test_detect_query_type_rows() {
        let parsed =
            velesql::Parser::parse("SELECT name, price FROM products WHERE price > 100").unwrap();
        assert_eq!(detect_query_type(&parsed), QueryType::Rows);
    }

    #[test]
    fn test_detect_query_type_graph() {
        let parsed =
            velesql::Parser::parse("MATCH (n:Person)-[:KNOWS]->(m) RETURN n.name, m.name LIMIT 10")
                .unwrap();
        assert_eq!(detect_query_type(&parsed), QueryType::Graph);
    }

    #[test]
    fn test_detect_query_type_hybrid_vector_aggregation() {
        // When both vector search and aggregation, aggregation takes priority
        let parsed = velesql::Parser::parse(
            "SELECT category, COUNT(*) FROM docs WHERE similarity(embedding, $v) > 0.7 GROUP BY category",
        )
        .unwrap();
        assert_eq!(detect_query_type(&parsed), QueryType::Aggregation);
    }

    #[test]
    fn test_detect_query_type_ddl_create() {
        let parsed =
            velesql::Parser::parse("CREATE COLLECTION docs (dimension = 768, metric = 'cosine');")
                .unwrap();
        assert_eq!(detect_query_type(&parsed), QueryType::Ddl);
    }

    #[test]
    fn test_detect_query_type_ddl_drop() {
        let parsed = velesql::Parser::parse("DROP COLLECTION docs;").unwrap();
        assert_eq!(detect_query_type(&parsed), QueryType::Ddl);
    }

    #[test]
    fn test_detect_query_type_dml_insert_edge() {
        let parsed = velesql::Parser::parse(
            "INSERT EDGE INTO kg (source = 1, target = 2, label = 'KNOWS');",
        )
        .unwrap();
        assert_eq!(detect_query_type(&parsed), QueryType::Dml);
    }

    #[test]
    fn test_detect_query_type_dml_delete() {
        let parsed = velesql::Parser::parse("DELETE FROM docs WHERE id = 1;").unwrap();
        assert_eq!(detect_query_type(&parsed), QueryType::Dml);
    }
}
