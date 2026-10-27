//! `QuantizationOptions`: the `[quantization]` engine config section.

use pyo3::prelude::*;
use velesdb_core::config::{QuantizationConfig as CoreQuantizationConfig, QuantizationType};

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
    pub(super) fn from_core(core: &CoreQuantizationConfig) -> PyResult<Self> {
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
