//! VelesQL query, match, explain, and explain-analyze methods for Collection.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyString};
use std::collections::HashMap;

use velesdb_core::QueryOperationKind;

use crate::collection_helpers::{core_err, id_score_pairs_to_dicts};
use crate::utils::{extract_vector, json_to_python, python_to_json};

use super::{deny_if_scoped, Collection};

/// Convert a `MatchResult` to a Python dict.
///
/// Extracts node_id, depth, path, bindings, edge_bindings, edge_paths, score,
/// and projected fields into a flat Python dict with interned keys. The two
/// edge maps are core's full MATCH result: `edge_bindings` holds fixed-length
/// relationship aliases, `edge_paths` holds the ordered edge-id list of
/// variable-length (`-[r*1..3]->`) aliases (openCypher list semantics).
pub(crate) fn match_result_to_dict(
    py: Python<'_>,
    r: velesdb_core::collection::search::query::match_exec::MatchResult,
) -> PyResult<Py<PyAny>> {
    let dict = PyDict::new(py);
    dict.set_item(PyString::intern(py, "node_id"), r.node_id)?;
    dict.set_item(PyString::intern(py, "depth"), r.depth)?;
    dict.set_item(PyString::intern(py, "path"), r.path)?;
    dict.set_item(PyString::intern(py, "bindings"), r.bindings)?;
    dict.set_item(PyString::intern(py, "edge_bindings"), r.edge_bindings)?;
    dict.set_item(PyString::intern(py, "edge_paths"), r.edge_paths)?;
    dict.set_item(PyString::intern(py, "score"), r.score)?;
    let projected = PyDict::new(py);
    for (k, v) in r.projected {
        projected.set_item(k, json_to_python(py, &v)?)?;
    }
    dict.set_item(PyString::intern(py, "projected"), projected)?;
    Ok(dict.into_any().unbind())
}

/// Returns `query` retargeted so its FROM (top-level select and every UNION /
/// INTERSECT / EXCEPT operand) is `collection_name`.
///
/// The collection-level VelesQL methods historically executed on the detached
/// collection leaf, which always ran against itself: the FROM name was purely
/// nominal (the LangChain / LlamaIndex adapters rely on this, issuing e.g.
/// ``SELECT * FROM vectors`` against a collection with another name). Routing
/// through the gated `Database` facade (audit F-5.4, #1392) resolves the
/// collection from the FROM clause instead, so the FROM must be rewritten to
/// the wrapping collection's name — this preserves the leaf contract exactly
/// (with and without an observer) and keys the control-plane read gate on the
/// collection actually read, so gate and data plane can never disagree.
/// Borrowed (zero clone) when the query already targets `collection_name`.
pub(crate) fn retarget_to_collection<'q>(
    query: &'q velesdb_core::velesql::Query,
    collection_name: &str,
) -> std::borrow::Cow<'q, velesdb_core::velesql::Query> {
    let compound_retargeted = |q: &velesdb_core::velesql::Query| {
        q.compound
            .as_ref()
            .is_none_or(|c| c.operations.iter().all(|(_, s)| s.from == collection_name))
    };
    if query.select.from == collection_name && compound_retargeted(query) {
        return std::borrow::Cow::Borrowed(query);
    }
    let mut owned = query.clone();
    owned.select.from = collection_name.to_string();
    if let Some(compound) = owned.compound.as_mut() {
        for (_, operand) in &mut compound.operations {
            operand.from = collection_name.to_string();
        }
    }
    std::borrow::Cow::Owned(owned)
}

/// Parses a VelesQL string into an AST, mapping parse errors to `PyValueError`.
pub(crate) fn parse_velesql(query_str: &str) -> PyResult<velesdb_core::velesql::Query> {
    velesdb_core::velesql::Parser::parse(query_str).map_err(|e| {
        PyValueError::new_err(format!(
            "VelesQL parse error at position {}: {}",
            e.position, e.message
        ))
    })
}

