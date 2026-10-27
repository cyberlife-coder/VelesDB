//! `VelesConfigOptions`: the database-wide configuration and its TOML loaders.

use super::hnsw_config::HnswConfigOptions;
use super::limits::LimitsOptions;
use super::quantization::QuantizationOptions;
use super::search::SearchConfigOptions;
use super::storage::StorageOptions;
use pyo3::prelude::*;
use std::path::PathBuf;
use velesdb_core::config::{ConfigError, VelesConfig as CoreVelesConfig};

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
/// There is no `wal_batch` section: the core removed that never-wired
/// table in 7.0.0 (#2174), and concurrent WAL writing is a
/// velesdb-premium Enterprise feature (see
/// `docs/guides/WRITE_CONCURRENCY.md`). A file that still carries
/// `[wal_batch]` loads; the core loader names the table in a warning and
/// applies nothing from it.
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
    /// `server`/`logging` are intentionally not carried over (see the
    /// type-level docs).
    pub(super) fn from_core(core: &CoreVelesConfig) -> PyResult<Self> {
        Ok(Self {
            limits: Some(LimitsOptions::from_core(&core.limits)),
            search: Some(SearchConfigOptions::from_core(&core.search)?),
            hnsw: Some(HnswConfigOptions::from_core(&core.hnsw)),
            storage: Some(StorageOptions::from_core(&core.storage)),
            quantization: Some(QuantizationOptions::from_core(&core.quantization)?),
        })
    }
}
