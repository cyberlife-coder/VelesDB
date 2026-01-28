//! Abstract Syntax Tree (AST) for VelesQL queries.
//!
//! This module defines the data structures representing parsed VelesQL queries.

mod aggregation;
mod condition;
mod fusion;
mod join;
mod select;
mod values;
mod with_clause;

use serde::{Deserialize, Serialize};

// Re-export all types for backward compatibility
pub use aggregation::{
    AggregateArg, AggregateFunction, AggregateType, GroupByClause, HavingClause, HavingCondition,
    LogicalOp,
};
pub use condition::{
    BetweenCondition, CompareOp, Comparison, Condition, InCondition, IsNullCondition,
    LikeCondition, MatchCondition, SimilarityCondition, VectorFusedSearch, VectorSearch,
};
pub use fusion::{FusionClause, FusionConfig, FusionStrategyType};
pub use join::{ColumnRef, JoinClause, JoinCondition, JoinType};
pub use select::{
    Column, DistinctMode, OrderByExpr, SelectColumns, SelectOrderBy, SelectStatement,
    SimilarityOrderBy,
};
pub use values::{
    CorrelatedColumn, IntervalUnit, IntervalValue, Subquery, TemporalExpr, Value, VectorExpr,
};
pub use with_clause::{WithClause, WithOption, WithValue};

/// A complete VelesQL query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Query {
    /// The SELECT statement.
    pub select: SelectStatement,
    /// Compound query (UNION/INTERSECT/EXCEPT) - EPIC-040 US-006.
    #[serde(default)]
    pub compound: Option<CompoundQuery>,
    /// MATCH clause for graph pattern matching (EPIC-045 US-001).
    #[serde(default)]
    pub match_clause: Option<crate::velesql::MatchClause>,
}

impl Query {
    /// Returns true if this is a MATCH query.
    #[must_use]
    pub fn is_match_query(&self) -> bool {
        self.match_clause.is_some()
    }

    /// Returns true if this is a SELECT query.
    #[must_use]
    pub fn is_select_query(&self) -> bool {
        self.match_clause.is_none()
    }

    /// Creates a new SELECT query.
    #[must_use]
    pub fn new_select(select: SelectStatement) -> Self {
        Self {
            select,
            compound: None,
            match_clause: None,
        }
    }

    /// Creates a new MATCH query (EPIC-045).
    #[must_use]
    pub fn new_match(match_clause: crate::velesql::MatchClause) -> Self {
        let select = SelectStatement {
            distinct: DistinctMode::None,
            columns: SelectColumns::All,
            from: String::new(),
            from_alias: None,
            joins: Vec::new(),
            where_clause: match_clause.where_clause.clone(),
            order_by: None,
            limit: match_clause.return_clause.limit,
            offset: None,
            with_clause: None,
            group_by: None,
            having: None,
            fusion_clause: None,
        };
        Self {
            select,
            compound: None,
            match_clause: Some(match_clause),
        }
    }
}

/// SQL set operator for compound queries (EPIC-040 US-006).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SetOperator {
    /// UNION - merge results, remove duplicates.
    Union,
    /// UNION ALL - merge results, keep duplicates.
    UnionAll,
    /// INTERSECT - keep only common results.
    Intersect,
    /// EXCEPT - subtract second query from first.
    Except,
}

/// Compound query combining two queries with a set operator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompoundQuery {
    /// The set operator.
    pub operator: SetOperator,
    /// The second query.
    pub right: Box<SelectStatement>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_with_clause_new() {
        let clause = WithClause::new();
        assert!(clause.options.is_empty());
    }

    #[test]
    fn test_with_clause_with_option() {
        let clause = WithClause::new()
            .with_option("mode", WithValue::String("accurate".to_string()))
            .with_option("ef_search", WithValue::Integer(512));
        assert_eq!(clause.options.len(), 2);
    }

    #[test]
    fn test_with_clause_get() {
        let clause = WithClause::new().with_option("mode", WithValue::String("fast".to_string()));
        assert!(clause.get("mode").is_some());
        assert!(clause.get("MODE").is_some());
        assert!(clause.get("unknown").is_none());
    }

    #[test]
    fn test_with_clause_get_mode() {
        let clause =
            WithClause::new().with_option("mode", WithValue::String("accurate".to_string()));
        assert_eq!(clause.get_mode(), Some("accurate"));
    }

    #[test]
    fn test_with_value_as_str() {
        let v = WithValue::String("test".to_string());
        assert_eq!(v.as_str(), Some("test"));
    }

    #[test]
    fn test_with_value_as_integer() {
        let v = WithValue::Integer(100);
        assert_eq!(v.as_integer(), Some(100));
    }

    #[test]
    fn test_with_value_as_float() {
        let v = WithValue::Float(1.234);
        assert!((v.as_float().unwrap() - 1.234).abs() < 1e-5);
    }

    #[test]
    fn test_interval_to_seconds() {
        assert_eq!(
            IntervalValue {
                magnitude: 30,
                unit: IntervalUnit::Seconds
            }
            .to_seconds(),
            30
        );
        assert_eq!(
            IntervalValue {
                magnitude: 1,
                unit: IntervalUnit::Days
            }
            .to_seconds(),
            86400
        );
    }

    #[test]
    fn test_temporal_now() {
        let expr = TemporalExpr::Now;
        let epoch = expr.to_epoch_seconds();
        assert!(epoch > 1_577_836_800);
    }

    #[test]
    fn test_value_from_i64() {
        let v: Value = 42i64.into();
        assert_eq!(v, Value::Integer(42));
    }

    #[test]
    fn test_fusion_config_default() {
        let config = FusionConfig::default();
        assert_eq!(config.strategy, "rrf");
    }

    #[test]
    fn test_fusion_config_rrf() {
        let config = FusionConfig::rrf();
        assert_eq!(config.strategy, "rrf");
        assert!((config.params.get("k").unwrap() - 60.0).abs() < 1e-5);
    }

    #[test]
    fn test_fusion_clause_default() {
        let clause = FusionClause::default();
        assert_eq!(clause.strategy, FusionStrategyType::Rrf);
        assert_eq!(clause.k, Some(60));
    }

    #[test]
    fn test_group_by_clause_default() {
        let clause = GroupByClause::default();
        assert!(clause.columns.is_empty());
    }

    #[test]
    fn test_having_clause_default() {
        let clause = HavingClause::default();
        assert!(clause.conditions.is_empty());
    }
}