/// Validates a parsed VelesQL query, mapping validation failures to a typed
/// `ValueError` so EXPLAIN rejects semantically-invalid queries instead of
/// silently building a plan (mirrors `Database::explain_analyze_query`).
pub(crate) fn validate_query(parsed: &velesdb_core::velesql::Query) -> PyResult<()> {
    velesdb_core::velesql::QueryValidator::validate(parsed)
        .map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Writes the four plan keys every EXPLAIN dict carries — `tree`,
/// `estimated_cost_ms`, `filter_strategy`, `index_used` — into `dict`.
fn set_plan_items(
    py: Python<'_>,
    dict: &Bound<'_, PyDict>,
    plan: &velesdb_core::velesql::QueryPlan,
) -> PyResult<()> {
    dict.set_item(PyString::intern(py, "tree"), plan.to_tree())?;
    dict.set_item(
        PyString::intern(py, "estimated_cost_ms"),
        plan.estimated_cost_ms,
    )?;
    dict.set_item(
        PyString::intern(py, "filter_strategy"),
        plan.filter_strategy.as_str(),
    )?;
    dict.set_item(
        PyString::intern(py, "index_used"),
        plan.index_used.map(|i| i.as_str().to_string()),
    )
}

/// Builds an EXPLAIN dict from a parsed VelesQL query, threading the live
/// indexed-field set and calibrated collection statistics so the plan emits
/// `IndexLookup` nodes and a calibrated cost for SELECT, and a real traversal
/// strategy for MATCH (backlog #14) — mirroring `Database::explain_query`.
pub(crate) fn build_explain_dict(
    py: Python<'_>,
    parsed: &velesdb_core::velesql::Query,
    indexed_fields: &std::collections::HashSet<String>,
    stats: Option<&velesdb_core::collection::stats::CollectionStats>,
) -> PyResult<Py<PyAny>> {
    let plan = velesdb_core::velesql::QueryPlan::from_query_with_all_stats(
        parsed,
        indexed_fields,
        stats,
        None,
    );

    let dict = PyDict::new(py);
    set_plan_items(py, &dict, &plan)?;
    Ok(dict.into_any().unbind())
}

/// Convert a core `QueryPlan` (as returned by the database-level
/// `explain_query_with_params`) to a Python dict, including the compiled-plan
/// cache fields `cache_hit` / `plan_reuse_count` that the collection-level
/// leaf path cannot see.
pub(crate) fn plan_to_dict(
    py: Python<'_>,
    plan: &velesdb_core::velesql::QueryPlan,
) -> PyResult<Py<PyAny>> {
    let dict = PyDict::new(py);
    set_plan_items(py, &dict, plan)?;
    dict.set_item(PyString::intern(py, "cache_hit"), plan.cache_hit)?;
    dict.set_item(
        PyString::intern(py, "plan_reuse_count"),
        plan.plan_reuse_count,
    )?;
    Ok(dict.into_any().unbind())
}

/// Builds an EXPLAIN ANALYZE dict from an `ExplainOutput`.
///
/// Returns a Python dict with keys: `plan`, `actual_stats`, `node_stats`.
/// The `plan` sub-dict mirrors `build_explain_dict` output.
/// `actual_stats` is a dict with measured execution metrics (or `None`).
/// `node_stats` is a list of per-node **estimated** statistic dicts
/// (heuristic values derived from `actual_stats`, not individually measured).
pub(crate) fn build_explain_analyze_dict(
    py: Python<'_>,
    output: &velesdb_core::velesql::ExplainOutput,
) -> PyResult<Py<PyAny>> {
    let dict = PyDict::new(py);

    // Plan sub-dict (same keys as build_explain_dict)
    let plan_dict = PyDict::new(py);
    set_plan_items(py, &plan_dict, &output.plan)?;
    dict.set_item(PyString::intern(py, "plan"), plan_dict)?;

    dict.set_item(
        PyString::intern(py, "actual_stats"),
        output
            .actual_stats
            .as_ref()
            .map(|stats| actual_stats_dict(py, stats))
            .transpose()?,
    )?;
    let node_dicts = output
        .node_stats
        .iter()
        .map(|ns| node_stats_dict(py, ns))
        .collect::<PyResult<Vec<_>>>()?;
    dict.set_item(
        PyString::intern(py, "node_stats"),
        PyList::new(py, &node_dicts)?,
    )?;
    dict.set_item(
        PyString::intern(py, "cost_factors"),
        output
            .cost_factors
            .as_ref()
            .map(|factors| cost_factors_dict(py, factors))
            .transpose()?,
    )?;
    dict.set_item(
        PyString::intern(py, "calibration_source"),
        output
            .calibration_source
            .as_deref()
            .map(|s| PyString::intern(py, s)),
    )?;

    Ok(dict.into_any().unbind())
}

/// The measured execution metrics of an EXPLAIN ANALYZE.
fn actual_stats_dict<'py>(
    py: Python<'py>,
    stats: &velesdb_core::velesql::ActualStats,
) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item(PyString::intern(py, "actual_rows"), stats.actual_rows)?;
    d.set_item(PyString::intern(py, "actual_time_ms"), stats.actual_time_ms)?;
    d.set_item(PyString::intern(py, "loops"), stats.loops)?;
    d.set_item(PyString::intern(py, "nodes_visited"), stats.nodes_visited)?;
    d.set_item(
        PyString::intern(py, "edges_traversed"),
        stats.edges_traversed,
    )?;
    Ok(d)
}

