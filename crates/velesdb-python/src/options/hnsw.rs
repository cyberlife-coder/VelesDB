//! `HnswOptions`: per-collection HNSW parameters.

use pyo3::prelude::*;
use velesdb_core::index::hnsw::HnswParams;

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

    /// **Preset — turbo**: maximum insert throughput, at a lower recall.
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
