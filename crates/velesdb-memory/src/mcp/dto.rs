//! Tool parameter / result DTOs for the MCP transport.
//!
//! Request envelopes, small id-results, and the id-echoing wire wrappers live
//! here, split out of [`super`] (`mcp.rs`) so that file stays focused on the
//! server and tool wiring.
//!
//! The `..._str` id twins ([`RecollectionDto::id_str`] and friends) are a
//! **wire concern of this MCP layer only** (issue #1468: a u64 id above 2^53
//! is rounded by float-lossy JSON clients): the domain types in
//! [`crate::model`] stay untouched — no extra field, no changed constructor —
//! so the crate's public Rust API is unchanged and library consumers
//! (bindings, crates.io users) see no breakage. Where a tool used to
//! serialize a domain type directly ([`Recollection`], [`Explanation`]), a
//! thin `Dto` wrapper here adds the string twins at the serialization
//! boundary via `From`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::model::{
    deserialize_id, ColumnFilter, Explanation, Link, MemoryEdge, MemoryNode, Recollection,
};
use crate::service::Metadata;

/// Parameters for the `remember` tool.
#[derive(Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct RememberParams {
    /// The fact to store in memory.
    pub(super) fact: String,
    /// Optional typed links from this fact to existing memories.
    #[serde(default, deserialize_with = "super::wire::lenient")]
    pub(super) links: Vec<Link>,
    /// Optional structured metadata for later filtering (e.g.
    /// `{"project": "veles", "author": "julien", "status": "open"}`).
    #[serde(default, deserialize_with = "super::wire::lenient")]
    pub(super) metadata: Option<Metadata>,
    /// Optional time-to-live in seconds. When set, the fact expires (and stops
    /// being recalled) after this many seconds — a durable TTL that survives a
    /// restart. Omit for a permanent memory. Falls back to the server's
    /// `VELESDB_MEMORY_DEFAULT_TTL` when unset.
    #[serde(default, deserialize_with = "super::wire::lenient")]
    pub(super) ttl_seconds: Option<u64>,
}

/// Result of the `remember` tool.
#[derive(Serialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct RememberResult {
    /// Stable id assigned to the remembered fact.
    pub(super) id: u64,
    /// Decimal-string twin of `id` — always relay THIS to `relate`/`forget`/
    /// `feedback`, never `id` itself: a u64 above 2^53 loses precision
    /// through a float-lossy JSON client (issue #1468). Additive: `id` is
    /// unchanged, so 0.9.x callers are unaffected.
    pub(super) id_str: String,
}

/// Parameters for the `recall` tool.
#[derive(Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct RecallParams {
    /// Natural-language query to match semantically.
    pub(super) query: String,
    /// Maximum number of memories to return (default 10).
    #[serde(default, deserialize_with = "super::wire::lenient")]
    pub(super) limit: Option<usize>,
    /// Optional exact-match metadata filter (e.g.
    /// `{"project": "veles", "status": "resolved"}`).
    #[serde(default, deserialize_with = "super::wire::lenient")]
    pub(super) filter: Option<Metadata>,
}

/// Wire shape of one recalled memory: [`Recollection`] plus the `id_str`
/// twin (issue #1468). Built via `From<Recollection>` at the serialization
/// boundary so the domain type itself stays untouched.
#[derive(Serialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct RecollectionDto {
    /// Stable id of the memory.
    pub(super) id: u64,
    /// Decimal-string twin of `id` — always relay THIS to `relate`/`forget`/
    /// `feedback`, never `id` itself: a u64 above 2^53 loses precision
    /// through a float-lossy JSON client (issue #1468). Additive: `id` is
    /// unchanged and stays present for 0.9.x callers.
    pub(super) id_str: String,
    /// Similarity score (higher is closer).
    pub(super) score: f32,
    /// Stored fact content.
    pub(super) content: String,
    /// Caller-supplied structured metadata stored with the fact, reserved
    /// system keys excluded — the exact field [`Recollection::metadata`]
    /// carries, forwarded unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) metadata: Option<Map<String, Value>>,
}

impl From<Recollection> for RecollectionDto {
    fn from(memory: Recollection) -> Self {
        Self {
            id: memory.id,
            id_str: memory.id.to_string(),
            score: memory.score,
            content: memory.content,
            metadata: memory.metadata,
        }
    }
}

/// Result of the `recall` tool.
#[derive(Serialize, JsonSchema)]
pub(super) struct RecallResult {
    /// Recalled memories, most similar first.
    pub(super) memories: Vec<RecollectionDto>,
}

