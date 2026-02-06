//! Query cost estimation module.
//!
//! This module provides cost estimation for vector queries before execution,
//! allowing rejection of expensive queries or parameter adjustment.
//!
//! # Features
//!
//! - **Cost estimation**: Predict query cost based on dataset size, ef_search, etc.
//! - **Latency estimation**: Convert cost to estimated milliseconds
//! - **Cost limits**: Reject queries exceeding max_cost threshold
//! - **EXPLAIN support**: Provide cost breakdown for VelesQL queries
//!
//! # Example
//!
//! ```ignore
//! use velesdb_core::collection::query_cost::{QueryCostEstimator, QueryParams};
//!
//! let estimator = QueryCostEstimator::default();
//! let params = QueryParams {
//!     dataset_size: 100_000,
//!     ef_search: 128,
//!     top_k: 10,
//!     filter_selectivity: Some(0.1),
//! };
//!
//! let estimate = estimator.estimate(&params);
//! println!("Estimated cost: {}", estimate.total_cost);
//! println!("Estimated latency: {}ms", estimate.estimated_latency_ms);
//! ```

// SAFETY: Numeric casts in query cost estimation are intentional:
// - All casts are for cost modeling and latency estimation
// - f64/usize conversions for computing operation costs
// - Values bounded by dataset size and hardware limits
// - Precision loss acceptable for cost estimates (approximate by design)
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]

use std::fmt;

pub mod cost_model;
pub mod plan_generator;
pub mod query_executor;

#[cfg(test)]
mod tests;

pub use cost_model::{CostEstimator, OperationCost, OperationCostFactors};
pub use plan_generator::{CandidatePlan, PhysicalPlan, PlanGenerator, QueryCharacteristics};
pub use query_executor::{ExecutionContext, PlanCache, QueryOptimizer};

/// Parameters for cost estimation
#[derive(Debug, Clone)]
pub struct QueryParams {
    /// Number of vectors in the dataset
    pub dataset_size: usize,
    /// ef_search parameter for HNSW
    pub ef_search: usize,
    /// Number of results requested
    pub top_k: usize,
    /// Filter selectivity (0.0-1.0, fraction of vectors passing filter)
    /// None means no filter (selectivity = 1.0)
    pub filter_selectivity: Option<f64>,
}

impl Default for QueryParams {
    fn default() -> Self {
        Self {
            dataset_size: 10_000,
            ef_search: 128,
            top_k: 10,
            filter_selectivity: None,
        }
    }
}

impl QueryParams {
    /// Creates new query params
    #[must_use]
    pub fn new(dataset_size: usize, ef_search: usize, top_k: usize) -> Self {
        Self {
            dataset_size,
            ef_search,
            top_k,
            filter_selectivity: None,
        }
    }

    /// Sets filter selectivity
    #[must_use]
    pub fn with_filter_selectivity(mut self, selectivity: f64) -> Self {
        self.filter_selectivity = Some(selectivity.clamp(0.001, 1.0));
        self
    }
}

/// Breakdown of cost factors
#[derive(Debug, Clone)]
pub struct CostFactors {
    /// Cost from dataset size (O(log n) for HNSW)
    pub dataset_size_factor: f64,
    /// Cost from ef_search parameter
    pub ef_search_factor: f64,
    /// Cost reduction from filter selectivity
    pub filter_selectivity_factor: f64,
    /// Cost from top_k (sub-linear)
    pub top_k_factor: f64,
}

impl Default for CostFactors {
    fn default() -> Self {
        Self {
            dataset_size_factor: 1.0,
            ef_search_factor: 1.0,
            filter_selectivity_factor: 1.0,
            top_k_factor: 1.0,
        }
    }
}

/// Estimated cost of a query
#[derive(Debug, Clone)]
pub struct QueryCostEstimate {
    /// Total estimated cost (abstract units)
    pub total_cost: f64,
    /// Estimated latency in milliseconds
    pub estimated_latency_ms: f64,
    /// Breakdown of cost factors
    pub factors: CostFactors,
}

