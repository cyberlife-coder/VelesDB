//! Query executor with plan optimization (EPIC-046 US-005).
//!
//! Integrates the plan generator with query execution,
//! providing automatic query optimization.

use super::plan_generator::{CandidatePlan, PlanGenerator, QueryCharacteristics};
use crate::collection::stats::CollectionStats;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// LRU-style plan cache for reusing query plans.
#[derive(Debug)]
pub struct PlanCache {
    cache: RwLock<HashMap<u64, CachedPlan>>,
    max_entries: usize,
}

#[derive(Debug, Clone)]
struct CachedPlan {
    plan: CandidatePlan,
    access_count: u64,
}

impl Default for PlanCache {
    fn default() -> Self {
        Self::new(1000)
    }
}

impl PlanCache {
    /// Creates a new plan cache with the given capacity.
    #[must_use]
    pub fn new(max_entries: usize) -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            max_entries,
        }
    }

    /// Gets a cached plan if available.
    #[must_use]
    pub fn get(&self, key: u64) -> Option<CandidatePlan> {
        let mut cache = self.cache.write();
        if let Some(entry) = cache.get_mut(&key) {
            entry.access_count += 1;
            return Some(entry.plan.clone());
        }
        None
    }

    /// Inserts a plan into the cache.
    pub fn insert(&self, key: u64, plan: CandidatePlan) {
        let mut cache = self.cache.write();

        // Evict if at capacity
        if cache.len() >= self.max_entries {
            // Simple eviction: remove least accessed
            if let Some((&evict_key, _)) = cache.iter().min_by_key(|(_, v)| v.access_count) {
                cache.remove(&evict_key);
            }
        }

        cache.insert(
            key,
            CachedPlan {
                plan,
                access_count: 1,
            },
        );
    }

    /// Invalidates all plans for a collection.
    pub fn invalidate_collection(&self, collection: &str) {
        let mut cache = self.cache.write();
        cache.retain(|_, v| {
            // Use exact match via plan traversal (avoids substring false positives)
            !Self::plan_references_collection(&v.plan.plan, collection)
        });
    }

    /// Recursively checks if a plan references a collection.
    fn plan_references_collection(
        plan: &super::plan_generator::PhysicalPlan,
        collection: &str,
    ) -> bool {
        match plan {
            super::plan_generator::PhysicalPlan::SeqScan { collection: c, .. }
            | super::plan_generator::PhysicalPlan::IndexScan { collection: c, .. }
            | super::plan_generator::PhysicalPlan::VectorSearch { collection: c, .. }
            | super::plan_generator::PhysicalPlan::GraphTraversal { collection: c, .. } => {
                c == collection
            }
            super::plan_generator::PhysicalPlan::Filter { input, .. }
            | super::plan_generator::PhysicalPlan::Limit { input, .. } => {
                Self::plan_references_collection(input, collection)
            }
        }
    }

    /// Clears all cached plans.
    pub fn clear(&self) {
        self.cache.write().clear();
    }

    /// Returns the number of cached plans.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cache.read().len()
    }

    /// Returns true if the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cache.read().is_empty()
    }
}

/// Query optimizer that generates and caches execution plans.
#[derive(Debug)]
pub struct QueryOptimizer {
    generator: PlanGenerator,
    cache: PlanCache,
}

impl Default for QueryOptimizer {
    fn default() -> Self {
        Self::new(PlanGenerator::default(), PlanCache::default())
    }
}

impl QueryOptimizer {
    /// Creates a new query optimizer.
    #[must_use]
    pub fn new(generator: PlanGenerator, cache: PlanCache) -> Self {
        Self { generator, cache }
    }

    /// Optimizes a query, returning the best execution plan.
    ///
    /// Uses cached plan if available, otherwise generates and caches a new one.
    #[must_use]
    pub fn optimize(
        &self,
        query: &QueryCharacteristics,
        stats: &CollectionStats,
    ) -> Option<CandidatePlan> {
        let cache_key = Self::compute_cache_key(query);

        // Check cache first
        if let Some(cached) = self.cache.get(cache_key) {
            return Some(cached);
        }

        // Generate new plan
        let best = self.generator.optimize(query, stats)?;

        // Cache it
        self.cache.insert(cache_key, best.clone());

        Some(best)
    }

