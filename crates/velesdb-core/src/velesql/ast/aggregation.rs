//! Aggregation types for GROUP BY and HAVING clauses.
//!
//! This module defines aggregate functions and grouping types
//! used in `VelesQL` aggregation queries.

use serde::{Deserialize, Serialize};

use super::condition::CompareOp;
use super::values::Value;

/// Aggregate function type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
}

/// Argument to an aggregate function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AggregateArg {
    /// Wildcard (*) - only valid for COUNT.
    Wildcard,
    /// Column reference.
    Column(String),
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

/// GROUP BY clause for aggregation queries.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct GroupByClause {
    /// Columns to group by.
    pub columns: Vec<String>,
}

/// Logical operator for combining HAVING conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
