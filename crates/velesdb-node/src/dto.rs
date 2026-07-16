//! `#[napi(object)]` data-transfer objects and `From<domain>` conversions.
//!
//! Every `u64` id is re-typed to a decimal `String` because a JS `number` is an
//! f64 and silently loses precision above 2^53. Domain types stay napi-agnostic;
//! all marshalling lives here and in [`crate::convert`].

use napi_derive::napi;
use serde_json::Value;
use velesdb_memory::{Explanation, MemoryEdge, MemoryNode, Recollection};

use crate::convert::id_to_string;

/// A typed link to an existing memory (input to `remember`).
#[napi(object)]
pub struct LinkJs {
    /// Decimal-string id of the memory being linked to.
    pub target: String,
    /// Relationship label, e.g. `"decided_in"`.
    pub relation: String,
}

/// A structured predicate for `recallWhere` (input).
#[napi(object)]
pub struct ColumnFilterJs {
    /// Metadata field name (alphanumeric/underscore).
    pub field: String,
    /// Comparison operator: one of `eq` `ne` `lt` `le` `gt` `ge`.
    pub op: String,
    /// Value to compare against (number, string, or boolean).
    pub value: Value,
}

/// Tuning knobs for `recallFused` (input). Every field is optional; an
/// omitted field falls back to the proven default from
/// [`velesdb_memory::FusionOptions::default`] (via
/// [`crate::convert::to_fusion_options`]).
#[napi(object)]
pub struct FusionOptionsJs {
    /// Hops the graph traversal walks from the top vector seed.
    pub hops: Option<u32>,
    /// Weight added to a graph-reached fact's normalised vector score.
    pub graph_boost: Option<f64>,
    /// Depth of the oversampled vector pool fusion re-ranks.
    pub pool: Option<u32>,
}

/// One recalled memory (output of `recall` / `recallWhere`).
#[napi(object)]
pub struct RecollectionJs {
    /// Decimal-string id of the memory.
    pub id: String,
    /// Similarity score (higher is closer).
    pub score: f64,
    /// Stored fact content.
    pub content: String,
    /// Caller-supplied structured metadata stored with the fact, or
    /// `undefined` when the fact carries none. `recall`, `recallWhere`, and
    /// `recallFused` all populate this; `why()`'s subgraph nodes don't carry
    /// metadata (a different shape, `MemoryNodeJs`).
    pub metadata: Option<Value>,
}

impl From<Recollection> for RecollectionJs {
    fn from(r: Recollection) -> Self {
        Self {
            id: id_to_string(r.id),
            score: f64::from(r.score),
            content: r.content,
            metadata: r.metadata.map(Value::Object),
        }
    }
}

/// Result of `recallFusedDated`: the recalled memories plus a dated timeline.
#[napi(object)]
pub struct DatedRecallJs {
    /// Recalled memories, most relevant first.
    pub memories: Vec<RecollectionJs>,
    /// Chronological, date-prefixed rendering of `memories` (`- [YYYY-MM-DD]
    /// content` per line, oldest first, undated facts last).
    pub dated_context: String,
    /// The most recent date across `memories` (`YYYY-MM-DD`), or `undefined`
    /// when no memory carries a valid date.
    pub now: Option<String>,
}

/// A node in a `why()` explanation subgraph.
#[napi(object)]
pub struct MemoryNodeJs {
    /// Decimal-string id of the memory.
    pub id: String,
    /// Stored fact content.
    pub content: String,
    /// Distance in hops from the seed (seed is `0`).
    pub hop: u32,
}

impl From<MemoryNode> for MemoryNodeJs {
    fn from(n: MemoryNode) -> Self {
        // SAFETY: hop is bounded by MAX_WHY_HOPS (10), which always fits in u32.
        #[allow(clippy::cast_possible_truncation)]
        let hop = n.hop as u32;
        Self {
            id: id_to_string(n.id),
            content: n.content,
            hop,
        }
    }
}

/// A typed edge in a `why()` explanation subgraph.
#[napi(object)]
pub struct MemoryEdgeJs {
    /// Source memory id (decimal string).
    pub from: String,
    /// Target memory id (decimal string).
    pub to: String,
    /// Relationship label.
    pub relation: String,
}

impl From<MemoryEdge> for MemoryEdgeJs {
    fn from(e: MemoryEdge) -> Self {
        Self {
            from: id_to_string(e.from),
            to: id_to_string(e.to),
            relation: e.relation,
        }
    }
}

/// The connected answer to a `why()` question (output): seed memory plus its
/// reachable subgraph — the wedge a plain recall misses.
#[napi(object)]
pub struct ExplanationJs {
    /// Memories in the subgraph, seed first.
    pub nodes: Vec<MemoryNodeJs>,
    /// Typed edges connecting the nodes.
    pub edges: Vec<MemoryEdgeJs>,
}

impl From<Explanation> for ExplanationJs {
    fn from(e: Explanation) -> Self {
        Self {
            nodes: e.nodes.into_iter().map(MemoryNodeJs::from).collect(),
            edges: e.edges.into_iter().map(MemoryEdgeJs::from).collect(),
        }
    }
}

/// Result of [`compileContext`](crate::MemoryStore::compile_context): the
/// top-level fields are typed; the nested trees (`decisions`, `sources`, …)
/// are plain JSON objects in exactly the MCP wire shape (snake_case keys),
/// with every id field already converted to a decimal string.
#[napi(object)]
pub struct CompiledContextJs {
    /// The assembled context, ready to inject into a prompt.
    pub content: String,
    /// Ordered output blocks (cache prefix first), wire shape.
    pub sections: Value,
    /// One auditable decision per input fragment, wire shape.
    pub decisions: Value,
    /// One source pointer per distinct fragment, wire shape.
    pub sources: Value,
    /// Handles of externalized fragments, wire shape.
    pub retrieval_handles: Value,
    /// Token/cost savings of this compilation, wire shape.
    pub insights: Value,
    /// Overall fidelity risk: "low" | "medium" | "high".
    pub risk: String,
}
