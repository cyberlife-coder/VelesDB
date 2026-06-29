//! Domain data model: the request/response value types of the memory layer.
//!
//! These are pure data — the shapes a caller links, recalls, filters on, and
//! gets back — with no dependency on [`MemoryService`](crate::service::MemoryService)
//! itself. Keeping them here separates *what the memory layer exchanges* from
//! *how the service computes it*, and gives every adapter (MCP, bindings) one
//! canonical place to import the contract from.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A typed link from a freshly remembered fact to an existing memory.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct Link {
    /// Id of the memory being linked to.
    pub target: u64,
    /// Relationship label (e.g. `"decided_in"`, `"references"`, `"depends_on"`).
    pub relation: String,
}

/// One semantically recalled memory.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct Recollection {
    /// Stable id of the memory.
    pub id: u64,
    /// Similarity score (higher is closer).
    pub score: f32,
    /// Stored fact content.
    pub content: String,
}

/// Comparison operator for a [`ColumnFilter`] in
/// [`MemoryService::recall_where`](crate::service::MemoryService::recall_where).
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ColumnOp {
    /// `=`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
}

impl ColumnOp {
    /// The `VelesQL` operator token.
    #[must_use]
    pub(crate) fn as_sql(self) -> &'static str {
        match self {
            Self::Eq => "=",
            Self::Ne => "!=",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
        }
    }
}

/// A structured predicate over a memory's metadata column, for the fused
/// vector+`ColumnStore` recall
/// [`MemoryService::recall_where`](crate::service::MemoryService::recall_where).
/// Unlike the exact-match filter on
/// [`MemoryService::recall`](crate::service::MemoryService::recall), this supports
/// ranges and comparisons (e.g. `timestamp >= …`), so temporal and numeric facets
/// become queryable, not just equal-matchable.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ColumnFilter {
    /// Metadata field name (alphanumeric/underscore).
    pub field: String,
    /// Comparison operator.
    pub op: ColumnOp,
    /// Value to compare against (numbers, strings, booleans).
    pub value: Value,
}

/// A node in an [`Explanation`] subgraph.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct MemoryNode {
    /// Stable id of the memory.
    pub id: u64,
    /// Stored fact content.
    pub content: String,
    /// Distance in hops from the seed memory (the seed is hop `0`).
    pub hop: usize,
}

/// A typed edge in an [`Explanation`] subgraph.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct MemoryEdge {
    /// Source memory id.
    pub from: u64,
    /// Target memory id.
    pub to: u64,
    /// Relationship label.
    pub relation: String,
}

/// The connected answer to a `why` question: the best-matching seed memory plus
/// everything reachable from it within a hop budget. This connected subgraph is
/// the differentiator — it surfaces related memories a purely vector recall is
/// blind to (no textual similarity required).
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
pub struct Explanation {
    /// Memories in the subgraph, seed first.
    pub nodes: Vec<MemoryNode>,
    /// Typed edges connecting the nodes.
    pub edges: Vec<MemoryEdge>,
}
