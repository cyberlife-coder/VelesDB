//! `VelesQL` - SQL-like query language for `VelesDB`.
//!
//! `VelesQL` combines familiar SQL syntax with vector search extensions.
//!
//! # Example
//!
//! ```ignore
//! use velesdb_core::velesql::{Parser, Query, QueryCache, QueryPlan};
//!
//! // Direct parsing
//! let query = Parser::parse("SELECT * FROM documents WHERE vector NEAR $v LIMIT 10")?;
//!
//! // Cached parsing (recommended for repetitive workloads)
//! let cache = QueryCache::new(1000);
//! let query = cache.parse("SELECT * FROM documents LIMIT 10")?;
//!
//! // EXPLAIN query plan
//! let plan = QueryPlan::from_select(&query.select);
//! println!("{}", plan.to_tree());
//! ```
#![allow(clippy::doc_markdown, clippy::uninlined_format_args)]

#[cfg(test)]
mod aggregation_executor_tests;
#[cfg(test)]
mod aggregation_tests;
mod aggregator;
#[cfg(test)]
mod aggregator_tests;
mod ast;
#[cfg(test)]
mod ast_tests;
mod cache;
#[cfg(test)]
mod cache_tests;
#[cfg(all(test, feature = "persistence"))]
mod cbo_tests;
#[cfg(test)]
mod complex_parser_tests;
#[cfg(feature = "persistence")]
mod cost_estimator;
#[cfg(test)]
mod ddl_tests;
#[cfg(test)]
mod distinct_tests;
#[cfg(test)]
mod dml_tests;
mod error;
#[cfg(test)]
mod error_tests;
#[cfg(feature = "persistence")]
mod explain;
#[cfg(all(test, feature = "persistence"))]
mod explain_tests;
mod graph_pattern;
#[cfg(test)]
mod graph_pattern_tests;
#[cfg(test)]
mod groupby_tests;
#[cfg(test)]
mod having_tests;
#[cfg(test)]
mod join_extended_tests;
pub mod json_path;
#[cfg(test)]
mod json_path_tests;
#[cfg(test)]
mod orderby_multi_tests;
#[cfg(test)]
mod parallel_aggregation_tests;
mod parser;
#[cfg(test)]
mod parser_tests;
#[cfg(feature = "persistence")]
mod planner;
#[cfg(all(test, feature = "persistence"))]
mod planner_tests;
#[cfg(test)]
mod projection_parser_tests;
#[cfg(feature = "persistence")]
mod query_stats;
mod validation;
#[cfg(test)]
mod validation_parity_tests;
#[cfg(test)]
mod validation_tests;
mod validation_types;

#[cfg(test)]
mod aggregation_params_tests;
#[cfg(test)]
mod fusion_clause_tests;
#[cfg(test)]
mod let_clause_tests;
#[cfg(test)]
mod pr_review_bugfix_tests;
#[cfg(test)]
mod quantization_hints_tests;
#[cfg(test)]
mod self_join_tests;
#[cfg(test)]
mod set_operations_tests;
#[cfg(test)]
mod similarity_tests;
#[cfg(test)]
mod train_tests;
#[cfg(test)]
mod velesql_v2_integration_tests;
#[cfg(test)]
mod with_options_tests;

#[cfg(test)]
mod admin_ddl_tests;
#[cfg(test)]
mod flush_tests;
#[cfg(test)]
mod index_ddl_tests;
#[cfg(test)]
mod introspection_tests;

#[cfg(test)]
mod clause_edge_tests;
#[cfg(test)]
mod comment_tests;
#[cfg(test)]
mod dml_extended_tests;
#[cfg(test)]
mod dml_v2_tests;
#[cfg(test)]
mod graph_query_tests;
#[cfg(test)]
mod like_edge_tests;
#[cfg(test)]
mod match_text_extended_tests;
#[cfg(test)]
mod negative_edge_tests;
#[cfg(test)]
mod operator_edge_tests;
#[cfg(test)]
mod temporal_extended_tests;

