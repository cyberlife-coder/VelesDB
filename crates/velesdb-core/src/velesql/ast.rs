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
    /// LIMIT value (optional).
    pub limit: Option<u64>,
    /// OFFSET value (optional).
    pub offset: Option<u64>,
    /// WITH clause for query-time configuration (optional).
    pub with_clause: Option<WithClause>,
}

/// WITH clause for query-time configuration overrides.
///
/// Allows overriding search parameters on a per-query basis.
///
/// # Example
///
/// ```sql
/// SELECT * FROM docs WHERE vector NEAR $v LIMIT 10
/// WITH (mode = 'high_recall', timeout_ms = 5000)
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
    /// String value (e.g., '`high_recall`').
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
    /// Vector similarity search: vector NEAR [metric] $param
    VectorSearch(VectorSearch),
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

/// Vector expression in a NEAR clause.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum VectorExpr {
    /// Literal vector: [0.1, 0.2, ...]
    Literal(Vec<f32>),
    /// Parameter reference: `$param_name`
    Parameter(String),
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

/// LIKE condition: column LIKE pattern
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LikeCondition {
    /// Column name.
    pub column: String,
    /// Pattern (with % and _ wildcards).
    pub pattern: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_column_new() {
        let col = Column::new("id");
        assert_eq!(col.name, "id");
        assert!(col.alias.is_none());
    }

    #[test]
    fn test_column_with_alias() {
        let col = Column::with_alias("payload.title", "title");
        assert_eq!(col.name, "payload.title");
        assert_eq!(col.alias, Some("title".to_string()));
    }

    #[test]
    fn test_value_from_integer() {
        let v: Value = 42i64.into();
        assert_eq!(v, Value::Integer(42));
    }

    #[test]
    fn test_value_from_float() {
        let v: Value = 2.5f64.into();
        assert_eq!(v, Value::Float(2.5));
    }

    #[test]
    fn test_value_from_string() {
        let v: Value = "hello".into();
        assert_eq!(v, Value::String("hello".to_string()));
    }

    #[test]
    fn test_value_from_bool() {
        let v: Value = true.into();
        assert_eq!(v, Value::Boolean(true));
    }

    #[test]
    fn test_query_serialization() {
        let query = Query {
            select: SelectStatement {
                columns: SelectColumns::All,
                from: "documents".to_string(),
                where_clause: None,
                limit: Some(10),
                offset: None,
                with_clause: None,
            },
        };

        let json = serde_json::to_string(&query).unwrap();
        let parsed: Query = serde_json::from_str(&json).unwrap();
        assert_eq!(query, parsed);
    }
}
