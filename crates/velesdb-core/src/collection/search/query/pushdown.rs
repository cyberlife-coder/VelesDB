//! Filter pushdown analysis for JOIN optimization (EPIC-031 US-006).
//!
//! This module analyzes WHERE conditions and classifies predicates by their
//! data source, enabling filters to be pushed down before JOIN operations
//! for significant performance improvements.

#![allow(dead_code)]

use crate::velesql::{Condition, JoinClause};
use std::collections::HashSet;

/// Result of pushdown analysis, classifying conditions by data source.
#[derive(Debug, Clone, Default)]
#[allow(clippy::struct_field_names)]
pub struct PushdownAnalysis {
    /// Filters that can be applied to ColumnStore before JOIN.
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
/// - Column references matching JOIN tables → ColumnStore filters (pushdown)
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
pub fn analyze_for_pushdown(
    condition: &Condition,
    graph_vars: &HashSet<String>,
    join_tables: &HashSet<String>,
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
fn classify_condition(
    condition: &Condition,
    graph_vars: &HashSet<String>,
    join_tables: &HashSet<String>,
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
    /// Condition references only ColumnStore columns.
    ColumnStore,
    /// Condition references only graph variables.
    Graph,
    /// Condition references both sources.
    Mixed,
    /// Unable to determine source.
    Unknown,
}

/// Determines the data source for a condition.
fn get_condition_source(
    condition: &Condition,
    graph_vars: &HashSet<String>,
    join_tables: &HashSet<String>,
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

        // Vector conditions are graph-based (applied to collection/graph)
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
/// - Qualified names with known table → ColumnStore or Graph
/// - Qualified names with unknown table → Unknown (for post-join filtering)
/// - Unqualified names → Graph (collection columns)
fn classify_column(
    column: &str,
    graph_vars: &HashSet<String>,
    join_tables: &HashSet<String>,
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

    // Simple column name: default to graph for unqualified names (collection columns)
    Source::Graph
}

/// Combines two sources into a single classification.
///
/// Unknown is treated conservatively - combining with Unknown yields Unknown
/// to ensure conditions with unresolved references go to post_join_filters.
fn combine_sources(a: Source, b: Source) -> Source {
    match (a, b) {
        (Source::ColumnStore, Source::ColumnStore) => Source::ColumnStore,
        (Source::Graph, Source::Graph) => Source::Graph,
        // Unknown must propagate conservatively - don't inherit other source
        (Source::Unknown, _) | (_, Source::Unknown) => Source::Unknown,
        _ => Source::Mixed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::velesql::{ColumnRef, CompareOp, Comparison, JoinCondition, Value};

    fn make_comparison(column: &str, value: i64) -> Condition {
        Condition::Comparison(Comparison {
            column: column.to_string(),
            operator: CompareOp::Eq,
            value: Value::Integer(value),
        })
    }

    fn make_graph_vars() -> HashSet<String> {
        let mut vars = HashSet::new();
        vars.insert("t".to_string());
        vars.insert("p".to_string());
        vars
    }

    fn make_join_tables() -> HashSet<String> {
        let mut tables = HashSet::new();
        tables.insert("prices".to_string());
        tables.insert("availability".to_string());
        tables
    }

    #[test]
    fn test_pushdown_analysis_new() {
        let analysis = PushdownAnalysis::new();
        assert!(!analysis.has_pushdown());
        assert_eq!(analysis.total_conditions(), 0);
    }

    #[test]
    fn test_classify_column_store_filter() {
        let graph_vars = make_graph_vars();
        let join_tables = make_join_tables();

        // prices.price should be ColumnStore
        let condition = make_comparison("prices.price", 1000);
        let analysis = analyze_for_pushdown(&condition, &graph_vars, &join_tables);

        assert!(analysis.has_pushdown());
        assert_eq!(analysis.column_store_filters.len(), 1);
        assert!(analysis.graph_filters.is_empty());
        assert!(analysis.post_join_filters.is_empty());
    }

    #[test]
    fn test_classify_graph_filter() {
        let graph_vars = make_graph_vars();
        let join_tables = make_join_tables();

        // t.category should be Graph (t is a graph variable)
        let condition = make_comparison("t.category", 1);
        let analysis = analyze_for_pushdown(&condition, &graph_vars, &join_tables);

        assert!(!analysis.has_pushdown());
        assert!(analysis.column_store_filters.is_empty());
        assert_eq!(analysis.graph_filters.len(), 1);
        assert!(analysis.post_join_filters.is_empty());
    }

    #[test]
    fn test_classify_simple_column_defaults_to_graph() {
        let graph_vars = make_graph_vars();
        let join_tables = make_join_tables();

        // Simple column name without table prefix defaults to graph
        let condition = make_comparison("category", 1);
        let analysis = analyze_for_pushdown(&condition, &graph_vars, &join_tables);

        assert!(!analysis.has_pushdown());
        assert_eq!(analysis.graph_filters.len(), 1);
    }

    #[test]
    fn test_classify_and_splits_conditions() {
        let graph_vars = make_graph_vars();
        let join_tables = make_join_tables();

        // AND of graph + column store should split
        let graph_filter = make_comparison("t.category", 1);
        let column_filter = make_comparison("prices.price", 1000);
        let condition = Condition::And(Box::new(graph_filter), Box::new(column_filter));

        let analysis = analyze_for_pushdown(&condition, &graph_vars, &join_tables);

        assert!(analysis.has_pushdown());
        assert_eq!(analysis.column_store_filters.len(), 1);
        assert_eq!(analysis.graph_filters.len(), 1);
        assert!(analysis.post_join_filters.is_empty());
    }

    #[test]
    fn test_classify_or_keeps_together() {
        let graph_vars = make_graph_vars();
        let join_tables = make_join_tables();

        // OR of graph + column store cannot be split
        let graph_filter = make_comparison("t.category", 1);
        let column_filter = make_comparison("prices.price", 1000);
        let condition = Condition::Or(Box::new(graph_filter), Box::new(column_filter));

        let analysis = analyze_for_pushdown(&condition, &graph_vars, &join_tables);

        assert!(!analysis.has_pushdown());
        assert!(analysis.column_store_filters.is_empty());
        assert!(analysis.graph_filters.is_empty());
        assert_eq!(analysis.post_join_filters.len(), 1);
    }

    #[test]
    fn test_classify_or_same_source() {
        let graph_vars = make_graph_vars();
        let join_tables = make_join_tables();

        // OR of two column store filters can be pushed down together
        let filter1 = make_comparison("prices.price", 1000);
        let filter2 = make_comparison("prices.discount", 10);
        let condition = Condition::Or(Box::new(filter1), Box::new(filter2));

        let analysis = analyze_for_pushdown(&condition, &graph_vars, &join_tables);

        assert!(analysis.has_pushdown());
        assert_eq!(analysis.column_store_filters.len(), 1);
    }

    #[test]
    fn test_extract_join_tables() {
        let joins = vec![JoinClause {
            table: "prices".to_string(),
            alias: Some("pr".to_string()),
            condition: JoinCondition {
                left: ColumnRef {
                    table: Some("prices".to_string()),
                    column: "trip_id".to_string(),
                },
                right: ColumnRef {
                    table: Some("t".to_string()),
                    column: "id".to_string(),
                },
            },
        }];

        let tables = extract_join_tables(&joins);

        assert!(tables.contains("prices"));
        assert!(tables.contains("pr"));
        assert_eq!(tables.len(), 2);
    }

    #[test]
    fn test_complex_nested_and() {
        let graph_vars = make_graph_vars();
        let join_tables = make_join_tables();

        // (t.a = 1 AND prices.b = 2) AND t.c = 3
        let inner_and = Condition::And(
            Box::new(make_comparison("t.a", 1)),
            Box::new(make_comparison("prices.b", 2)),
        );
        let condition = Condition::And(Box::new(inner_and), Box::new(make_comparison("t.c", 3)));

        let analysis = analyze_for_pushdown(&condition, &graph_vars, &join_tables);

        assert_eq!(analysis.column_store_filters.len(), 1); // prices.b
        assert_eq!(analysis.graph_filters.len(), 2); // t.a, t.c
    }

    #[test]
    fn test_unknown_table_prefix_goes_to_post_join() {
        let graph_vars = make_graph_vars();
        let join_tables = make_join_tables();

        // unknown_table.column should go to post_join_filters (not graph!)
        // This is a regression test for PR #86 review bug
        let condition = make_comparison("unknown_table.column", 1);
        let analysis = analyze_for_pushdown(&condition, &graph_vars, &join_tables);

        assert!(!analysis.has_pushdown());
        assert!(analysis.column_store_filters.is_empty());
        assert!(analysis.graph_filters.is_empty());
        assert_eq!(analysis.post_join_filters.len(), 1);
    }

    #[test]
    fn test_misspelled_table_goes_to_post_join() {
        let graph_vars = make_graph_vars();
        let join_tables = make_join_tables();

        // typo: "price" instead of "prices" should be caught
        let condition = make_comparison("price.value", 100);
        let analysis = analyze_for_pushdown(&condition, &graph_vars, &join_tables);

        assert!(!analysis.has_pushdown());
        assert_eq!(analysis.post_join_filters.len(), 1);
    }

    #[test]
    fn test_unknown_combined_with_known_stays_post_join() {
        let graph_vars = make_graph_vars();
        let join_tables = make_join_tables();

        // Regression test: AND(unknown_table.x, prices.y) should NOT go to column_store
        // because unknown_table is not resolvable - must stay in post_join
        let unknown_filter = make_comparison("unknown_table.x", 1);
        let known_filter = make_comparison("prices.y", 2);
        let condition = Condition::And(Box::new(unknown_filter), Box::new(known_filter));

        let analysis = analyze_for_pushdown(&condition, &graph_vars, &join_tables);

        // The known filter can be pushed, but the unknown one must go to post_join
        assert_eq!(analysis.column_store_filters.len(), 1); // prices.y
        assert_eq!(analysis.post_join_filters.len(), 1); // unknown_table.x
    }

    #[test]
    fn test_or_with_unknown_goes_to_post_join() {
        let graph_vars = make_graph_vars();
        let join_tables = make_join_tables();

        // OR(unknown_table.x, prices.y) cannot be split and must stay in post_join
        let unknown_filter = make_comparison("unknown_table.x", 1);
        let known_filter = make_comparison("prices.y", 2);
        let condition = Condition::Or(Box::new(unknown_filter), Box::new(known_filter));

        let analysis = analyze_for_pushdown(&condition, &graph_vars, &join_tables);

        // OR with unknown source must go to post_join (cannot be split)
        assert!(!analysis.has_pushdown());
        assert!(analysis.column_store_filters.is_empty());
        assert_eq!(analysis.post_join_filters.len(), 1);
    }
}