/// One plan node's (estimated) statistics.
fn node_stats_dict<'py>(
    py: Python<'py>,
    ns: &velesdb_core::velesql::NodeStats,
) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item(PyString::intern(py, "node_label"), &ns.node_label)?;
    d.set_item(PyString::intern(py, "actual_time_ms"), ns.actual_time_ms)?;
    d.set_item(PyString::intern(py, "actual_rows_in"), ns.actual_rows_in)?;
    d.set_item(PyString::intern(py, "actual_rows_out"), ns.actual_rows_out)?;
    d.set_item(PyString::intern(py, "loops"), ns.loops)?;
    d.set_item(PyString::intern(py, "estimated"), ns.estimated)?;
    Ok(d)
}

/// The cost-model factors the plan was costed with.
fn cost_factors_dict<'py>(
    py: Python<'py>,
    factors: &velesdb_core::collection::query_cost::OperationCostFactors,
) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item(PyString::intern(py, "seq_page_cost"), factors.seq_page_cost)?;
    d.set_item(
        PyString::intern(py, "random_page_cost"),
        factors.random_page_cost,
    )?;
    d.set_item(
        PyString::intern(py, "cpu_tuple_cost"),
        factors.cpu_tuple_cost,
    )?;
    d.set_item(
        PyString::intern(py, "cpu_index_cost"),
        factors.cpu_index_cost,
    )?;
    d.set_item(
        PyString::intern(py, "cpu_distance_cost"),
        factors.cpu_distance_cost,
    )?;
    d.set_item(PyString::intern(py, "cpu_edge_cost"), factors.cpu_edge_cost)?;
    Ok(d)
}

/// Converts `Vec<SearchResult>` to id/score dicts.
pub(crate) fn search_results_to_id_score(
    py: Python<'_>,
    results: Vec<velesdb_core::SearchResult>,
) -> PyResult<Vec<Py<PyAny>>> {
    let tuples: Vec<(u64, f32)> = results.into_iter().map(|r| (r.point.id, r.score)).collect();
    id_score_pairs_to_dicts(py, tuples)
}

/// Converts Python params dict to Rust `HashMap<String, serde_json::Value>`.
pub(crate) fn convert_params(
    py: Python<'_>,
    params: Option<HashMap<String, Py<PyAny>>>,
) -> PyResult<HashMap<String, serde_json::Value>> {
    params
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| python_to_json(py, &v).map(|json_val| (k, json_val)))
        .collect()
}

// ---------------------------------------------------------------------------
// Shared VelesQL execution helpers (used by Collection + PyGraphCollection)
// ---------------------------------------------------------------------------

/// The executor a collection-scoped VelesQL read hands its parsed,
/// read-only statement to.
pub(crate) trait VelesqlReadExecutor:
    FnOnce(
        &velesdb_core::velesql::Query,
        &HashMap<String, serde_json::Value>,
    ) -> velesdb_core::error::Result<Vec<velesdb_core::SearchResult>>
    + Send
{
}

impl<F> VelesqlReadExecutor for F where
    F: FnOnce(
            &velesdb_core::velesql::Query,
            &HashMap<String, serde_json::Value>,
        ) -> velesdb_core::error::Result<Vec<velesdb_core::SearchResult>>
        + Send
{
}

