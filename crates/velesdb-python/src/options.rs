//! Typed options dataclasses exposing core configuration to Python.
//!
//! Wave 3 B2 Commit 10 — replaces the flat `m=`, `ef_construction=`,
//! `expected_vectors=` kwargs on `Database.create_collection` with
//! explicit `#[pyclass]` dataclasses that map 1:1 to the core config
//! structs.
//!
//! Four options types:
//! - [`HnswOptions`] — per-collection HNSW parameters (maps to
//!   [`velesdb_core::index::hnsw::HnswParams`])
//! - [`LimitsOptions`] — global tenant-wide limits (maps to
//!   [`velesdb_core::config::LimitsConfig`])
//! - [`AutoReindexOptions`] — per-collection auto-reindex policy (maps
//!   to [`velesdb_core::collection::auto_reindex::AutoReindexConfig`])
//! - [`VelesConfigOptions`] — global database-level configuration
//!   wrapper (maps to [`velesdb_core::config::VelesConfig`])
//!
//! `WalBatchOptions` is intentionally **not** exposed: the concurrent
//! WAL writer is a velesdb-premium Enterprise feature (see Commit 8
//! `docs/CORE_WIRING_DEBT.md` and `docs/guides/WRITE_CONCURRENCY.md`).

use pyo3::prelude::*;
use std::time::Duration;

use velesdb_core::collection::auto_reindex::AutoReindexConfig as CoreAutoReindexConfig;
use velesdb_core::config::{LimitsConfig as CoreLimitsConfig, VelesConfig as CoreVelesConfig};
use velesdb_core::index::hnsw::HnswParams;

// ---------------------------------------------------------------------------
// HnswOptions
// ---------------------------------------------------------------------------

/// Typed HNSW parameters for `Database.create_collection`.
///
/// All fields are optional — unspecified fields fall back to the
/// engine default via [`HnswParams::default`]. Use
/// [`HnswOptions.for_dataset_size`][Self::for_dataset_size] to get
/// auto-tuned values for a target dataset size.
///
/// Example:
///     >>> from velesdb import HnswOptions
///     >>> opts = HnswOptions(m=48, ef_construction=600)
///     >>> db.create_collection("docs", dimension=768, hnsw=opts)
///     >>> # Auto-tuned:
///     >>> db.create_collection(
///     ...     "big",
///     ...     dimension=128,
///     ...     hnsw=HnswOptions.for_dataset_size(128, 1_000_000),
///     ... )
#[pyclass(module = "velesdb")]
#[derive(Clone, Debug, Default)]
pub struct HnswOptions {
    /// Maximum connections per node (M parameter). Higher = better
    /// recall, more memory, slower insert.
    #[pyo3(get, set)]
    pub m: Option<usize>,
    /// Size of the dynamic candidate list during construction.
    #[pyo3(get, set)]
    pub ef_construction: Option<usize>,
    /// Initial capacity (grows automatically if exceeded).
    #[pyo3(get, set)]
    pub max_elements: Option<usize>,
    /// VAMANA alpha for neighbor diversification (default: 1.2).
    #[pyo3(get, set)]
    pub alpha: Option<f32>,
    /// PQ rescore oversampling factor. Applied only to collections
    /// using `storage_mode="pq"`.
    #[pyo3(get, set)]
    pub pq_rescore_oversampling: Option<u32>,
}

#[pymethods]
impl HnswOptions {
    /// Creates a new `HnswOptions` with the given per-field overrides.
    ///
    /// All arguments are keyword-only and optional.
    #[new]
    #[pyo3(signature = (
        m = None,
        ef_construction = None,
        max_elements = None,
        alpha = None,
        pq_rescore_oversampling = None,
    ))]
    fn new(
        m: Option<usize>,
        ef_construction: Option<usize>,
        max_elements: Option<usize>,
        alpha: Option<f32>,
        pq_rescore_oversampling: Option<u32>,
    ) -> Self {
        Self {
            m,
            ef_construction,
            max_elements,
            alpha,
            pq_rescore_oversampling,
        }
    }

    /// Returns an `HnswOptions` pre-tuned for a specific dataset size.
    ///
    /// Equivalent to calling
    /// [`velesdb_core::index::hnsw::HnswParams::for_dataset_size`].
    ///
    /// Args:
    ///     dimension: vector dimension
    ///     expected_vectors: expected total number of vectors in the
    ///         collection over its lifetime
    #[staticmethod]
    fn for_dataset_size(dimension: usize, expected_vectors: usize) -> Self {
        let params = HnswParams::for_dataset_size(dimension, expected_vectors);
        Self {
            m: Some(params.max_connections),
            ef_construction: Some(params.ef_construction),
            max_elements: Some(params.max_elements),
            alpha: Some(params.alpha),
            pq_rescore_oversampling: None,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "HnswOptions(m={:?}, ef_construction={:?}, max_elements={:?}, alpha={:?}, pq_rescore_oversampling={:?})",
            self.m, self.ef_construction, self.max_elements, self.alpha, self.pq_rescore_oversampling
        )
    }
}

impl HnswOptions {
    /// Materializes this options dataclass into a concrete
    /// [`HnswParams`] by filling in defaults for any unset field.
    pub(crate) fn to_hnsw_params(&self) -> HnswParams {
        let base = HnswParams::default();
        HnswParams {
            max_connections: self.m.unwrap_or(base.max_connections),
            ef_construction: self.ef_construction.unwrap_or(base.ef_construction),
            max_elements: self.max_elements.unwrap_or(base.max_elements),
            alpha: self.alpha.unwrap_or(base.alpha),
            storage_mode: base.storage_mode,
        }
    }
}