impl QueryCostEstimate {
    /// Creates a new estimate
    #[must_use]
    pub fn new(total_cost: f64, estimated_latency_ms: f64, factors: CostFactors) -> Self {
        Self {
            total_cost,
            estimated_latency_ms,
            factors,
        }
    }
}

/// Calibration constants for cost estimation
#[derive(Debug, Clone)]
pub struct CostCalibration {
    /// Base cost unit (normalized to 1.0)
    pub base_cost: f64,
    /// Reference ef_search for normalization (default 100)
    pub reference_ef_search: f64,
    /// Reference top_k for normalization (default 10)
    pub reference_top_k: f64,
    /// Milliseconds per cost unit (calibrated via benchmarks)
    pub ms_per_cost_unit: f64,
    /// Exponent for filter selectivity impact (0.3 = mild impact)
    pub filter_exponent: f64,
}

impl Default for CostCalibration {
    fn default() -> Self {
        Self {
            base_cost: 1.0,
            reference_ef_search: 100.0,
            reference_top_k: 10.0,
            ms_per_cost_unit: 0.1,
            filter_exponent: 0.3,
        }
    }
}

impl CostCalibration {
    /// Creates calibration for fast systems (lower latency per cost)
    #[must_use]
    pub fn fast_system() -> Self {
        Self {
            ms_per_cost_unit: 0.05,
            ..Default::default()
        }
    }

    /// Creates calibration for slow systems (higher latency per cost)
    #[must_use]
    pub fn slow_system() -> Self {
        Self {
            ms_per_cost_unit: 0.2,
            ..Default::default()
        }
    }
}

/// Error when query cost exceeds limit
#[derive(Debug, Clone)]
pub struct QueryCostExceeded {
    /// Estimated cost
    pub estimated: f64,
    /// Maximum allowed cost
    pub max_allowed: f64,
}

impl fmt::Display for QueryCostExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Query cost {:.1} exceeds limit {:.1}",
            self.estimated, self.max_allowed
        )
    }
}

impl std::error::Error for QueryCostExceeded {}

/// Query cost estimator
#[derive(Debug, Clone)]
pub struct QueryCostEstimator {
    /// Calibration constants
    calibration: CostCalibration,
    /// Optional maximum cost limit for the collection
    max_cost: Option<f64>,
}

impl Default for QueryCostEstimator {
    fn default() -> Self {
        Self::new(CostCalibration::default())
    }
}

impl QueryCostEstimator {
    /// Creates a new estimator with the given calibration
    #[must_use]
    pub fn new(calibration: CostCalibration) -> Self {
        Self {
            calibration,
            max_cost: None,
        }
    }

    /// Creates an estimator with a cost limit
    #[must_use]
    pub fn with_max_cost(mut self, max_cost: f64) -> Self {
        self.max_cost = Some(max_cost);
        self
    }

    /// Sets the maximum allowed cost
    pub fn set_max_cost(&mut self, max_cost: Option<f64>) {
        self.max_cost = max_cost;
    }

    /// Gets the current max cost limit
    #[must_use]
    pub fn max_cost(&self) -> Option<f64> {
        self.max_cost
    }

    /// Estimates the cost of a query
    ///
    /// # Cost Formula
    ///
    /// ```text
    /// cost = base_cost
    ///      * log2(dataset_size + 1)      // O(log n) HNSW traversal
    ///      * (ef_search / 100)           // Linear with ef_search
    ///      * sqrt(top_k / 10)            // Sub-linear with k
    ///      * (1.0 / selectivity)^0.3     // Filter overhead
    /// ```
    #[must_use]
    pub fn estimate(&self, params: &QueryParams) -> QueryCostEstimate {
        let cal = &self.calibration;

        // Dataset size factor: O(log n) for HNSW
        let dataset_size_factor = if params.dataset_size > 0 {
            (params.dataset_size as f64 + 1.0).log2()
        } else {
            1.0
        };

        // ef_search factor: linear scaling
        let ef_search_factor = params.ef_search as f64 / cal.reference_ef_search;

        // top_k factor: sub-linear (sqrt)
        let top_k_factor = (params.top_k as f64 / cal.reference_top_k).sqrt();

        // Filter selectivity factor: inverse relationship with exponent
        let selectivity = params.filter_selectivity.unwrap_or(1.0).max(0.001);
        let filter_selectivity_factor = (1.0 / selectivity).powf(cal.filter_exponent);

        // Total cost
        let total_cost = cal.base_cost
            * dataset_size_factor
            * ef_search_factor
            * top_k_factor
            * filter_selectivity_factor;

        // Estimated latency
        let estimated_latency_ms = total_cost * cal.ms_per_cost_unit;

        let factors = CostFactors {
            dataset_size_factor,
            ef_search_factor,
            filter_selectivity_factor,
            top_k_factor,
        };

        QueryCostEstimate::new(total_cost, estimated_latency_ms, factors)
    }

