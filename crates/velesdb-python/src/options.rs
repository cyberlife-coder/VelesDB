//! Typed options dataclasses exposing core configuration to Python.
//!
//! Wave 3 B2 Commit 10 — replaces the flat `m=`, `ef_construction=`,
//! `expected_vectors=` kwargs on `Database.create_collection` with
//! explicit `#[pyclass]` dataclasses that map 1:1 to the core config
//! structs.
//!
//! Options types:
//! - [`HnswOptions`] — per-collection HNSW parameters (maps to
//!   [`velesdb_core::index::hnsw::HnswParams`])
//! - [`LimitsOptions`] — global tenant-wide limits (maps to
//!   [`velesdb_core::config::LimitsConfig`])
//! - [`AutoReindexOptions`] — per-collection auto-reindex policy (maps
//!   to [`velesdb_core::collection::auto_reindex::AutoReindexConfig`])
//! - [`SearchConfigOptions`] / [`HnswConfigOptions`] /
//!   [`StorageOptions`] / [`QuantizationOptions`] — engine config
//!   sections (issue #1549; map to the same-named
//!   `velesdb_core::config` sections)
//! - [`VelesConfigOptions`] — global database-level configuration
//!   wrapper (maps to [`velesdb_core::config::VelesConfig`]), with TOML
//!   loading via `from_toml` / `from_toml_path`
//!
//! `WalBatchOptions` is intentionally **not** exposed: the concurrent
//! WAL writer is a velesdb-premium Enterprise feature (see Commit 8
//! `docs/CORE_WIRING_DEBT.md` and `docs/guides/WRITE_CONCURRENCY.md`).

use pyo3::prelude::*;
use std::path::PathBuf;
use std::time::Duration;

use velesdb_core::collection::auto_reindex::AutoReindexConfig as CoreAutoReindexConfig;
use velesdb_core::config::{
    ConfigError, HnswConfig as CoreHnswConfig, LimitsConfig as CoreLimitsConfig,
    QuantizationConfig as CoreQuantizationConfig, QuantizationType,
    SearchConfig as CoreSearchConfig, SearchMode, StorageConfig as CoreStorageConfig,
    VelesConfig as CoreVelesConfig,
};
use velesdb_core::index::hnsw::HnswParams;

