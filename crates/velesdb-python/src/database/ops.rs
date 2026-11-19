//! Database inspection and ops methods (diagnostics, guardrails, explain,
//! training) — split out of `database.rs` to stay under the file budget.

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::sync::Arc;

use crate::collection_helpers::core_err;
use crate::utils;

use super::{validate_guardrail_keys, Database};
use velesdb_core::velesql::{Query, TrainStatement, WithValue};

#[pymethods]
impl Database {
    /// Get health diagnostics for a collection.
    ///
    /// Reports index readiness without relying on the REST server — useful
    /// for embedded health checks.
    ///
    /// Args:
    ///     name: Collection name
    ///
    /// Returns:
    ///     dict with keys ``has_vectors``, ``search_ready``,
    ///     ``dimension_configured``, ``point_count``, ``index_health``
    ///     ("healthy" | "empty" | "needs_rebuild"), and
    ///     ``index_health_detail`` (only when a rebuild is needed).
    ///
    /// Raises:
    ///     RuntimeError: If the collection does not exist.
    #[pyo3(signature = (name))]
    fn collection_diagnostics(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        use velesdb_core::collection::IndexHealth;
        let name_owned = name.to_string();
        let inner = Arc::clone(&self.inner);
        let diag = py
            .detach(move || inner.collection_diagnostics(&name_owned))
            .map_err(core_err)?;

        let dict = pyo3::types::PyDict::new(py);
        dict.set_item("has_vectors", diag.has_vectors)?;
        dict.set_item("search_ready", diag.search_ready)?;
        dict.set_item("dimension_configured", diag.dimension_configured)?;
        dict.set_item("point_count", diag.point_count)?;
        let (health, detail) = match &diag.index_health {
            IndexHealth::Healthy => ("healthy", None),
            IndexHealth::Empty => ("empty", None),
            IndexHealth::NeedsRebuild(reason) => ("needs_rebuild", Some(reason.clone())),
            _ => ("unknown", None),
        };
        dict.set_item("index_health", health)?;
        if let Some(detail) = detail {
            dict.set_item("index_health_detail", detail)?;
        }
        Ok(dict.into())
    }

    /// Update query guardrail limits for every collection in this database.
    ///
    /// This is a **full replacement**, not a partial patch (matching the
    /// Mobile/Tauri bindings): the dict must contain *all* guardrail fields.
    /// A missing or misspelled key raises ``ValueError`` rather than silently
    /// resetting limits to their defaults. To change one field, read the
    /// current limits first (``Collection.guard_rails()``), mutate, and submit
    /// the complete dict.
    ///
    /// Args:
    ///     limits: dict with exactly these keys: ``max_depth``,
    ///         ``max_cardinality``, ``memory_limit_bytes``, ``timeout_ms``,
    ///         ``rate_limit_qps``, ``circuit_failure_threshold``,
    ///         ``circuit_recovery_seconds``.
    ///
    /// Raises:
    ///     ValueError: If a field is missing, unknown, or has an invalid
    ///         value, or if ``rate_limit_qps`` or ``circuit_failure_threshold``
    ///         is ``0`` (core's ``QueryLimits::validate``; nothing changes).
    #[pyo3(signature = (limits))]
    fn update_guardrails(&self, py: Python<'_>, limits: Py<PyAny>) -> PyResult<()> {
        let json = utils::python_to_json(py, &limits)?;
        let obj = json
            .as_object()
            .ok_or_else(|| PyValueError::new_err("guardrails must be a dict"))?;
        validate_guardrail_keys(obj)?;
        let parsed: velesdb_core::guardrails::QueryLimits = serde_json::from_value(json)
            .map_err(|e| PyValueError::new_err(format!("invalid guardrails: {e}")))?;
        py.detach(|| {
            self.inner
                .update_guardrails_with(|live| live.clone_from(&parsed))
        })
        .map_err(core_err)?;
        Ok(())
    }

    /// The database directory path on disk.
    #[getter]
    fn data_dir(&self) -> String {
        self.path.display().to_string()
    }

