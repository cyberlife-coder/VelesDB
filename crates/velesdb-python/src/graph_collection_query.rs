//! VelesQL query methods for `PyGraphCollection`, extracted from `graph_collection.rs`.
//!
//! Contains the `#[pymethods]` impl block for VelesQL parity: `query`,
//! `match_query`, `explain`, `explain_analyze`, and `query_ids`.

use pyo3::prelude::*;
use std::collections::HashMap;

use crate::collection::query::{
    build_explain_analyze_dict, build_explain_dict, convert_params, parse_velesql,
    retarget_to_collection, run_velesql_match, run_velesql_select, run_velesql_select_ids,
    validate_query,
};
use crate::collection_helpers::core_err;
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
    /// # Cross-Collection MATCH
    ///
    /// For MATCH queries that reference nodes from other collections via
    /// the ``@collection`` annotation, pass ``_collection`` in ``params``
    /// to specify the primary collection (the one with graph edges).
    /// Annotated nodes will have their payloads enriched from the named
    /// collection after traversal.
    ///
    /// Example:
    ///     >>> results = graph.query(
    ///     ...     "SELECT * FROM kg WHERE category = 'person' LIMIT 10"
    ///     ... )
    ///     >>> # Cross-collection MATCH
    ///     >>> results = graph.query(
    ///     ...     "MATCH (p:Product)-[:IN]->(c:Category@categories) "
    ///     ...     "RETURN p.name, c.label LIMIT 20",
    ///     ...     params={"_collection": "product_graph"},
    ///     ... )
    #[pyo3(signature = (query_str, params=None))]
    fn query(
        &self,
        py: Python<'_>,
        query_str: &str,
        params: Option<HashMap<String, Py<PyAny>>>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        // Execute through the owning database (not the detached graph leaf) so
        // the VelesQL read path passes the control-plane observer gate (audit
        // F-5.4, #1392; mirrors `Collection.query`). A bare MATCH inherits
        // this collection's name, preserving the leaf semantics (the leaf
        // always executed against itself, so `params["_collection"]` never
        // re-targeted this method) while keying the gate on the collection
        // actually read. `@collection` payload enrichment applies at the
        // facade as it does for `Database.execute_query`.
        run_velesql_select(py, query_str, params, |q, p| {
            self.db
                .execute_query(&retarget_to_collection(q, &self.name), p)
        })
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
        params: Option<HashMap<String, Py<PyAny>>>,
        vector: Option<Py<PyAny>>,
        threshold: f32,
    ) -> PyResult<Vec<Py<PyAny>>> {
        // Through the owning database, as REST `/match` runs: the read gate
        // on this collection and on every `@collection` the clause names
        // (a scope is refused: a MATCH row carries no payload to narrow),
        // the annotated nodes' properties, `search.max_results`, and the
        // observer's `on_query`.
        let db = &self.db;
        let name = &self.name;
        run_velesql_match(py, query_str, params, vector, move |mc, p, qv| {
            let scoring = qv
                .as_deref()
                .map(|vector| velesdb_core::MatchScoring { vector, threshold });
            db.match_query(name, &mc, &p, scoring)
        })
    }

    /// Return query execution plan (EXPLAIN).
    ///
    /// Args:
    ///     query_str: VelesQL query string
    ///
    /// Returns:
    ///     Dict with tree, estimated_cost_ms, filter_strategy, index_used
    ///
    /// Deliberately NOT routed through the read gate: EXPLAIN builds a plan
    /// from the AST and never touches point data, so there is nothing for a
    /// governance observer to scope or leak (`explain_analyze` is gated).
    #[pyo3(signature = (query_str))]
    fn explain(&self, py: Python<'_>, query_str: &str) -> PyResult<Py<PyAny>> {
        let parsed = parse_velesql(query_str)?;
        validate_query(&parsed)?;
        // GraphCollection does not expose the calibrated stats / indexed-field
        // set publicly; MATCH still reports a real strategy via from_match's
        // default graph stats.
        let indexed = std::collections::HashSet::new();
        Ok(build_explain_dict(py, &parsed, &indexed, None))
    }

    /// Execute a query with instrumentation and return plan + actual stats (EXPLAIN ANALYZE).
    ///
    /// Unlike `explain()` (plan only), this method executes the query and
    /// measures actual execution statistics.
    ///
    /// Args:
    ///     query_str: VelesQL query string
    ///     params: Optional query parameters (vectors as lists/numpy arrays, scalars)
    ///
    /// Returns:
    ///     Dict with keys: plan, actual_stats, node_stats
    #[pyo3(signature = (query_str, params=None))]
    fn explain_analyze(
        &self,
        py: Python<'_>,
        query_str: &str,
        params: Option<HashMap<String, Py<PyAny>>>,
    ) -> PyResult<Py<PyAny>> {
        let parsed = parse_velesql(query_str)?;
        let rust_params = convert_params(py, params)?;
        // EXPLAIN ANALYZE *executes* the query — route through the gated
        // `Database` facade twin (deny fails closed, and a scope is refused:
        // the plan step's calibrated statistics cannot be row-narrowed,
        // VELES-009), mirroring `Collection.explain_analyze`.
        let output = py.detach(move || {
            self.db
                .explain_analyze_query(&retarget_to_collection(&parsed, &self.name), &rust_params)
                .map_err(core_err)
        })?;
        Ok(build_explain_analyze_dict(py, &output))
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
        params: Option<HashMap<String, Py<PyAny>>>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        // Gated facade routing, same rationale as `query()`; a scope filter is
        // AND-composed into the AST in core, so projected ids are pre-narrowed.
        run_velesql_select_ids(py, velesql, params, |q, p| {
            self.db
                .execute_query(&retarget_to_collection(q, &self.name), p)
        })
    }
}
