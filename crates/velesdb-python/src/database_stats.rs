//! Plan cache and collection statistics methods for the Python `Database` binding.

use pyo3::prelude::*;

use crate::database::Database;

#[pymethods]
impl Database {
    /// Get plan cache statistics.
    ///
    /// Returns a dict with keys:
    ///   - l1_size: Number of entries in L1 (hot) cache
    ///   - l2_size: Number of entries in L2 (LRU) cache
    ///   - l1_hits: L1 cache hits
    ///   - l2_hits: L2 cache hits (L1 miss, L2 hit)
    ///   - misses: Total cache misses
    ///   - hits: Total plan-level cache hits
    ///   - hit_rate: Hit rate as a float in [0.0, 1.0]
    ///
    /// Example:
    ///     >>> stats = db.plan_cache_stats()
    ///     >>> print(stats["hit_rate"])
    fn plan_cache_stats(&self, py: Python<'_>) -> PyResult<PyObject> {
        let cache = self.inner().plan_cache();
        let stats = cache.stats();
        let metrics = cache.metrics();

        let dict = pyo3::types::PyDict::new(py);
        dict.set_item("l1_size", stats.l1_size)?;
        dict.set_item("l2_size", stats.l2_size)?;
        dict.set_item("l1_hits", stats.l1_hits)?;
        dict.set_item("l2_hits", stats.l2_hits)?;
        dict.set_item("misses", stats.misses)?;
        dict.set_item("hits", metrics.hits())?;
        dict.set_item("hit_rate", metrics.hit_rate())?;
        Ok(dict.into())
    }

    /// Clear all cached query plans.
    ///
    /// Evicts all compiled plans from both L1 and L2 tiers.
    /// Hit/miss counters are not reset.
    ///
    /// Example:
    ///     >>> db.clear_plan_cache()
    fn clear_plan_cache(&self) {
        self.inner().plan_cache().clear();
    }
}
