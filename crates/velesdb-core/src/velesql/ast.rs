//! Abstract Syntax Tree (AST) for `VelesQL` queries.
//!
//! This module defines the data structures representing parsed `VelesQL` queries.

use serde::{Deserialize, Serialize};

/// A complete `VelesQL` query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Query {
    /// The SELECT statement.
    pub select: SelectStatement,
    /// Compound query (UNION/INTERSECT/EXCEPT) - EPIC-040 US-006.
    #[serde(default)]
    pub compound: Option<CompoundQuery>,
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

/// Compound query combining two queries with a set operator (EPIC-040 US-006).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompoundQuery {
    /// The set operator (UNION, INTERSECT, EXCEPT).
    pub operator: SetOperator,
    /// The second query (right-hand side).
    pub right: Box<SelectStatement>,
}

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
    /// Alias for the FROM table (EPIC-052 US-003: Self-JOIN support).
    #[serde(default)]
    pub from_alias: Option<String>,
    /// JOIN clauses for cross-store queries (EPIC-031 US-004).
    #[serde(default)]
    pub joins: Vec<JoinClause>,
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
    /// GROUP BY clause (optional).
    #[serde(default)]
    pub group_by: Option<GroupByClause>,
    /// HAVING clause for filtering groups (optional).
    #[serde(default)]
    pub having: Option<HavingClause>,
    /// USING FUSION clause for hybrid search (EPIC-040 US-005).
    #[serde(default)]
    pub fusion_clause: Option<FusionClause>,
}

/// JOIN clause for cross-store queries (EPIC-031 US-004).
///
/// Allows joining graph traversal results with ColumnStore data.
///
/// # Example
/// ```sql
/// MATCH (p:Product)
/// JOIN prices AS pr ON pr.product_id = p.id
/// WHERE pr.available = true
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JoinClause {
    /// Type of join (INNER, LEFT, RIGHT, FULL).
    pub join_type: JoinType,
    /// Table/store name to join.
    pub table: String,
    /// Optional alias for the joined table.
    pub alias: Option<String>,
    /// Join condition (ON clause) - None if USING is used.
    pub condition: Option<JoinCondition>,
    /// USING clause columns - alternative to ON condition.
    pub using_columns: Option<Vec<String>>,
}

/// Type of SQL JOIN operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum JoinType {
    /// INNER JOIN - only matching rows from both tables.
    #[default]
    Inner,
    /// LEFT JOIN - all rows from left table, matching from right.
    Left,
    /// RIGHT JOIN - all rows from right table, matching from left.
    Right,
    /// FULL JOIN - all rows from both tables.
    Full,
}

/// Join condition specifying how to link tables.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JoinCondition {
    /// Left side of the join (table.column).
    pub left: ColumnRef,
    /// Right side of the join (match_var.property).
    pub right: ColumnRef,
}

/// Column reference with optional table/alias prefix.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColumnRef {
    /// Optional table or alias prefix.
    pub table: Option<String>,
    /// Column or property name.
    pub column: String,
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
    /// Aggregate function (e.g., `COUNT(*)`, `SUM(price)`).
    Aggregate(AggregateFunction),
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

/// Fusion strategy type for hybrid search (EPIC-040 US-005).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FusionStrategyType {
    /// Reciprocal Rank Fusion (default) - position-based fusion.
    #[default]
    Rrf,
    /// Weighted sum of normalized scores.
    Weighted,
    /// Take maximum score from either source.
    Maximum,
}

/// USING FUSION clause for hybrid vector+graph search (EPIC-040 US-005).
///
/// Combines results from NEAR (vector) and MATCH (graph) queries.
///
/// # Example
/// ```sql
/// SELECT * FROM docs
/// NEAR([0.1, 0.2], 10)
/// MATCH (d)-[:CITES]->(ref)
/// USING FUSION(strategy = 'rrf', k = 60)
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FusionClause {
    /// Fusion strategy (rrf, weighted, maximum).
    pub strategy: FusionStrategyType,
    /// RRF k parameter (default 60).
    pub k: Option<u32>,
    /// Vector weight for weighted fusion (0.0-1.0).
    pub vector_weight: Option<f64>,
    /// Graph weight for weighted fusion (0.0-1.0).
    pub graph_weight: Option<f64>,
}

