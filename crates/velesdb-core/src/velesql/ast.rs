//! Abstract Syntax Tree (AST) for `VelesQL` queries.
//!
//! This module defines the data structures representing parsed `VelesQL` queries.

use serde::{Deserialize, Serialize};

/// A complete `VelesQL` query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Query {
    /// The SELECT statement.
    pub select: SelectStatement,
}

/// A SELECT statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectStatement {
    /// Columns to select.
    pub columns: SelectColumns,
    /// Collection name (FROM clause).
    pub from: String,
    /// WHERE conditions (optional).
    pub where_clause: Option<Condition>,
    /// ORDER BY clause (optional).
    pub order_by: Option<Vec<SelectOrderBy>>,
    /// LIMIT value (optional).
    pub limit: Option<u64>,
    /// OFFSET value (optional).
    pub offset: Option<u64>,
    /// WITH clause for query-time configuration (optional).
    pub with_clause: Option<WithClause>,
}

/// ORDER BY item for sorting SELECT results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectOrderBy {
    /// Expression to order by (field or similarity).
    pub expr: OrderByExpr,
    /// Sort direction (true = DESC, false = ASC).
    pub descending: bool,
}

/// Expression types supported in ORDER BY clause.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum OrderByExpr {
    /// Simple field reference (e.g., `created_at`).
    Field(String),
    /// Similarity function (e.g., `similarity(embedding, $v)`).
    Similarity(SimilarityOrderBy),
}

/// Similarity expression for ORDER BY.
///
/// # Example
/// ```sql
/// ORDER BY similarity(embedding, $query_vec) DESC
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimilarityOrderBy {
    /// Field containing the embedding vector.
    pub field: String,
    /// Vector to compare against.
    pub vector: VectorExpr,
}

/// WITH clause for query-time configuration overrides.
///
/// Allows overriding search parameters on a per-query basis.
///
/// # Example
///
/// ```sql
/// SELECT * FROM docs WHERE vector NEAR $v LIMIT 10
/// WITH (mode = 'accurate', timeout_ms = 5000)
/// ```
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WithClause {
    /// Configuration options as key-value pairs.
    pub options: Vec<WithOption>,
}

impl WithClause {
    /// Creates a new empty WITH clause.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an option to the WITH clause.
    #[must_use]
    pub fn with_option(mut self, key: impl Into<String>, value: WithValue) -> Self {
        self.options.push(WithOption {
            key: key.into(),
            value,
        });
        self
    }

    /// Gets an option value by key.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&WithValue> {
        self.options
            .iter()
            .find(|opt| opt.key.eq_ignore_ascii_case(key))
            .map(|opt| &opt.value)
    }

    /// Gets the search mode if specified.
    #[must_use]
    pub fn get_mode(&self) -> Option<&str> {
        self.get("mode").and_then(|v| v.as_str())
    }

    /// Gets `ef_search` if specified.
    #[must_use]
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    pub fn get_ef_search(&self) -> Option<usize> {
        self.get("ef_search")
            .and_then(WithValue::as_integer)
            .map(|v| v as usize)
    }

    /// Gets timeout in milliseconds if specified.
    #[must_use]
    #[allow(clippy::cast_sign_loss)]
    pub fn get_timeout_ms(&self) -> Option<u64> {
        self.get("timeout_ms")
            .and_then(WithValue::as_integer)
            .map(|v| v as u64)
    }

    /// Gets rerank option if specified.
    #[must_use]
    pub fn get_rerank(&self) -> Option<bool> {
        self.get("rerank").and_then(WithValue::as_bool)
    }
}

/// A single option in a WITH clause.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WithOption {
    /// Option key (e.g., "mode", "`ef_search`").
    pub key: String,
    /// Option value.
    pub value: WithValue,
}

/// Value type for WITH clause options.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WithValue {
    /// String value (e.g., 'accurate').
    String(String),
    /// Integer value (e.g., 512).
    Integer(i64),
    /// Float value (e.g., 0.95).
    Float(f64),
    /// Boolean value (true/false).
    Boolean(bool),
    /// Identifier (unquoted string).
    Identifier(String),
}

