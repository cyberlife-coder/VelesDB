//! Plan generator for query optimization (EPIC-046 US-003).
//!
//! Generates alternative execution plans and selects the best one
//! based on cost estimation.

use super::cost_model::{CostEstimator, OperationCost};
use crate::collection::stats::CollectionStats;

/// Physical execution plan types.
#[derive(Debug, Clone)]
#[allow(missing_docs)] // Variant fields are self-explanatory
pub enum PhysicalPlan {
    /// Full sequential scan of collection
    SeqScan {
        /// Collection name
        collection: String,
        /// Estimated row count
        estimated_rows: u64,
    },
    /// Index-based lookup
    IndexScan {
        /// Collection name
        collection: String,
        /// Index name
        index_name: String,
        /// Filter selectivity (0.0-1.0)
        selectivity: f64,
    },
    /// HNSW vector similarity search
    VectorSearch {
        /// Collection name
        collection: String,
        /// Top-k results
        k: u64,
        /// ef_search parameter
        ef_search: u64,
    },
    /// Graph pattern traversal
    GraphTraversal {
        /// Collection name
        collection: String,
        /// Maximum traversal depth
        max_depth: u32,
        /// Result limit
        limit: u64,
    },
    /// Filter operation on input
    Filter {
        /// Input plan
        input: Box<PhysicalPlan>,
        /// Filter selectivity (0.0-1.0)
        selectivity: f64,
    },
    /// Limit/offset operation
    Limit {
        /// Input plan
        input: Box<PhysicalPlan>,
        /// Maximum rows
        limit: u64,
        /// Rows to skip
        offset: u64,
    },
}

impl PhysicalPlan {
    /// Returns the plan type name for display.
    #[must_use]
    pub fn plan_type(&self) -> &'static str {
        match self {
            Self::SeqScan { .. } => "SeqScan",
            Self::IndexScan { .. } => "IndexScan",
            Self::VectorSearch { .. } => "VectorSearch",
            Self::GraphTraversal { .. } => "GraphTraversal",
            Self::Filter { .. } => "Filter",
            Self::Limit { .. } => "Limit",
        }
    }
}

/// A candidate execution plan with its estimated cost.
#[derive(Debug, Clone)]
pub struct CandidatePlan {
    /// The physical execution plan
    pub plan: PhysicalPlan,
    /// Estimated cost
    pub cost: OperationCost,
    /// Human-readable description
    pub description: String,
}

impl CandidatePlan {
    /// Creates a new candidate plan.
    #[must_use]
    pub fn new(plan: PhysicalPlan, cost: OperationCost, description: impl Into<String>) -> Self {
        Self {
            plan,
            cost,
            description: description.into(),
        }
    }
}

/// Query characteristics for plan generation.
#[derive(Debug, Clone, Default)]
pub struct QueryCharacteristics {
    /// Collection name
    pub collection: String,
    /// Has vector similarity condition
    pub has_similarity: bool,
    /// Has graph MATCH clause
    pub has_match: bool,
    /// Has filter conditions
    pub has_filter: bool,
    /// Filter selectivity estimate (0.0-1.0)
    pub filter_selectivity: Option<f64>,
    /// Top-k for vector search
    pub top_k: Option<u64>,
    /// ef_search parameter
    pub ef_search: Option<u64>,
    /// Graph traversal depth
    pub max_depth: Option<u32>,
    /// Result limit
    pub limit: Option<u64>,
}

/// Generates and selects execution plans.
#[derive(Debug, Clone)]
pub struct PlanGenerator {
    estimator: CostEstimator,
}

impl Default for PlanGenerator {
    fn default() -> Self {
        Self::new(CostEstimator::default())
    }
}

impl PlanGenerator {
    /// Creates a new plan generator with the given cost estimator.
    #[must_use]
    pub fn new(estimator: CostEstimator) -> Self {
        Self { estimator }
    }

    /// Generates all applicable execution plans for a query.
    #[must_use]
    pub fn generate_plans(
        &self,
        query: &QueryCharacteristics,
        stats: &CollectionStats,
    ) -> Vec<CandidatePlan> {
        let mut plans = Vec::new();

        // Always consider full scan as baseline
        plans.push(self.generate_scan_plan(query, stats));

        // Consider index scans if filter is present
        if query.has_filter {
            if let Some(selectivity) = query.filter_selectivity {
                plans.extend(self.generate_index_plans(query, stats, selectivity));
            }
        }

        // Consider vector search if similarity in query
        if query.has_similarity {
            plans.push(self.generate_vector_plan(query, stats));
        }

        // Consider graph traversal if MATCH clause
        if query.has_match {
            plans.push(self.generate_graph_plan(query, stats));
        }

        // For hybrid queries, generate combined plans
        if query.has_similarity && query.has_match {
            plans.extend(self.generate_hybrid_plans(query, stats));
        }

        plans
    }