impl Default for FusionClause {
    fn default() -> Self {
        Self {
            strategy: FusionStrategyType::Rrf,
            k: Some(60),
            vector_weight: None,
            graph_weight: None,
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
    /// Select aggregate functions (COUNT, SUM, etc.).
    Aggregations(Vec<AggregateFunction>),
    /// Mixed: columns + aggregations (for GROUP BY queries).
    Mixed {
        /// Regular columns (must appear in GROUP BY).
        columns: Vec<Column>,
        /// Aggregate functions.
        aggregations: Vec<AggregateFunction>,
    },
}

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
    /// Temporal function (EPIC-038).
    Temporal(TemporalExpr),
    /// Scalar subquery (EPIC-039).
    Subquery(Box<Subquery>),
}

/// Scalar subquery expression (EPIC-039).
///
/// A subquery that returns a single value, used in WHERE comparisons.
///
/// # Examples
/// ```sql
/// WHERE price < (SELECT AVG(price) FROM products)
/// WHERE (SELECT COUNT(*) FROM items WHERE order_id = o.id) > 5
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subquery {
    /// The SELECT statement of the subquery.
    pub select: SelectStatement,
    /// Correlated columns (references to outer query).
    #[serde(default)]
    pub correlations: Vec<CorrelatedColumn>,
}

/// A correlated column reference in a subquery (EPIC-039).
///
/// Represents `outer_table.column` references in correlated subqueries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorrelatedColumn {
    /// Outer query table/alias reference.
    pub outer_table: String,
    /// Column name in outer query.
    pub outer_column: String,
    /// Column in subquery that references it.
    pub inner_column: String,
}

/// Temporal expression for date/time operations (EPIC-038).
///
/// # Examples
/// ```sql
/// WHERE created_at > NOW()
/// WHERE timestamp > NOW() - INTERVAL '7 days'
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TemporalExpr {
    /// Current timestamp: `NOW()`
    Now,
    /// Interval expression: `INTERVAL '7 days'`
    Interval(IntervalValue),
    /// Arithmetic: `NOW() - INTERVAL '7 days'`
    Subtract(Box<TemporalExpr>, Box<TemporalExpr>),
    /// Arithmetic: `NOW() + INTERVAL '1 hour'`
    Add(Box<TemporalExpr>, Box<TemporalExpr>),
}

/// Interval value with magnitude and unit (EPIC-038).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntervalValue {
    /// Numeric magnitude (e.g., 7 for '7 days').
    pub magnitude: i64,
    /// Time unit.
    pub unit: IntervalUnit,
}

/// Time unit for INTERVAL expressions (EPIC-038).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntervalUnit {
    /// Seconds.
    Seconds,
    /// Minutes.
    Minutes,
    /// Hours.
    Hours,
    /// Days.
    Days,
    /// Weeks.
    Weeks,
    /// Months.
    Months,
}

impl IntervalValue {
    /// Converts the interval to seconds.
    #[must_use]
    pub fn to_seconds(&self) -> i64 {
        match self.unit {
            IntervalUnit::Seconds => self.magnitude,
            IntervalUnit::Minutes => self.magnitude * 60,
            IntervalUnit::Hours => self.magnitude * 3600,
            IntervalUnit::Days => self.magnitude * 86400,
            IntervalUnit::Weeks => self.magnitude * 604_800,
            IntervalUnit::Months => self.magnitude * 2_592_000, // ~30 days
        }
    }
}

impl TemporalExpr {
    /// Evaluates the temporal expression to epoch seconds (Unix timestamp).
    ///
    /// Uses current system time for `NOW()`.
    #[must_use]
    pub fn to_epoch_seconds(&self) -> i64 {
        use std::time::{SystemTime, UNIX_EPOCH};

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        match self {
            Self::Now => now,
            Self::Interval(iv) => iv.to_seconds(),
            Self::Subtract(left, right) => left.to_epoch_seconds() - right.to_epoch_seconds(),
            Self::Add(left, right) => left.to_epoch_seconds() + right.to_epoch_seconds(),
        }
    }
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

/// GROUP BY clause for aggregation queries.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct GroupByClause {
    /// Columns to group by.
    pub columns: Vec<String>,
}

/// Logical operator for combining HAVING conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogicalOp {
    /// Logical AND - all conditions must be true.
    And,
    /// Logical OR - at least one condition must be true.
    Or,
}

