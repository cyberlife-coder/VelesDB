//! Filter pushdown analysis for JOIN optimization (EPIC-031 US-006).
//!
//! This module analyzes WHERE conditions and classifies predicates by their
//! data source, enabling filters to be pushed down before JOIN operations
//! for significant performance improvements.

#![allow(dead_code)]

use crate::velesql::{Condition, JoinClause};
use std::collections::HashSet;
use std::hash::BuildHasher;

/// Result of pushdown analysis, classifying conditions by data source.
#[derive(Debug, Clone, Default)]
#[allow(clippy::struct_field_names)]
pub struct PushdownAnalysis {
    /// Filters that can be applied to `ColumnStore` before JOIN.
    pub column_store_filters: Vec<Condition>,
    /// Filters that should remain on graph traversal (pre-traversal).
    pub graph_filters: Vec<Condition>,
    /// Filters that must be applied after JOIN (cross-source predicates).
    pub post_join_filters: Vec<Condition>,
}

impl PushdownAnalysis {
    /// Creates an empty pushdown analysis.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if any filters can be pushed down.
    #[must_use]
    pub fn has_pushdown(&self) -> bool {
        !self.column_store_filters.is_empty()
    }

    /// Returns the total number of conditions analyzed.
    #[must_use]
    pub fn total_conditions(&self) -> usize {
        self.column_store_filters.len() + self.graph_filters.len() + self.post_join_filters.len()
    }
}

/// Analyzes a WHERE condition for filter pushdown optimization.
///
/// Classifies each predicate based on which data source it references:
/// - Column references matching JOIN tables → `ColumnStore` filters (pushdown)
/// - Graph variable references → Graph filters (pre-traversal)
/// - Mixed references → Post-JOIN filters
///
/// # Arguments
///
/// * `condition` - The WHERE condition to analyze
/// * `graph_vars` - Set of graph variable names from MATCH clause (e.g., {"t", "p"})
/// * `join_tables` - Set of table names from JOIN clauses (e.g., {"prices", "availability"})
///
/// # Returns
///
/// A `PushdownAnalysis` with conditions classified by source.
#[must_use]
pub fn analyze_for_pushdown<S1: BuildHasher, S2: BuildHasher>(
    condition: &Condition,
    graph_vars: &HashSet<String, S1>,
    join_tables: &HashSet<String, S2>,
) -> PushdownAnalysis {
    let mut analysis = PushdownAnalysis::new();
    classify_condition(condition, graph_vars, join_tables, &mut analysis);
    analysis
}

/// Extracts table names from JOIN clauses.
#[must_use]
pub fn extract_join_tables(joins: &[JoinClause]) -> HashSet<String> {
    let mut tables = HashSet::new();
    for join in joins {
        tables.insert(join.table.clone());
        if let Some(ref alias) = join.alias {
            tables.insert(alias.clone());
        }
    }
    tables
}

/// Classifies a condition and adds it to the appropriate category.
fn classify_condition<S1: BuildHasher, S2: BuildHasher>(
    condition: &Condition,
    graph_vars: &HashSet<String, S1>,
    join_tables: &HashSet<String, S2>,
    analysis: &mut PushdownAnalysis,
) {
    match condition {
        // AND: recursively classify both sides
        Condition::And(left, right) => {
            classify_condition(left, graph_vars, join_tables, analysis);
            classify_condition(right, graph_vars, join_tables, analysis);
        }

        // OR: must keep together, classify based on combined sources
        Condition::Or(left, right) => {
            let left_source = get_condition_source(left, graph_vars, join_tables);
            let right_source = get_condition_source(right, graph_vars, join_tables);

            match (left_source, right_source) {
                (Source::ColumnStore, Source::ColumnStore) => {
                    analysis.column_store_filters.push(condition.clone());
                }
                (Source::Graph, Source::Graph) => {
                    analysis.graph_filters.push(condition.clone());
                }
                _ => {
                    // Mixed OR must be post-join
                    analysis.post_join_filters.push(condition.clone());
                }
            }
        }

        // NOT: classify based on inner condition source
        Condition::Not(inner) => {
            let source = get_condition_source(inner, graph_vars, join_tables);
            match source {
                Source::ColumnStore => analysis.column_store_filters.push(condition.clone()),
                Source::Graph => analysis.graph_filters.push(condition.clone()),
                Source::Mixed | Source::Unknown => {
                    analysis.post_join_filters.push(condition.clone());
                }
            }
        }

        // Group: unwrap and classify inner
        Condition::Group(inner) => {
            classify_condition(inner, graph_vars, join_tables, analysis);
        }

        // Leaf conditions: classify by column reference
        _ => {
            let source = get_condition_source(condition, graph_vars, join_tables);
            match source {
                Source::ColumnStore => analysis.column_store_filters.push(condition.clone()),
                Source::Graph => analysis.graph_filters.push(condition.clone()),
                Source::Mixed | Source::Unknown => {
                    analysis.post_join_filters.push(condition.clone());
                }
            }
        }
    }
}

