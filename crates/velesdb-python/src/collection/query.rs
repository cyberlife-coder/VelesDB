//! VelesQL query, match, and explain methods for Collection.

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::collections::HashMap;

use crate::utils::{extract_vector, json_to_python, python_to_json, to_pyobject};

use super::Collection;

#[pymethods]
impl Collection {
    /// Execute a VelesQL query (EPIC-031 US-008).
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

            let rust_params: HashMap<String, serde_json::Value> = params
                .unwrap_or_default()
                .into_iter()
                .filter_map(|(k, v)| python_to_json(py, &v).map(|json_val| (k, json_val)))
                .collect();

            let results = self
                .inner
                .execute_query(&parsed, &rust_params)
                .map_err(|e| PyRuntimeError::new_err(format!("Query failed: {e}")))?;

            Ok(crate::collection_helpers::search_results_to_multimodel_dicts(py, results))
        })
    }

    /// Execute a MATCH graph traversal query.
    ///
    /// Args:
    ///     query_str: VelesQL MATCH query string
    ///     params: Query parameters (default: empty dict)
    ///     vector: Optional query vector for similarity scoring
    ///     threshold: Similarity threshold (default: 0.0)
    ///
    /// Returns:
    ///     List of dicts with keys: node_id, depth, path, bindings, score, projected
    #[pyo3(signature = (query_str, params = None, vector = None, threshold = 0.0))]
    fn match_query(
        &self,
        query_str: &str,
        params: Option<HashMap<String, PyObject>>,
        vector: Option<PyObject>,
        threshold: f32,
    ) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            let parsed = velesdb_core::velesql::Parser::parse(query_str).map_err(|e| {
                PyValueError::new_err(format!("VelesQL parse error: {}", e.message))
            })?;
            let match_clause = parsed
                .match_clause
                .as_ref()
                .ok_or_else(|| PyValueError::new_err("Query is not a MATCH query"))?;

            let rust_params: HashMap<String, serde_json::Value> = params
                .unwrap_or_default()
                .into_iter()
                .filter_map(|(k, v)| python_to_json(py, &v).map(|json_val| (k, json_val)))
                .collect();

            let results = if let Some(vector_obj) = vector {
                let query_vector = extract_vector(py, &vector_obj)?;
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

            let py_results: Vec<HashMap<String, PyObject>> = results
                .into_iter()
                .map(|r| {
                    let mut dict = HashMap::new();
                    dict.insert("node_id".to_string(), to_pyobject(py, r.node_id));
                    dict.insert("depth".to_string(), to_pyobject(py, r.depth));
                    dict.insert("path".to_string(), to_pyobject(py, r.path));
                    dict.insert("bindings".to_string(), to_pyobject(py, r.bindings));
                    dict.insert("score".to_string(), to_pyobject(py, r.score));
                    let projected: HashMap<String, PyObject> = r
                        .projected
                        .into_iter()
                        .map(|(k, v)| (k, json_to_python(py, &v)))
                        .collect();
                    dict.insert("projected".to_string(), to_pyobject(py, projected));
                    dict
                })
                .collect();
            Ok(py_results)
        })
    }

    /// Return query execution plan (EXPLAIN).
    #[pyo3(signature = (query_str))]
    fn explain(&self, query_str: &str) -> PyResult<HashMap<String, PyObject>> {
        Python::with_gil(|py| {
            let parsed = velesdb_core::velesql::Parser::parse(query_str).map_err(|e| {
                PyValueError::new_err(format!("VelesQL parse error: {}", e.message))
            })?;

            let plan = if let Some(match_clause) = parsed.match_clause.as_ref() {
                let stats = velesdb_core::collection::search::query::match_planner::CollectionStats::default();
                velesdb_core::velesql::QueryPlan::from_match(match_clause, &stats)
            } else {
                velesdb_core::velesql::QueryPlan::from_select(&parsed.select)
            };

            let mut out = HashMap::new();
            out.insert("tree".to_string(), to_pyobject(py, plan.to_tree()));
            out.insert(
                "estimated_cost_ms".to_string(),
                to_pyobject(py, plan.estimated_cost_ms),
            );
            out.insert(
                "filter_strategy".to_string(),
                to_pyobject(py, plan.filter_strategy.as_str()),
            );
            out.insert(
                "index_used".to_string(),
                to_pyobject(py, plan.index_used.map(|i| i.as_str().to_string())),
            );
            Ok(out)
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

            let json_params: HashMap<String, serde_json::Value> = params
                .unwrap_or_default()
                .into_iter()
                .filter_map(|(k, v)| python_to_json(py, &v).map(|json_val| (k, json_val)))
                .collect();

            let results = self
                .inner
                .execute_query(&parsed_query, &json_params)
                .map_err(|e| PyRuntimeError::new_err(format!("Query execution failed: {e}")))?;

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
}
