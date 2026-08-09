//! Domain data model: the request/response value types of the memory layer.
//!
//! These are pure data — the shapes a caller links, recalls, filters on, and
//! gets back — with no dependency on [`MemoryService`](crate::service::MemoryService)
//! itself. Keeping them here separates *what the memory layer exchanges* from
//! *how the service computes it*, and gives every adapter (MCP, bindings) one
//! canonical place to import the contract from.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Serde `deserialize_with` for a required `u64` id field: accepts a JSON
/// number or a decimal string (issue #1468). Sibling of
/// [`crate::context::wire::deserialize_optional_id`] (that one is
/// `Option`-shaped and lives behind the `context` feature) — this one is
/// deliberately feature-independent because [`Link`] is compiled whenever
/// `model` is, regardless of `context`. Reused by `crate::mcp::dto`'s
/// `relate`/`forget`/`feedback` id parameters so the accepted-forms rule
/// lives in exactly one place. Input-side only and purely widening — the
/// serialized (output) shape of every domain type is unchanged.
///
/// # Errors
/// Returns a deserialize error naming the offending value if it is neither a
/// `u64` number nor a decimal-`u64` string.
pub(crate) fn deserialize_id<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let expected = "expected a u64 number or a decimal u64 string";
    match Value::deserialize(deserializer)? {
        Value::Number(number) => number
            .as_u64()
            .ok_or_else(|| Error::custom(format!("invalid id {number} ({expected})"))),
        Value::String(text) => text
            .trim()
            .parse()
            .map_err(|_| Error::custom(format!("invalid id '{text}' ({expected})"))),
        other => Err(Error::custom(format!("invalid id {other} ({expected})"))),
    }
}

/// [`deserialize_id`]'s `Option`-shaped sibling for OPTIONAL id fields
/// (`list_memories.cursor`): absent and `null` mean `None`, anything else
/// takes the same number-or-decimal-string rule. Feature-independent like
/// its sibling and for the same reason — `crate::context::wire`'s
/// equivalent lives behind the `context` feature, and an `mcp`-only build
/// must still parse the cursor.
///
/// Gated on `mcp` — its one consumer is the tool DTO layer — because the
/// wasm build (`context` alone, `-D warnings`) rejects it as dead code
/// otherwise. `deserialize_id` above stays ungated only because [`Link`]'s
/// own deserialization uses it feature-free.
#[cfg(feature = "mcp")]
pub(crate) fn deserialize_optional_id<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let expected = "expected a u64 number, a decimal u64 string, or null";
    match Value::deserialize(deserializer)? {
        Value::Null => Ok(None),
        Value::Number(number) => number
            .as_u64()
            .map(Some)
            .ok_or_else(|| Error::custom(format!("invalid id {number} ({expected})"))),
        Value::String(text) => text
            .trim()
            .parse()
            .map(Some)
            .map_err(|_| Error::custom(format!("invalid id '{text}' ({expected})"))),
        other => Err(Error::custom(format!("invalid id {other} ({expected})"))),
    }
}

/// A typed link from a freshly remembered fact to an existing memory.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct Link {
    /// Id of the memory being linked to. Accepts a JSON number or a decimal
    /// string — ids can exceed 2^53, where float-lossy JSON clients (JS
    /// `number`) round a plain integer, so a caller relaying an `id_str`
    /// value straight from a previous response must be able to resubmit it
    /// as-is (see issue #1468).
    #[serde(deserialize_with = "deserialize_id")]
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
    /// Caller-supplied structured metadata stored with the fact (the `ColumnStore`
    /// facet), with reserved system keys (`content`, `_veles_*`) excluded —
    /// EXCEPT [`crate::storage::AUTO_DATE_FIELD`] (`_veles_date`), the
    /// `YYYYMMDD` date `remember` auto-stamps onto (almost) every fact, which
    /// stays visible here on purpose so `recall_fused`'s `date_field` can read
    /// it back with no caller effort. `None` only when the fact carries no
    /// metadata at all AND no auto-date could be stamped (`wasm32-unknown-unknown`,
    /// which has no clock). This is what makes dated recall work: store a date
    /// (e.g. `occurred_at`, or just rely on the automatic `_veles_date`) and it
    /// round-trips here, so a `recall_where`/`recall_fused` result can be
    /// ordered into a chronological timeline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Map<String, Value>>,
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
    /// The `VelesQL` operator token. Only [`crate::storage::NativeStore`]
    /// builds `VelesQL` text; a non-`persistence` backend (e.g.
    /// `velesdb-wasm`'s in-memory one) filters `ColumnFilter`s directly, with
    /// no query-string step.
    #[cfg(feature = "persistence")]
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