/// HAVING clause for filtering aggregation groups.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HavingClause {
    /// Conditions to filter groups (aggregate comparisons).
    pub conditions: Vec<HavingCondition>,
    /// Logical operators between conditions (len = conditions.len() - 1).
    /// Empty means all AND (backward compatible).
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

// Graph Pattern Matching types are in graph_pattern.rs

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // WithClause tests
    // =========================================================================

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
        assert!(clause.get("MODE").is_some()); // case insensitive
        assert!(clause.get("unknown").is_none());
    }

    #[test]
    fn test_with_clause_get_mode() {
        let clause =
            WithClause::new().with_option("mode", WithValue::String("accurate".to_string()));
        assert_eq!(clause.get_mode(), Some("accurate"));
    }

    #[test]
    fn test_with_clause_get_mode_identifier() {
        let clause =
            WithClause::new().with_option("mode", WithValue::Identifier("fast".to_string()));
        assert_eq!(clause.get_mode(), Some("fast"));
    }

    #[test]
    fn test_with_clause_get_ef_search() {
        let clause = WithClause::new().with_option("ef_search", WithValue::Integer(256));
        assert_eq!(clause.get_ef_search(), Some(256));
    }

    #[test]
    fn test_with_clause_get_timeout_ms() {
        let clause = WithClause::new().with_option("timeout_ms", WithValue::Integer(5000));
        assert_eq!(clause.get_timeout_ms(), Some(5000));
    }

    #[test]
    fn test_with_clause_get_rerank() {
        let clause = WithClause::new().with_option("rerank", WithValue::Boolean(true));
        assert_eq!(clause.get_rerank(), Some(true));
    }

    // =========================================================================
    // WithValue tests
    // =========================================================================

    #[test]
    fn test_with_value_as_str_string() {
        let v = WithValue::String("test".to_string());
        assert_eq!(v.as_str(), Some("test"));
    }

    #[test]
    fn test_with_value_as_str_identifier() {
        let v = WithValue::Identifier("ident".to_string());
        assert_eq!(v.as_str(), Some("ident"));
    }

    #[test]
    fn test_with_value_as_str_integer() {
        let v = WithValue::Integer(42);
        assert_eq!(v.as_str(), None);
    }

    #[test]
    fn test_with_value_as_integer() {
        let v = WithValue::Integer(100);
        assert_eq!(v.as_integer(), Some(100));
    }

    #[test]
    fn test_with_value_as_integer_from_string() {
        let v = WithValue::String("not an int".to_string());
        assert_eq!(v.as_integer(), None);
    }

    #[test]
    fn test_with_value_as_float_from_float() {
        let v = WithValue::Float(1.234);
        assert!((v.as_float().unwrap() - 1.234).abs() < 1e-5);
    }

    #[test]
    fn test_with_value_as_float_from_integer() {
        let v = WithValue::Integer(42);
        assert!((v.as_float().unwrap() - 42.0).abs() < 1e-5);
    }

    #[test]
    fn test_with_value_as_float_from_string() {
        let v = WithValue::String("not a float".to_string());
        assert_eq!(v.as_float(), None);
    }

    #[test]
    fn test_with_value_as_bool() {
        let v = WithValue::Boolean(true);
        assert_eq!(v.as_bool(), Some(true));
    }

    #[test]
    fn test_with_value_as_bool_from_integer() {
        let v = WithValue::Integer(1);
        assert_eq!(v.as_bool(), None);
    }

    // =========================================================================
    // IntervalValue tests
    // =========================================================================

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
                magnitude: 5,
                unit: IntervalUnit::Minutes
            }
            .to_seconds(),
            300
        );
        assert_eq!(
            IntervalValue {
                magnitude: 2,
                unit: IntervalUnit::Hours
            }
            .to_seconds(),
            7200
        );
        assert_eq!(
            IntervalValue {
                magnitude: 1,
                unit: IntervalUnit::Days
            }
            .to_seconds(),
            86400
        );
        assert_eq!(
            IntervalValue {
                magnitude: 1,
                unit: IntervalUnit::Weeks
            }
            .to_seconds(),
            604_800
        );
        assert_eq!(
            IntervalValue {
                magnitude: 1,
                unit: IntervalUnit::Months
            }
            .to_seconds(),
            2_592_000
        );
    }

    // =========================================================================
    // TemporalExpr tests
    // =========================================================================

    #[test]
    fn test_temporal_now() {
        let expr = TemporalExpr::Now;
        let epoch = expr.to_epoch_seconds();
        // Should be a reasonable Unix timestamp (after 2020)
        assert!(epoch > 1_577_836_800);
    }

    #[test]
    fn test_temporal_interval() {
        let expr = TemporalExpr::Interval(IntervalValue {
            magnitude: 60,
            unit: IntervalUnit::Seconds,
        });
        assert_eq!(expr.to_epoch_seconds(), 60);
    }

    #[test]
    fn test_temporal_subtract() {
        let left = TemporalExpr::Interval(IntervalValue {
            magnitude: 100,
            unit: IntervalUnit::Seconds,
        });
        let right = TemporalExpr::Interval(IntervalValue {
            magnitude: 30,
            unit: IntervalUnit::Seconds,
        });
        let expr = TemporalExpr::Subtract(Box::new(left), Box::new(right));
        assert_eq!(expr.to_epoch_seconds(), 70);
    }

    #[test]
    fn test_temporal_add() {
        let left = TemporalExpr::Interval(IntervalValue {
            magnitude: 50,
            unit: IntervalUnit::Seconds,
        });
        let right = TemporalExpr::Interval(IntervalValue {
            magnitude: 25,
            unit: IntervalUnit::Seconds,
        });
        let expr = TemporalExpr::Add(Box::new(left), Box::new(right));
        assert_eq!(expr.to_epoch_seconds(), 75);
    }

    // =========================================================================
    // Value From implementations tests
    // =========================================================================

    #[test]
    fn test_value_from_i64() {
        let v: Value = 42i64.into();
        assert_eq!(v, Value::Integer(42));
    }

    #[test]
    fn test_value_from_f64() {
        let v: Value = 1.234f64.into();
        assert_eq!(v, Value::Float(1.234));
    }

    #[test]
    fn test_value_from_str() {
        let v: Value = "hello".into();
        assert_eq!(v, Value::String("hello".to_string()));
    }

    #[test]
    fn test_value_from_string() {
        let v: Value = String::from("world").into();
        assert_eq!(v, Value::String("world".to_string()));
    }

    #[test]
    fn test_value_from_bool() {
        let v: Value = true.into();
        assert_eq!(v, Value::Boolean(true));
    }

    // =========================================================================
    // FusionConfig tests
    // =========================================================================

    #[test]
    fn test_fusion_config_default() {
        let config = FusionConfig::default();
        assert_eq!(config.strategy, "rrf");
        assert!(config.params.is_empty());
    }

    #[test]
    fn test_fusion_config_rrf() {
        let config = FusionConfig::rrf();
        assert_eq!(config.strategy, "rrf");
        assert!((config.params.get("k").unwrap() - 60.0).abs() < 1e-5);
    }

    #[test]
    fn test_fusion_config_weighted() {
        let config = FusionConfig::weighted(0.5, 0.3, 0.2);
        assert_eq!(config.strategy, "weighted");
        assert!((config.params.get("avg_weight").unwrap() - 0.5).abs() < 1e-5);
        assert!((config.params.get("max_weight").unwrap() - 0.3).abs() < 1e-5);
        assert!((config.params.get("hit_weight").unwrap() - 0.2).abs() < 1e-5);
    }

    // =========================================================================
    // FusionClause tests
    // =========================================================================

    #[test]
    fn test_fusion_clause_default() {
        let clause = FusionClause::default();
        assert_eq!(clause.strategy, FusionStrategyType::Rrf);
        assert_eq!(clause.k, Some(60));
        assert!(clause.vector_weight.is_none());
        assert!(clause.graph_weight.is_none());
    }

    // =========================================================================
    // GroupByClause tests
    // =========================================================================

    #[test]
    fn test_group_by_clause_default() {
        let clause = GroupByClause::default();
        assert!(clause.columns.is_empty());
    }

    // =========================================================================
    // HavingClause tests
    // =========================================================================

    #[test]
    fn test_having_clause_default() {
        let clause = HavingClause::default();
        assert!(clause.conditions.is_empty());
        assert!(clause.operators.is_empty());
    }
}
