//! Subquery optimization hints and strategies (EPIC-039/US-004).
//!
//! Provides configuration and hints for optimizing subquery execution.

use serde::{Deserialize, Serialize};

/// Optimization strategy for subquery execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[allow(dead_code)]
pub enum SubqueryStrategy {
    /// Execute subquery once and cache result (default for non-correlated).
    #[default]
    CacheResult,
    /// Execute subquery for each outer row (required for correlated).
    PerRow,
    /// Rewrite subquery as JOIN if possible.
    RewriteAsJoin,
    /// Materialize subquery results in temp table.
    Materialize,
}

/// Configuration for subquery optimization.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct SubqueryOptConfig {
    /// Maximum cached results before switching to per-row.
    pub cache_threshold: usize,
    /// Enable automatic strategy selection.
    pub auto_optimize: bool,
    /// Enable subquery-to-join rewriting.
    pub enable_join_rewrite: bool,
}

impl Default for SubqueryOptConfig {
    fn default() -> Self {
        Self {
            cache_threshold: 10_000,
            auto_optimize: true,
            enable_join_rewrite: false, // Conservative default
        }
    }
}

#[allow(dead_code)]
impl SubqueryOptConfig {
    /// Creates a new configuration with defaults.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an aggressive optimization configuration.
    #[must_use]
    pub fn aggressive() -> Self {
        Self {
            cache_threshold: 100_000,
            auto_optimize: true,
            enable_join_rewrite: true,
        }
    }
}

/// Hint for subquery execution based on analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct SubqueryHint {
    /// Recommended execution strategy.
    pub strategy: SubqueryStrategy,
    /// Is this a correlated subquery?
    pub is_correlated: bool,
    /// Estimated result cardinality (if known).
    pub estimated_cardinality: Option<usize>,
    /// Can be cached across rows?
    pub cacheable: bool,
}

#[allow(dead_code)]
impl SubqueryHint {
    /// Creates a hint for a non-correlated subquery.
    #[must_use]
    pub fn non_correlated() -> Self {
        Self {
            strategy: SubqueryStrategy::CacheResult,
            is_correlated: false,
            estimated_cardinality: None,
            cacheable: true,
        }
    }

    /// Creates a hint for a correlated subquery.
    #[must_use]
    pub fn correlated() -> Self {
        Self {
            strategy: SubqueryStrategy::PerRow,
            is_correlated: true,
            estimated_cardinality: None,
            cacheable: false,
        }
    }

    /// Analyzes a subquery and returns optimization hints.
    #[must_use]
    pub fn analyze(correlation_count: usize, _config: &SubqueryOptConfig) -> Self {
        if correlation_count > 0 {
            Self::correlated()
        } else {
            Self::non_correlated()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_strategy_is_cache() {
        assert_eq!(SubqueryStrategy::default(), SubqueryStrategy::CacheResult);
    }

    #[test]
    fn test_config_default() {
        let config = SubqueryOptConfig::default();
        assert_eq!(config.cache_threshold, 10_000);
        assert!(config.auto_optimize);
        assert!(!config.enable_join_rewrite);
    }

    #[test]
    fn test_config_aggressive() {
        let config = SubqueryOptConfig::aggressive();
        assert!(config.enable_join_rewrite);
        assert_eq!(config.cache_threshold, 100_000);
    }

    #[test]
    fn test_hint_non_correlated() {
        let hint = SubqueryHint::non_correlated();
        assert!(!hint.is_correlated);
        assert!(hint.cacheable);
        assert_eq!(hint.strategy, SubqueryStrategy::CacheResult);
    }

    #[test]
    fn test_hint_correlated() {
        let hint = SubqueryHint::correlated();
        assert!(hint.is_correlated);
        assert!(!hint.cacheable);
        assert_eq!(hint.strategy, SubqueryStrategy::PerRow);
    }

    #[test]
    fn test_analyze_with_correlations() {
        let config = SubqueryOptConfig::default();
        let hint = SubqueryHint::analyze(2, &config);
        assert!(hint.is_correlated);
        assert_eq!(hint.strategy, SubqueryStrategy::PerRow);
    }

    #[test]
    fn test_analyze_without_correlations() {
        let config = SubqueryOptConfig::default();
        let hint = SubqueryHint::analyze(0, &config);
        assert!(!hint.is_correlated);
        assert_eq!(hint.strategy, SubqueryStrategy::CacheResult);
    }
}
