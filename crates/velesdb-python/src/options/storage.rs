//! `StorageOptions`: the `[storage]` engine config section.

use pyo3::prelude::*;
use velesdb_core::config::StorageConfig as CoreStorageConfig;

/// Storage engine settings mapped to
/// [`velesdb_core::config::StorageConfig`] (the `[storage]` TOML section).
/// Its one field does not reach the engine yet (core issue #2087).
/// Unspecified, it falls back to the engine default (`storage_mode="mmap"`).
///
/// `data_dir`, `mmap_cache_mb` and `vector_alignment` were removed in
/// 7.0.0 with the core fields they mirrored (#2087): none had an engine
/// counterpart. Passing one is a `TypeError`.
#[pyclass(module = "velesdb", from_py_object)]
#[derive(Clone, Debug, Default)]
pub struct StorageOptions {
    /// Storage mode: `"mmap"` or `"memory"`. Default: `"mmap"`. Reserved (core #2087): parsed but not yet applied.
    #[pyo3(get, set)]
    pub storage_mode: Option<String>,
}

#[pymethods]
impl StorageOptions {
    #[new]
    #[pyo3(signature = (storage_mode = None))]
    fn new(storage_mode: Option<String>) -> Self {
        Self { storage_mode }
    }

    fn __repr__(&self) -> String {
        format!("StorageOptions(storage_mode={:?})", self.storage_mode)
    }
}

impl StorageOptions {
    pub(crate) fn to_core(&self) -> CoreStorageConfig {
        let mut cfg = CoreStorageConfig::default();
        cfg.storage_mode = self.storage_mode.clone().unwrap_or(cfg.storage_mode);
        cfg
    }

    /// Builds a fully-populated options object from a core section
    /// (see [`LimitsOptions::from_core`]).
    pub(super) fn from_core(core: &CoreStorageConfig) -> Self {
        Self {
            storage_mode: Some(core.storage_mode.clone()),
        }
    }
}