/// Parses `query_str` and refuses any statement that writes.
///
/// The collection-scoped methods that execute a statement (`query`,
/// `query_ids` and `explain_analyze` on `Collection` and `GraphCollection`)
/// retarget a SELECT's `FROM` to their
/// own collection, but a DDL or DML statement names its own target: run as
/// is, `docs.query("DROP COLLECTION other")` would drop `other`. Those belong
/// to `Database.execute_query`, so only what core's
/// [`Query::is_read_only`](velesdb_core::velesql::Query::is_read_only) admits
/// passes here.
pub(crate) fn parse_read_only_velesql(query_str: &str) -> PyResult<velesdb_core::velesql::Query> {
    let parsed = parse_velesql(query_str)?;
    if parsed.is_read_only() {
        Ok(parsed)
    } else {
        Err(PyValueError::new_err(
            "a collection's query methods run read-only VelesQL only (SELECT, \
             MATCH, SELECT EDGES, SHOW, DESCRIBE, EXPLAIN); run DDL and DML \
             through Database.execute_query",
        ))
    }
}

/// Parses a read-only statement (see [`parse_read_only_velesql`]), converts
/// `params` and runs `execute` off the GIL.
fn run_velesql_read(
    py: Python<'_>,
    query_str: &str,
    params: Option<HashMap<String, Py<PyAny>>>,
    execute: impl VelesqlReadExecutor,
) -> PyResult<Vec<velesdb_core::SearchResult>> {
    let parsed = parse_read_only_velesql(query_str)?;
    let rust_params = convert_params(py, params)?;
    py.detach(move || execute(&parsed, &rust_params).map_err(core_err))
}

/// Execute a read-only VelesQL query and convert results to multimodel dicts.
pub(crate) fn run_velesql_select(
    py: Python<'_>,
    query_str: &str,
    params: Option<HashMap<String, Py<PyAny>>>,
    execute: impl VelesqlReadExecutor,
) -> PyResult<Vec<Py<PyAny>>> {
    let results = run_velesql_read(py, query_str, params, execute)?;
    crate::collection_helpers::search_results_to_multimodel_dicts(py, results)
}

/// Execute a read-only VelesQL query and return only IDs and scores.
pub(crate) fn run_velesql_select_ids(
    py: Python<'_>,
    query_str: &str,
    params: Option<HashMap<String, Py<PyAny>>>,
    execute: impl VelesqlReadExecutor,
) -> PyResult<Vec<Py<PyAny>>> {
    let results = run_velesql_read(py, query_str, params, execute)?;
    search_results_to_id_score(py, results)
}

