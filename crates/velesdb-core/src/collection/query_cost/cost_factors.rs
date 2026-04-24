//! Cost factor types and hardware profiles (EPIC-046 US-002).
//!
//! Defines [`OperationCostFactors`] — the per-operation weights used by the
//! CBO — along with hardware-profile constructors (`ssd_optimized`,
//! `in_memory`, `hdd_optimized`), safety bounds (`CostFactorBounds`, private),
//! and the `clamp_with_log` (private) utility. Extracted from `cost_model.rs`
//! to keep each file under the 500-NLOC limit.

// Reason: Numeric casts in cost model are intentional:
// - All casts are for cost estimation/statistics (not user data)
// - f64->f32 precision loss acceptable for query planning heuristics
// - Values are bounded by collection stats (cardinality, vector dimensions)
// - Cost estimates are approximate by design (order-of-magnitude accuracy)
#![allow(clippy::cast_precision_loss)]

use serde::{Deserialize, Serialize};
use tracing::debug;

/// Facteurs de coût pour les différentes opérations du CBO.
///
/// Ces valeurs sont calibrées dynamiquement à partir des statistiques de la
/// collection lors de `analyze()`. Les constructeurs statiques (`default()`,
/// `ssd_optimized()`, `in_memory()`, `hdd_optimized()`) fournissent des
/// bases pré-configurées pour différents profils matériels.
///
/// # Bornes de sécurité
///
/// Chaque facteur est borné dans un intervalle pour éviter les estimations
/// dégénérées (voir `CostFactorBounds`, privé au crate).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationCostFactors {
    /// Coût par accès séquentiel de page (8 KB). Borné dans \[0.01, 10.0\].
    #[serde(default = "default_seq_page_cost")]
    pub seq_page_cost: f64,
    /// Coût par accès aléatoire de page. Borné dans \[0.1, 20.0\].
    #[serde(default = "default_random_page_cost")]
    pub random_page_cost: f64,
    /// Coût CPU par tuple traité. Borné dans \[0.001, 0.1\].
    #[serde(default = "default_cpu_tuple_cost")]
    pub cpu_tuple_cost: f64,
    /// Coût CPU par lookup d'index. Borné dans \[0.001, 0.05\].
    #[serde(default = "default_cpu_index_cost")]
    pub cpu_index_cost: f64,
    /// Coût CPU par calcul de distance vectorielle. Borné dans \[0.01, 1.0\].
    #[serde(default = "default_cpu_distance_cost")]
    pub cpu_distance_cost: f64,
    /// Coût CPU par traversée d'arête de graphe. Borné dans \[0.005, 0.2\].
    #[serde(default = "default_cpu_edge_cost")]
    pub cpu_edge_cost: f64,
}

/// Returns the default value for `seq_page_cost`.
fn default_seq_page_cost() -> f64 {
    1.0
}

/// Returns the default value for `random_page_cost`.
fn default_random_page_cost() -> f64 {
    4.0
}

/// Returns the default value for `cpu_tuple_cost`.
fn default_cpu_tuple_cost() -> f64 {
    0.01
}

/// Returns the default value for `cpu_index_cost`.
fn default_cpu_index_cost() -> f64 {
    0.005
}

/// Returns the default value for `cpu_distance_cost`.
fn default_cpu_distance_cost() -> f64 {
    0.1
}

/// Returns the default value for `cpu_edge_cost`.
fn default_cpu_edge_cost() -> f64 {
    0.02
}

impl Default for OperationCostFactors {
    fn default() -> Self {
        Self {
            seq_page_cost: default_seq_page_cost(),
            random_page_cost: default_random_page_cost(),
            cpu_tuple_cost: default_cpu_tuple_cost(),
            cpu_index_cost: default_cpu_index_cost(),
            cpu_distance_cost: default_cpu_distance_cost(),
            cpu_edge_cost: default_cpu_edge_cost(),
        }
    }
}

/// Bornes de sécurité pour les facteurs de coût calibrés.
///
/// Empêche les estimations dégénérées causées par des statistiques aberrantes.
/// Chaque borne est un tuple `(min, max)` inclusif.
pub(crate) struct CostFactorBounds;

impl CostFactorBounds {
    /// Bornes pour `seq_page_cost`.
    pub const SEQ_PAGE_COST: (f64, f64) = (0.01, 10.0);
    /// Bornes pour `random_page_cost`.
    pub const RANDOM_PAGE_COST: (f64, f64) = (0.1, 20.0);
    /// Bornes pour `cpu_tuple_cost`.
    pub const CPU_TUPLE_COST: (f64, f64) = (0.001, 0.1);
    /// Bornes pour `cpu_index_cost`.
    pub const CPU_INDEX_COST: (f64, f64) = (0.001, 0.05);
    /// Bornes pour `cpu_distance_cost`.
    pub const CPU_DISTANCE_COST: (f64, f64) = (0.01, 1.0);
    /// Bornes pour `cpu_edge_cost`.
    pub const CPU_EDGE_COST: (f64, f64) = (0.005, 0.2);
}

