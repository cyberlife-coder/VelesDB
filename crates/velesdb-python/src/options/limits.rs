//! `LimitsOptions`: global tenant-wide limits.

use pyo3::prelude::*;
use velesdb_core::config::LimitsConfig as CoreLimitsConfig;

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
    /// Maximum vectors per collection, enforced at ingest (GuardRail VELES-027).
    #[pyo3(get, set)]
    pub max_vectors_per_collection: Option<usize>,
    /// Maximum payload size in bytes, enforced at ingest (GuardRail VELES-027).
    #[pyo3(get, set)]
    pub max_payload_size: Option<usize>,
    /// Collection size above which a Perfect search is refused with a
    /// guard-rail error rather than scanned (GuardRail VELES-027).
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
    pub(super) fn from_core(core: &CoreLimitsConfig) -> Self {
        Self {
            max_collections: Some(core.max_collections),
            max_dimensions: Some(core.max_dimensions),
            max_vectors_per_collection: Some(core.max_vectors_per_collection),
            max_payload_size: Some(core.max_payload_size),
            max_perfect_mode_vectors: Some(core.max_perfect_mode_vectors),
        }
    }
}
