//! Tool parameter / result DTOs for the MCP transport.
//!
//! Output shapes reuse the domain types from [`crate::model`] / [`crate::service`]
//! directly (they derive `Serialize` + `JsonSchema`), so there is no duplicate
//! wire/domain struct. Only request envelopes and small id-results live here,
//! split out of [`super`] (`mcp.rs`) so that file stays focused on the server
//! and tool wiring.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::model::{ColumnFilter, Link, Recollection};
use crate::service::Metadata;

/// Parameters for the `remember` tool.
#[derive(Deserialize, JsonSchema)]
pub(super) struct RememberParams {
    /// The fact to store in memory.
    pub(super) fact: String,
    /// Optional typed links from this fact to existing memories.
    #[serde(default)]
    pub(super) links: Vec<Link>,
    /// Optional structured metadata for later filtering (e.g.
    /// `{"project": "veles", "author": "julien", "status": "open"}`).
    pub(super) metadata: Option<Metadata>,
}

/// Result of the `remember` tool.
#[derive(Serialize, JsonSchema)]
pub(super) struct RememberResult {
    /// Stable id assigned to the remembered fact.
    pub(super) id: u64,
}

/// Parameters for the `recall` tool.
#[derive(Deserialize, JsonSchema)]
pub(super) struct RecallParams {
    /// Natural-language query to match semantically.
    pub(super) query: String,
    /// Maximum number of memories to return (default 10).
    pub(super) limit: Option<usize>,
    /// Optional exact-match metadata filter (e.g.
    /// `{"project": "veles", "status": "resolved"}`).
    pub(super) filter: Option<Metadata>,
}

/// Result of the `recall` tool.
#[derive(Serialize, JsonSchema)]
pub(super) struct RecallResult {
    /// Recalled memories, most similar first.
    pub(super) memories: Vec<Recollection>,
}

/// Parameters for the `recall_where` tool.
#[derive(Deserialize, JsonSchema)]
pub(super) struct RecallWhereParams {
    /// Natural-language query to match semantically.
    pub(super) query: String,
    /// Maximum number of memories to return (default 10).
    pub(super) limit: Option<usize>,
    /// Structured `ColumnStore` predicates (ranges/comparisons) combined with AND,
    /// e.g. a date window `[{"field":"ts","op":"ge","value":20230101},
    /// {"field":"ts","op":"le","value":20231231}]`. Each `op` is one of
    /// `eq`/`ne`/`lt`/`le`/`gt`/`ge`.
    #[serde(default)]
    pub(super) filters: Vec<ColumnFilter>,
}

/// Parameters for the `relate` tool.
#[derive(Deserialize, JsonSchema)]
pub(super) struct RelateParams {
    /// Source memory id.
    pub(super) from: u64,
    /// Target memory id.
    pub(super) to: u64,
    /// Relationship label.
    pub(super) relation: String,
}

/// Result of the `relate` tool.
#[derive(Serialize, JsonSchema)]
pub(super) struct RelateResult {
    /// Id of the created edge.
    pub(super) edge_id: u64,
}

/// Parameters for the `forget` tool.
#[derive(Deserialize, JsonSchema)]
pub(super) struct ForgetParams {
    /// Id of the memory to forget.
    pub(super) id: u64,
}

/// Result of the `forget` tool.
#[derive(Serialize, JsonSchema)]
pub(super) struct ForgetResult {
    /// Id of the forgotten memory.
    pub(super) id: u64,
}

/// Parameters for the `why` tool.
#[derive(Deserialize, JsonSchema)]
pub(super) struct WhyParams {
    /// The decision (or fact) to explain.
    pub(super) decision: String,
    /// How many hops of typed links to follow (default 2).
    pub(super) max_hops: Option<usize>,
    /// Optional exact-match metadata filter to scope the seed (e.g.
    /// `{"project": "veles"}`).
    pub(super) filter: Option<Metadata>,
}

/// Parameters for the `remember_extracted` tool.
#[derive(Deserialize, JsonSchema)]
pub(super) struct RememberExtractedParams {
    /// Raw text to extract atomic facts from and store as a connected graph.
    pub(super) text: String,
    /// Optional structured metadata applied to every extracted fact.
    pub(super) metadata: Option<Metadata>,
}

/// Result of the `remember_extracted` tool.
#[derive(Serialize, JsonSchema)]
pub(super) struct RememberExtractedResult {
    /// Stable ids of the stored facts, in extraction order.
    pub(super) ids: Vec<u64>,
}