impl WithValue {
    /// Returns the value as a string if it is a String or Identifier.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) | Self::Identifier(s) => Some(s),
            _ => None,
        }
    }

    /// Returns the value as an integer if it is an Integer.
    #[must_use]
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Self::Integer(i) => Some(*i),
            _ => None,
        }
    }

    /// Returns the value as a float if it is a Float or Integer.
    #[must_use]
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Self::Float(f) => Some(*f),
            #[allow(clippy::cast_precision_loss)]
            Self::Integer(i) => Some(*i as f64),
            _ => None,
        }
    }

    /// Returns the value as a boolean if it is a Boolean.
    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Boolean(b) => Some(*b),
            _ => None,
        }
    }
}

/// Columns in a SELECT statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SelectColumns {
    /// Select all columns (*).
    All,
    /// Select specific columns.
    Columns(Vec<Column>),
}

/// A column reference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Column {
    /// Column name (e.g., "id", "payload.title").
    pub name: String,
    /// Optional alias (AS clause).
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
///
/// Note: The distance metric is defined at collection creation time,
/// not per-query. The search uses the collection's configured metric.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorSearch {
    /// Vector expression (literal or parameter).
    pub vector: VectorExpr,
}

/// Multi-vector fused search condition.
///
/// Allows searching with multiple vectors and fusing results.
///
/// # Example
///
/// ```sql
/// SELECT * FROM docs WHERE vector NEAR_FUSED [$v1, $v2, $v3]
///     USING FUSION 'rrf' (k = 60)
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorFusedSearch {
    /// List of vector expressions (literals or parameters).
    pub vectors: Vec<VectorExpr>,
    /// Fusion strategy configuration.
    pub fusion: FusionConfig,
}

/// Configuration for multi-vector fusion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FusionConfig {
    /// Fusion strategy name: "average", "maximum", "rrf", "weighted".
    pub strategy: String,
    /// Strategy-specific parameters.
    pub params: std::collections::HashMap<String, f64>,
}

impl Default for FusionConfig {
    fn default() -> Self {
        Self {
            strategy: "rrf".to_string(),
            params: std::collections::HashMap::new(),
        }
    }
}

impl FusionConfig {
    /// Creates a new RRF fusion config with default k=60.
    #[must_use]
    pub fn rrf() -> Self {
        let mut params = std::collections::HashMap::new();
        params.insert("k".to_string(), 60.0);
        Self {
            strategy: "rrf".to_string(),
            params,
        }
    }

    /// Creates a weighted fusion config.
    #[must_use]
    pub fn weighted(avg_weight: f64, max_weight: f64, hit_weight: f64) -> Self {
        let mut params = std::collections::HashMap::new();
        params.insert("avg_weight".to_string(), avg_weight);
        params.insert("max_weight".to_string(), max_weight);
        params.insert("hit_weight".to_string(), hit_weight);
        Self {
            strategy: "weighted".to_string(),
            params,
        }
    }
}

/// Vector expression in a NEAR clause.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum VectorExpr {
    /// Literal vector: [0.1, 0.2, ...]
    Literal(Vec<f32>),
    /// Parameter reference: `$param_name`
    Parameter(String),
}

/// Similarity function condition: `similarity(field, vector) op threshold`
///
/// Used in hybrid queries combining graph traversal with vector similarity.
///
/// # Example
///
/// ```sql
/// MATCH (d:Document)-[:MENTIONS]->(e:Entity)
/// WHERE similarity(d.embedding, $query_vector) > 0.8
/// RETURN d, e
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimilarityCondition {
    /// Field name containing the embedding (e.g., "embedding", "node.embedding")
    pub field: String,
    /// Vector to compare against (literal or parameter)
    pub vector: VectorExpr,
    /// Comparison operator (>, >=, <, <=, =)
    pub operator: CompareOp,
    /// Similarity threshold (typically 0.0 to 1.0)
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

/// LIKE/ILIKE condition: column LIKE pattern or column ILIKE pattern
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LikeCondition {
    /// Column name.
    pub column: String,
    /// Pattern (with % and _ wildcards).
    pub pattern: String,
    /// True for ILIKE (case-insensitive), false for LIKE (case-sensitive).
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

/// A value in `VelesQL`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    /// Integer value.
    Integer(i64),
    /// Float value.
    Float(f64),
    /// String value.
    String(String),
    /// Boolean value.
    Boolean(bool),
    /// Null value.
    Null,
    /// Parameter reference.
    Parameter(String),
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Self::Integer(v)
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Self::Float(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Self::String(v.to_string())
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Self::String(v)
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Self::Boolean(v)
    }
}

// Graph Pattern Matching types are in graph_pattern.rs
