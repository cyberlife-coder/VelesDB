//! VelesQL query methods for `PyGraphCollection`, extracted from `graph_collection.rs`.
//!
//! Contains the `#[pymethods]` impl block for VelesQL parity: `query`,
//! `match_query`, `explain`, and `query_ids`.

use pyo3::prelude::*;
use std::collections::HashMap;

use crate::collection::query::{
    build_explain_dict, parse_velesql, run_velesql_match, run_velesql_select,
    run_velesql_select_ids,
};
use crate::graph_collection::PyGraphCollection;

#[pymethods]
impl PyGraphCollection {
    /// Execute a VelesQL query (SELECT or MATCH).
    ///
    /// Args:
    ///     query_str: VelesQL query string
    ///     params: Query parameters (vectors as lists/numpy arrays, scalars)
    ///
    /// Returns:
    ///     List of result dicts
    ///
    /// Example:
    ///     >>> results = graph.query(
    ///     ...     "SELECT * FROM kg WHERE category = 'person' LIMIT 10"
    ///     ... )
    #[pyo3(signature = (query_str, params=None))]
    fn query(
        &self,
        py: Python<'_>,
        query_str: &str,
        params: Option<HashMap<String, PyObject>>,
    ) -> PyResult<Vec<PyObject>> {
        let inner = &self.inner;
        run_velesql_select(py, query_str, params, |q, p| inner.execute_query(q, p))
    }

    /// Execute a MATCH graph traversal query.
    ///
    /// This is the primary method for Cypher-like graph pattern matching
    /// in VelesQL. Edges added via `add_edge()` are found by this method.
    ///
    /// Args:
    ///     query_str: VelesQL MATCH query string
    ///     params: Query parameters (default: empty dict)
    ///     vector: Optional query vector for similarity scoring
    ///     threshold: Similarity threshold (default: 0.0)
    ///
    /// Returns:
    ///     List of dicts with keys: node_id, depth, path, bindings, score, projected
    ///
    /// Example:
    ///     >>> results = graph.match_query(
    ///     ...     "MATCH (a:Person)-[:KNOWS]->(b) RETURN a.name, b.name LIMIT 10"
    ///     ... )
    #[pyo3(signature = (query_str, params = None, vector = None, threshold = 0.0))]
    fn match_query(
        &self,
        py: Python<'_>,
        query_str: &str,
        params: Option<HashMap<String, PyObject>>,
        vector: Option<PyObject>,
        threshold: f32,
    ) -> PyResult<Vec<PyObject>> {
        let inner = &self.inner;
        run_velesql_match(py, query_str, params, vector, move |mc, p, qv| {
            if let Some(ref qv) = qv {
                inner.execute_match_with_similarity(&mc, qv, threshold, &p)
            } else {
                inner.execute_match(&mc, &p)
            }
        })
    }

    /// Return query execution plan (EXPLAIN).
    ///
    /// Args:
    ///     query_str: VelesQL query string
    ///
    /// Returns:
    ///     Dict with tree, estimated_cost_ms, filter_strategy, index_used
    #[pyo3(signature = (query_str))]
    fn explain(&self, py: Python<'_>, query_str: &str) -> PyResult<PyObject> {
        let parsed = parse_velesql(query_str)?;
        Ok(build_explain_dict(py, &parsed))
    }

    /// Execute a VelesQL query returning only IDs and scores (no payload).
    ///
    /// Args:
    ///     velesql: VelesQL query string
    ///     params: Optional dict of query parameters
    ///
    /// Returns:
    ///     List of dicts with 'id' and 'score' fields
    #[pyo3(signature = (velesql, params = None))]
    fn query_ids(
        &self,
        py: Python<'_>,
        velesql: &str,
        params: Option<HashMap<String, PyObject>>,
    ) -> PyResult<Vec<PyObject>> {
        let inner = &self.inner;
        run_velesql_select_ids(py, velesql, params, |q, p| inner.execute_query(q, p))
    }
}