    /// Generates all candidate plans without caching.
    #[must_use]
    pub fn generate_all_plans(
        &self,
        query: &QueryCharacteristics,
        stats: &CollectionStats,
    ) -> Vec<CandidatePlan> {
        self.generator.generate_plans(query, stats)
    }

    /// Invalidates cached plans for a collection (call after DDL).
    pub fn invalidate(&self, collection: &str) {
        self.cache.invalidate_collection(collection);
    }

    /// Clears all cached plans.
    pub fn clear_cache(&self) {
        self.cache.clear();
    }

    /// Returns cache statistics.
    #[must_use]
    pub fn cache_size(&self) -> usize {
        self.cache.len()
    }

    fn compute_cache_key(query: &QueryCharacteristics) -> u64 {
        use std::collections::hash_map::DefaultHasher;

        let mut hasher = DefaultHasher::new();
        query.collection.hash(&mut hasher);
        query.has_similarity.hash(&mut hasher);
        query.has_match.hash(&mut hasher);
        query.has_filter.hash(&mut hasher);

        if let Some(sel) = query.filter_selectivity {
            // Quantize selectivity to reduce cache misses with bounds checking
            // Clamp to [0.0, 1.0] to prevent overflow on invalid values
            let clamped = sel.clamp(0.0, 1.0);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let quantized = (clamped * 100.0) as u32;
            quantized.hash(&mut hasher);
        }

        if let Some(k) = query.top_k {
            k.hash(&mut hasher);
        }

        // Include ef_search, max_depth, limit in cache key (PR #152 bug fix)
        if let Some(ef) = query.ef_search {
            ef.hash(&mut hasher);
        }

        if let Some(depth) = query.max_depth {
            depth.hash(&mut hasher);
        }

        if let Some(lim) = query.limit {
            lim.hash(&mut hasher);
        }

        hasher.finish()
    }
}

/// Execution context with optimizer integration.
#[derive(Debug)]
pub struct ExecutionContext {
    optimizer: QueryOptimizer,
}

impl Default for ExecutionContext {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecutionContext {
    /// Creates a new execution context.
    #[must_use]
    pub fn new() -> Self {
        Self {
            optimizer: QueryOptimizer::default(),
        }
    }

    /// Returns the optimizer for direct access.
    #[must_use]
    pub fn optimizer(&self) -> &QueryOptimizer {
        &self.optimizer
    }

    /// Plans and returns the best execution strategy.
    #[must_use]
    pub fn plan_query(
        &self,
        query: &QueryCharacteristics,
        stats: &CollectionStats,
    ) -> Option<CandidatePlan> {
        self.optimizer.optimize(query, stats)
    }

    /// Generates an EXPLAIN output for a query.
    #[must_use]
    #[allow(clippy::items_after_statements)]
    pub fn explain(&self, query: &QueryCharacteristics, stats: &CollectionStats) -> String {
        use std::fmt::Write;

        let plans = self.optimizer.generate_all_plans(query, stats);

        if plans.is_empty() {
            return "No plans generated".to_string();
        }

        let mut output = String::new();
        output.push_str("Query Plan Analysis\n");
        output.push_str("===================\n\n");

        for (i, plan) in plans.iter().enumerate() {
            let _ = writeln!(
                output,
                "Plan {}: {} (cost: {:.2})",
                i + 1,
                plan.plan.plan_type(),
                plan.cost.total
            );
            let _ = writeln!(output, "  Description: {}", plan.description);
            let _ = writeln!(output, "  Estimated rows: {}", plan.cost.rows);
            let _ = writeln!(output, "  Startup cost: {:.2}\n", plan.cost.startup);
        }

        if let Some(best) = self.optimizer.optimize(query, stats) {
            let _ = writeln!(
                output,
                "Selected: {} (lowest cost: {:.2})",
                best.plan.plan_type(),
                best.cost.total
            );
        }

        output
    }
}

#[cfg(test)]
#[path = "query_executor_tests.rs"]
mod tests;
