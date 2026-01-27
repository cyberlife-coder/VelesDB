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
mod complex_parser_tests;
#[cfg(test)]
mod distinct_tests;
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
mod planner;
#[cfg(test)]
mod planner_tests;
mod validation;
#[cfg(test)]
mod validation_tests;

#[cfg(test)]
mod aggregation_params_tests;
#[cfg(test)]
mod fusion_clause_tests;
#[cfg(test)]
mod pr_review_bugfix_tests;
#[cfg(test)]
mod self_join_tests;
#[cfg(test)]
mod set_operations_tests;
#[cfg(test)]
mod similarity_tests;
#[cfg(test)]
mod velesql_v2_integration_tests;
#[cfg(test)]
mod with_options_tests;

pub use aggregator::{AggregateResult, Aggregator};
pub use ast::*;
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
pub use planner::{ExecutionStrategy, QueryPlanner, QueryStats};
pub use validation::{QueryValidator, ValidationConfig, ValidationError, ValidationErrorKind};
