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
    /// proven default. The MCP `recall_fused` tool (which exposes no `pool`, so
    /// passes `None`) and the Python `recall_fused` binding both build their
    /// options here so the transports can't drift on what they accept. A
    /// non-finite `graph_boost` is not filtered here — that guard lives in
    /// [`Self::sanitized`], applied by fusion itself so *every* caller is
    /// covered, not just this constructor.
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

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
