//! Query and MATCH methods for Collection (extracted from collection.rs).
//!
//! Contains: query, query_ids, match_query, explain.

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::collections::HashMap;

use crate::collection::Collection;
use crate::collection_helpers::{match_result_to_dict, search_results_to_multimodel_dicts};
use crate::utils::{extract_vector, python_to_json, to_pyobject};

#[pymethods]
impl Collection {
    /// Execute a VelesQL query (EPIC-031 US-008).
    ///
    /// Executes SELECT-style VelesQL queries with vector similarity search.
    ///
    /// Note: Currently supports SELECT syntax only. MATCH/graph traversal
    /// syntax is planned for a future release (see EPIC-010).
    ///
    /// Args:
    ///     query_str: VelesQL SELECT query string
    ///     params: Query parameters (vectors as lists/numpy arrays, scalars)
    ///
    /// Returns:
    ///     List of query results with node_id, vector_score, graph_score,
    ///     fused_score, bindings (payload), and column_data
    ///
    /// Example:
    ///     >>> results = collection.query(
    ///     ...     "SELECT * FROM docs WHERE vector NEAR $q LIMIT 20",
    ///     ...     params={"q": query_embedding}
    ///     ... )
    ///     >>> for r in results:
    ///     ...     print(f"Node: {r['node_id']}, Score: {r['fused_score']:.3f}")
    #[pyo3(signature = (query_str, params=None))]
    fn query(
        &self,
        query_str: &str,
        params: Option<HashMap<String, PyObject>>,
    ) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            let parsed = velesdb_core::velesql::Parser::parse(query_str).map_err(|e| {
                PyValueError::new_err(format!("VelesQL parse error: {}", e.message))
            })?;

            let rust_params: std::collections::HashMap<String, serde_json::Value> = params
                .unwrap_or_default()
                .into_iter()
                .filter_map(|(k, v)| python_to_json(py, &v).map(|json_val| (k, json_val)))
                .collect();

            let results = self
                .inner
                .execute_query(&parsed, &rust_params)
                .map_err(|e| PyRuntimeError::new_err(format!("Query failed: {e}")))?;