/// Maps a typed core [`ConfigError`] onto the closest Python exception.
///
/// - missing file → `FileNotFoundError`
/// - other IO failures → `OSError`
/// - parse/validation failures → `ValueError` carrying the typed message
///   (`Invalid configuration value for 'limits.max_collections': ...`)
///
/// Never falls back to a default configuration: every loader error is
/// surfaced fail-fast to the Python caller.
fn config_err_to_py(err: ConfigError) -> PyErr {
    match err {
        ConfigError::IoError(io) if io.kind() == std::io::ErrorKind::NotFound => {
            pyo3::exceptions::PyFileNotFoundError::new_err(io.to_string())
        }
        ConfigError::IoError(io) => pyo3::exceptions::PyOSError::new_err(io.to_string()),
        other => pyo3::exceptions::PyValueError::new_err(other.to_string()),
    }
}

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
#[pyclass(module = "velesdb", from_py_object)]
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
        Self::from_params(HnswParams::for_dataset_size(dimension, expected_vectors))
    }

    /// **Preset — fast**: optimized for insertion speed, lower recall.
    ///
    /// Best for small datasets (<10K), dev workflows, and rapid
    /// iteration. Maps to
    /// [`velesdb_core::index::hnsw::HnswParams::fast`]
    /// (`M=16`, `ef_construction=150`).
    #[staticmethod]
    fn fast() -> Self {
        Self::from_params(HnswParams::fast())
    }

    /// **Preset — turbo**: maximum insert throughput, ~85% recall.
    ///
    /// Best for bulk loading, benchmarking, and cold-start ingestion
    /// where search quality is not yet the priority. Maps to
    /// [`velesdb_core::index::hnsw::HnswParams::turbo`]
    /// (`M=12`, `ef_construction=100`). Not recommended for production
    /// search workloads — rebuild with [`HnswOptions::high_recall`] or
    /// [`HnswOptions::max_recall`] after ingestion.
    #[staticmethod]
    fn turbo() -> Self {
        Self::from_params(HnswParams::turbo())
    }

    /// **Preset — balanced**: the engine-default mix of recall and
    /// speed for the given dimension.
    ///
    /// Wraps [`velesdb_core::index::hnsw::HnswParams::auto`]:
    /// - dimension ≤ 256: `M=24`, `ef_construction=300`
    /// - dimension ≥ 257: `M=32`, `ef_construction=400`
    #[staticmethod]
    fn balanced(dimension: usize) -> Self {
        Self::from_params(HnswParams::auto(dimension))
    }

    /// **Preset — high recall**: bumps `M` and `ef_construction` above
    /// the engine default for the given dimension.
    ///
    /// Maps to [`velesdb_core::index::hnsw::HnswParams::high_recall`]
    /// (`M = auto.M + 8`, `ef_construction = auto.ef_construction + 200`).
    #[staticmethod]
    fn high_recall(dimension: usize) -> Self {
        Self::from_params(HnswParams::high_recall(dimension))
    }

    /// **Preset — max recall**: the tightest recall-oriented preset.
    ///
    /// Maps to [`velesdb_core::index::hnsw::HnswParams::max_recall`].
    /// Trades insert throughput and memory for the highest achievable
    /// recall at this dimension.
    #[staticmethod]
    fn max_recall(dimension: usize) -> Self {
        Self::from_params(HnswParams::max_recall(dimension))
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
    ///
    /// Rejects an out-of-range `alpha` (must be finite and `>= 1.0`) with a
    /// `ValueError`, mirroring the REST and Tauri boundaries.
    pub(crate) fn to_hnsw_params(&self) -> PyResult<HnswParams> {
        let base = HnswParams::default();
        let params = HnswParams {
            max_connections: self.m.unwrap_or(base.max_connections),
            ef_construction: self.ef_construction.unwrap_or(base.ef_construction),
            max_elements: self.max_elements.unwrap_or(base.max_elements),
            alpha: self.alpha.unwrap_or(base.alpha),
            storage_mode: base.storage_mode,
        };
        params
            .validate()
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        Ok(params)
    }

    /// Shared conversion from a concrete [`HnswParams`] to a fully-
    /// populated `HnswOptions`. Used by every preset classmethod so
    /// they share the same field-mapping contract.
    fn from_params(params: HnswParams) -> Self {
        Self {
            m: Some(params.max_connections),
            ef_construction: Some(params.ef_construction),
            max_elements: Some(params.max_elements),
            alpha: Some(params.alpha),
            pq_rescore_oversampling: None,
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
/// Enforcement status:
/// - `max_collections` — enforced at collection creation (Commit 7)
/// - `max_dimensions` — enforced at collection creation (Commit 7)
/// - `max_vectors_per_collection` — enforced at the runtime ingest/search boundary (GuardRail VELES-027)
/// - `max_payload_size` — enforced at the runtime ingest/search boundary (GuardRail VELES-027)
/// - `max_perfect_mode_vectors` — enforced at the runtime ingest/search boundary (GuardRail VELES-027)
///
/// See `docs/CORE_WIRING_DEBT.md` for the enforcement roadmap.
#[pyclass(module = "velesdb", from_py_object)]
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
        // `CoreLimitsConfig` is `#[non_exhaustive]`; build from `Default` and
        // override each field (struct literal is disallowed outside velesdb-core).
        let mut cfg = CoreLimitsConfig::default();
        cfg.max_dimensions = self.max_dimensions.unwrap_or(cfg.max_dimensions);
        cfg.max_vectors_per_collection = self
            .max_vectors_per_collection
            .unwrap_or(cfg.max_vectors_per_collection);
        cfg.max_collections = self.max_collections.unwrap_or(cfg.max_collections);
        cfg.max_payload_size = self.max_payload_size.unwrap_or(cfg.max_payload_size);
        cfg.max_perfect_mode_vectors = self
            .max_perfect_mode_vectors
            .unwrap_or(cfg.max_perfect_mode_vectors);
        cfg
    }

    /// Builds a fully-populated options object from a core config section
    /// (every field `Some`), so a TOML-loaded config round-trips
    /// losslessly through [`Self::to_core`].
    fn from_core(core: &CoreLimitsConfig) -> Self {
        Self {
            max_collections: Some(core.max_collections),
            max_dimensions: Some(core.max_dimensions),
            max_vectors_per_collection: Some(core.max_vectors_per_collection),
            max_payload_size: Some(core.max_payload_size),
            max_perfect_mode_vectors: Some(core.max_perfect_mode_vectors),
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

// ---------------------------------------------------------------------------
// SearchConfigOptions
// ---------------------------------------------------------------------------

/// Valid values for `SearchConfigOptions.default_mode`, mirroring the
/// serde `snake_case` names of [`SearchMode`].
const SEARCH_MODES: &[&str] = &["fast", "balanced", "accurate", "perfect"];

/// Parses a Python-side mode string into a core [`SearchMode`].
fn parse_search_mode(mode: &str) -> PyResult<SearchMode> {
    match mode {
        "fast" => Ok(SearchMode::Fast),
        "balanced" => Ok(SearchMode::Balanced),
        "accurate" => Ok(SearchMode::Accurate),
        "perfect" => Ok(SearchMode::Perfect),
        other => Err(pyo3::exceptions::PyValueError::new_err(format!(
            "invalid search.default_mode '{other}' (expected one of: {SEARCH_MODES:?})"
        ))),
    }
}

/// Renders a core [`SearchMode`] as its Python-side mode string.
///
/// Fallible: [`SearchMode`] is `#[non_exhaustive]`, so a future core
/// variant unknown to this binding is rejected loudly instead of being
/// silently re-mapped.
fn search_mode_str(mode: SearchMode) -> PyResult<&'static str> {
    match mode {
        SearchMode::Fast => Ok("fast"),
        SearchMode::Balanced => Ok("balanced"),
        SearchMode::Accurate => Ok("accurate"),
        SearchMode::Perfect => Ok("perfect"),
        other => Err(pyo3::exceptions::PyValueError::new_err(format!(
            "unsupported search mode in configuration: {other:?}"
        ))),
    }
}

/// Global search defaults mapped to
/// [`velesdb_core::config::SearchConfig`] (the `[search]` TOML section).
///
/// All fields are optional — unspecified fields fall back to the engine
/// defaults (`default_mode="balanced"`, `max_results=1000`,
/// `query_timeout_ms=30000`). Distinct from the per-query
/// `SearchOptions` class, which tunes a single search call.
#[pyclass(module = "velesdb", from_py_object)]
#[derive(Clone, Debug, Default)]
pub struct SearchConfigOptions {
    /// Default search mode: `"fast"`, `"balanced"`, `"accurate"` or
    /// `"perfect"`. Default: `"balanced"`.
    #[pyo3(get, set)]
    pub default_mode: Option<String>,
    /// Override `ef_search` (if set, overrides the mode). Range [16, 4096].
    #[pyo3(get, set)]
    pub ef_search: Option<usize>,
    /// Maximum results per query. Default: 1000.
    #[pyo3(get, set)]
    pub max_results: Option<usize>,
    /// Query timeout in milliseconds (0 disables). Default: 30000.
    #[pyo3(get, set)]
    pub query_timeout_ms: Option<u64>,
}

#[pymethods]
impl SearchConfigOptions {
    #[new]
    #[pyo3(signature = (
        default_mode = None,
        ef_search = None,
        max_results = None,
        query_timeout_ms = None,
    ))]
    fn new(
        default_mode: Option<String>,
        ef_search: Option<usize>,
        max_results: Option<usize>,
        query_timeout_ms: Option<u64>,
    ) -> Self {
        Self {
            default_mode,
            ef_search,
            max_results,
            query_timeout_ms,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "SearchConfigOptions(default_mode={:?}, ef_search={:?}, max_results={:?}, query_timeout_ms={:?})",
            self.default_mode, self.ef_search, self.max_results, self.query_timeout_ms,
        )
    }
}