    /// Schema version of the on-disk database format.
    fn schema_version(&self) -> u64 {
        self.inner.schema_version()
    }

    /// The live query guardrail limits of the whole database (the last value
    /// ``update_guardrails`` applied, or the defaults).
    ///
    /// Returns:
    ///     dict with keys ``max_depth``, ``max_cardinality``,
    ///     ``memory_limit_bytes``, ``timeout_ms``, ``rate_limit_qps``,
    ///     ``circuit_failure_threshold``, ``circuit_recovery_seconds``.
    fn guardrail_limits(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let limits = self.inner.guardrail_limits();
        let value = serde_json::to_value(&limits)
            .map_err(|e| PyRuntimeError::new_err(format!("guardrail serialization failed: {e}")))?;
        crate::utils::json_to_python(py, &value)
    }

    /// Flush every collection (vector, graph, metadata) to disk.
    ///
    /// Returns:
    ///     int: Number of collections whose flush failed (0 = all durable).
    fn flush_all(&self, py: Python<'_>) -> usize {
        py.detach(|| self.inner.flush_all())
    }

    /// Explain a VelesQL statement at database scope (plan only, no execution),
    /// through the same gate and compiled-plan cache as ``execute_query``.
    ///
    /// Unlike the collection-scoped ``Collection.explain``, this resolves JOIN
    /// targets and cross-collection MATCH from the database registry, and the
    /// returned plan carries the cache fields ``cache_hit`` /
    /// ``plan_reuse_count``.
    ///
    /// Args:
    ///     sql: VelesQL statement string.
    ///     params: Optional parameter bindings (bare names, e.g. ``{"q": [...]}``).
    ///
    /// Returns:
    ///     dict with keys ``tree``, ``estimated_cost_ms``, ``filter_strategy``,
    ///     ``index_used``, ``cache_hit``, ``plan_reuse_count``.
    #[pyo3(signature = (sql, params = None))]
    fn explain(
        &self,
        py: Python<'_>,
        sql: &str,
        params: Option<std::collections::HashMap<String, Py<PyAny>>>,
    ) -> PyResult<Py<PyAny>> {
        use crate::collection::query::{convert_params, parse_velesql, plan_to_dict};

        let parsed = parse_velesql(sql)?;
        let rust_params = convert_params(py, params)?;
        let inner = Arc::clone(&self.inner);
        let plan = py
            .detach(move || inner.explain_query_with_params(&parsed, &rust_params))
            .map_err(core_err)?;
        plan_to_dict(py, &plan)
    }

    /// Explain a VelesQL statement at database scope WITH execution, returning
    /// measured statistics (EXPLAIN ANALYZE). Database-level counterpart of
    /// ``Collection.explain_analyze``; runs the statement, so it is a read
    /// through the gate, not a free diagnostic.
    ///
    /// Args:
    ///     sql: VelesQL statement string.
    ///     params: Optional parameter bindings (bare names, e.g. ``{"q": [...]}``).
    ///
    /// Returns:
    ///     dict with keys ``plan``, ``actual_stats``, ``node_stats``,
    ///     ``cost_factors``, ``calibration_source``.
    #[pyo3(signature = (sql, params = None))]
    fn explain_analyze(
        &self,
        py: Python<'_>,
        sql: &str,
        params: Option<std::collections::HashMap<String, Py<PyAny>>>,
    ) -> PyResult<Py<PyAny>> {
        use crate::collection::query::{build_explain_analyze_dict, convert_params, parse_velesql};

        let parsed = parse_velesql(sql)?;
        let rust_params = convert_params(py, params)?;
        let inner = Arc::clone(&self.inner);
        let output = py
            .detach(move || inner.explain_analyze_query(&parsed, &rust_params))
            .map_err(core_err)?;
        build_explain_analyze_dict(py, &output)
    }

