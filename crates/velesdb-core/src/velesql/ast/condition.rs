//! WHERE clause condition types for VelesQL.
//!
//! This module defines all condition types used in WHERE clauses,
//! including vector search, comparisons, and logical operators.

use serde::{Deserialize, Serialize};

use super::fusion::FusionConfig;
use super::values::{Value, VectorExpr};
use crate::sparse_index::SparseVector;
use crate::velesql::GraphPattern;

/// A condition in a WHERE clause.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Condition {
    /// Vector similarity search: `vector NEAR [metric] $param`
    VectorSearch(VectorSearch),
    /// Multi-vector fused search: `vector NEAR_FUSED [$v1, $v2] USING FUSION 'rrf'`
    VectorFusedSearch(VectorFusedSearch),
    /// Sparse vector search: `vector SPARSE_NEAR $sv [USING 'index-name']`
    SparseVectorSearch(SparseVectorSearch),
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
    /// Graph match predicate inside WHERE: `MATCH (a)-[:REL]->(b)`
    GraphMatch(GraphMatchPredicate),
    /// Array containment: `column CONTAINS value` / `CONTAINS ANY` / `CONTAINS ALL`
    Contains(ContainsCondition),
    /// Strict text substring filter: `column CONTAINS_TEXT 'query'`
    ContainsText(ContainsTextCondition),
    /// Geospatial distance: `GEO_DISTANCE(column, lat, lng) op threshold`
    GeoDistance(GeoDistanceCondition),
    /// Geospatial bounding box: `GEO_BBOX(column, lat_min, lng_min, lat_max, lng_max)`
    GeoBbox(GeoBboxCondition),
    /// Logical AND
    And(Box<Condition>, Box<Condition>),
    /// Logical OR
    Or(Box<Condition>, Box<Condition>),
    /// Logical NOT
    Not(Box<Condition>),
    /// Grouped condition (parentheses)
    Group(Box<Condition>),
}

/// Graph predicate condition used in SELECT WHERE clauses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphMatchPredicate {
    /// Graph pattern to evaluate.
    pub pattern: GraphPattern,
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

/// Sparse vector search condition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SparseVectorSearch {
    /// Sparse vector expression (literal or parameter).
    pub vector: SparseVectorExpr,
    /// Optional named sparse index (from USING clause).
    pub index_name: Option<String>,
}

/// Expression representing a sparse vector value in a query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SparseVectorExpr {
    /// Inline sparse literal: `{12: 0.8, 45: 0.3}`
    Literal(SparseVector),
    /// Bind parameter: `$sv`
    Parameter(String),
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
#[non_exhaustive]
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

/// IN / NOT IN condition: `column [NOT] IN (value1, value2, ...)`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InCondition {
    /// Column name.
    pub column: String,
    /// List of values.
    pub values: Vec<Value>,
    /// `true` when this is a `NOT IN` condition.
    #[serde(default)]
    pub negated: bool,
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

/// Strict text substring filter: `column CONTAINS_TEXT 'query'`
///
/// Unlike [`MatchCondition`] (RRF-boosted text search), this performs
/// exact case-sensitive substring matching as a metadata filter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContainsTextCondition {
    /// Column name to search within.
    pub column: String,
    /// Substring to search for.
    pub query: String,
}

/// Containment mode for array CONTAINS operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ContainsMode {
    /// Single value: `column CONTAINS value`
    Single,
    /// At least one: `column CONTAINS ANY (v1, v2, ...)`
    Any,
    /// All values: `column CONTAINS ALL (v1, v2, ...)`
    All,
}

/// CONTAINS condition for array columns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContainsCondition {
    /// Column name.
    pub column: String,
    /// Containment mode.
    pub mode: ContainsMode,
    /// Values to check for containment.
    pub values: Vec<Value>,
}

/// GEO_DISTANCE condition: `GEO_DISTANCE(column, lat, lng) op threshold`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeoDistanceCondition {
    /// Column name containing GeoPoint data.
    pub column: String,
    /// Reference latitude in degrees.
    pub lat: f64,
    /// Reference longitude in degrees.
    pub lng: f64,
    /// Comparison operator.
    pub operator: CompareOp,
    /// Distance threshold in meters.
    pub threshold: f64,
}

/// GEO_BBOX condition: `GEO_BBOX(column, lat_min, lng_min, lat_max, lng_max)`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeoBboxCondition {
    /// Column name containing GeoPoint data.
    pub column: String,
    /// Minimum latitude of the bounding box.
    pub lat_min: f64,
    /// Minimum longitude of the bounding box.
    pub lng_min: f64,
    /// Maximum latitude of the bounding box.
    pub lat_max: f64,
    /// Maximum longitude of the bounding box.
    pub lng_max: f64,
}

impl Condition {
    /// Returns `true` if this condition (or any nested sub-condition) contains
    /// a vector search (`NEAR`, `NEAR_FUSED`, or `SPARSE_NEAR`).
    #[must_use]
    pub fn has_vector_search(&self) -> bool {
        match self {
            Self::VectorSearch(_) | Self::VectorFusedSearch(_) | Self::SparseVectorSearch(_) => {
                true
            }
            Self::And(l, r) | Self::Or(l, r) => l.has_vector_search() || r.has_vector_search(),
            Self::Group(inner) | Self::Not(inner) => inner.has_vector_search(),
            Self::Contains(_)
            | Self::ContainsText(_)
            | Self::Comparison(_)
            | Self::In(_)
            | Self::Between(_)
            | Self::Like(_)
            | Self::IsNull(_)
            | Self::Match(_)
            | Self::GraphMatch(_)
            | Self::Similarity(_)
            | Self::GeoDistance(_)
            | Self::GeoBbox(_) => false,
        }
    }
}