impl SearchConfigOptions {
    /// Materializes into the core section, rejecting an unknown
    /// `default_mode` string with a `ValueError`.
    pub(crate) fn to_core(&self) -> PyResult<CoreSearchConfig> {
        let mut cfg = CoreSearchConfig::default();
        if let Some(ref mode) = self.default_mode {
            cfg.default_mode = parse_search_mode(mode)?;
        }
        if self.ef_search.is_some() {
            cfg.ef_search = self.ef_search;
        }
        cfg.max_results = self.max_results.unwrap_or(cfg.max_results);
        cfg.query_timeout_ms = self.query_timeout_ms.unwrap_or(cfg.query_timeout_ms);
        Ok(cfg)
    }

    /// Builds a fully-populated options object from a core section
    /// (see [`LimitsOptions::from_core`]). `ef_search` stays `None` when
    /// the core leaves it unset (mode-driven), preserving the override
    /// semantics on round-trip. Fallible because [`SearchMode`] is
    /// `#[non_exhaustive]` (see [`search_mode_str`]).
    fn from_core(core: &CoreSearchConfig) -> PyResult<Self> {
        Ok(Self {
            default_mode: Some(search_mode_str(core.default_mode)?.to_string()),
            ef_search: core.ef_search,
            max_results: Some(core.max_results),
            query_timeout_ms: Some(core.query_timeout_ms),
        })
    }
}

// ---------------------------------------------------------------------------
// HnswConfigOptions
// ---------------------------------------------------------------------------

/// Global HNSW index defaults mapped to
/// [`velesdb_core::config::HnswConfig`] (the `[hnsw]` TOML section).
///
/// Distinct from the per-collection [`HnswOptions`] passed to
/// `Database.create_collection` — this section sets the database-wide
/// defaults instead. `None` fields fall back to the engine defaults
/// (`m`/`ef_construction` auto by dimension, `max_layers=0` = auto).
#[pyclass(module = "velesdb", from_py_object)]
#[derive(Clone, Debug, Default)]
pub struct HnswConfigOptions {
    /// Connections per node (M parameter), range [4, 128].
    /// `None` = auto based on dimension.
    #[pyo3(get, set)]
    pub m: Option<usize>,
    /// Candidate pool size during construction, range [100, 2000].
    /// `None` = auto based on dimension.
    #[pyo3(get, set)]
    pub ef_construction: Option<usize>,
    /// Maximum number of layers (0 = auto). Default: 0.
    #[pyo3(get, set)]
    pub max_layers: Option<usize>,
}

