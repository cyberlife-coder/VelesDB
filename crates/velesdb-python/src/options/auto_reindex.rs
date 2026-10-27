//! `AutoReindexOptions`: per-collection auto-reindex policy.

use pyo3::prelude::*;
use std::time::Duration;
use velesdb_core::collection::auto_reindex::AutoReindexConfig as CoreAutoReindexConfig;

/// Per-collection auto-reindex policy mapped to
/// [`velesdb_core::collection::auto_reindex::AutoReindexConfig`].
///
/// Pass an instance to
/// [`Database.create_collection(..., auto_reindex=...)`] to attach a
/// runtime-only [`AutoReindexManager`][velesdb_core::collection::auto_reindex::AutoReindexManager]
/// to the newly-created collection. The manager is **not** persisted —
/// it must be re-attached after every `Database.__new__`.
///
/// `cooldown_secs` is exposed as an integer (seconds) rather than a
/// `Duration` to avoid the Python/Rust serde mismatch.
#[pyclass(module = "velesdb", from_py_object)]
#[derive(Clone, Debug)]
pub struct AutoReindexOptions {
    /// Enable auto-reindex divergence detection. Default: `true`.
    #[pyo3(get, set)]
    pub enabled: bool,
    /// Threshold ratio for triggering reindex. Default: 1.5.
    #[pyo3(get, set)]
    pub param_divergence_threshold: f64,
    /// Minimum dataset size before considering reindex. Default: 10_000.
    #[pyo3(get, set)]
    pub min_size_for_reindex: usize,
    /// Maximum acceptable latency regression percentage before
    /// rollback. Default: 10.0.
    #[pyo3(get, set)]
    pub max_latency_regression_percent: f64,
    /// Maximum acceptable recall regression percentage before
    /// rollback. Default: 2.0.
    #[pyo3(get, set)]
    pub max_recall_regression_percent: f64,
    /// Cooldown period between reindex attempts, in seconds.
    /// Default: 3600 (1 hour).
    #[pyo3(get, set)]
    pub cooldown_secs: u64,
}

impl Default for AutoReindexOptions {
    fn default() -> Self {
        let base = CoreAutoReindexConfig::default();
        Self {
            enabled: base.enabled,
            param_divergence_threshold: base.param_divergence_threshold,
            min_size_for_reindex: base.min_size_for_reindex,
            max_latency_regression_percent: base.max_latency_regression_percent,
            max_recall_regression_percent: base.max_recall_regression_percent,
            cooldown_secs: base.cooldown.as_secs(),
        }
    }
}

#[pymethods]
impl AutoReindexOptions {
    #[new]
    #[pyo3(signature = (
        enabled = true,
        param_divergence_threshold = 1.5,
        min_size_for_reindex = 10_000,
        max_latency_regression_percent = 10.0,
        max_recall_regression_percent = 2.0,
        cooldown_secs = 3_600,
    ))]
    fn new(
        enabled: bool,
        param_divergence_threshold: f64,
        min_size_for_reindex: usize,
        max_latency_regression_percent: f64,
        max_recall_regression_percent: f64,
        cooldown_secs: u64,
    ) -> Self {
        Self {
            enabled,
            param_divergence_threshold,
            min_size_for_reindex,
            max_latency_regression_percent,
            max_recall_regression_percent,
            cooldown_secs,
        }
    }

    /// Returns a disabled configuration that never triggers a reindex.
    #[staticmethod]
    fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "AutoReindexOptions(enabled={}, param_divergence_threshold={}, min_size_for_reindex={}, max_latency_regression_percent={}, max_recall_regression_percent={}, cooldown_secs={})",
            self.enabled,
            self.param_divergence_threshold,
            self.min_size_for_reindex,
            self.max_latency_regression_percent,
            self.max_recall_regression_percent,
            self.cooldown_secs,
        )
    }
}

impl AutoReindexOptions {
    pub(crate) fn to_core(&self) -> CoreAutoReindexConfig {
        // `CoreAutoReindexConfig` is `#[non_exhaustive]`; build from its
        // `Default` and override each field (struct literal is disallowed
        // outside velesdb-core).
        let mut cfg = CoreAutoReindexConfig::default();
        cfg.enabled = self.enabled;
        cfg.param_divergence_threshold = self.param_divergence_threshold;
        cfg.min_size_for_reindex = self.min_size_for_reindex;
        cfg.max_latency_regression_percent = self.max_latency_regression_percent;
        cfg.max_recall_regression_percent = self.max_recall_regression_percent;
        cfg.cooldown = Duration::from_secs(self.cooldown_secs);
        cfg
    }
}
