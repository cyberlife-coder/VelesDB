//! The fusion strategy *name* grammar: the one reading of `"rrf"`, `"avg"`,
//! `"weighted"`… that every surface taking a strategy by name delegates to.
//!
//! Before this module each surface (REST, CLI, REPL, Tauri, WASM) carried its
//! own `match` over the name, and they drifted: different alias sets,
//! case-sensitive in one place, and the CLI's `weighted` frozen on the
//! pre-#1545 weights. The names, their aliases, the case rule and the
//! defaults a name implies now live here, and the weights go through the
//! validating constructors.

use super::strategy::{
    FusionError, FusionStrategy, DEFAULT_RELATIVE_SCORE_DENSE_WEIGHT,
    DEFAULT_RELATIVE_SCORE_SPARSE_WEIGHT, DEFAULT_RRF_K, DEFAULT_WEIGHTED_AVG_WEIGHT,
    DEFAULT_WEIGHTED_HIT_WEIGHT, DEFAULT_WEIGHTED_MAX_WEIGHT,
};

/// The names [`FusionStrategy::from_name`] accepts, as its refusal lists them.
pub(super) const FUSION_STRATEGY_NAMES: &str = "'rrf', 'weighted_rrf', \
     'rsf' (alias: 'relative_score'), 'average' (alias: 'avg'), \
     'maximum' (alias: 'max'), 'weighted'";

/// The parameters a strategy name may carry, for [`FusionStrategy::from_name`].
///
/// Every field is optional: an absent one takes core's canonical default
/// ([`DEFAULT_RRF_K`], the `DEFAULT_WEIGHTED_*` weights, and the
/// `DEFAULT_RELATIVE_SCORE_*` weights). A field the named strategy does not
/// read is ignored, so a caller can forward a whole request body whatever
/// strategy it names.
///
/// `#[non_exhaustive]` so a strategy gaining a parameter is not a breaking
/// change: build one from [`FusionParams::default`] and the `with_*` setters.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[non_exhaustive]
pub struct FusionParams {
    /// `rrf`: the ranking constant `k`.
    pub rrf_k: Option<u32>,
    /// `weighted`: the weight of the average-score component.
    pub avg_weight: Option<f32>,
    /// `weighted`: the weight of the maximum-score component.
    pub max_weight: Option<f32>,
    /// `weighted`: the weight of the hit-ratio component.
    pub hit_weight: Option<f32>,
    /// `relative_score`: the weight of the dense branch.
    pub dense_weight: Option<f32>,
    /// `relative_score`: the weight of the sparse branch.
    pub sparse_weight: Option<f32>,
}

impl FusionParams {
    /// Sets the `rrf` ranking constant.
    #[must_use]
    pub const fn with_rrf_k(mut self, k: Option<u32>) -> Self {
        self.rrf_k = k;
        self
    }

    /// Sets the `weighted` average-score weight.
    #[must_use]
    pub const fn with_avg_weight(mut self, weight: Option<f32>) -> Self {
        self.avg_weight = weight;
        self
    }

    /// Sets the `weighted` maximum-score weight.
    #[must_use]
    pub const fn with_max_weight(mut self, weight: Option<f32>) -> Self {
        self.max_weight = weight;
        self
    }

    /// Sets the `weighted` hit-ratio weight.
    #[must_use]
    pub const fn with_hit_weight(mut self, weight: Option<f32>) -> Self {
        self.hit_weight = weight;
        self
    }

    /// Sets the `relative_score` dense-branch weight.
    #[must_use]
    pub const fn with_dense_weight(mut self, weight: Option<f32>) -> Self {
        self.dense_weight = weight;
        self
    }

    /// Sets the `relative_score` sparse-branch weight.
    #[must_use]
    pub const fn with_sparse_weight(mut self, weight: Option<f32>) -> Self {
        self.sparse_weight = weight;
        self
    }