/// Whether a STORED value satisfies `op` against a filter's `target`.
///
/// The single definition of what a [`ColumnFilter`] means once its field has
/// been found, so a backend that evaluates payloads directly and one that
/// translates to `VelesQL` cannot answer differently. `ne` on an absent field
/// diverged between the two for the API's whole life precisely because each
/// carried its own copy of this rule (#1759).
///
/// **A `null` satisfies nothing**, whatever the operator — a comparison
/// against null is not true, as in SQL, and `ne` is no exception. Querying for
/// null-ness is what `IsNull`/`IsNotNull` are for at the `VelesQL` layer. The
/// caller is responsible for the *absent* case: no value here means no match.
///
/// Comparison is numeric when both sides are numbers, lexicographic when both
/// are strings, and equality-only otherwise — an ordering over two unrelated
/// JSON shapes has no meaning, so it is false rather than arbitrary.
#[must_use]
pub fn column_value_matches(stored: &Value, op: ColumnOp, target: &Value) -> bool {
    if stored.is_null() {
        return false;
    }
    if let (Some(left), Some(right)) = (stored.as_f64(), target.as_f64()) {
        return match op {
            ColumnOp::Eq => (left - right).abs() < f64::EPSILON,
            ColumnOp::Ne => (left - right).abs() >= f64::EPSILON,
            ColumnOp::Lt => left < right,
            ColumnOp::Le => left <= right,
            ColumnOp::Gt => left > right,
            ColumnOp::Ge => left >= right,
        };
    }
    if let (Some(left), Some(right)) = (stored.as_str(), target.as_str()) {
        return match op {
            ColumnOp::Eq => left == right,
            ColumnOp::Ne => left != right,
            ColumnOp::Lt => left < right,
            ColumnOp::Le => left <= right,
            ColumnOp::Gt => left > right,
            ColumnOp::Ge => left >= right,
        };
    }
    match op {
        ColumnOp::Eq => stored == target,
        ColumnOp::Ne => stored != target,
        ColumnOp::Lt | ColumnOp::Le | ColumnOp::Gt | ColumnOp::Ge => false,
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
    /// Value to compare against.
    ///
    /// The comparison is TYPE-STRICT with no coercion, so the JSON type sent
    /// here is part of the query: `20260601` (number) never matches a fact
    /// stored as `"20260601"` (string) — same value, no match, and **no
    /// error**. A wrong type here is therefore the one mistake this API
    /// cannot report; it just returns nothing.
    ///
    /// Which is why the advertised type is spelled out rather than left as
    /// the empty schema `serde_json::Value` would produce. `{}` says "send
    /// anything", on the single field where sending the wrong thing fails
    /// silently.
    #[schemars(schema_with = "comparable_json_value")]
    pub value: Value,
}

/// The JSON types a `ColumnFilter` can actually compare: number, string,
/// boolean. Objects and arrays are not orderable and never match; `null` is
/// not a value to compare against.
fn comparable_json_value(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": ["number", "string", "boolean"],
        "description": "Value to compare against. TYPE-STRICT: the JSON type must match \
                        how the fact was stored (a number never matches a string), and a \
                        mismatch returns no results rather than an error.",
    })
}

/// Tuning knobs for
/// [`MemoryService::recall_fused`](crate::service::MemoryService::recall_fused).
///
/// `Default` matches the values validated on the LoCoMo/HotpotQA/TimeQA
/// benchmarks (`examples/locomo`, `examples/multihop`, `examples/timeqa`):
/// `graph_boost = 0.15` was the optimum of a sweep (0.30/0.50/0.80 all
/// degraded ranking quality), and `hops = 2` is the minimum depth at which a
/// fact wired only through a shared topic (the `remember_extracted` hub
/// scaffolding: fact → hub is hop 1, hub → sibling fact is hop 2) becomes
/// reachable at all.
#[derive(Debug, Clone, Copy)]
pub struct FusionOptions {
    /// Hops the graph traversal walks from the top vector seed.
    pub hops: usize,
    /// Weight added to a graph-reached fact's normalised vector score.
    pub graph_boost: f64,
    /// Depth of the oversampled vector pool fusion re-ranks. `None` uses the
    /// proven default (`k` scaled up, floored at 64 — see
    /// `crate::fusion::pool_size`). Widen this to give
    /// [`MemoryService::recall_fused_reranked`](crate::service::MemoryService::recall_fused_reranked)'s
    /// reranker more candidates to work with.
    pub pool: Option<usize>,
}

