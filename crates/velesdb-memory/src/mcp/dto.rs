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
    /// Optional time-to-live in seconds. When set, the fact expires (and stops
    /// being recalled) after this many seconds — a durable TTL that survives a
    /// restart. Omit for a permanent memory. Falls back to the server's
    /// `VELESDB_MEMORY_DEFAULT_TTL` when unset.
    #[serde(default)]
    pub(super) ttl_seconds: Option<u64>,
}

/// Result of the `remember` tool.
#[derive(Serialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct RememberResult {
    /// Stable id assigned to the remembered fact.
    pub(super) id: u64,
}

/// Parameters for the `recall` tool.
#[derive(Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
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
#[schemars(transform = crate::schema::strip_int_formats)]
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

/// Parameters for the `recall_fused` tool.
#[derive(Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct RecallFusedParams {
    /// Natural-language query to match semantically.
    pub(super) query: String,
    /// Maximum number of memories to return (default 10). Multi-hop reasoning
    /// benefits from a larger budget (~32-64); simple and temporal recall
    /// saturate early, where a larger budget only adds tokens.
    pub(super) limit: Option<usize>,
    /// Optional exact-match metadata filter (e.g.
    /// `{"project": "veles", "status": "resolved"}`).
    pub(super) filter: Option<Metadata>,
    /// Graph hops walked from the top vector hit (default 2). Higher reaches
    /// further but adds noise; capped at the `why` hop ceiling.
    pub(super) hops: Option<usize>,
    /// Weight added to a graph-reached fact's normalised vector score
    /// (default 0.15). Raise to trust the graph more, lower to trust vector
    /// similarity more.
    pub(super) graph_boost: Option<f64>,
    /// Name of the metadata field holding each fact's date as a `YYYYMMDD`
    /// integer (e.g. `"ts"`, `"occurred_at"`). When set, the result adds a
    /// `dated_context` timeline (facts date-prefixed and ordered oldest-first)
    /// plus a `now` anchor — the representation that lifts temporal reasoning.
    /// Omit for plain results.
    pub(super) date_field: Option<String>,
}

/// Result of the `recall_fused` tool: the recalled memories, plus a dated
/// timeline when `date_field` was given.
#[derive(Serialize, JsonSchema)]
pub(super) struct RecallFusedResult {
    /// Recalled memories, most relevant first.
    pub(super) memories: Vec<Recollection>,
    /// Chronological, date-prefixed rendering of `memories` (`- [YYYY-MM-DD]
    /// content` per line, oldest first, undated facts last). Present only when
    /// `date_field` was set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) dated_context: Option<String>,
    /// The most recent date across `memories` (`YYYY-MM-DD`), the "now" anchor.
    /// Present only when `date_field` was set and at least one fact is dated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) now: Option<String>,
}

/// Parameters for the `relate` tool.
#[derive(Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct RelateParams {
    /// Source memory id — the link points FROM here (as returned by `remember`/`recall`).
    pub(super) from: u64,
    /// Target memory id — the link points TO here (as returned by `remember`/`recall`).
    pub(super) to: u64,
    /// Directional relationship label, read as `from` <relation> `to`.
    /// Examples: `caused_by`, `depends_on`, `authored_by`, `supersedes`.
    pub(super) relation: String,
}

/// Result of the `relate` tool.
#[derive(Serialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct RelateResult {
    /// Id of the created edge.
    pub(super) edge_id: u64,
}

/// Parameters for the `forget` tool.
#[derive(Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct ForgetParams {
    /// Id of the memory to permanently delete (as returned by `remember` or `recall`).
    pub(super) id: u64,
}

/// Result of the `forget` tool.
#[derive(Serialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct ForgetResult {
    /// Id of the forgotten memory.
    pub(super) id: u64,
}

/// Parameters for the `feedback` tool.
#[derive(Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct FeedbackParams {
    /// Id of the recalled memory to reinforce (as returned by `recall`/`remember`).
    pub(super) id: u64,
    /// `true` if the memory was useful (reinforce it), `false` if it was noise
    /// (weaken it).
    pub(super) success: bool,
}

/// Result of the `feedback` tool.
#[derive(Serialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct FeedbackResult {
    /// Id of the reinforced memory.
    pub(super) id: u64,
    /// The memory's new learned confidence in `[0.0, 1.0]` after this feedback.
    pub(super) confidence: f32,
}

/// Parameters for the `why` tool.
#[derive(Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
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
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct RememberExtractedResult {
    /// Stable ids of the stored facts, in extraction order.
    pub(super) ids: Vec<u64>,
}
