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
#[cfg(test)]
mod cbo_tests;
#[cfg(test)]
mod complex_parser_tests;
#[cfg(feature = "persistence")]
mod cost_estimator;
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
mod hybrid;
#[cfg(test)]
mod hybrid_tests;
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
#[cfg(feature = "persistence")]
mod query_stats;
mod validation;
#[cfg(test)]
mod validation_parity_tests;
#[cfg(test)]
mod validation_tests;

#[cfg(test)]
mod aggregation_params_tests;
#[cfg(test)]
mod fusion_clause_tests;
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

pub use aggregator::{AggregateResult, Aggregator};
// Explicit AST exports (replaces `pub use ast::*` — prevents accidental internal type leakage)
pub use ast::{
    // Aggregation
    AggregateArg,
    AggregateFunction,
    AggregateType,
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
    CorrelatedColumn,
    DistinctMode,
    // DML (used by database execute_dml)
    DmlStatement,
    // Fusion
    FusionClause,
    FusionConfig,
    FusionStrategyType,
    GraphMatchPredicate,
    GroupByClause,
    HavingClause,
    HavingCondition,
    InCondition,
    InsertStatement,
    IntervalUnit,
    IntervalValue,
    IsNullCondition,
    JoinClause,
    JoinCondition,
    JoinType,
    LikeCondition,
    LogicalOp,
    MatchCondition,
    OrderByExpr,
    // WITH clause
    QuantizationMode,
    Query,
    SelectColumns,
    SelectOrderBy,
    SelectStatement,
    SetOperator,
    SimilarityCondition,
    SimilarityOrderBy,
    SparseVectorExpr,
    SparseVectorSearch,
    Subquery,
    TemporalExpr,
    // TRAIN statement
    TrainStatement,
    UpdateAssignment,
    UpdateStatement,
    Value,
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
    FilterPlan, FilterStrategy, IndexLookupPlan, IndexType, LimitPlan, OffsetPlan, PlanNode,
    QueryPlan, TableScanPlan, VectorSearchPlan,
};
pub use parser::match_clause;
pub use parser::Parser;
#[cfg(feature = "persistence")]
pub use planner::{Cost, CostEstimator, ExecutionStrategy, QueryPlanner, QueryStats};
pub use validation::{QueryValidator, ValidationConfig, ValidationError, ValidationErrorKind};