impl Default for FusionOptions {
    fn default() -> Self {
        Self {
            hops: 2,
            graph_boost: 0.15,
            pool: None,
        }
    }
}

impl FusionOptions {
    /// Build options from optional, untrusted tuning knobs, applying the
    /// defaults and clamps every binding must enforce identically: `hops`
    /// clamped to the graph-traversal ceiling
    /// ([`clamp_hops`](crate::limits::clamp_hops)), `graph_boost` defaulted when
    /// absent, and `pool` clamped to the recall ceiling
    /// ([`clamp_recall_limit`](crate::limits::clamp_recall_limit)) or left at the
    /// proven default. The MCP `recall_fused` tool and the Python
    /// `recall_fused` binding both build their options here — same three
    /// knobs, same clamps — so the transports can't drift on what they
    /// accept. A non-finite `graph_boost` is not filtered here — that guard
    /// lives in [`Self::sanitized`], applied by fusion itself so *every*
    /// caller is covered, not just this constructor.
    #[must_use]
    pub fn from_knobs(hops: Option<usize>, graph_boost: Option<f64>, pool: Option<usize>) -> Self {
        let defaults = Self::default();
        Self {
            hops: crate::limits::clamp_hops(hops.unwrap_or(defaults.hops)),
            graph_boost: graph_boost.unwrap_or(defaults.graph_boost),
            pool: pool
                .map(crate::limits::clamp_recall_limit)
                .or(defaults.pool),
        }
    }

    /// A copy with any non-finite `graph_boost` (NaN or ±∞) reset to the
    /// default. A non-finite boost poisons fusion catastrophically: the score
    /// term `graph_boost · weight` is `NaN` for *every* candidate — even a
    /// pool-only one, since `NaN · 0.0 == NaN` — so `crate::fusion::fuse`'s
    /// `total_cmp` sort sees all scores as equal, degenerates to a no-op, and
    /// then truncates away the graph-reached facts fusion exists to surface
    /// (they are appended after the vector pool). The result is silently worse
    /// than a plain `recall`. Applied inside
    /// [`recall_fused`](crate::service::MemoryService::recall_fused) so no
    /// caller — any binding, or a direct Rust user who filled the struct — can
    /// trip it, however the options were built.
    #[must_use]
    pub fn sanitized(mut self) -> Self {
        if !self.graph_boost.is_finite() {
            self.graph_boost = Self::default().graph_boost;
        }
        self
    }
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
    /// Stable id of the edge itself — what
    /// [`MemoryStore::unrelate`](crate::storage::MemoryStore::unrelate)
    /// removes by.
    pub id: u64,
    /// Source memory id.
    pub from: u64,
    /// Target memory id.
    pub to: u64,
    /// Relationship label.
    pub relation: String,
}

/// At most `cap` edges of one memory point, plus the honest signal that the
/// point carries more — returned by the bounded accessors of
/// [`MemoryStore`](crate::storage::MemoryStore) (#1820).
///
/// `truncated` is a separate field because `edges.len() == cap` cannot carry
/// the signal: a node with exactly `cap` edges is indistinguishable from a
/// truncated one. It compares the node's TOTAL stored degree against the
/// scan cap, so expired far ends dropped inside the scanned window (never
/// replaced — the O(cap) bound is the contract) can leave `edges.len() <
/// cap` with `truncated == true` as a normal outcome.
#[derive(Debug, Clone)]
pub struct BoundedMemoryEdges {
    /// At most `cap` edges, in storage index order.
    pub edges: Vec<MemoryEdge>,
    /// Whether the node's total stored degree exceeded the scan cap.
    pub truncated: bool,
}

/// Outcome of [`MemoryService::unrelate`](crate::service::MemoryService::unrelate):
/// idempotent by design, so an absent edge is a `found: false` answer, not an
/// error — a cleanup must be replayable.
#[derive(Debug, Clone, Copy, Serialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct UnrelateOutcome {
    /// Whether at least one matching edge existed and was removed.
    pub found: bool,
    /// How many matching edges were removed.
    ///
    /// `relate` is idempotent per (from, relation, to), so anything it wrote
    /// removes as 0 or 1. Higher counts mean parallel edges predating that
    /// guarantee, or a direct graph write that bypassed `relate`.
    pub removed: usize,
}

