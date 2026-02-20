//! VelesQL query execution handler.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use std::sync::Arc;

use crate::types::{
    AggregationResponse, ExplainCost, ExplainFeatures, ExplainRequest, ExplainResponse,
    ExplainStep, QueryErrorDetail, QueryErrorResponse, QueryRequest, QueryResponse,
    QueryResponseMeta, QueryType, SearchResultResponse, VelesqlErrorDetail, VelesqlErrorResponse,
    VELESQL_CONTRACT_VERSION,
};
use crate::AppState;
use velesdb_core::velesql::{self, Condition, Query, SelectColumns};

/// Execute a VelesQL query.
///
/// BUG-1 FIX: Automatically detects aggregation queries (GROUP BY, COUNT, SUM, etc.)
/// and routes them to execute_aggregate for proper handling.
#[utoipa::path(
    post,
    path = "/query",
    tag = "query",
    request_body = QueryRequest,
    responses(
        (status = 200, description = "Query results", body = QueryResponse),
        (status = 400, description = "Query syntax error", body = QueryErrorResponse),
        (status = 422, description = "Query validation/execution error", body = VelesqlErrorResponse),
        (status = 404, description = "Collection not found", body = VelesqlErrorResponse)
    )
)]
#[allow(clippy::unused_async)]
pub async fn query(
    State(state): State<Arc<AppState>>,
    Json(req): Json<QueryRequest>,
) -> impl IntoResponse {
    let start = std::time::Instant::now();

    let parsed = match velesql::Parser::parse(&req.query) {
        Ok(q) => q,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(QueryErrorResponse {
                    error: QueryErrorDetail {
                        error_type: format!("{:?}", e.kind),
                        message: e.message.clone(),
                        position: e.position,
                        query: e.fragment.clone(),
                    },
                }),
            )
                .into_response()
        }
    };

    if let Err(e) = velesql::QueryValidator::validate(&parsed) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(VelesqlErrorResponse {
                error: VelesqlErrorDetail {
                    code: "VELESQL_VALIDATION_ERROR".to_string(),
                    message: e.to_string(),
                    hint: e.suggestion,
                    details: None,
                },
            }),
        )
            .into_response();
    }

    let select = &parsed.select;
    let collection_name = if parsed.is_match_query() {
        match req.collection.as_ref().filter(|name| !name.is_empty()) {
            Some(name) => name.clone(),
            None => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(VelesqlErrorResponse {
                        error: VelesqlErrorDetail {
                            code: "VELESQL_MISSING_COLLECTION".to_string(),
                            message: "MATCH query via /query requires `collection` in request body"
                                .to_string(),
                            hint: "Add `collection` to the /query JSON body or use /collections/{name}/match".to_string(),
                            details: Some(serde_json::json!({
                                "field": "collection",
                                "endpoint": "/query",
                                "query_type": "MATCH"
                            })),
                        },
                    }),
                )
                    .into_response()
            }
        }
    } else {
        select.from.clone()
    };

    // BUG-1 FIX: Detect aggregation queries and route to execute_aggregate
    let is_aggregation = matches!(
        &select.columns,
        SelectColumns::Aggregations(_) | SelectColumns::Mixed { .. }
    ) || select.group_by.is_some();

    if is_aggregation {
        let collection = match state.db.get_collection(&collection_name) {
            Some(c) => c,
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(VelesqlErrorResponse {
                        error: VelesqlErrorDetail {
                            code: "VELESQL_COLLECTION_NOT_FOUND".to_string(),
                            message: format!("Collection '{}' not found", collection_name),
                            hint: "Create the collection first or correct the collection name"
                                .to_string(),
                            details: Some(serde_json::json!({
                                "collection": collection_name
                            })),
                        },
                    }),
                )
                    .into_response()
            }
        };
        // Route to aggregation execution
        let result =
            match collection.execute_aggregate(&parsed, &req.params) {
                Ok(r) => r,
                Err(e) => return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(VelesqlErrorResponse {
                        error: VelesqlErrorDetail {
                            code: "VELESQL_AGGREGATION_ERROR".to_string(),
                            message: e.to_string(),
                            hint: "Verify GROUP BY/HAVING clauses and aggregate function arguments"
                                .to_string(),
                            details: None,
                        },
                    }),
                )
                    .into_response(),
            };

        let timing_ms = start.elapsed().as_secs_f64() * 1000.0;
        return Json(AggregationResponse { result, timing_ms }).into_response();
    }

    // Standard query execution:
    // - top-level MATCH executes in requested collection context
    // - SELECT executes through database-level dispatcher for cross-collection JOIN support
    let execute_result = if parsed.is_match_query() {
        match state.db.get_collection(&collection_name) {
            Some(c) => c.execute_query(&parsed, &req.params),
            None => Err(velesdb_core::Error::CollectionNotFound(
                collection_name.clone(),
            )),
        }
    } else {
        state.db.execute_query(&parsed, &req.params)
    };

    let results = match execute_result {
        Ok(r) => r,
        Err(velesdb_core::Error::CollectionNotFound(name)) => {
            return (
                StatusCode::NOT_FOUND,
                Json(VelesqlErrorResponse {
                    error: VelesqlErrorDetail {
                        code: "VELESQL_COLLECTION_NOT_FOUND".to_string(),
                        message: format!("Collection '{}' not found", name),
                        hint: "Create the collection first or correct the collection name"
                            .to_string(),
                        details: Some(serde_json::json!({
                            "collection": name
                        })),
                    },
                }),
            )
                .into_response()
        }
        Err(e) => return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(VelesqlErrorResponse {
                error: VelesqlErrorDetail {
                    code: "VELESQL_EXECUTION_ERROR".to_string(),
                    message: e.to_string(),
                    hint:
                        "Validate query semantics and parameter types against the target collection"
                            .to_string(),
                    details: None,
                },
            }),
        )
            .into_response(),
    };

    let timing_ms = start.elapsed().as_secs_f64() * 1000.0;
    let took_ms = timing_ms.round() as u64;
    let rows_returned = results.len();

    Json(QueryResponse {
        results: results
            .into_iter()
            .map(|r| SearchResultResponse {
                id: r.point.id,
                score: r.score,
                payload: r.point.payload,
            })
            .collect(),
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

/// Explain a VelesQL query without executing it (EPIC-058 US-002).
///
/// Returns the query plan, estimated costs, and detected features.
#[allow(clippy::too_many_lines)]
#[utoipa::path(
    post,
    path = "/query/explain",
    tag = "query",
    request_body = ExplainRequest,
    responses(
        (status = 200, description = "Query plan", body = ExplainResponse),
        (status = 400, description = "Query syntax error", body = QueryErrorResponse),
        (status = 404, description = "Collection not found", body = VelesqlErrorResponse)
    )
)]
#[allow(clippy::unused_async)]
pub async fn explain(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ExplainRequest>,
) -> impl IntoResponse {
    // Parse the query
    let parsed = match velesql::Parser::parse(&req.query) {
        Ok(q) => q,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(QueryErrorResponse {
                    error: QueryErrorDetail {
                        error_type: format!("{:?}", e.kind),
                        message: e.message.clone(),
                        position: e.position,
                        query: e.fragment.clone(),
                    },
                }),
            )
                .into_response()
        }
    };

    let select = &parsed.select;

    // Check collection exists
    let collection_exists = state.db.get_collection(&select.from).is_some();
    if !collection_exists && !select.from.is_empty() {
        return (
            StatusCode::NOT_FOUND,
            Json(VelesqlErrorResponse {
                error: VelesqlErrorDetail {
                    code: "VELESQL_COLLECTION_NOT_FOUND".to_string(),
                    message: format!("Collection '{}' not found", select.from),
                    hint: "Create the collection first or correct the FROM collection".to_string(),
                    details: Some(serde_json::json!({
                        "collection": select.from
                    })),
                },
            }),
        )
            .into_response();
    }

    // Detect query features
    let has_vector_search = select
        .where_clause
        .as_ref()
        .map(condition_has_vector_search)
        .unwrap_or(false);

    let has_filter = select.where_clause.is_some() && !has_vector_search;

    let has_aggregation = matches!(
        &select.columns,
        SelectColumns::Aggregations(_) | SelectColumns::Mixed { .. }
    );

    let features = ExplainFeatures {
        has_vector_search,
        has_filter,
        has_order_by: select.order_by.is_some(),
        has_group_by: select.group_by.is_some(),
        has_aggregation,
        has_join: !select.joins.is_empty(),
        has_fusion: select.fusion_clause.is_some(),
        limit: select.limit,
        offset: select.offset,
    };

    // Build execution plan
    let mut plan = Vec::new();
    let mut step_num = 1;

    // Step 1: Source scan or vector search
    if has_vector_search {
        plan.push(ExplainStep {
            step: step_num,
            operation: "VectorSearch".to_string(),
            description: "ANN search using HNSW index with NEAR clause".to_string(),
            estimated_rows: select.limit.map(|l| l as usize),
        });
    } else {
        plan.push(ExplainStep {
            step: step_num,
            operation: "FullScan".to_string(),
            description: format!("Scan collection '{}'", select.from),
            estimated_rows: None,
        });
    }
    step_num += 1;

    // Step 2: Filter (if present and not just vector search)
    if has_filter {
        plan.push(ExplainStep {
            step: step_num,
            operation: "Filter".to_string(),
            description: "Apply WHERE clause predicates".to_string(),
            estimated_rows: None,
        });
        step_num += 1;
    }

    // Step 3: JOIN (if present)
    if !select.joins.is_empty() {
        for join in &select.joins {
            plan.push(ExplainStep {
                step: step_num,
                operation: format!("{:?}Join", join.join_type),
                description: format!("Join with '{}'", join.table),
                estimated_rows: None,
            });
            step_num += 1;
        }
    }

    // Step 4: GROUP BY (if present)
    if select.group_by.is_some() {
        plan.push(ExplainStep {
            step: step_num,
            operation: "GroupBy".to_string(),
            description: "Group rows by specified columns".to_string(),
            estimated_rows: None,
        });
        step_num += 1;
    }

    // Step 5: Aggregation (if present)
    if has_aggregation {
        plan.push(ExplainStep {
            step: step_num,
            operation: "Aggregate".to_string(),
            description: "Compute aggregate functions (COUNT, SUM, etc.)".to_string(),
            estimated_rows: None,
        });
        step_num += 1;
    }

    // Step 6: ORDER BY (if present)
    if select.order_by.is_some() {
        plan.push(ExplainStep {
            step: step_num,
            operation: "Sort".to_string(),
            description: "Sort results by ORDER BY clause".to_string(),
            estimated_rows: None,
        });
        step_num += 1;
    }

    // Step 7: LIMIT/OFFSET (if present)
    if select.limit.is_some() || select.offset.is_some() {
        plan.push(ExplainStep {
            step: step_num,
            operation: "Limit".to_string(),
            description: format!(
                "Apply LIMIT {} OFFSET {}",
                select.limit.unwrap_or(0),
                select.offset.unwrap_or(0)
            ),
            estimated_rows: select.limit.map(|l| l as usize),
        });
    }

    // Estimate cost
    let complexity = if has_vector_search {
        "O(log n)"
    } else {
        "O(n)"
    };

    let estimated_cost = ExplainCost {
        uses_index: has_vector_search,
        index_name: if has_vector_search {
            Some("HNSW".to_string())
        } else {
            None
        },
        selectivity: if has_vector_search { 0.01 } else { 1.0 },
        complexity: complexity.to_string(),
    };

    let query_type = if parsed.is_match_query() {
        "MATCH"
    } else {
        "SELECT"
    };

    Json(ExplainResponse {
        query: req.query,
        query_type: query_type.to_string(),
        collection: select.from.clone(),
        plan,
        estimated_cost,
        features,
    })
    .into_response()
}

