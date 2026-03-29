//! SELECT statement types for VelesQL.
//!
//! This module defines the SELECT statement and related types.

use serde::{Deserialize, Serialize};
use std::fmt;

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
    /// Aliases visible in scope: FROM alias + JOIN aliases (BUG-8 fix).
    #[serde(default)]
    pub from_alias: Vec<String>,
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
    /// Mixed: columns + aggregations + similarity scores + qualified wildcards.
    Mixed {
        /// Regular columns.
        columns: Vec<Column>,
        /// Aggregate functions.
        aggregations: Vec<AggregateFunction>,
        /// similarity() score expressions.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        similarity_scores: Vec<SimilarityScoreExpr>,
        /// Qualified wildcards (e.g., `ctx.*`).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        qualified_wildcards: Vec<String>,
    },
    /// Select similarity() score only (zero-arg form).
    SimilarityScore(SimilarityScoreExpr),
    /// Select alias.* (qualified wildcard).
    QualifiedWildcard(String),
}

impl SelectColumns {
    /// Returns human-readable column names for display.
    ///
    /// Used by Python/WASM bindings to expose column metadata.
    #[must_use]
    pub fn to_display_names(&self) -> Vec<String> {
        match self {
            Self::All => vec!["*".to_string()],
            Self::Columns(cols) => cols.iter().map(|c| c.name.clone()).collect(),
            Self::Aggregations(aggs) => aggs
                .iter()
                .map(|a| format!("{:?}", a.function_type))
                .collect(),
            Self::Mixed {
                columns,
                aggregations,
                ..
            } => {
                let mut result: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();
                result.extend(
                    aggregations
                        .iter()
                        .map(|a| format!("{:?}", a.function_type)),
                );
                result
            }
            Self::SimilarityScore(expr) => {
                vec![expr
                    .alias
                    .clone()
                    .unwrap_or_else(|| "similarity".to_string())]
            }
            Self::QualifiedWildcard(alias) => vec![format!("{alias}.*")],
        }
    }
}

/// A `similarity()` zero-arg expression in SELECT, with optional alias.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimilarityScoreExpr {
    /// Optional alias (e.g., `similarity() AS relevance`).
    pub alias: Option<String>,
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

impl SelectOrderBy {
    /// Returns a `(column_name, direction)` pair for display.
    #[must_use]
    pub fn to_display_pair(&self) -> (String, String) {
        let dir = if self.descending { "DESC" } else { "ASC" };
        let col = match &self.expr {
            OrderByExpr::Field(f) => f.clone(),
            OrderByExpr::Similarity(_) | OrderByExpr::SimilarityBare => "similarity()".to_string(),
            OrderByExpr::Aggregate(agg) => format!("{:?}", agg.function_type),
            OrderByExpr::Arithmetic(expr) => format!("{expr}"),
        };
        (col, dir.to_string())
    }
}

/// Expression types supported in ORDER BY clause.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum OrderByExpr {
    /// Simple field reference.
    Field(String),
    /// Similarity function with field and vector args.
    Similarity(SimilarityOrderBy),
    /// Similarity zero-arg: uses pre-computed search score.
    SimilarityBare,
    /// Aggregate function.
    Aggregate(AggregateFunction),
    /// Arithmetic expression combining scores (EPIC-042).
    ///
    /// Example: `0.7 * vector_score + 0.3 * graph_score`
    Arithmetic(ArithmeticExpr),
}

/// A named score binding defined by a `LET` clause (VelesQL v1.10 Phase 3).
///
/// Each binding assigns an arithmetic expression to a name. Bindings are
/// evaluated in declaration order; later bindings may reference earlier ones.
///
/// # Example
///
/// ```sql
/// LET hybrid = 0.7 * vector_score + 0.3 * bm25_score
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LetBinding {
    /// Binding name (identifier).
    pub name: String,
    /// Expression to evaluate.
    pub expr: ArithmeticExpr,
}

/// Arithmetic expression for ORDER BY custom scoring (EPIC-042).
///
/// Supports binary operations (+, -, *, /) with numeric literals,
/// variables (field references), and similarity() function calls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ArithmeticExpr {
    /// Numeric literal (e.g., `0.7`, `2`).
    Literal(f64),
    /// Score variable or field reference (e.g., `vector_score`, `price`).
    Variable(String),
    /// Similarity function call (zero-arg or with field+vector).
    Similarity(Box<OrderByExpr>),
    /// Binary operation with operator precedence.
    BinaryOp {
        /// Left operand.
        left: Box<ArithmeticExpr>,
        /// Arithmetic operator.
        op: ArithmeticOp,
        /// Right operand.
        right: Box<ArithmeticExpr>,
    },
}

/// Arithmetic operators for ORDER BY expressions (EPIC-042).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArithmeticOp {
    /// Addition (`+`).
    Add,
    /// Subtraction (`-`).
    Sub,
    /// Multiplication (`*`).
    Mul,
    /// Division (`/`).
    Div,
}

impl fmt::Display for ArithmeticOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Add => write!(f, "+"),
            Self::Sub => write!(f, "-"),
            Self::Mul => write!(f, "*"),
            Self::Div => write!(f, "/"),
        }
    }
}

impl fmt::Display for ArithmeticExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Literal(v) => write!(f, "{v}"),
            Self::Variable(name) => write!(f, "{name}"),
            Self::Similarity(inner) => match inner.as_ref() {
                OrderByExpr::Similarity(sim) => {
                    let vec_str = match &sim.vector {
                        VectorExpr::Parameter(name) => format!("${name}"),
                        VectorExpr::Literal(vals) => format!("{vals:?}"),
                    };
                    write!(f, "similarity({}, {vec_str})", sim.field)
                }
                _ => write!(f, "similarity()"),
            },
            Self::BinaryOp { left, op, right } => write!(f, "({left} {op} {right})"),
        }
    }
}

/// Similarity expression for ORDER BY.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimilarityOrderBy {
    /// Field containing the embedding vector.
    pub field: String,
    /// Vector to compare against.
    pub vector: VectorExpr,
}

impl SelectStatement {
    /// Returns an empty `SelectStatement` with all fields at their defaults.
    ///
    /// Used by [`Query::new_dml`], [`Query::new_train`], and [`Query::new_match`]
    /// to avoid repeating the 14-field struct literal.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            distinct: DistinctMode::None,
            columns: SelectColumns::All,
            from: String::new(),
            from_alias: Vec::new(),
            joins: Vec::new(),
            where_clause: None,
            order_by: None,
            limit: None,
            offset: None,
            with_clause: None,
            group_by: None,
            having: None,
            fusion_clause: None,
        }
    }
}