impl RecallResult {
    /// Wrap recalled domain memories into their wire shape.
    pub(super) fn new(memories: Vec<Recollection>) -> Self {
        Self {
            memories: memories.into_iter().map(RecollectionDto::from).collect(),
        }
    }
}

/// Parameters for the `recall_where` tool.
#[derive(Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct RecallWhereParams {
    /// Natural-language query to match semantically.
    pub(super) query: String,
    /// Maximum number of memories to return (default 10).
    #[serde(default, deserialize_with = "super::wire::lenient")]
    pub(super) limit: Option<usize>,
    /// Structured `ColumnStore` predicates (ranges/comparisons) combined with AND,
    /// e.g. a date window `[{"field":"ts","op":"ge","value":20230101},
    /// {"field":"ts","op":"le","value":20231231}]`. Each `op` is one of
    /// `eq`/`ne`/`lt`/`le`/`gt`/`ge`. **Type-strict, no coercion** (issue
    /// #1473): `value` is compared to the stored metadata's JSON type
    /// as-is — a numeric `20230101` never matches a fact stored with
    /// `{"ts": "20230101"}` (a string). Store comparable values (dates,
    /// counters) NUMERICALLY at `remember` time so these filters match them.
    #[serde(default, deserialize_with = "super::wire::lenient")]
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
    #[serde(default, deserialize_with = "super::wire::lenient")]
    pub(super) limit: Option<usize>,
    /// Optional exact-match metadata filter (e.g.
    /// `{"project": "veles", "status": "resolved"}`).
    #[serde(default, deserialize_with = "super::wire::lenient")]
    pub(super) filter: Option<Metadata>,
    /// Graph hops walked from the top vector hit (default 2). Higher reaches
    /// further but adds noise; capped at the `why` hop ceiling.
    #[serde(default, deserialize_with = "super::wire::lenient")]
    pub(super) hops: Option<usize>,
    /// Weight added to a graph-reached fact's normalised vector score
    /// (default 0.15). Raise to trust the graph more, lower to trust vector
    /// similarity more.
    #[serde(default, deserialize_with = "super::wire::lenient")]
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
    pub(super) memories: Vec<RecollectionDto>,
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

impl RecallFusedResult {
    /// Wrap fused-recall domain memories into their wire shape.
    pub(super) fn new(
        memories: Vec<Recollection>,
        dated_context: Option<String>,
        now: Option<String>,
    ) -> Self {
        Self {
            memories: memories.into_iter().map(RecollectionDto::from).collect(),
            dated_context,
            now,
        }
    }
}

/// Parameters for the `relate` tool.
#[derive(Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct RelateParams {
    /// Source memory id — the link points FROM here (as returned by
    /// `remember`/`recall`). Accepts a JSON number or a decimal string:
    /// always relay a previous response's `id_str` here — a plain JSON
    /// number above 2^53 loses precision on a float-lossy client (issue
    /// #1468).
    #[serde(deserialize_with = "deserialize_id")]
    pub(super) from: u64,
    /// Target memory id — the link points TO here (as returned by
    /// `remember`/`recall`). Same string-or-number contract as `from`.
    #[serde(deserialize_with = "deserialize_id")]
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
    /// Decimal-string twin of `edge_id` (issue #1468) — see
    /// [`RememberResult::id_str`].
    pub(super) edge_id_str: String,
}

/// Parameters for the `forget` tool.
#[derive(Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct ForgetParams {
    /// Id of the memory to permanently delete (as returned by `remember` or
    /// `recall`). Accepts a JSON number or a decimal string — relay `id_str`
    /// to avoid float-precision loss above 2^53 (issue #1468).
    #[serde(deserialize_with = "deserialize_id")]
    pub(super) id: u64,
}

/// Result of the `forget` tool.
#[derive(Serialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct ForgetResult {
    /// Id that was requested for deletion.
    pub(super) id: u64,
    /// Decimal-string twin of `id` (issue #1468) — see
    /// [`RememberResult::id_str`].
    pub(super) id_str: String,
    /// Whether a memory actually existed under `id` and was deleted.
    /// `false` means nothing was stored there — a stale id or a typo, not a
    /// second successful deletion — so a caller can tell the two apart.
    pub(super) found: bool,
}