pub use aggregator::{AggregateResult, Aggregator};
// Explicit AST exports (replaces `pub use ast::*` — prevents accidental internal type leakage)
pub use ast::{
    // Admin (VelesQL v3.6 -- FLUSH)
    AdminStatement,
    // Aggregation
    AggregateArg,
    AggregateFunction,
    AggregateType,
    // DDL (VelesQL v3.3 -- used by database ddl_executor)
    AlterCollectionStatement,
    AnalyzeStatement,
    // Arithmetic (EPIC-042)
    ArithmeticExpr,
    ArithmeticOp,
    // Conditions (used by server, python, wasm, cli)
    BetweenCondition,
    // SELECT
    Column,
    // JOIN
    ColumnRef,
    CompareOp,
    Comparison,
    // Top-level query types
    CompoundQuery,
    Condition,
    // Values (used by cli, wasm)
    ContainsCondition,
    ContainsMode,
    ContainsTextCondition,
    CorrelatedColumn,
    CreateCollectionKind,
    CreateCollectionStatement,
    CreateIndexStatement,
    DdlStatement,
    // DML (used by database execute_dml)
    DeleteEdgeStatement,
    DeleteStatement,
    // Introspection (VelesQL v3.4 -- SHOW/DESCRIBE/EXPLAIN)
    DescribeCollectionStatement,
    DistinctMode,
    DmlStatement,
    DropCollectionStatement,
    DropIndexStatement,
    // Fusion
    FlushStatement,
    FusionClause,
    FusionConfig,
    FusionStrategyType,
    // Geospatial (Issue #514)
    GeoBboxCondition,
    GeoDistanceCondition,
    GraphCollectionParams,
    GraphMatchPredicate,
    GraphSchemaMode,
    GroupByClause,
    HavingClause,
    HavingCondition,
    InCondition,
    InsertEdgeStatement,
    // Graph query extensions (VelesQL v3.5 Phase 5)
    InsertNodeStatement,
    InsertStatement,
    IntervalUnit,
    IntervalValue,
    IntrospectionStatement,
    IsNullCondition,
    JoinClause,
    JoinCondition,
    JoinType,
    // LET clause (v1.10 Phase 3)
    LetBinding,
    LikeCondition,
    LogicalOp,
    MatchCondition,
    OrderByExpr,
    // WITH clause
    QuantizationMode,
    Query,
    SchemaDefinition,
    SelectColumns,
    SelectEdgesStatement,
    SelectOrderBy,
    SelectStatement,
    SetOperator,
    SimilarityCondition,
    SimilarityOrderBy,
    SimilarityScoreExpr,
    SparseVectorExpr,
    SparseVectorSearch,
    Subquery,
    TemporalExpr,
    // TRAIN statement
    TrainStatement,
    TruncateStatement,
    UpdateAssignment,
    UpdateStatement,
    Value,
    VectorCollectionParams,
    VectorExpr,
    VectorFusedSearch,
    VectorSearch,
    WithClause,
    WithOption,
    WithValue,
};
pub use graph_pattern::*;
// Re-export match_clause parser functions for benchmarks
pub use cache::{CacheStats, QueryCache};
pub use error::{ParseError, ParseErrorKind};
#[cfg(feature = "persistence")]
pub use explain::{
    build_leaf_node_stats, ActualStats, ExplainOutput, FilterPlan, FilterStrategy, FusionInfo,
    IndexLookupPlan, IndexType, LimitPlan, MatchTraversalPlan, NodeStats, OffsetPlan, PlanNode,
    QueryPlan, TableScanPlan, VectorSearchPlan,
};
pub use parser::match_clause;
pub use parser::Parser;
#[cfg(feature = "persistence")]
pub use planner::{Cost, CostEstimator, ExecutionStrategy, QueryPlanner, QueryStats};
pub use validation::{QueryValidator, ValidationConfig, ValidationError, ValidationErrorKind};
