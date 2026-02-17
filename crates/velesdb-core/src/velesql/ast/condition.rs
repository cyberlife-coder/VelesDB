//! WHERE clause condition types for `VelesQL`.
//!
//! This module defines all condition types used in WHERE clauses,
//! including vector search, comparisons, and logical operators.

use serde::{Deserialize, Serialize};

use super::fusion::FusionConfig;
use super::values::{Value, VectorExpr};

/// A condition in a WHERE clause.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Condition {
    /// Vector similarity search: `vector NEAR [metric] $param`
    VectorSearch(VectorSearch),
    /// Multi-vector fused search: `vector NEAR_FUSED [$v1, $v2] USING FUSION 'rrf'`
    VectorFusedSearch(VectorFusedSearch),
    /// Similarity function: `similarity(field, $vector) > threshold`
    Similarity(SimilarityCondition),
    /// Comparison: column op value
    Comparison(Comparison),
    /// IN operator: column IN (values)
    In(InCondition),
    /// BETWEEN operator: column BETWEEN a AND b
    Between(BetweenCondition),
    /// LIKE operator: column LIKE pattern
    Like(LikeCondition),
    /// IS NULL / IS NOT NULL
    IsNull(IsNullCondition),
    /// Full-text search: column MATCH 'query'
    Match(MatchCondition),
    /// Logical AND
    And(Box<Condition>, Box<Condition>),
    /// Logical OR
    Or(Box<Condition>, Box<Condition>),
    /// Logical NOT
    Not(Box<Condition>),
    /// Grouped condition (parentheses)
    Group(Box<Condition>),
}

/// Vector similarity search condition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorSearch {
    /// Vector expression (literal or parameter).
    pub vector: VectorExpr,
}

/// Multi-vector fused search condition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorFusedSearch {
    /// List of vector expressions (literals or parameters).
    pub vectors: Vec<VectorExpr>,
    /// Fusion strategy configuration.
    pub fusion: FusionConfig,
}

/// Similarity function condition: `similarity(field, vector) op threshold`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimilarityCondition {
    /// Field name containing the embedding.
    pub field: String,
    /// Vector to compare against.
    pub vector: VectorExpr,
    /// Comparison operator.
    pub operator: CompareOp,
    /// Similarity threshold.
    pub threshold: f64,
}

/// Comparison condition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    /// Column name.
    pub column: String,
    /// Comparison operator.
    pub operator: CompareOp,
    /// Value to compare against.
    pub value: Value,
}

/// Comparison operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompareOp {
    /// Equal (=)
    Eq,
    /// Not equal (!= or <>)
    NotEq,
    /// Greater than (>)
    Gt,
    /// Greater than or equal (>=)
    Gte,
    /// Less than (<)
    Lt,
    /// Less than or equal (<=)
    Lte,
}

/// IN condition: column IN (value1, value2, ...)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InCondition {
    /// Column name.
    pub column: String,
    /// List of values.
    pub values: Vec<Value>,
}

/// BETWEEN condition: column BETWEEN low AND high
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BetweenCondition {
    /// Column name.
    pub column: String,
    /// Low value.
    pub low: Value,
    /// High value.
    pub high: Value,
}

/// LIKE/ILIKE condition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LikeCondition {
    /// Column name.
    pub column: String,
    /// Pattern (with % and _ wildcards).
    pub pattern: String,
    /// True for ILIKE (case-insensitive).
    #[serde(default)]
    pub case_insensitive: bool,
}

/// IS NULL condition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IsNullCondition {
    /// Column name.
    pub column: String,
    /// True for IS NULL, false for IS NOT NULL.
    pub is_null: bool,
}

/// MATCH condition for full-text search.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchCondition {
    /// Column name.
    pub column: String,
    /// Search query.
    pub query: String,
}