/// Parameters for the `feedback` tool.
#[derive(Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct FeedbackParams {
    /// Id of the recalled memory to reinforce (as returned by
    /// `recall`/`remember`). Accepts a JSON number or a decimal string —
    /// relay `id_str` to avoid float-precision loss above 2^53 (issue
    /// #1468).
    #[serde(deserialize_with = "deserialize_id")]
    pub(super) id: u64,
    /// `true` if the memory was useful (reinforce it), `false` if it was noise
    /// (weaken it).
    #[serde(deserialize_with = "super::wire::lenient")]
    pub(super) success: bool,
}

/// Result of the `feedback` tool.
#[derive(Serialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct FeedbackResult {
    /// Id of the reinforced memory.
    pub(super) id: u64,
    /// Decimal-string twin of `id` (issue #1468) — see
    /// [`RememberResult::id_str`].
    pub(super) id_str: String,
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
    #[serde(default, deserialize_with = "super::wire::lenient")]
    pub(super) max_hops: Option<usize>,
    /// Optional exact-match metadata filter to scope the seed (e.g.
    /// `{"project": "veles"}`).
    #[serde(default, deserialize_with = "super::wire::lenient")]
    pub(super) filter: Option<Metadata>,
}

/// Wire shape of one node in a `why` subgraph: [`MemoryNode`] plus the
/// `id_str` twin (issue #1468).
#[derive(Serialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct MemoryNodeDto {
    /// Stable id of the memory.
    pub(super) id: u64,
    /// Decimal-string twin of `id` (issue #1468) — see
    /// [`RecollectionDto::id_str`].
    pub(super) id_str: String,
    /// Stored fact content.
    pub(super) content: String,
    /// Distance in hops from the seed memory (the seed is hop `0`).
    pub(super) hop: usize,
}

impl From<MemoryNode> for MemoryNodeDto {
    fn from(node: MemoryNode) -> Self {
        Self {
            id: node.id,
            id_str: node.id.to_string(),
            content: node.content,
            hop: node.hop,
        }
    }
}

/// Wire shape of one edge in a `why` subgraph: [`MemoryEdge`] plus the
/// `from_str`/`to_str` twins (issue #1468).
#[derive(Serialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct MemoryEdgeDto {
    /// Source memory id.
    pub(super) from: u64,
    /// Decimal-string twin of `from` (issue #1468) — see
    /// [`RecollectionDto::id_str`].
    pub(super) from_str: String,
    /// Target memory id.
    pub(super) to: u64,
    /// Decimal-string twin of `to` (issue #1468) — see
    /// [`RecollectionDto::id_str`].
    pub(super) to_str: String,
    /// Relationship label.
    pub(super) relation: String,
}

impl From<MemoryEdge> for MemoryEdgeDto {
    fn from(edge: MemoryEdge) -> Self {
        Self {
            from: edge.from,
            from_str: edge.from.to_string(),
            to: edge.to,
            to_str: edge.to.to_string(),
            relation: edge.relation,
        }
    }
}

/// Result of the `why` tool: the wire shape of [`Explanation`], with the
/// decimal-string id twins on every node and edge (issue #1468).
#[derive(Serialize, JsonSchema)]
pub(super) struct ExplanationDto {
    /// Memories in the subgraph, seed first.
    pub(super) nodes: Vec<MemoryNodeDto>,
    /// Typed edges connecting the nodes.
    pub(super) edges: Vec<MemoryEdgeDto>,
}

impl From<Explanation> for ExplanationDto {
    fn from(explanation: Explanation) -> Self {
        Self {
            nodes: explanation
                .nodes
                .into_iter()
                .map(MemoryNodeDto::from)
                .collect(),
            edges: explanation
                .edges
                .into_iter()
                .map(MemoryEdgeDto::from)
                .collect(),
        }
    }
}

/// Parameters for the `remember_extracted` tool.
#[derive(Deserialize, JsonSchema)]
pub(super) struct RememberExtractedParams {
    /// Raw text to extract atomic facts from and store as a connected graph.
    pub(super) text: String,
    /// Optional structured metadata applied to every extracted fact.
    #[serde(default, deserialize_with = "super::wire::lenient")]
    pub(super) metadata: Option<Metadata>,
}

/// Result of the `remember_extracted` tool.
#[derive(Serialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct RememberExtractedResult {
    /// Stable ids of the stored facts, in extraction order.
    pub(super) ids: Vec<u64>,
    /// Decimal-string twins of `ids`, same order (issue #1468) — see
    /// [`RememberResult::id_str`].
    pub(super) ids_str: Vec<String>,
}
