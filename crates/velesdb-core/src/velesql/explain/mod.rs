//! Query plan explanation for `VelesQL`.
//!
//! This module provides EXPLAIN functionality to display query execution plans.
//!
//! # Example
//!
//! ```ignore
//! use velesdb_core::velesql::{Parser, QueryPlan};
//!
//! let query = Parser::parse("SELECT * FROM docs WHERE vector NEAR $v LIMIT 10")?;
//! let plan = QueryPlan::from_select(&query.select);
//! println!("{}", plan.to_tree());
//! ```

mod filter_strategy;
mod formatter;
mod node_stats;
mod plan_builder;
mod types;

pub(crate) use filter_strategy::strip_vector_predicates;
pub use filter_strategy::{
    fallback_selectivity_threshold, set_fallback_selectivity_threshold,
    DEFAULT_FALLBACK_SELECTIVITY_THRESHOLD,
};
pub use node_stats::build_leaf_node_stats;
pub use types::*;