/// One typed edge leaving an entity, as reported by
/// [`MemoryService::entity_profile`](crate::service::MemoryService::entity_profile).
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct EntityRelation {
    /// The edge label the passage stated (e.g. `"pere de"`, `"soeur de"`).
    pub predicate: String,
    /// Stable id of the entity (or fact) on the far end.
    pub target_id: u64,
    /// Stored content of the far end — for an entity hub, `Entity: <name>`.
    pub target: String,
}

/// What [`crate::MemoryService::remember_extracted`] actually did with a
/// passage: the stored fact ids, and how many extracted facts it had to drop.
///
/// A separate struct rather than a bare `Vec<u64>` because the drop count is
/// part of the contract: an extracted fact past the embeddable cap is
/// *skipped* — one unusable element must not cost the others — and a skip the
/// caller cannot see is indistinguishable from the model simply extracting
/// fewer facts.
#[derive(Debug, Clone)]
pub struct RememberedExtraction {
    /// Stable ids of the stored facts, in extraction order.
    pub ids: Vec<u64>,
    /// Extracted facts dropped for exceeding
    /// [`crate::limits::MAX_EMBEDDABLE_TEXT_BYTES`].
    pub skipped_over_cap: usize,
}

/// Everything the auto-built graph knows about one named entity: the
/// attributes merged onto its hub and the typed edges touching it.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct EntityProfile {
    /// Stable, content-addressed id of the entity hub.
    pub id: u64,
    /// Canonical (trimmed, lowercased) entity name.
    pub name: String,
    /// Attributes learned about this entity, reserved keys stripped.
    pub attributes: crate::service::Metadata,
    /// Typed edges leaving this entity (bipartite scaffolding excluded), at
    /// most [`crate::limits::MAX_ENTITY_RELATIONS`] of them.
    pub relations: Vec<EntityRelation>,
    /// Typed edges pointing AT this entity (bipartite scaffolding excluded),
    /// at most [`crate::limits::MAX_ENTITY_RELATIONS`] of them.
    /// Here [`EntityRelation::target_id`]/[`EntityRelation::target`] name the
    /// far end the edge comes FROM — its source.
    pub relations_in: Vec<EntityRelation>,
    /// Whether `relations` is a PARTIAL view: true when the resolution cap
    /// ([`crate::limits::MAX_ENTITY_RELATIONS`]) or the raw scan window
    /// ([`crate::limits::MAX_ENTITY_SCAN_EDGES`]) cut the outgoing side. A
    /// list holding exactly the cap is otherwise indistinguishable from a
    /// cut one (#1820).
    pub relations_truncated: bool,
    /// Whether `relations_in` is a PARTIAL view — the incoming mirror of
    /// [`Self::relations_truncated`].
    pub relations_in_truncated: bool,
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
    /// Whether a width budget cut this walk before it exhausted the
    /// reachable graph (#1820). A subgraph sitting exactly at a cap
    /// ([`crate::limits::MAX_WHY_NODES`], [`crate::limits::MAX_WHY_EDGES`],
    /// [`crate::limits::MAX_WHY_NODE_DEGREE`]) is otherwise
    /// indistinguishable from a complete one — counts at a ceiling were the
    /// only signal, and they are ambiguous by construction.
    ///
    /// True when a node's degree exceeded the per-node budget, or when the
    /// node/edge budget stopped the walk while unexpanded work remained.
    /// The latter is conservative: expanding the rest is exactly what the
    /// budget forbids, so whether it held anything unseen is unknowable —
    /// and a rare cautious `true` is harmless where a false "complete" is
    /// the defect this field exists to close.
    pub truncated: bool,
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;

/// One audited fact, as `list_memories` returns it: the caller-facing shape
/// of a [`crate::storage::RawListedFact`] after the service applied its
/// visibility policy (hub filtering, reserved-key stripping).
#[derive(Debug, Clone)]
pub struct ListedMemory {
    /// Stable id of the memory.
    pub id: u64,
    /// Stored fact content.
    pub content: String,
    /// Metadata as the policy leaves it: business keys (plus the
    /// auto-stamped date) by default, the raw payload under
    /// `include_internal`. `None` when nothing survives.
    pub metadata: Option<crate::service::Metadata>,
}
