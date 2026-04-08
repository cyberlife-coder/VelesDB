//! `FusionStrategy` — PyO3-exported fusion strategy for combining multi-query results.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use velesdb_core::FusionStrategy as CoreFusionStrategy;

/// Fusion strategy for combining results from multiple vector searches.
///
/// Example:
///     >>> # Average fusion
///     >>> strategy = FusionStrategy.average()
///     >>> # RRF with default k=60
///     >>> strategy = FusionStrategy.rrf()
///     >>> # Weighted fusion
///     >>> strategy = FusionStrategy.weighted(avg_weight=0.6, max_weight=0.3, hit_weight=0.1)
#[pyclass(frozen)]
#[derive(Clone)]
pub struct FusionStrategy {
    inner: CoreFusionStrategy,
}

#[pymethods]
impl FusionStrategy {
    /// Create an Average fusion strategy.
    ///
    /// Computes the mean score for each document across all queries.
    ///
    /// Returns:
    ///     FusionStrategy: Average fusion strategy
    ///
    /// Example:
    ///     >>> strategy = FusionStrategy.average()
    #[staticmethod]
    fn average() -> Self {
        Self {
            inner: CoreFusionStrategy::Average,
        }
    }

    /// Create a Maximum fusion strategy.
    ///
    /// Takes the maximum score for each document across all queries.
    ///
    /// Returns:
    ///     FusionStrategy: Maximum fusion strategy
    ///
    /// Example:
    ///     >>> strategy = FusionStrategy.maximum()
    #[staticmethod]
    fn maximum() -> Self {
        Self {
            inner: CoreFusionStrategy::Maximum,
        }
    }

    /// Create a Reciprocal Rank Fusion (RRF) strategy.
    ///
    /// Uses position-based scoring: score = Σ 1/(k + rank)
    /// This is robust to score scale differences between queries.
    ///
    /// Args:
    ///     k: Ranking constant (default: 60). Lower k gives more weight to top ranks.
    ///
    /// Returns:
    ///     FusionStrategy: RRF fusion strategy
    ///
    /// Example:
    ///     >>> strategy = FusionStrategy.rrf()  # k=60
    ///     >>> strategy = FusionStrategy.rrf(k=30)  # More emphasis on top ranks
    #[staticmethod]
    #[pyo3(signature = (k = 60))]
    fn rrf(k: u32) -> Self {
        Self {
            inner: CoreFusionStrategy::RRF { k },
        }
    }

    /// Create a Weighted fusion strategy.
    ///
    /// Combines average score, maximum score, and hit ratio with custom weights.
    /// Formula: score = avg_weight * avg + max_weight * max + hit_weight * hit_ratio
    ///
    /// Args:
    ///     avg_weight: Weight for average score (0.0-1.0)
    ///     max_weight: Weight for maximum score (0.0-1.0)
    ///     hit_weight: Weight for hit ratio (0.0-1.0)
    ///
    /// Returns:
    ///     FusionStrategy: Weighted fusion strategy
    ///
    /// Raises:
    ///     ValueError: If weights don't sum to 1.0 or are negative
    ///
    /// Example:
    ///     >>> strategy = FusionStrategy.weighted(
    ///     ...     avg_weight=0.6,
    ///     ...     max_weight=0.3,
    ///     ...     hit_weight=0.1
    ///     ... )
    #[staticmethod]
    #[pyo3(signature = (avg_weight, max_weight, hit_weight))]
    fn weighted(avg_weight: f32, max_weight: f32, hit_weight: f32) -> PyResult<Self> {
        CoreFusionStrategy::weighted(avg_weight, max_weight, hit_weight)
            .map(|inner| Self { inner })
            .map_err(|e| PyValueError::new_err(format!("{e}")))
    }

    /// Create a Relative Score Fusion (RSF) strategy.
    ///
    /// Linearly combines dense and sparse scores with the given weights.
    /// Useful for hybrid dense+sparse search.
    ///
    /// Args:
    ///     dense_weight: Weight for dense vector scores (0.0-1.0)
    ///     sparse_weight: Weight for sparse scores (0.0-1.0)
    ///
    /// Returns:
    ///     FusionStrategy: Relative score fusion strategy
    ///
    /// Raises:
    ///     ValueError: If weights are invalid
    ///
    /// Example:
    ///     >>> strategy = FusionStrategy.relative_score(0.7, 0.3)
    #[staticmethod]
    #[pyo3(signature = (dense_weight, sparse_weight))]
    fn relative_score(dense_weight: f32, sparse_weight: f32) -> PyResult<Self> {
        CoreFusionStrategy::relative_score(dense_weight, sparse_weight)
            .map(|inner| Self { inner })
            .map_err(|e| PyValueError::new_err(format!("{e}")))
    }

    fn __repr__(&self) -> String {
        match &self.inner {
            CoreFusionStrategy::Average => "FusionStrategy.average()".to_string(),
            CoreFusionStrategy::Maximum => "FusionStrategy.maximum()".to_string(),
            CoreFusionStrategy::RRF { k } => format!("FusionStrategy.rrf(k={k})"),
            CoreFusionStrategy::Weighted {
                avg_weight,
                max_weight,
                hit_weight,
            } => format!(
                "FusionStrategy.weighted(avg_weight={avg_weight}, max_weight={max_weight}, hit_weight={hit_weight})"
            ),
            CoreFusionStrategy::RelativeScore {
                dense_weight,
                sparse_weight,
            } => format!(
                "FusionStrategy.relative_score(dense_weight={dense_weight}, sparse_weight={sparse_weight})"
            ),
            // Forward-compat: future variants added behind #[non_exhaustive].
            _ => format!("FusionStrategy(<unknown variant: {:?}>)", self.inner),
        }
    }
}

impl FusionStrategy {
    /// Get the inner CoreFusionStrategy.
    pub fn inner(&self) -> CoreFusionStrategy {
        self.inner.clone()
    }
}
