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

mod ast;
mod cache;
mod error;
mod explain;
mod parser;

pub use ast::*;
pub use cache::{CacheStats, QueryCache};
pub use error::{ParseError, ParseErrorKind};
pub use explain::{
    FilterPlan, FilterStrategy, IndexType, LimitPlan, OffsetPlan, PlanNode, QueryPlan,
    TableScanPlan, VectorSearchPlan,
};
pub use parser::Parser;
