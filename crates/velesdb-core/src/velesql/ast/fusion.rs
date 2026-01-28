//! Fusion configuration types for hybrid search.
//!
//! This module defines fusion strategies and configurations
//! for combining vector and graph search results.

use serde::{Deserialize, Serialize};

/// Fusion strategy type for hybrid search (EPIC-040 US-005).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FusionStrategyType {
    /// Reciprocal Rank Fusion (default).
    #[default]
    Rrf,
    /// Weighted sum of normalized scores.
    Weighted,
    /// Take maximum score from either source.
    Maximum,
}

/// USING FUSION clause for hybrid search (EPIC-040 US-005).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FusionClause {
    /// Fusion strategy (rrf, weighted, maximum).
    pub strategy: FusionStrategyType,
    /// RRF k parameter (default 60).
    pub k: Option<u32>,
    /// Vector weight for weighted fusion (0.0-1.0).
    pub vector_weight: Option<f64>,
    /// Graph weight for weighted fusion (0.0-1.0).
    pub graph_weight: Option<f64>,
}

impl Default for FusionClause {
    fn default() -> Self {
        Self {
            strategy: FusionStrategyType::Rrf,
            k: Some(60),
            vector_weight: None,
            graph_weight: None,
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
    #[must_use]
    pub fn weighted(avg_weight: f64, max_weight: f64, hit_weight: f64) -> Self {
        let mut params = std::collections::HashMap::new();
        params.insert("avg_weight".to_string(), avg_weight);
        params.insert("max_weight".to_string(), max_weight);
        params.insert("hit_weight".to_string(), hit_weight);
        Self {
            strategy: "weighted".to_string(),
            params,
        }
    }
}