/// Check if a condition contains vector search.
fn condition_has_vector_search(cond: &Condition) -> bool {
    match cond {
        Condition::VectorSearch(_)
        | Condition::VectorFusedSearch { .. }
        | Condition::Similarity(_) => true,
        Condition::And(left, right) | Condition::Or(left, right) => {
            condition_has_vector_search(left) || condition_has_vector_search(right)
        }
        Condition::Group(inner) | Condition::Not(inner) => condition_has_vector_search(inner),
        _ => false,
    }
}

/// Detect query type from parsed AST (EPIC-052 US-006).
///
/// Priority order:
/// 1. MATCH clause → Graph
/// 2. GROUP BY or aggregates → Aggregation
/// 3. Vector search → Search
/// 4. Default → Rows
#[allow(dead_code)] // Used in tests, will be used in unified handler
pub fn detect_query_type(query: &Query) -> QueryType {
    // Check for MATCH clause first
    if query.is_match_query() {
        return QueryType::Graph;
    }

    let select = &query.select;

    // Check for aggregation
    let is_aggregation = matches!(
        &select.columns,
        SelectColumns::Aggregations(_) | SelectColumns::Mixed { .. }
    ) || select.group_by.is_some();

    if is_aggregation {
        return QueryType::Aggregation;
    }

    // Check for vector search
    let has_vector = select
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
}
