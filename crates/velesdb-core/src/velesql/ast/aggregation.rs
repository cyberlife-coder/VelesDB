//! Aggregation types for GROUP BY and HAVING clauses.
//!
//! This module defines aggregate functions and grouping types
//! used in VelesQL aggregation queries.

use serde::{Deserialize, Serialize};

use super::condition::CompareOp;
use super::values::Value;

/// Aggregate function type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AggregateType {
    /// COUNT(*) or COUNT(column)
    Count,
    /// SUM(column)
    Sum,
    /// AVG(column)
    Avg,
    /// MIN(column)
    Min,
    /// MAX(column)
    Max,
    /// FIRST(column) — returns the value from the highest-scoring row in a group.
    First,
}

/// Argument to an aggregate function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AggregateArg {
    /// Wildcard (*) - only valid for COUNT.
    Wildcard,
    /// Column reference.
    Column(String),
    /// The `score` pseudo-column representing `SearchResult.score`.
    Score,
}

impl AggregateArg {
    /// Whether this argument is the `score` pseudo-column. The parser reads
    /// `score` in any letter case as [`Self::Score`]; a deserialized AST may
    /// still carry it as a column, which reads the same.
    #[must_use]
    pub(crate) fn is_score(&self) -> bool {
        match self {
            Self::Score => true,
            Self::Column(column) => column.eq_ignore_ascii_case("score"),
            Self::Wildcard => false,
        }
    }

    /// The column this argument reads: `score` for the score pseudo-column
    /// however it is spelled, `None` for `*`. Every engine names an
    /// aggregate's running values by it.
    #[must_use]
    pub(crate) fn column(&self) -> Option<&str> {
        match self {
            arg if arg.is_score() => Some("score"),
            Self::Column(column) => Some(column),
            Self::Score | Self::Wildcard => None,
        }
    }
}

/// An aggregate function call in a SELECT statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AggregateFunction {
    /// Type of aggregate function.
    pub function_type: AggregateType,
    /// Argument to the function.
    pub argument: AggregateArg,
    /// Optional alias (AS clause).
    pub alias: Option<String>,
}

impl AggregateType {
    /// The lowercase name an unaliased aggregate's output field starts with.
    const fn key_prefix(self) -> &'static str {
        match self {
            Self::Count => "count",
            Self::Sum => "sum",
            Self::Avg => "avg",
            Self::Min => "min",
            Self::Max => "max",
            Self::First => "first",
        }
    }
}

impl AggregateFunction {
    /// The output field of this aggregate in every engine and result shape —
    /// SQL aggregation rows, vector `GROUP BY` results, the WASM executor: its
    /// alias, else a default name, `count` for `COUNT(*)` and
    /// `<function>_<column>` otherwise (`sum_price`, `avg_score`).
    #[must_use]
    pub(crate) fn result_key(&self) -> String {
        if let Some(alias) = &self.alias {
            return alias.clone();
        }
        let prefix = self.function_type.key_prefix();
        match &self.argument {
            AggregateArg::Wildcard => prefix.to_string(),
            AggregateArg::Score => format!("{prefix}_score"),
            AggregateArg::Column(col) => format!("{prefix}_{col}"),
        }
    }
}

/// GROUP BY clause for aggregation queries.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct GroupByClause {
    /// Columns to group by.
    pub columns: Vec<String>,
}

/// Logical operator for combining HAVING conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum LogicalOp {
    /// Logical AND.
    And,
    /// Logical OR.
    Or,
}

/// HAVING clause for filtering aggregation groups.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HavingClause {
    /// Conditions to filter groups.
    pub conditions: Vec<HavingCondition>,
    /// Logical operators between conditions.
    #[serde(default)]
    pub operators: Vec<LogicalOp>,
}

impl HavingClause {
    /// Returns `true` if any HAVING threshold value is a scalar subquery.
    ///
    /// Mirrors [`Condition::has_subquery`](super::condition::Condition::has_subquery)
    /// for HAVING thresholds, which live outside the WHERE condition tree.
    #[must_use]
    pub fn has_subquery(&self) -> bool {
        self.conditions.iter().any(|cond| cond.value.is_subquery())
    }

    /// Returns `true` if any HAVING threshold value is a subquery **genuinely
    /// correlated** against `outer_tables` (referencing one of the outer query's
    /// tables/aliases).
    #[must_use]
    pub fn has_correlated_subquery(&self, outer_tables: &[&str]) -> bool {
        self.conditions
            .iter()
            .any(|cond| cond.value.is_correlated_subquery_with(outer_tables))
    }
}

/// A single HAVING condition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HavingCondition {
    /// Aggregate function to compare.
    pub aggregate: AggregateFunction,
    /// Comparison operator.
    pub operator: CompareOp,
    /// Value to compare against.
    pub value: Value,
}