/// Execute a VelesQL MATCH query and convert results to dicts.
pub(crate) fn run_velesql_match<F>(
    py: Python<'_>,
    query_str: &str,
    params: Option<HashMap<String, Py<PyAny>>>,
    vector: Option<Py<PyAny>>,
    execute: F,
) -> PyResult<Vec<Py<PyAny>>>
where
    F: FnOnce(
            velesdb_core::velesql::MatchClause,
            HashMap<String, serde_json::Value>,
            Option<Vec<f32>>,
        ) -> velesdb_core::error::Result<
            Vec<velesdb_core::collection::search::query::match_exec::MatchResult>,
        > + Send,
{
    let parsed = parse_velesql(query_str)?;
    let match_clause = parsed
        .match_clause
        .as_ref()
        .ok_or_else(|| PyValueError::new_err("Query is not a MATCH query"))?
        .clone();
    let rust_params = convert_params(py, params)?;
    let query_vector = vector.map(|v| extract_vector(py, &v)).transpose()?;
    let results =
        py.detach(move || execute(match_clause, rust_params, query_vector).map_err(core_err))?;
    results
        .into_iter()
        .map(|r| match_result_to_dict(py, r))
        .collect()
}

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
    /// Raises:
    ///     ValueError: If the statement writes (DDL, DML, ``TRAIN``,
    ///         ``FLUSH``). A collection's query methods run reads only: a
    ///         write names its own target, so it goes through
    ///         ``Database.execute_query``.
    ///
    /// Example:
    ///     >>> results = collection.query(
    ///     ...     "SELECT * FROM docs WHERE vector NEAR $q LIMIT 20",
    ///     ...     params={"q": query_embedding}
    ///     ... )
    #[pyo3(signature = (query_str, params=None))]
    fn query(
        &self,
        py: Python<'_>,
        query_str: &str,
        params: Option<HashMap<String, Py<PyAny>>>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        // Execute through the owning database (not the detached collection
        // leaf) so the VelesQL read path passes the control-plane observer
        // gate — the detached leaf has no observer reference and would bypass
        // governance (audit F-5.4, #1392; mirrors the mobile fix, PR #1404).
        // The query is retargeted to this collection first: the leaf always
        // executed against itself (FROM was nominal — adapters rely on it),
        // and the retarget keys the gate on the collection actually read.
        run_velesql_select(py, query_str, params, |q, p| {
            self.db
                .execute_query(&retarget_to_collection(q, &self.name), p)
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
    /// The plan is calibrated with this collection's live indexed-field set
    /// and statistics, so a SELECT whose WHERE targets an indexed field shows
    /// an `IndexLookup` node and a MATCH reports a real traversal strategy.
    /// Semantically invalid queries raise `ValueError`.
    ///
    /// The read gate decides a plan-only EXPLAIN too: `analyze()` walks the
    /// column store to calibrate the plan, so a denied read must refuse and a
    /// scope — no rows to narrow — refuses rather than leak the statistics
    /// (`explain_analyze`, which *executes* the query, is gated the same way).
    #[pyo3(signature = (query_str))]
    fn explain(&self, py: Python<'_>, query_str: &str) -> PyResult<Py<PyAny>> {
        let parsed = parse_velesql(query_str)?;
        // The plan is built from this collection's stats AND names the query's
        // FROM — gate both: `coll.explain("SELECT * FROM denied")` must not
        // produce a plan for a collection the caller cannot read.
        let scope = self.authorize(QueryOperationKind::Select, None, None)?;
        deny_if_scoped(scope, "explain")?;
        if !parsed.select.from.is_empty() && parsed.select.from != self.name {
            let scope = self
                .db
                .authorize_read(&parsed.select.from, QueryOperationKind::Select, None, None)
                .map_err(core_err)?;
            deny_if_scoped(scope, "explain")?;
        }
        validate_query(&parsed)?;
        let indexed = self.inner.config().indexed_fields.iter().cloned().collect();
        // `analyze()` computes fresh statistics (cardinality, size histograms)
        // by walking the column store — the same work as `Database.analyze_collection`,
        // which crosses the 1-second mark on large collections. Release the GIL
        // so other Python threads keep running while the plan is calibrated.
        let stats = py.detach(|| self.inner.analyze().ok());
        build_explain_dict(py, &parsed, &indexed, stats.as_ref())
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
    ///
    /// Raises:
    ///     ValueError: If the statement writes (DDL, DML, ``TRAIN``,
    ///         ``FLUSH``). A collection's query methods run reads only: a
    ///         write names its own target, so it goes through
    ///         ``Database.execute_query``.
    #[pyo3(signature = (query_str, params=None))]
    fn explain_analyze(
        &self,
        py: Python<'_>,
        query_str: &str,
        params: Option<HashMap<String, Py<PyAny>>>,
    ) -> PyResult<Py<PyAny>> {
        let parsed = parse_read_only_velesql(query_str)?;
        let rust_params = convert_params(py, params)?;
        // EXPLAIN ANALYZE *executes* the query, so it must pass the same
        // control-plane read gate as `query()`. The `Database` facade twin is
        // gated internally — deny fails closed, and a scope is refused rather
        // than narrowed: the plan step's calibrated statistics cannot be
        // row-narrowed (VELES-009), unlike the detached leaf method used
        // before (#1392).
        let output = py.detach(move || {
            self.db
                .explain_analyze_query(&retarget_to_collection(&parsed, &self.name), &rust_params)
                .map_err(core_err)
        })?;
        build_explain_analyze_dict(py, &output)
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
    /// Raises:
    ///     ValueError: If the statement writes (DDL, DML, ``TRAIN``,
    ///         ``FLUSH``). A collection's query methods run reads only: a
    ///         write names its own target, so it goes through
    ///         ``Database.execute_query``.
    ///
    /// Example:
    ///     >>> ids = collection.query_ids("SELECT * FROM docs WHERE price > 100 LIMIT 5")
    #[pyo3(signature = (velesql, params = None))]
    fn query_ids(
        &self,
        py: Python<'_>,
        velesql: &str,
        params: Option<HashMap<String, Py<PyAny>>>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        // Gated facade routing, same rationale as `query()`. A scope filter is
        // AND-composed into the query AST inside core before execution, so the
        // ids projected here are already narrowed — no unscoped id can leak.
        run_velesql_select_ids(py, velesql, params, |q, p| {
            self.db
                .execute_query(&retarget_to_collection(q, &self.name), p)
        })
    }
}