    /// Checks if query exceeds max cost
    ///
    /// # Errors
    ///
    /// Returns `QueryCostExceeded` if the estimated cost exceeds `max_cost`.
    pub fn check_cost_limit(
        &self,
        params: &QueryParams,
        max_cost: f64,
    ) -> Result<QueryCostEstimate, QueryCostExceeded> {
        let estimate = self.estimate(params);

        if estimate.total_cost > max_cost {
            Err(QueryCostExceeded {
                estimated: estimate.total_cost,
                max_allowed: max_cost,
            })
        } else {
            Ok(estimate)
        }
    }

    /// Checks if query exceeds the collection's max cost (if set)
    ///
    /// # Errors
    ///
    /// Returns `QueryCostExceeded` if max_cost is set and exceeded.
    pub fn check_collection_limit(
        &self,
        params: &QueryParams,
    ) -> Result<QueryCostEstimate, QueryCostExceeded> {
        let estimate = self.estimate(params);

        if let Some(max) = self.max_cost {
            if estimate.total_cost > max {
                return Err(QueryCostExceeded {
                    estimated: estimate.total_cost,
                    max_allowed: max,
                });
            }
        }

        Ok(estimate)
    }

    /// Generates an EXPLAIN-style breakdown
    #[must_use]
    pub fn explain(&self, params: &QueryParams) -> String {
        let estimate = self.estimate(params);

        format!(
            "Query Cost Estimate\n\
             ===================\n\
             Total Cost: {:.2}\n\
             Estimated Latency: {:.2}ms\n\n\
             Cost Breakdown:\n\
             - Dataset Size Factor (log2({})): {:.2}\n\
             - ef_search Factor ({}/{}): {:.2}\n\
             - top_k Factor (sqrt({}/10)): {:.2}\n\
             - Filter Selectivity Factor: {:.2}\n",
            estimate.total_cost,
            estimate.estimated_latency_ms,
            params.dataset_size,
            estimate.factors.dataset_size_factor,
            params.ef_search,
            self.calibration.reference_ef_search as usize,
            estimate.factors.ef_search_factor,
            params.top_k,
            estimate.factors.top_k_factor,
            estimate.factors.filter_selectivity_factor,
        )
    }
}

/// Builder for convenient query param construction
#[derive(Debug, Default)]
pub struct QueryParamsBuilder {
    params: QueryParams,
}

impl QueryParamsBuilder {
    /// Creates a new builder
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets dataset size
    #[must_use]
    pub fn dataset_size(mut self, size: usize) -> Self {
        self.params.dataset_size = size;
        self
    }

    /// Sets ef_search
    #[must_use]
    pub fn ef_search(mut self, ef: usize) -> Self {
        self.params.ef_search = ef;
        self
    }

    /// Sets top_k
    #[must_use]
    pub fn top_k(mut self, k: usize) -> Self {
        self.params.top_k = k;
        self
    }

    /// Sets filter selectivity
    #[must_use]
    pub fn filter_selectivity(mut self, selectivity: f64) -> Self {
        self.params.filter_selectivity = Some(selectivity.clamp(0.001, 1.0));
        self
    }

    /// Builds the QueryParams
    #[must_use]
    pub fn build(self) -> QueryParams {
        self.params
    }
}