// ---------------------------------------------------------------------------
// LimitsOptions
// ---------------------------------------------------------------------------

/// Tenant-wide guard-rail limits mapped to
/// [`velesdb_core::config::LimitsConfig`].
///
/// All fields are optional — unspecified fields fall back to the
/// engine defaults (max_collections=1000, max_dimensions=4096, etc.).
///
/// Enforcement status in v1.13:
/// - `max_collections` — enforced at collection creation (Commit 7)
/// - `max_dimensions` — enforced at collection creation (Commit 7)
/// - `max_vectors_per_collection` — parsed but not yet enforced
/// - `max_payload_size` — parsed but not yet enforced
/// - `max_perfect_mode_vectors` — parsed but not yet enforced
///
/// See `docs/CORE_WIRING_DEBT.md` for the enforcement roadmap.
#[pyclass(module = "velesdb")]
#[derive(Clone, Debug, Default)]
pub struct LimitsOptions {
    /// Maximum number of collections in the database. Default: 1000.
    #[pyo3(get, set)]
    pub max_collections: Option<usize>,
    /// Maximum vector dimension. Default: 4096.
    #[pyo3(get, set)]
    pub max_dimensions: Option<usize>,
    /// Maximum vectors per collection (soft cap, not yet enforced).
    #[pyo3(get, set)]
    pub max_vectors_per_collection: Option<usize>,
    /// Maximum payload size in bytes (not yet enforced).
    #[pyo3(get, set)]
    pub max_payload_size: Option<usize>,
    /// Maximum vector count before "perfect" mode disengages (not yet
    /// enforced).
    #[pyo3(get, set)]
    pub max_perfect_mode_vectors: Option<usize>,
}

#[pymethods]
impl LimitsOptions {
    #[new]
    #[pyo3(signature = (
        max_collections = None,
        max_dimensions = None,
        max_vectors_per_collection = None,
        max_payload_size = None,
        max_perfect_mode_vectors = None,
    ))]
    fn new(
        max_collections: Option<usize>,
        max_dimensions: Option<usize>,
        max_vectors_per_collection: Option<usize>,
        max_payload_size: Option<usize>,
        max_perfect_mode_vectors: Option<usize>,
    ) -> Self {
        Self {
            max_collections,
            max_dimensions,
            max_vectors_per_collection,
            max_payload_size,
            max_perfect_mode_vectors,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "LimitsOptions(max_collections={:?}, max_dimensions={:?}, max_vectors_per_collection={:?}, max_payload_size={:?}, max_perfect_mode_vectors={:?})",
            self.max_collections,
            self.max_dimensions,
            self.max_vectors_per_collection,
            self.max_payload_size,
            self.max_perfect_mode_vectors,
        )
    }
}

impl LimitsOptions {
    pub(crate) fn to_core(&self) -> CoreLimitsConfig {
        let base = CoreLimitsConfig::default();
        CoreLimitsConfig {
            max_dimensions: self.max_dimensions.unwrap_or(base.max_dimensions),
            max_vectors_per_collection: self
                .max_vectors_per_collection
                .unwrap_or(base.max_vectors_per_collection),
            max_collections: self.max_collections.unwrap_or(base.max_collections),
            max_payload_size: self.max_payload_size.unwrap_or(base.max_payload_size),
            max_perfect_mode_vectors: self
                .max_perfect_mode_vectors
                .unwrap_or(base.max_perfect_mode_vectors),
        }
    }
}

// ---------------------------------------------------------------------------
// AutoReindexOptions
// ---------------------------------------------------------------------------

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
#[pyclass(module = "velesdb")]
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
        CoreAutoReindexConfig {
            enabled: self.enabled,
            param_divergence_threshold: self.param_divergence_threshold,
            min_size_for_reindex: self.min_size_for_reindex,
            max_latency_regression_percent: self.max_latency_regression_percent,
            max_recall_regression_percent: self.max_recall_regression_percent,
            cooldown: Duration::from_secs(self.cooldown_secs),
        }
    }
}

// ---------------------------------------------------------------------------
// VelesConfigOptions
// ---------------------------------------------------------------------------

/// Global database-level configuration exposed to Python.
///
/// Maps to [`velesdb_core::config::VelesConfig`]. Currently exposes
/// the `limits` sub-section. Other sub-sections (search, hnsw,
/// storage, wal_batch) are left at their engine defaults — user-level
/// tuning is done per-collection via [`HnswOptions`].
///
/// `wal_batch` is intentionally not exposed: the concurrent WAL
/// writer is a velesdb-premium Enterprise feature. See
/// `docs/guides/WRITE_CONCURRENCY.md`.
#[pyclass(module = "velesdb")]
#[derive(Clone, Debug, Default)]
pub struct VelesConfigOptions {
    /// Tenant-wide guard-rail limits.
    #[pyo3(get, set)]
    pub limits: Option<LimitsOptions>,
}

#[pymethods]
impl VelesConfigOptions {
    #[new]
    #[pyo3(signature = (limits = None))]
    fn new(limits: Option<LimitsOptions>) -> Self {
        Self { limits }
    }

    fn __repr__(&self) -> String {
        format!("VelesConfigOptions(limits={:?})", self.limits)
    }
}

impl VelesConfigOptions {
    pub(crate) fn to_core(&self) -> CoreVelesConfig {
        let mut core = CoreVelesConfig::default();
        if let Some(ref limits) = self.limits {
            core.limits = limits.to_core();
        }
        core
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Registers every option dataclass on the top-level `velesdb` module.
pub fn register_options(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<HnswOptions>()?;
    m.add_class::<LimitsOptions>()?;
    m.add_class::<AutoReindexOptions>()?;
    m.add_class::<VelesConfigOptions>()?;
    Ok(())
}
