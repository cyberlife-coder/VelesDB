//! `HnswConfigOptions`: the `[hnsw]` engine config section.

use pyo3::prelude::*;
use velesdb_core::config::HnswConfig as CoreHnswConfig;

/// Global HNSW index defaults mapped to
/// [`velesdb_core::config::HnswConfig`] (the `[hnsw]` TOML section).
///
/// Distinct from the per-collection [`HnswOptions`], which wins over it.
/// `m` / `ef_construction` apply at collection creation (issue #2087):
/// per-collection option > this section > auto by dimension, fixing topology
/// for new collections only. `max_layers` is reserved — applied by nothing.
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
    pub(super) fn from_core(core: &CoreHnswConfig) -> Self {
        Self {
            m: core.m,
            ef_construction: core.ef_construction,
            max_layers: Some(core.max_layers),
        }
    }
}