#[pymethods]
impl HnswConfigOptions {
    #[new]
    #[pyo3(signature = (m = None, ef_construction = None, max_layers = None))]
    fn new(m: Option<usize>, ef_construction: Option<usize>, max_layers: Option<usize>) -> Self {
        Self {
            m,
            ef_construction,
            max_layers,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "HnswConfigOptions(m={:?}, ef_construction={:?}, max_layers={:?})",
            self.m, self.ef_construction, self.max_layers,
        )
    }
}

impl HnswConfigOptions {
    pub(crate) fn to_core(&self) -> CoreHnswConfig {
        let mut cfg = CoreHnswConfig::default();
        if self.m.is_some() {
            cfg.m = self.m;
        }
        if self.ef_construction.is_some() {
            cfg.ef_construction = self.ef_construction;
        }
        cfg.max_layers = self.max_layers.unwrap_or(cfg.max_layers);
        cfg
    }

    /// Builds an options object from a core section. `m`/`ef_construction`
    /// stay `None` when the core leaves them auto, preserving auto-tuning
    /// semantics on round-trip.
    fn from_core(core: &CoreHnswConfig) -> Self {
        Self {
            m: core.m,
            ef_construction: core.ef_construction,
            max_layers: Some(core.max_layers),
        }
    }
}

// ---------------------------------------------------------------------------
// StorageOptions
// ---------------------------------------------------------------------------

/// Storage engine settings mapped to
/// [`velesdb_core::config::StorageConfig`] (the `[storage]` TOML section).
///
/// All fields are optional — unspecified fields fall back to the engine
/// defaults (`data_dir="./velesdb_data"`, `storage_mode="mmap"`,
/// `mmap_cache_mb=1024`, `vector_alignment=64`).
#[pyclass(module = "velesdb", from_py_object)]
#[derive(Clone, Debug, Default)]
pub struct StorageOptions {
    /// Data directory path. Default: `"./velesdb_data"`.
    #[pyo3(get, set)]
    pub data_dir: Option<String>,
    /// Storage mode: `"mmap"` or `"memory"`. Default: `"mmap"`.
    #[pyo3(get, set)]
    pub storage_mode: Option<String>,
    /// Mmap cache size in megabytes. Default: 1024.
    #[pyo3(get, set)]
    pub mmap_cache_mb: Option<usize>,
    /// Vector alignment in bytes. Default: 64.
    #[pyo3(get, set)]
    pub vector_alignment: Option<usize>,
}

#[pymethods]
impl StorageOptions {
    #[new]
    #[pyo3(signature = (
        data_dir = None,
        storage_mode = None,
        mmap_cache_mb = None,
        vector_alignment = None,
    ))]
    fn new(
        data_dir: Option<String>,
        storage_mode: Option<String>,
        mmap_cache_mb: Option<usize>,
        vector_alignment: Option<usize>,
    ) -> Self {
        Self {
            data_dir,
            storage_mode,
            mmap_cache_mb,
            vector_alignment,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "StorageOptions(data_dir={:?}, storage_mode={:?}, mmap_cache_mb={:?}, vector_alignment={:?})",
            self.data_dir, self.storage_mode, self.mmap_cache_mb, self.vector_alignment,
        )
    }
}

impl StorageOptions {
    pub(crate) fn to_core(&self) -> CoreStorageConfig {
        let mut cfg = CoreStorageConfig::default();
        if let Some(ref data_dir) = self.data_dir {
            cfg.data_dir.clone_from(data_dir);
        }
        if let Some(ref storage_mode) = self.storage_mode {
            cfg.storage_mode.clone_from(storage_mode);
        }
        cfg.mmap_cache_mb = self.mmap_cache_mb.unwrap_or(cfg.mmap_cache_mb);
        cfg.vector_alignment = self.vector_alignment.unwrap_or(cfg.vector_alignment);
        cfg
    }

    /// Builds a fully-populated options object from a core section
    /// (see [`LimitsOptions::from_core`]).
    fn from_core(core: &CoreStorageConfig) -> Self {
        Self {
            data_dir: Some(core.data_dir.clone()),
            storage_mode: Some(core.storage_mode.clone()),
            mmap_cache_mb: Some(core.mmap_cache_mb),
            vector_alignment: Some(core.vector_alignment),
        }
    }
}

// ---------------------------------------------------------------------------
// QuantizationOptions
// ---------------------------------------------------------------------------

/// Valid values for `QuantizationOptions.mode`, mirroring the serde tags
/// of [`QuantizationType`].
const QUANTIZATION_MODES: &[&str] = &["none", "sq8", "binary", "pq", "rabitq"];