            Ok(search_results_to_multimodel_dicts(py, results))
        })
    }

    /// Execute a VelesQL query returning only IDs and scores (no payload).
    ///
    /// More efficient than `query()` when payload is not needed.
    ///
    /// Args:
    ///     velesql: VelesQL query string
    ///     params: Optional dict of query parameters
    ///
    /// Returns:
    ///     List of dicts with 'id' and 'score' fields
    ///
    /// Example:
    ///     >>> ids = collection.query_ids("SELECT * FROM docs WHERE price > 100 LIMIT 5")
    #[pyo3(signature = (velesql, params = None))]
    fn query_ids(
        &self,
        velesql: &str,
        params: Option<HashMap<String, PyObject>>,
    ) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            let parsed_query = velesdb_core::velesql::Parser::parse(velesql).map_err(|e| {
                PyRuntimeError::new_err(format!(
                    "VelesQL syntax error at position {}: {}",
                    e.position, e.message
                ))
            })?;

            let json_params: std::collections::HashMap<String, serde_json::Value> = params
                .unwrap_or_default()
                .into_iter()
                .filter_map(|(k, v)| python_to_json(py, &v).map(|json_val| (k, json_val)))
                .collect();

            let results = self
                .inner
                .execute_query(&parsed_query, &json_params)
                .map_err(|e| PyRuntimeError::new_err(format!("Query execution failed: {e}")))?;

            // Return only IDs and scores
            Ok(results
                .into_iter()
                .map(|r| {
                    let mut dict = HashMap::new();
                    dict.insert("id".to_string(), to_pyobject(py, r.point.id));
                    dict.insert("score".to_string(), to_pyobject(py, r.score));
                    dict
                })
                .collect())
        })
    }

    // ========================================================================
    // MATCH Graph Traversal (Phase 4.3 Plan 01)
    // ========================================================================

    /// Execute a MATCH graph traversal query.
    ///
    /// Delegates to core's execute_match() and execute_match_with_similarity().
    ///
    /// Args:
    ///     query_str: VelesQL MATCH query string
    ///     params: Query parameters (default: empty dict)
    ///     vector: Optional query vector for similarity scoring
    ///     threshold: Similarity threshold 0.0-1.0 (default: 0.0)
    ///
    /// Returns:
    ///     List of dicts with keys: node_id, depth, path, bindings, score, projected
    ///
    /// Example:
    ///     >>> results = collection.match_query(
    ///     ...     "MATCH (a:Person)-[:KNOWS]->(b) RETURN a.name",
    ///     ...     params={}
    ///     ... )
    ///     >>> for r in results:
    ///     ...     print(f"Node {r['node_id']} at depth {r['depth']}")
    #[pyo3(signature = (query_str, params = None, vector = None, threshold = 0.0))]
    fn match_query(
        &self,
        query_str: &str,
        params: Option<HashMap<String, PyObject>>,
        vector: Option<PyObject>,
        threshold: f32,
    ) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            // 1. Parse query
            let parsed = velesdb_core::velesql::Parser::parse(query_str).map_err(|e| {
                PyValueError::new_err(format!("VelesQL parse error: {}", e.message))
            })?;

            // 2. Extract match_clause (error if not MATCH)
            let match_clause = parsed.match_clause.as_ref().ok_or_else(|| {
                PyValueError::new_err("Query is not a MATCH query. Use query() for SELECT queries.")
            })?;

            // 3. Convert params from Python dict to HashMap<String, serde_json::Value>
            let rust_params: std::collections::HashMap<String, serde_json::Value> = params
                .unwrap_or_default()
                .into_iter()
                .filter_map(|(k, v)| python_to_json(py, &v).map(|json_val| (k, json_val)))
                .collect();

            // 4. Execute: with or without similarity
            let results = if let Some(ref vec_obj) = vector {
                let query_vector = extract_vector(py, vec_obj)?;
                self.inner
                    .execute_match_with_similarity(
                        match_clause,
                        &query_vector,
                        threshold,
                        &rust_params,
                    )
                    .map_err(|e| PyRuntimeError::new_err(format!("MATCH query failed: {e}")))?
            } else {
                self.inner
                    .execute_match(match_clause, &rust_params)
                    .map_err(|e| PyRuntimeError::new_err(format!("MATCH query failed: {e}")))?
            };

            // 5. Convert MatchResult to Python dicts
            Ok(results
                .into_iter()
                .map(|r| match_result_to_dict(py, r))
                .collect())
        })
    }

    // ========================================================================
    // Query Plan / EXPLAIN (Phase 4.3 Plan 02)
    // ========================================================================

    /// Explain a VelesQL query without executing it.
    ///
    /// Returns the query plan with estimated costs and detected features.
    ///
    /// Args:
    ///     query_str: VelesQL query string to explain
    ///
    /// Returns:
    ///     Dict with keys: query_type, plan, estimated_cost_ms,
    ///     index_used, filter_strategy
    ///
    /// Example:
    ///     >>> plan = collection.explain("SELECT * FROM docs WHERE vector NEAR $v LIMIT 10")
    ///     >>> print(plan['estimated_cost_ms'])
    #[pyo3(signature = (query_str))]
    fn explain(&self, query_str: &str) -> PyResult<HashMap<String, PyObject>> {
        Python::with_gil(|py| {
            let parsed = velesdb_core::velesql::Parser::parse(query_str).map_err(|e| {
                PyValueError::new_err(format!("VelesQL parse error: {}", e.message))
            })?;

            let (query_plan, query_type) = if let Some(ref match_clause) = parsed.match_clause {
                // MATCH query: use from_match with basic stats
                use velesdb_core::collection::search::query::match_planner::CollectionStats;
                let stats = CollectionStats {
                    total_nodes: self.inner.config().point_count,
                    ..CollectionStats::default()
                };
                let plan = velesdb_core::velesql::QueryPlan::from_match(match_clause, &stats);
                (plan, "MATCH")
            } else {
                // SELECT query: use from_select
                let plan = velesdb_core::velesql::QueryPlan::from_select(&parsed.select);
                (plan, "SELECT")
            };

            // Serialize QueryPlan to JSON, then convert to Python dict
            let plan_json = serde_json::to_value(&query_plan).map_err(|e| {
                PyRuntimeError::new_err(format!("Failed to serialize query plan: {e}"))
            })?;

            let mut result = HashMap::new();
            result.insert("query_type".to_string(), to_pyobject(py, query_type));
            result.insert(
                "plan".to_string(),
                crate::utils::json_to_python(py, &plan_json),
            );
            result.insert(
                "estimated_cost_ms".to_string(),
                to_pyobject(py, query_plan.estimated_cost_ms),
            );
            result.insert(
                "index_used".to_string(),
                match query_plan.index_used {
                    Some(idx) => to_pyobject(py, format!("{idx:?}")),
                    None => py.None(),
                },
            );
            result.insert(
                "filter_strategy".to_string(),
                to_pyobject(py, format!("{:?}", query_plan.filter_strategy)),
            );

            Ok(result)
        })
    }
}
