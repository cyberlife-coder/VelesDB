//! Auto-index suggestion based on query pattern tracking (EPIC-047 US-005).
//!
//! Tracks query patterns and suggests indexes that would improve performance.

// Reason: Numeric casts in property indexing are intentional:
// - u128->u64 for millisecond timestamps: values fit within u64 range
// - u64/usize->f64 for statistics: precision loss acceptable for query planning
// - All values are bounded by collection sizes and query counts
// - Used for index selection heuristics, not financial calculations
// EPIC-047 US-005 — auto-index advisor based on query pattern tracking.
// Fields are initialized in Collection; query-side methods are wired
// but not yet called from the MATCH pipeline.
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Predicate types for query pattern tracking.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum PredicateType {
    /// Equality check (=)
    Equality,
    /// Range comparison (>, <, >=, <=)
    Range,
    /// IN list
    In,
    /// LIKE pattern
    Like,
}

/// A query pattern for index suggestion analysis.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QueryPattern {
    /// Labels involved
    pub labels: Vec<String>,
    /// Properties filtered on
    pub properties: Vec<String>,
    /// Types of predicates used
    pub predicates: Vec<PredicateType>,
}

/// Statistics for a query pattern.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PatternStats {
    /// Number of times this pattern was seen
    pub count: u64,
    /// Total execution time in milliseconds
    pub total_time_ms: u64,
    /// Average execution time
    pub avg_time_ms: f64,
    /// Last seen timestamp (unix millis)
    pub last_seen_ms: u64,
}

/// Tracks query patterns for index suggestion.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct QueryPatternTracker {
    /// Pattern -> stats mapping
    patterns: HashMap<QueryPattern, PatternStats>,
    /// Threshold for slow query (ms)
    slow_query_threshold_ms: u64,
}

impl QueryPatternTracker {
    /// Creates a new tracker with default threshold (100ms).
    #[must_use]
    pub fn new() -> Self {
        Self {
            patterns: HashMap::new(),
            slow_query_threshold_ms: 100,
        }
    }

    /// Sets the slow query threshold.
    #[allow(dead_code)] // Reason: Public API — used by tests and future auto-tuning configuration
    pub fn set_threshold(&mut self, threshold_ms: u64) {
        self.slow_query_threshold_ms = threshold_ms;
    }

    /// Records a query execution.
    pub fn record(&mut self, pattern: QueryPattern, execution_time_ms: u64) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);

        let stats = self.patterns.entry(pattern).or_default();
        stats.count += 1;
        stats.total_time_ms += execution_time_ms;
        #[allow(clippy::cast_precision_loss)]
        {
            stats.avg_time_ms = stats.total_time_ms as f64 / stats.count as f64;
        }
        stats.last_seen_ms = now_ms;
    }

    /// Returns patterns sorted by total time (most expensive first).
    #[must_use]
    pub fn expensive_patterns(&self) -> Vec<(&QueryPattern, &PatternStats)> {
        let mut patterns: Vec<_> = self.patterns.iter().collect();
        patterns.sort_by_key(|b| std::cmp::Reverse(b.1.total_time_ms));
        patterns
    }

    /// Returns patterns that are slow (above threshold).
    #[must_use]
    #[allow(dead_code)] // Reason: Public API — used by tests and future EXPLAIN/ANALYZE output
    pub fn slow_patterns(&self) -> Vec<(&QueryPattern, &PatternStats)> {
        self.patterns
            .iter()
            .filter(|(_, stats)| stats.avg_time_ms > self.slow_query_threshold_ms as f64)
            .collect()
    }
}

/// An index suggestion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexSuggestion {
    /// DDL statement to create the index
    pub ddl: String,
    /// The pattern this would help
    pub pattern: QueryPattern,
    /// Estimated improvement (0.0 to 1.0)
    pub estimated_improvement: f64,
    /// Number of queries that would benefit
    pub query_count: u64,
    /// Priority score (higher = more important)
    pub priority_score: f64,
}

/// Advisor that suggests indexes based on query patterns.
#[derive(Debug, Default)]
pub struct IndexAdvisor {
    /// Existing index names (to avoid duplicates)
    existing_indexes: std::collections::HashSet<String>,
}

impl IndexAdvisor {
    /// Creates a new advisor.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an existing index.
    #[allow(dead_code)] // Reason: Public API — used by tests and future DDL CREATE INDEX handler
    pub fn register_index(&mut self, name: impl Into<String>) {
        self.existing_indexes.insert(name.into());
    }

    /// Generates suggestions from tracked patterns.
    #[must_use]
    pub fn suggest(&self, tracker: &QueryPatternTracker) -> Vec<IndexSuggestion> {
        let mut suggestions = Vec::new();

        for (pattern, stats) in tracker.expensive_patterns() {
            // Skip if no properties to index
            if pattern.properties.is_empty() || pattern.labels.is_empty() {
                continue;
            }

            let index_name = format!(
                "idx_{}_{}",
                pattern.labels.join("_").to_lowercase(),
                pattern.properties.join("_").to_lowercase()
            );

            // Skip if index already exists
            if self.existing_indexes.contains(&index_name) {
                continue;
            }

            // Estimate improvement based on predicate type
            let improvement = Self::estimate_improvement(pattern);
            if improvement < 0.2 {
                continue;
            }

            // Calculate priority: frequency * improvement * avg_time
            let priority = stats.count as f64 * improvement * stats.avg_time_ms;

            let ddl = format!(
                "CREATE INDEX {} ON :{}({})",
                index_name,
                pattern.labels.first().unwrap_or(&String::new()),
                pattern.properties.join(", ")
            );

            suggestions.push(IndexSuggestion {
                ddl,
                pattern: pattern.clone(),
                estimated_improvement: improvement,
                query_count: stats.count,
                priority_score: priority,
            });
        }

        // Sort by priority (highest first)
        suggestions.sort_by(|a, b| {
            b.priority_score
                .partial_cmp(&a.priority_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        suggestions
    }

    /// Estimates improvement from adding an index.
    fn estimate_improvement(pattern: &QueryPattern) -> f64 {
        let mut improvement = 0.0;

        for pred in &pattern.predicates {
            match pred {
                PredicateType::Equality => improvement += 0.9,
                PredicateType::Range => improvement += 0.7,
                PredicateType::In => improvement += 0.6,
                PredicateType::Like => improvement += 0.3,
            }
        }

        // Normalize to 0.0-1.0
        (improvement / pattern.predicates.len().max(1) as f64).min(1.0)
    }
}