/// Quantization settings mapped to
/// [`velesdb_core::config::QuantizationConfig`] (the `[quantization]`
/// TOML section).
///
/// `mode` selects the algorithm (`"none"`, `"sq8"`, `"binary"`, `"pq"`,
/// `"rabitq"`). The `pq_*` fields configure Product Quantization and are
/// only valid with `mode="pq"` (where `pq_m` is required); combining
/// them with any other mode raises `ValueError` — they are never
/// silently dropped.
#[pyclass(module = "velesdb", from_py_object)]
#[derive(Clone, Debug, Default)]
pub struct QuantizationOptions {
    /// Quantization mode. Default: `"none"`.
    #[pyo3(get, set)]
    pub mode: Option<String>,
    /// PQ: number of subspaces (dimension must be divisible by it).
    /// Required when `mode="pq"`.
    #[pyo3(get, set)]
    pub pq_m: Option<usize>,
    /// PQ: codebook size per subspace. Default: 256.
    #[pyo3(get, set)]
    pub pq_k: Option<usize>,
    /// PQ: enable Optimized Product Quantization (OPQ) rotation.
    /// Default: `False`.
    #[pyo3(get, set)]
    pub pq_opq_enabled: Option<bool>,
    /// PQ: oversampling factor for training. Default: 4.
    #[pyo3(get, set)]
    pub pq_oversampling: Option<u32>,
    /// Enable reranking after quantized search. Default: `True`.
    #[pyo3(get, set)]
    pub rerank_enabled: Option<bool>,
    /// Reranking multiplier for candidates. Default: 2.
    #[pyo3(get, set)]
    pub rerank_multiplier: Option<usize>,
    /// Auto-enable quantization for large collections. Default: `True`.
    #[pyo3(get, set)]
    pub auto_quantization: Option<bool>,
    /// Vector-count threshold for auto-quantization. Default: 10000.
    #[pyo3(get, set)]
    pub auto_quantization_threshold: Option<usize>,
}

#[pymethods]
impl QuantizationOptions {
    #[new]
    #[pyo3(signature = (
        mode = None,
        pq_m = None,
        pq_k = None,
        pq_opq_enabled = None,
        pq_oversampling = None,
        rerank_enabled = None,
        rerank_multiplier = None,
        auto_quantization = None,
        auto_quantization_threshold = None,
    ))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        mode: Option<String>,
        pq_m: Option<usize>,
        pq_k: Option<usize>,
        pq_opq_enabled: Option<bool>,
        pq_oversampling: Option<u32>,
        rerank_enabled: Option<bool>,
        rerank_multiplier: Option<usize>,
        auto_quantization: Option<bool>,
        auto_quantization_threshold: Option<usize>,
    ) -> Self {
        Self {
            mode,
            pq_m,
            pq_k,
            pq_opq_enabled,
            pq_oversampling,
            rerank_enabled,
            rerank_multiplier,
            auto_quantization,
            auto_quantization_threshold,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "QuantizationOptions(mode={:?}, pq_m={:?}, pq_k={:?}, pq_opq_enabled={:?}, pq_oversampling={:?}, rerank_enabled={:?}, rerank_multiplier={:?}, auto_quantization={:?}, auto_quantization_threshold={:?})",
            self.mode,
            self.pq_m,
            self.pq_k,
            self.pq_opq_enabled,
            self.pq_oversampling,
            self.rerank_enabled,
            self.rerank_multiplier,
            self.auto_quantization,
            self.auto_quantization_threshold,
        )
    }
}

impl QuantizationOptions {
    /// Returns `true` when any `pq_*` field is set.
    fn has_pq_fields(&self) -> bool {
        self.pq_m.is_some()
            || self.pq_k.is_some()
            || self.pq_opq_enabled.is_some()
            || self.pq_oversampling.is_some()
    }

