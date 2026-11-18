//! Fusion configuration types for hybrid search.
//!
//! This module defines fusion strategies and configurations
//! for combining vector and graph search results.

use serde::{Deserialize, Serialize};

/// Fusion strategy type for hybrid search (EPIC-040 US-005).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FusionStrategyType {
    /// Reciprocal Rank Fusion (default).
    #[default]
    Rrf,
    /// Weighted sum of normalized scores.
    Weighted,
    /// Take maximum score from either source.
    Maximum,
    /// Reciprocal Sparse Fusion for dense + sparse hybrid search.
    Rsf,
    /// Average score across all queries where the document appears.
    Average,
    /// Weighted Reciprocal Rank Fusion over per-branch weights — the named
    /// form of what the dense+sparse `weighted` path already computes
    /// (`dense_weight`/`sparse_weight`, defaults 0.5 / 0.5). Last variant:
    /// keeps every earlier discriminant stable for `as`-cast consumers.
    WeightedRrf,
}

/// USING FUSION clause for hybrid search (EPIC-040 US-005).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FusionClause {
    /// Fusion strategy (rrf, weighted, maximum, rsf, average).
    pub strategy: FusionStrategyType,
    /// RRF k parameter (default 60).
    pub k: Option<u32>,
    /// Vector weight for weighted fusion (0.0-1.0).
    pub vector_weight: Option<f64>,
    /// Graph weight for weighted fusion (0.0-1.0).
    pub graph_weight: Option<f64>,
    /// Dense vector weight for RSF fusion (0.0-1.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dense_weight: Option<f32>,
    /// Sparse vector weight for RSF fusion (0.0-1.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sparse_weight: Option<f32>,
}

impl Default for FusionClause {
    fn default() -> Self {
        Self {
            strategy: FusionStrategyType::Rrf,
            k: Some(60),
            vector_weight: None,
            graph_weight: None,
            dense_weight: None,
            sparse_weight: None,
        }
    }
}

impl FusionClause {
    /// The strategy that fuses a dense `NEAR` branch (branch 0) with a
    /// `SPARSE_NEAR` branch (branch 1) under this clause. Every engine running
    /// a hybrid dense + sparse search maps the clause through this one
    /// function — core's query pipeline and the WASM executor alike.
    ///
    /// `rrf`, `average` and `maximum` map directly. `rsf` takes the
    /// `dense_weight` / `sparse_weight` pair; `weighted` and `weighted_rrf`
    /// both build weighted Reciprocal Rank Fusion over the two branches from
    /// that pair and `k` — `weighted_rrf` is simply the honest name of the
    /// math `weighted` has always run here. A weight pair the strategy
    /// refuses (validate-time checks make it unreachable from a parsed
    /// query) falls back to RRF.
    #[must_use]
    pub fn dense_sparse_strategy(&self) -> crate::fusion::FusionStrategy {
        use crate::fusion::FusionStrategy;
        let dense = self.dense_weight.unwrap_or(0.5);
        let sparse = self.sparse_weight.unwrap_or(0.5);
        match self.strategy {
            FusionStrategyType::Rsf => FusionStrategy::relative_score(dense, sparse)
                .unwrap_or_else(|e| {
                    tracing::warn!(
                        dense_weight = dense,
                        sparse_weight = sparse,
                        error = %e,
                        "RSF fusion strategy invalid; falling back to RRF"
                    );
                    FusionStrategy::rrf_default()
                }),
            FusionStrategyType::Average => FusionStrategy::Average,
            FusionStrategyType::Maximum => FusionStrategy::Maximum,
            FusionStrategyType::Weighted | FusionStrategyType::WeightedRrf => {
                #[allow(clippy::cast_precision_loss)]
                let k = self.k.unwrap_or(60) as f32;
                FusionStrategy::weighted_rrf(vec![dense, sparse], k).unwrap_or_else(|e| {
                    tracing::warn!(
                        dense_weight = dense,
                        sparse_weight = sparse,
                        k,
                        error = %e,
                        "Weighted RRF fusion strategy invalid; falling back to RRF"
                    );
                    FusionStrategy::rrf_default()
                })
            }
            FusionStrategyType::Rrf => FusionStrategy::RRF {
                k: self.k.unwrap_or(60),
            },
        }
    }
}

/// Configuration for multi-vector fusion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FusionConfig {
    /// Fusion strategy name: "average", "maximum", "rrf", "weighted".
    pub strategy: String,
    /// Strategy-specific parameters.
    pub params: std::collections::HashMap<String, f64>,
}

impl Default for FusionConfig {
    fn default() -> Self {
        Self {
            strategy: "rrf".to_string(),
            params: std::collections::HashMap::new(),
        }
    }
}

impl FusionConfig {
    /// The strategy a `NEAR_FUSED` runs, on every engine (core's
    /// `multi_query_search`, the WASM executor).
    ///
    /// `rrf` (default), `average`, and `maximum` map directly. `weighted` /
    /// `rsf` take per-branch dense/sparse weights that are ill-defined for N
    /// homogeneous query vectors, so they — and any unknown strategy — fall
    /// back to RRF (matching the parser's default strategy).
    #[must_use]
    pub fn near_fused_strategy(&self) -> crate::fusion::FusionStrategy {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        // Reason: `k` is a small positive RRF constant the parser accepts.
        let k = self.params.get("k").map_or(60, |v| *v as u32);
        match self.strategy.to_lowercase().as_str() {
            "average" => crate::fusion::FusionStrategy::Average,
            "maximum" => crate::fusion::FusionStrategy::Maximum,
            _ => crate::fusion::FusionStrategy::RRF { k },
        }
    }

    /// Creates a new RRF fusion config with default k=60.
    #[must_use]
    pub fn rrf() -> Self {
        let mut params = std::collections::HashMap::new();
        params.insert("k".to_string(), 60.0);
        Self {
            strategy: "rrf".to_string(),
            params,
        }
    }

    /// Creates a weighted fusion config.
    ///
    /// # Errors
    ///
    /// Returns `Error::Config` if any weight is negative or if their sum
    /// is not approximately 1.0 (tolerance ±0.001).
    pub fn weighted(
        avg_weight: f64,
        max_weight: f64,
        hit_weight: f64,
    ) -> crate::error::Result<Self> {
        // Validate weights are non-negative
        if avg_weight < 0.0 || max_weight < 0.0 || hit_weight < 0.0 {
            return Err(crate::error::Error::Config(format!(
                "FusionConfig::weighted: all weights must be non-negative, \
                 got avg={avg_weight}, max={max_weight}, hit={hit_weight}"
            )));
        }

        // Validate weights sum to 1.0 (with tolerance for floating-point errors)
        let sum = avg_weight + max_weight + hit_weight;
        if (sum - 1.0).abs() >= 0.001 {
            return Err(crate::error::Error::Config(format!(
                "FusionConfig::weighted: weights must sum to 1.0, got sum={sum}"
            )));
        }

        let mut params = std::collections::HashMap::new();
        params.insert("avg_weight".to_string(), avg_weight);
        params.insert("max_weight".to_string(), max_weight);
        params.insert("hit_weight".to_string(), hit_weight);
        Ok(Self {
            strategy: "weighted".to_string(),
            params,
        })
    }
}