/// Clamps `value` into `[min, max]` and emits a `debug!` log if clamped.
pub(crate) fn clamp_with_log(name: &str, value: f64, bounds: (f64, f64)) -> f64 {
    let clamped = value.clamp(bounds.0, bounds.1);
    if (clamped - value).abs() > f64::EPSILON {
        debug!(
            field = name,
            original = value,
            clamped = clamped,
            "cost factor clamped to bounds"
        );
    }
    clamped
}

impl OperationCostFactors {
    /// Creates factors optimized for SSD storage.
    ///
    /// SSDs have lower random access penalty compared to HDDs.
    #[must_use]
    pub fn ssd_optimized() -> Self {
        Self {
            random_page_cost: 1.5,
            ..Default::default()
        }
    }

    /// Creates factors optimized for in-memory operations.
    ///
    /// Both sequential and random page costs are minimal.
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            seq_page_cost: 0.1,
            random_page_cost: 0.1,
            ..Default::default()
        }
    }

    /// Creates factors optimized for HDD storage (rotational disks).
    ///
    /// `random_page_cost = 8.0` reflects the seek latency of rotational disks.
    /// `seq_page_cost = 1.0` remains standard (sequential reads are efficient on HDD).
    #[must_use]
    pub fn hdd_optimized() -> Self {
        Self {
            random_page_cost: 8.0,
            ..Default::default()
        }
    }

    /// Applies safety bounds to all cost factors.
    ///
    /// Each factor is clamped into its allowed interval defined by
    /// `CostFactorBounds` (private). Emits a `debug!` log for every clamped field.
    #[must_use]
    pub fn clamped(self) -> Self {
        Self {
            seq_page_cost: clamp_with_log(
                "seq_page_cost",
                self.seq_page_cost,
                CostFactorBounds::SEQ_PAGE_COST,
            ),
            random_page_cost: clamp_with_log(
                "random_page_cost",
                self.random_page_cost,
                CostFactorBounds::RANDOM_PAGE_COST,
            ),
            cpu_tuple_cost: clamp_with_log(
                "cpu_tuple_cost",
                self.cpu_tuple_cost,
                CostFactorBounds::CPU_TUPLE_COST,
            ),
            cpu_index_cost: clamp_with_log(
                "cpu_index_cost",
                self.cpu_index_cost,
                CostFactorBounds::CPU_INDEX_COST,
            ),
            cpu_distance_cost: clamp_with_log(
                "cpu_distance_cost",
                self.cpu_distance_cost,
                CostFactorBounds::CPU_DISTANCE_COST,
            ),
            cpu_edge_cost: clamp_with_log(
                "cpu_edge_cost",
                self.cpu_edge_cost,
                CostFactorBounds::CPU_EDGE_COST,
            ),
        }
    }

    /// Returns `true` if all factors equal the default values.
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ssd_optimized_factors() {
        let factors = OperationCostFactors::ssd_optimized();
        assert!(factors.random_page_cost < OperationCostFactors::default().random_page_cost);
    }

    #[test]
    fn test_hdd_optimized_factors() {
        let factors = OperationCostFactors::hdd_optimized();
        assert!(factors.random_page_cost > OperationCostFactors::default().random_page_cost);
    }

    #[test]
    fn test_in_memory_factors() {
        let factors = OperationCostFactors::in_memory();
        assert!(factors.seq_page_cost < OperationCostFactors::default().seq_page_cost);
        assert!(factors.random_page_cost < OperationCostFactors::default().random_page_cost);
    }

    #[test]
    fn test_is_default() {
        assert!(OperationCostFactors::default().is_default());
        assert!(!OperationCostFactors::ssd_optimized().is_default());
    }

    #[test]
    fn test_clamped_within_bounds() {
        let extreme = OperationCostFactors {
            seq_page_cost: 999.0,
            random_page_cost: -1.0,
            cpu_tuple_cost: 0.0,
            cpu_index_cost: 100.0,
            cpu_distance_cost: 0.0,
            cpu_edge_cost: 0.0,
        };
        let clamped = extreme.clamped();
        assert!((clamped.seq_page_cost - CostFactorBounds::SEQ_PAGE_COST.1).abs() < f64::EPSILON);
        assert!(
            (clamped.random_page_cost - CostFactorBounds::RANDOM_PAGE_COST.0).abs() < f64::EPSILON
        );
        assert!((clamped.cpu_tuple_cost - CostFactorBounds::CPU_TUPLE_COST.0).abs() < f64::EPSILON);
        assert!((clamped.cpu_index_cost - CostFactorBounds::CPU_INDEX_COST.1).abs() < f64::EPSILON);
    }

    #[test]
    fn test_clamp_with_log_no_change() {
        let result = clamp_with_log("test", 5.0, (1.0, 10.0));
        assert!((result - 5.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_clamp_with_log_clamps_low() {
        let result = clamp_with_log("test", -1.0, (0.0, 10.0));
        assert!(result.abs() < f64::EPSILON);
    }

    #[test]
    fn test_clamp_with_log_clamps_high() {
        let result = clamp_with_log("test", 99.0, (0.0, 10.0));
        assert!((result - 10.0).abs() < f64::EPSILON);
    }
}
