//! SELECT statement types for VelesQL.
//!
//! This module defines the SELECT statement and related types.

use serde::{Deserialize, Serialize};

use super::aggregation::{AggregateFunction, GroupByClause, HavingClause};
use super::condition::Condition;
use super::fusion::FusionClause;
use super::join::JoinClause;
use super::values::VectorExpr;
use super::with_clause::WithClause;

/// DISTINCT mode for SELECT queries (EPIC-052 US-001).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum DistinctMode {
    /// No deduplication.
    #[default]
    None,
    /// DISTINCT - deduplicate by all selected columns.
    All,
}

/// A SELECT statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectStatement {
    /// DISTINCT mode (EPIC-052 US-001).
    #[serde(default)]
    pub distinct: DistinctMode,
    /// Columns to select.
    pub columns: SelectColumns,
    /// Collection name (FROM clause).
    pub from: String,
    /// Alias for the FROM table (EPIC-052 US-003).
    #[serde(default)]
    pub from_alias: Option<String>,
    /// JOIN clauses (EPIC-031 US-004).
    #[serde(default)]
    pub joins: Vec<JoinClause>,
    /// WHERE conditions.
    pub where_clause: Option<Condition>,
    /// ORDER BY clause.
    pub order_by: Option<Vec<SelectOrderBy>>,
    /// LIMIT value.
    pub limit: Option<u64>,
    /// OFFSET value.
    pub offset: Option<u64>,
    /// WITH clause.
    pub with_clause: Option<WithClause>,
    /// GROUP BY clause.
    #[serde(default)]
    pub group_by: Option<GroupByClause>,
    /// HAVING clause.
    #[serde(default)]
    pub having: Option<HavingClause>,
    /// USING FUSION clause (EPIC-040 US-005).
    #[serde(default)]
    pub fusion_clause: Option<FusionClause>,
}

/// Columns in a SELECT statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SelectColumns {
    /// Select all columns (*).
    All,
    /// Select specific columns.
    Columns(Vec<Column>),
    /// Select aggregate functions.
    Aggregations(Vec<AggregateFunction>),
    /// Mixed: columns + aggregations.
    Mixed {
        /// Regular columns.
        columns: Vec<Column>,
        /// Aggregate functions.
        aggregations: Vec<AggregateFunction>,
    },
}

/// A column reference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Column {
    /// Column name.
    pub name: String,
    /// Optional alias.
    pub alias: Option<String>,
}

impl Column {
    /// Creates a new column reference.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            alias: None,
        }
    }

    /// Creates a column with an alias.
    #[must_use]
    pub fn with_alias(name: impl Into<String>, alias: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            alias: Some(alias.into()),
        }
    }
}

/// ORDER BY item for sorting SELECT results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectOrderBy {
    /// Expression to order by.
    pub expr: OrderByExpr,
    /// Sort direction (true = DESC).
    pub descending: bool,
}

/// Expression types supported in ORDER BY clause.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum OrderByExpr {
    /// Simple field reference.
    Field(String),
    /// Similarity function.
    Similarity(SimilarityOrderBy),
    /// Aggregate function.
    Aggregate(AggregateFunction),
}

/// Similarity expression for ORDER BY.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimilarityOrderBy {
    /// Field containing the embedding vector.
    pub field: String,
    /// Vector to compare against.
    pub vector: VectorExpr,
}