/// Data source for a condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    /// Condition references only `ColumnStore` columns.
    ColumnStore,
    /// Condition references only graph variables.
    Graph,
    /// Condition references both sources.
    Mixed,
    /// Unable to determine source.
    Unknown,
}

/// Determines the data source for a condition.
fn get_condition_source<S1: BuildHasher, S2: BuildHasher>(
    condition: &Condition,
    graph_vars: &HashSet<String, S1>,
    join_tables: &HashSet<String, S2>,
) -> Source {
    match condition {
        Condition::Comparison(cmp) => classify_column(&cmp.column, graph_vars, join_tables),

        Condition::In(inc) => classify_column(&inc.column, graph_vars, join_tables),

        Condition::Between(btw) => classify_column(&btw.column, graph_vars, join_tables),

        Condition::Like(like) => classify_column(&like.column, graph_vars, join_tables),

        Condition::IsNull(is_null) => classify_column(&is_null.column, graph_vars, join_tables),

        Condition::Match(m) => {
            // MATCH uses column for full-text search
            classify_column(&m.column, graph_vars, join_tables)
        }

        // Design: Vector conditions are classified as Graph because VelesDB stores
        // embeddings in the collection/graph layer. If future versions support vector
        // columns in ColumnStore, this classification will need to be extended.
        Condition::VectorSearch(_) | Condition::VectorFusedSearch(_) | Condition::Similarity(_) => {
            Source::Graph
        }

        Condition::And(left, right) | Condition::Or(left, right) => {
            let left_source = get_condition_source(left, graph_vars, join_tables);
            let right_source = get_condition_source(right, graph_vars, join_tables);
            combine_sources(left_source, right_source)
        }

        Condition::Not(inner) | Condition::Group(inner) => {
            get_condition_source(inner, graph_vars, join_tables)
        }
    }
}

/// Classifies a column reference to determine its data source.
///
/// Supports both simple column names and qualified names (table.column).
/// - Qualified names with known table → `ColumnStore` or Graph
/// - Qualified names with unknown table → Unknown (for post-join filtering)
/// - Unqualified names → Graph (collection columns)
fn classify_column<S1: BuildHasher, S2: BuildHasher>(
    column: &str,
    graph_vars: &HashSet<String, S1>,
    join_tables: &HashSet<String, S2>,
) -> Source {
    // Check for qualified name: table.column
    if let Some((table, _column)) = column.split_once('.') {
        if join_tables.contains(table) {
            return Source::ColumnStore;
        }
        if graph_vars.contains(table) {
            return Source::Graph;
        }
        // Unknown table prefix - cannot determine source
        return Source::Unknown;
    }

    // Design: Unqualified column names default to Graph (collection layer).
    // This follows SQL convention where unqualified names in JOIN queries refer
    // to the "main" table (here: the MATCH clause graph pattern).
    // Users must qualify ColumnStore columns explicitly: prices.amount, not just amount.
    Source::Graph
}

/// Combines two sources into a single classification.
///
/// Unknown is treated conservatively - combining with Unknown yields Unknown
/// to ensure conditions with unresolved references go to `post_join_filters`.
fn combine_sources(a: Source, b: Source) -> Source {
    match (a, b) {
        (Source::ColumnStore, Source::ColumnStore) => Source::ColumnStore,
        (Source::Graph, Source::Graph) => Source::Graph,
        // Unknown must propagate conservatively - don't inherit other source
        (Source::Unknown, _) | (_, Source::Unknown) => Source::Unknown,
        _ => Source::Mixed,
    }
}

// Tests moved to pushdown_tests.rs per project rules