    /// Materializes the `mode` string (plus `pq_*` fields) into a core
    /// [`QuantizationType`], failing fast on every inconsistent combination.
    fn mode_to_core(&self) -> PyResult<QuantizationType> {
        let mode = self.mode.as_deref().unwrap_or("none");
        if mode != "pq" && self.has_pq_fields() {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "quantization pq_m/pq_k/pq_opq_enabled/pq_oversampling require mode='pq' (got mode={mode:?})"
            )));
        }
        match mode {
            "none" => Ok(QuantizationType::None),
            "sq8" => Ok(QuantizationType::SQ8),
            "binary" => Ok(QuantizationType::Binary),
            "rabitq" => Ok(QuantizationType::RaBitQ),
            "pq" => {
                let m = self.pq_m.ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err(
                        "quantization mode='pq' requires pq_m (number of subspaces)",
                    )
                })?;
                Ok(QuantizationType::PQ {
                    m,
                    k: self.pq_k.unwrap_or(256),
                    opq_enabled: self.pq_opq_enabled.unwrap_or(false),
                    oversampling: self.pq_oversampling.or(Some(4)),
                })
            }
            other => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "invalid quantization.mode '{other}' (expected one of: {QUANTIZATION_MODES:?})"
            ))),
        }
    }

    pub(crate) fn to_core(&self) -> PyResult<CoreQuantizationConfig> {
        let mut cfg = CoreQuantizationConfig::default();
        cfg.mode = self.mode_to_core()?;
        cfg.rerank_enabled = self.rerank_enabled.unwrap_or(cfg.rerank_enabled);
        cfg.rerank_multiplier = self.rerank_multiplier.unwrap_or(cfg.rerank_multiplier);
        cfg.auto_quantization = self.auto_quantization.unwrap_or(cfg.auto_quantization);
        cfg.auto_quantization_threshold = self
            .auto_quantization_threshold
            .unwrap_or(cfg.auto_quantization_threshold);
        Ok(cfg)
    }

    /// Builds a fully-populated options object from a core section.
    ///
    /// Fallible: [`QuantizationType`] is `#[non_exhaustive]`, so a future
    /// core variant unknown to this binding is rejected loudly instead of
    /// being silently re-mapped.
    fn from_core(core: &CoreQuantizationConfig) -> PyResult<Self> {
        let (mode, pq_m, pq_k, pq_opq_enabled, pq_oversampling) = match &core.mode {
            QuantizationType::None => ("none", None, None, None, None),
            QuantizationType::SQ8 => ("sq8", None, None, None, None),
            QuantizationType::Binary => ("binary", None, None, None, None),
            QuantizationType::RaBitQ => ("rabitq", None, None, None, None),
            QuantizationType::PQ {
                m,
                k,
                opq_enabled,
                oversampling,
            } => ("pq", Some(*m), Some(*k), Some(*opq_enabled), *oversampling),
            other => {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "unsupported quantization mode in configuration: {other:?}"
                )));
            }
        };
        Ok(Self {
            mode: Some(mode.to_string()),
            pq_m,
            pq_k,
            pq_opq_enabled,
            pq_oversampling,
            rerank_enabled: Some(core.rerank_enabled),
            rerank_multiplier: Some(core.rerank_multiplier),
            auto_quantization: Some(core.auto_quantization),
            auto_quantization_threshold: Some(core.auto_quantization_threshold),
        })
    }
}

// ---------------------------------------------------------------------------
// VelesConfigOptions
// ---------------------------------------------------------------------------

/// Global database-level configuration exposed to Python.
///
/// Maps to [`velesdb_core::config::VelesConfig`], covering every *engine*
/// section: `limits` ([`LimitsOptions`]), `search`
/// ([`SearchConfigOptions`]), `hnsw` ([`HnswConfigOptions`]), `storage`
/// ([`StorageOptions`]) and `quantization` ([`QuantizationOptions`]).
/// Unset sections stay at their engine defaults.
///
/// Load from TOML with [`VelesConfigOptions::from_toml`] /
/// [`VelesConfigOptions::from_toml_path`] (engine-only semantics — a
/// shell-owned `[server]`/`[logging]` table in a shared file is ignored,
/// mirroring `VelesConfig::from_toml_engine_only`).
///
/// `wal_batch` is intentionally not exposed: the concurrent WAL
/// writer is a velesdb-premium Enterprise feature. See
/// `docs/guides/WRITE_CONCURRENCY.md`. A `[wal_batch]` TOML section is
/// accepted by the loaders (it is a valid engine section for the
/// server/CLI) but is not surfaced on — nor applied by — this embedded
/// options type.
///
/// `server`/`logging` are likewise not exposed: they configure hosting
/// shells, not the embedded engine surface.
#[pyclass(module = "velesdb", from_py_object)]
#[derive(Clone, Debug, Default)]
pub struct VelesConfigOptions {
    /// Tenant-wide guard-rail limits.
    #[pyo3(get, set)]
    pub limits: Option<LimitsOptions>,
    /// Global search defaults (`[search]`).
    #[pyo3(get, set)]
    pub search: Option<SearchConfigOptions>,
    /// Database-wide HNSW defaults (`[hnsw]`).
    #[pyo3(get, set)]
    pub hnsw: Option<HnswConfigOptions>,
    /// Storage engine settings (`[storage]`).
    #[pyo3(get, set)]
    pub storage: Option<StorageOptions>,
    /// Quantization settings (`[quantization]`).
    #[pyo3(get, set)]
    pub quantization: Option<QuantizationOptions>,
}