    /// Selects the best plan based on cost.
    #[must_use]
    pub fn select_best(&self, plans: Vec<CandidatePlan>) -> Option<CandidatePlan> {
        plans.into_iter().min_by(|a, b| {
            a.cost
                .total
                .partial_cmp(&b.cost.total)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }

    /// Generates and selects the best plan in one step.
    #[must_use]
    pub fn optimize(
        &self,
        query: &QueryCharacteristics,
        stats: &CollectionStats,
    ) -> Option<CandidatePlan> {
        let plans = self.generate_plans(query, stats);
        self.select_best(plans)
    }

    fn generate_scan_plan(
        &self,
        query: &QueryCharacteristics,
        stats: &CollectionStats,
    ) -> CandidatePlan {
        let mut cost = self.estimator.estimate_scan(stats);

        // Add filter cost if applicable
        if query.has_filter {
            let selectivity = query.filter_selectivity.unwrap_or(0.1);
            let filter_cost = self.estimator.estimate_filter(cost.rows, selectivity);
            cost = cost.then(filter_cost);
        }

        let plan = PhysicalPlan::SeqScan {
            collection: query.collection.clone(),
            estimated_rows: cost.rows,
        };

        CandidatePlan::new(plan, cost, "Full scan with optional filter")
    }

    fn generate_index_plans(
        &self,
        query: &QueryCharacteristics,
        stats: &CollectionStats,
        selectivity: f64,
    ) -> Vec<CandidatePlan> {
        let mut plans = Vec::new();

        for (name, index_stats) in &stats.index_stats {
            if name.starts_with("prop_") || name == "bm25_text" {
                let cost = self
                    .estimator
                    .estimate_index_lookup(index_stats, selectivity);

                let plan = PhysicalPlan::IndexScan {
                    collection: query.collection.clone(),
                    index_name: name.clone(),
                    selectivity,
                };

                plans.push(CandidatePlan::new(
                    plan,
                    cost,
                    format!("Index scan on {name}"),
                ));
            }
        }

        plans
    }

    fn generate_vector_plan(
        &self,
        query: &QueryCharacteristics,
        stats: &CollectionStats,
    ) -> CandidatePlan {
        let k = query.top_k.unwrap_or(10);
        let ef_search = query.ef_search.unwrap_or(100);

        let cost = self
            .estimator
            .estimate_vector_search(k, ef_search, stats.row_count);

        let plan = PhysicalPlan::VectorSearch {
            collection: query.collection.clone(),
            k,
            ef_search,
        };

        CandidatePlan::new(plan, cost, "HNSW vector search")
    }

    fn generate_graph_plan(
        &self,
        query: &QueryCharacteristics,
        stats: &CollectionStats,
    ) -> CandidatePlan {
        let max_depth = query.max_depth.unwrap_or(3);
        let limit = query.limit.unwrap_or(100);

        // Estimate average degree from stats
        let avg_degree = if stats.row_count > 0 {
            (stats
                .index_stats
                .get("hnsw_primary")
                .map_or(0, |i| i.entry_count) as f64
                / stats.row_count as f64)
                .max(2.0)
        } else {
            5.0
        };

        let cost = self
            .estimator
            .estimate_graph_traversal(avg_degree, max_depth, limit);

        let plan = PhysicalPlan::GraphTraversal {
            collection: query.collection.clone(),
            max_depth,
            limit,
        };

        CandidatePlan::new(plan, cost, "Graph pattern traversal")
    }

    fn generate_hybrid_plans(
        &self,
        query: &QueryCharacteristics,
        stats: &CollectionStats,
    ) -> Vec<CandidatePlan> {
        let mut plans = Vec::new();

        // Strategy 1: Vector-first, then graph filter
        let vector_plan = self.generate_vector_plan(query, stats);
        let graph_filter_cost = self.estimator.estimate_filter(
            vector_plan.cost.rows,
            0.5, // Assume 50% match graph pattern
        );
        let vector_first_cost = vector_plan.cost.then(graph_filter_cost);

        plans.push(CandidatePlan::new(
            PhysicalPlan::Filter {
                input: Box::new(vector_plan.plan.clone()),
                selectivity: 0.5,
            },
            vector_first_cost,
            "Vector search → Graph filter",
        ));

        // Strategy 2: Graph-first, then vector rerank
        let graph_plan = self.generate_graph_plan(query, stats);
        let vector_rerank_cost = self.estimator.estimate_vector_search(
            query.top_k.unwrap_or(10),
            query.ef_search.unwrap_or(50),
            graph_plan.cost.rows,
        );
        let graph_first_cost = graph_plan.cost.then(vector_rerank_cost);

        plans.push(CandidatePlan::new(
            PhysicalPlan::Filter {
                input: Box::new(graph_plan.plan.clone()),
                selectivity: 1.0,
            },
            graph_first_cost,
            "Graph traversal → Vector rerank",
        ));

        plans
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collection::stats::IndexStats;

    fn test_stats() -> CollectionStats {
        let mut stats = CollectionStats::with_counts(100_000, 0);
        stats.total_size_bytes = 100_000 * 256;
        stats.index_stats.insert(
            "hnsw_primary".to_string(),
            IndexStats::new("hnsw_primary", "HNSW").with_entry_count(100_000),
        );
        stats.index_stats.insert(
            "prop_category".to_string(),
            IndexStats::new("prop_category", "PropertyIndex")
                .with_entry_count(50)
                .with_depth(3),
        );
        stats
    }

    #[test]
    fn test_generate_scan_plan() {
        let generator = PlanGenerator::default();
        let stats = test_stats();

        let query = QueryCharacteristics {
            collection: "test".to_string(),
            ..Default::default()
        };

        let plans = generator.generate_plans(&query, &stats);

        assert!(!plans.is_empty());
        assert!(plans
            .iter()
            .any(|p| matches!(p.plan, PhysicalPlan::SeqScan { .. })));
    }

    #[test]
    fn test_generate_index_plan() {
        let generator = PlanGenerator::default();
        let stats = test_stats();

        let query = QueryCharacteristics {
            collection: "test".to_string(),
            has_filter: true,
            filter_selectivity: Some(0.01),
            ..Default::default()
        };

        let plans = generator.generate_plans(&query, &stats);

        assert!(plans
            .iter()
            .any(|p| matches!(p.plan, PhysicalPlan::IndexScan { .. })));
    }

    #[test]
    fn test_generate_vector_plan() {
        let generator = PlanGenerator::default();
        let stats = test_stats();

        let query = QueryCharacteristics {
            collection: "test".to_string(),
            has_similarity: true,
            top_k: Some(10),
            ef_search: Some(100),
            ..Default::default()
        };

        let plans = generator.generate_plans(&query, &stats);

        assert!(plans
            .iter()
            .any(|p| matches!(p.plan, PhysicalPlan::VectorSearch { .. })));
    }

    #[test]
    fn test_generate_hybrid_plans() {
        let generator = PlanGenerator::default();
        let stats = test_stats();

        let query = QueryCharacteristics {
            collection: "test".to_string(),
            has_similarity: true,
            has_match: true,
            top_k: Some(10),
            max_depth: Some(2),
            ..Default::default()
        };

        let plans = generator.generate_plans(&query, &stats);

        // Should have scan + vector + graph + 2 hybrid strategies
        assert!(plans.len() >= 4);
    }

    #[test]
    fn test_select_best_plan() {
        let generator = PlanGenerator::default();
        let stats = test_stats();

        let query = QueryCharacteristics {
            collection: "test".to_string(),
            has_similarity: true,
            has_filter: true,
            filter_selectivity: Some(0.01),
            top_k: Some(10),
            ..Default::default()
        };

        let best = generator.optimize(&query, &stats);

        assert!(best.is_some());
        let best = best.unwrap();
        // Vector search should typically win for similarity queries
        assert!(
            matches!(
                best.plan,
                PhysicalPlan::VectorSearch { .. } | PhysicalPlan::IndexScan { .. }
            ),
            "Expected VectorSearch or IndexScan, got {:?}",
            best.plan.plan_type()
        );
    }

    #[test]
    fn test_cost_ordering() {
        let generator = PlanGenerator::default();
        let stats = test_stats();

        let query = QueryCharacteristics {
            collection: "test".to_string(),
            has_filter: true,
            filter_selectivity: Some(0.001), // Very selective
            ..Default::default()
        };

        let plans = generator.generate_plans(&query, &stats);

        // Find scan and index plans
        let scan = plans
            .iter()
            .find(|p| matches!(p.plan, PhysicalPlan::SeqScan { .. }));
        let index = plans
            .iter()
            .find(|p| matches!(p.plan, PhysicalPlan::IndexScan { .. }));

        if let (Some(scan), Some(index)) = (scan, index) {
            assert!(
                index.cost.total < scan.cost.total,
                "Index should be cheaper for selective query"
            );
        }
    }
}