    /// Train a quantizer on a collection (typed counterpart of
    /// ``TRAIN QUANTIZER ON <c> WITH (...)``).
    ///
    /// Args:
    ///     collection_name: Name of the collection to train on.
    ///     quantizer: ``"pq"`` (default), ``"opq"``, ``"rabitq"`` or ``"sq8"``.
    ///     m: Number of subspaces for pq/opq (dimension must be divisible by m).
    ///     k: Number of centroids per subspace for pq/opq.
    ///     force: Retrain even if a quantizer is already installed.
    ///     sample_limit: Cap on the number of vectors used for training.
    ///
    /// Returns:
    ///     Status message from the training operation.
    #[allow(clippy::too_many_arguments)]
    // Reason: PyO3 maps each keyword argument of this Python method to one Rust parameter; grouping them would change the Python signature.
    #[pyo3(signature = (collection_name, quantizer="pq", m=8, k=256, force=false, sample_limit=None))]
    fn train_quantizer(
        &self,
        py: Python<'_>,
        collection_name: &str,
        quantizer: &str,
        m: usize,
        k: usize,
        force: bool,
        sample_limit: Option<usize>,
    ) -> PyResult<String> {
        let request = TrainRequest {
            collection: collection_name,
            quantizer,
            m,
            k,
            force,
            sample_limit,
        };
        let results = self.run_train(py, &request)?;
        Ok(format!(
            "{} training complete: {results} results",
            quantizer.to_ascii_lowercase()
        ))
    }
}

/// One `TRAIN QUANTIZER` run, as the Python training methods describe it.
pub(super) struct TrainRequest<'a> {
    pub(super) collection: &'a str,
    /// `pq`, `opq`, `rabitq` or `sq8`, in any letter case.
    pub(super) quantizer: &'a str,
    pub(super) m: usize,
    pub(super) k: usize,
    pub(super) force: bool,
    pub(super) sample_limit: Option<usize>,
}

impl Database {
    /// Shared TRAIN QUANTIZER implementation: validates the quantizer name,
    /// builds the statement as an AST (never via string interpolation, so no
    /// identifier sanitizer can drift from `velesdb_core::validation`), runs
    /// it off the GIL, and returns the result count.
    pub(super) fn run_train(&self, py: Python<'_>, request: &TrainRequest<'_>) -> PyResult<usize> {
        let TrainRequest {
            collection: collection_name,
            quantizer,
            m,
            k,
            force,
            sample_limit,
        } = *request;
        let quantizer = quantizer.to_ascii_lowercase();
        if !matches!(quantizer.as_str(), "pq" | "opq" | "rabitq" | "sq8") {
            return Err(PyValueError::new_err(format!(
                "unknown quantizer '{quantizer}'. Supported: pq, opq, rabitq, sq8"
            )));
        }
        let mut params = std::collections::HashMap::new();
        params.insert(
            "m".to_string(),
            WithValue::Integer(i64::try_from(m).map_err(|_| {
                PyValueError::new_err(format!(
                    "m={m} exceeds the maximum supported subspace count"
                ))
            })?),
        );
        params.insert(
            "k".to_string(),
            WithValue::Integer(i64::try_from(k).map_err(|_| {
                PyValueError::new_err(format!(
                    "k={k} exceeds the maximum supported centroid count"
                ))
            })?),
        );
        params.insert("type".to_string(), WithValue::Identifier(quantizer.clone()));
        params.insert("force".to_string(), WithValue::Boolean(force));
        if let Some(limit) = sample_limit {
            params.insert(
                "sample".to_string(),
                WithValue::Integer(i64::try_from(limit).map_err(|_| {
                    PyValueError::new_err(format!(
                        "sample_limit={limit} exceeds the maximum supported sample count"
                    ))
                })?),
            );
        }

        let parsed = Query::new_train(TrainStatement {
            collection: collection_name.to_string(),
            params,
        });

        // Drop the GIL for the training run: k-means codebook fitting is
        // CPU-bound Rust work that scales with the collection size and easily
        // reaches multi-second latency — holding the GIL for that long would
        // freeze every other Python thread.
        let inner = Arc::clone(&self.inner);
        let results = py
            .detach(move || {
                let empty_params = std::collections::HashMap::new();
                inner.execute_query(&parsed, &empty_params)
            })
            .map_err(core_err)?;
        Ok(results.len())
    }
}