    /// The `relative_score` pair. The two weights must sum to 1.0, so one
    /// given alone implies the other; neither given is the canonical pair.
    fn relative_score_weights(self) -> (f32, f32) {
        match (self.dense_weight, self.sparse_weight) {
            (Some(dense), Some(sparse)) => (dense, sparse),
            (Some(dense), None) => (dense, 1.0 - dense),
            (None, Some(sparse)) => (1.0 - sparse, sparse),
            (None, None) => (
                DEFAULT_RELATIVE_SCORE_DENSE_WEIGHT,
                DEFAULT_RELATIVE_SCORE_SPARSE_WEIGHT,
            ),
        }
    }
}

impl FusionStrategy {
    /// Builds the strategy a name stands for, with `params` filling its
    /// parameters.
    ///
    /// | Name | Aliases | Parameters (default) |
    /// |---|---|---|
    /// | `rrf` | — | `rrf_k` ([`DEFAULT_RRF_K`]) |
    /// | `weighted_rrf` | — | `rrf_k` ([`DEFAULT_RRF_K`]) + one weight per branch, passed to [`from_name_with_weights`] |
    /// | `average` | `avg` | — |
    /// | `maximum` | `max` | — |
    /// | `weighted` | — | `avg_weight`, `max_weight`, `hit_weight` (the `DEFAULT_WEIGHTED_*` constants, each on its own) |
    /// | `relative_score` | `rsf` | `dense_weight`, `sparse_weight` (0.5 / 0.5; one given alone implies its complement) |
    ///
    /// Names are case-insensitive and otherwise exact: no trimming, no prefix.
    ///
    /// # Errors
    ///
    /// Returns [`FusionError::UnknownStrategy`] for a name outside the table,
    /// [`FusionError::MissingWeights`] for `weighted_rrf` without weights,
    /// and the error of [`FusionStrategy::weighted`] or
    /// [`FusionStrategy::relative_score`] for weights they refuse.
    ///
    /// [`from_name_with_weights`]: FusionStrategy::from_name_with_weights
    pub fn from_name(name: &str, params: FusionParams) -> Result<Self, FusionError> {
        Self::from_name_with_weights(name, params, None)
    }

    /// Builds the strategy a name stands for, with `params` filling its scalar
    /// parameters and `weights` carrying `weighted_rrf`'s per-branch weights.
    ///
    /// `weights` is the companion channel the vector parameter needs:
    /// [`FusionParams`] stays `Copy`, so it cannot hold the per-branch
    /// vector — the one parameter `weighted_rrf` cannot default. As with
    /// scalar params, weights a named strategy does not read are ignored.
    ///
    /// # Errors
    ///
    /// Same contract as [`FusionStrategy::from_name`].
    // `rrf_k` is a rank offset in the same magnitude as `RRF`'s `u32` k —
    // far below 2^24, so the u32→f32 widening is exact.
    #[allow(clippy::cast_precision_loss)]
    pub fn from_name_with_weights(
        name: &str,
        params: FusionParams,
        weights: Option<&[f32]>,
    ) -> Result<Self, FusionError> {
        match name.to_lowercase().as_str() {
            "rrf" => Ok(Self::RRF {
                k: params.rrf_k.unwrap_or(DEFAULT_RRF_K),
            }),
            "weighted_rrf" => {
                let weights = weights.filter(|w| !w.is_empty()).ok_or_else(|| {
                    FusionError::MissingWeights {
                        name: name.to_string(),
                    }
                })?;
                Self::weighted_rrf(
                    weights.to_vec(),
                    params.rrf_k.unwrap_or(DEFAULT_RRF_K) as f32,
                )
            }
            "average" | "avg" => Ok(Self::Average),
            "maximum" | "max" => Ok(Self::Maximum),
            "weighted" => Self::weighted(
                params.avg_weight.unwrap_or(DEFAULT_WEIGHTED_AVG_WEIGHT),
                params.max_weight.unwrap_or(DEFAULT_WEIGHTED_MAX_WEIGHT),
                params.hit_weight.unwrap_or(DEFAULT_WEIGHTED_HIT_WEIGHT),
            ),
            "relative_score" | "rsf" => {
                let (dense, sparse) = params.relative_score_weights();
                Self::relative_score(dense, sparse)
            }
            _ => Err(FusionError::UnknownStrategy {
                name: name.to_string(),
            }),
        }
    }
}