#[pymethods]
impl VelesConfigOptions {
    #[new]
    #[pyo3(signature = (
        limits = None,
        search = None,
        hnsw = None,
        storage = None,
        quantization = None,
    ))]
    fn new(
        limits: Option<LimitsOptions>,
        search: Option<SearchConfigOptions>,
        hnsw: Option<HnswConfigOptions>,
        storage: Option<StorageOptions>,
        quantization: Option<QuantizationOptions>,
    ) -> Self {
        Self {
            limits,
            search,
            hnsw,
            storage,
            quantization,
        }
    }

    /// Loads engine configuration from an in-memory TOML string.
    ///
    /// Wraps [`CoreVelesConfig::from_toml_engine_only`]: only the engine
    /// sections are considered (a shell-owned `[server]`/`[logging]`
    /// table is dropped before parsing), and the loaded config is
    /// validated. Fail-fast: invalid TOML or an out-of-range value
    /// raises `ValueError` carrying the typed core message — there is no
    /// silent fallback to defaults.
    ///
    /// Every returned section is fully populated (engine defaults for
    /// keys the TOML does not set).
    #[staticmethod]
    fn from_toml(toml_str: &str) -> PyResult<Self> {
        let core = CoreVelesConfig::from_toml_engine_only(toml_str).map_err(config_err_to_py)?;
        Self::from_core(&core)
    }

    /// Loads engine configuration from a TOML file path.
    ///
    /// Wraps [`CoreVelesConfig::load_from_path_engine_only`]: engine-only
    /// section filtering plus validation, with `VELESDB_*` environment
    /// variables layered on top of the file (mirroring the server/CLI
    /// `--config` semantics from PR #1565). Fail-fast: a missing file
    /// raises `FileNotFoundError`; invalid TOML or an out-of-range value
    /// raises `ValueError` with the typed core message.
    #[staticmethod]
    fn from_toml_path(path: PathBuf) -> PyResult<Self> {
        let core = CoreVelesConfig::load_from_path_engine_only(&path).map_err(config_err_to_py)?;
        Self::from_core(&core)
    }

    fn __repr__(&self) -> String {
        format!(
            "VelesConfigOptions(limits={:?}, search={:?}, hnsw={:?}, storage={:?}, quantization={:?})",
            self.limits, self.search, self.hnsw, self.storage, self.quantization,
        )
    }
}

impl VelesConfigOptions {
    /// Assembles the full core config and validates it, so an invalid
    /// value fails fast as `ValueError` at `Database(..., config=...)`
    /// time (in addition to the core-side validation in
    /// `Database::open_with_config`).
    pub(crate) fn to_core(&self) -> PyResult<CoreVelesConfig> {
        let mut core = CoreVelesConfig::default();
        if let Some(ref limits) = self.limits {
            core.limits = limits.to_core();
        }
        if let Some(ref search) = self.search {
            core.search = search.to_core()?;
        }
        if let Some(ref hnsw) = self.hnsw {
            core.hnsw = hnsw.to_core();
        }
        if let Some(ref storage) = self.storage {
            core.storage = storage.to_core();
        }
        if let Some(ref quantization) = self.quantization {
            core.quantization = quantization.to_core()?;
        }
        core.validate().map_err(config_err_to_py)?;
        Ok(core)
    }

