//! `SearchConfigOptions`: the `[search]` engine config section.

use pyo3::prelude::*;
use velesdb_core::config::{SearchConfig as CoreSearchConfig, SearchMode};

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
    pub(super) fn from_core(core: &CoreSearchConfig) -> PyResult<Self> {
        Ok(Self {
            default_mode: Some(search_mode_str(core.default_mode)?.to_string()),
            ef_search: core.ef_search,
            max_results: Some(core.max_results),
            query_timeout_ms: Some(core.query_timeout_ms),
        })
    }
}