    /// Builds a fully-populated options object from a validated core
    /// config (every exposed section `Some`, every field set), so
    /// [`Self::to_core`] round-trips the loaded values losslessly.
    /// `wal_batch`/`server`/`logging` are intentionally not carried over
    /// (see the type-level docs).
    fn from_core(core: &CoreVelesConfig) -> PyResult<Self> {
        Ok(Self {
            limits: Some(LimitsOptions::from_core(&core.limits)),
            search: Some(SearchConfigOptions::from_core(&core.search)?),
            hnsw: Some(HnswConfigOptions::from_core(&core.hnsw)),
            storage: Some(StorageOptions::from_core(&core.storage)),
            quantization: Some(QuantizationOptions::from_core(&core.quantization)?),
        })
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
    m.add_class::<SearchConfigOptions>()?;
    m.add_class::<HnswConfigOptions>()?;
    m.add_class::<StorageOptions>()?;
    m.add_class::<QuantizationOptions>()?;
    m.add_class::<VelesConfigOptions>()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests that stringify a `PyErr` need a live interpreter (error
    /// formatting goes through the Python C-API). Idempotent.
    fn init_python() {
        Python::initialize();
    }

    // -- Section mapping: non-default values must propagate to core ------

    #[test]
    fn search_config_options_to_core_propagates_non_defaults() {
        let opts = SearchConfigOptions {
            default_mode: Some("accurate".to_string()),
            ef_search: Some(256),
            max_results: Some(42),
            query_timeout_ms: Some(5_000),
        };
        let core = opts.to_core().expect("valid section");
        assert!(matches!(core.default_mode, SearchMode::Accurate));
        assert_eq!(core.ef_search, Some(256));
        assert_eq!(core.max_results, 42);
        assert_eq!(core.query_timeout_ms, 5_000);
    }

    #[test]
    fn search_config_options_unset_fields_keep_engine_defaults() {
        let core = SearchConfigOptions::default()
            .to_core()
            .expect("empty section is valid");
        let defaults = CoreSearchConfig::default();
        assert_eq!(core.max_results, defaults.max_results);
        assert_eq!(core.ef_search, defaults.ef_search);
        assert_eq!(core.query_timeout_ms, defaults.query_timeout_ms);
    }

    #[test]
    fn search_config_options_rejects_unknown_mode() {
        let opts = SearchConfigOptions {
            default_mode: Some("warp".to_string()),
            ..SearchConfigOptions::default()
        };
        init_python();
        let err = opts.to_core().expect_err("unknown mode must fail");
        assert!(err.to_string().contains("default_mode"));
    }

    #[test]
    fn hnsw_config_options_to_core_propagates_non_defaults() {
        let opts = HnswConfigOptions {
            m: Some(32),
            ef_construction: Some(400),
            max_layers: Some(8),
        };
        let core = opts.to_core();
        assert_eq!(core.m, Some(32));
        assert_eq!(core.ef_construction, Some(400));
        assert_eq!(core.max_layers, 8);
    }

    #[test]
    fn storage_options_to_core_propagates_non_defaults() {
        let opts = StorageOptions {
            data_dir: Some("./custom".to_string()),
            storage_mode: Some("memory".to_string()),
            mmap_cache_mb: Some(256),
            vector_alignment: Some(32),
        };
        let core = opts.to_core();
        assert_eq!(core.data_dir, "./custom");
        assert_eq!(core.storage_mode, "memory");
        assert_eq!(core.mmap_cache_mb, 256);
        assert_eq!(core.vector_alignment, 32);
    }

    #[test]
    fn quantization_options_to_core_propagates_pq_mode() {
        let opts = QuantizationOptions {
            mode: Some("pq".to_string()),
            pq_m: Some(8),
            pq_k: Some(128),
            pq_opq_enabled: Some(true),
            rerank_enabled: Some(false),
            rerank_multiplier: Some(3),
            ..QuantizationOptions::default()
        };
        let core = opts.to_core().expect("valid pq section");
        match core.mode {
            QuantizationType::PQ {
                m,
                k,
                opq_enabled,
                oversampling,
            } => {
                assert_eq!(m, 8);
                assert_eq!(k, 128);
                assert!(opq_enabled);
                assert_eq!(oversampling, Some(4), "unset oversampling → core default");
            }
            other => panic!("expected PQ mode, got {other:?}"),
        }
        assert!(!core.rerank_enabled);
        assert_eq!(core.rerank_multiplier, 3);
    }

    #[test]
    fn quantization_options_pq_without_m_fails() {
        let opts = QuantizationOptions {
            mode: Some("pq".to_string()),
            ..QuantizationOptions::default()
        };
        init_python();
        let err = opts.to_core().expect_err("pq without pq_m must fail");
        assert!(err.to_string().contains("pq_m"));
    }

    #[test]
    fn quantization_options_pq_fields_without_pq_mode_fail() {
        let opts = QuantizationOptions {
            mode: Some("sq8".to_string()),
            pq_m: Some(8),
            ..QuantizationOptions::default()
        };
        init_python();
        let err = opts
            .to_core()
            .expect_err("pq fields with mode='sq8' must fail, not be dropped");
        assert!(err.to_string().contains("pq_"));
    }

    // -- Whole-config assembly + validation ------------------------------

    #[test]
    fn veles_config_options_to_core_applies_every_section() {
        let opts = VelesConfigOptions {
            limits: Some(LimitsOptions {
                max_collections: Some(5),
                ..LimitsOptions::default()
            }),
            search: Some(SearchConfigOptions {
                max_results: Some(42),
                ..SearchConfigOptions::default()
            }),
            hnsw: Some(HnswConfigOptions {
                m: Some(32),
                ..HnswConfigOptions::default()
            }),
            storage: Some(StorageOptions {
                mmap_cache_mb: Some(256),
                ..StorageOptions::default()
            }),
            quantization: Some(QuantizationOptions {
                mode: Some("sq8".to_string()),
                ..QuantizationOptions::default()
            }),
        };
        let core = opts.to_core().expect("valid full config");
        assert_eq!(core.limits.max_collections, 5);
        assert_eq!(core.search.max_results, 42);
        assert_eq!(core.hnsw.m, Some(32));
        assert_eq!(core.storage.mmap_cache_mb, 256);
        assert!(matches!(core.quantization.mode, QuantizationType::SQ8));
    }

    #[test]
    fn veles_config_options_to_core_validates_fail_fast() {
        let opts = VelesConfigOptions {
            search: Some(SearchConfigOptions {
                max_results: Some(0),
                ..SearchConfigOptions::default()
            }),
            ..VelesConfigOptions::default()
        };
        init_python();
        let err = opts.to_core().expect_err("max_results=0 must fail");
        assert!(err.to_string().contains("search.max_results"));
    }

    // -- TOML round-trip --------------------------------------------------

    #[test]
    fn from_core_round_trips_through_to_core() {
        let toml = "[limits]\nmax_collections = 7\n\n[search]\nmax_results = 42\n";
        let core = CoreVelesConfig::from_toml_engine_only(toml).expect("valid toml");
        let opts = VelesConfigOptions::from_core(&core).expect("mappable config");
        let back = opts.to_core().expect("round-trip stays valid");
        assert_eq!(back.limits.max_collections, 7);
        assert_eq!(back.search.max_results, 42);
        // Untouched engine defaults survive the round-trip:
        assert_eq!(back.storage.storage_mode, "mmap");
    }
}
