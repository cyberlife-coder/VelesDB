//! `#[napi(object)]` data-transfer objects and `From<domain>` conversions.
//!
//! Every `u64` id is re-typed to a decimal `String` because a JS `number` is an
//! f64 and silently loses precision above 2^53. Domain types stay napi-agnostic;
//! all marshalling lives here and in [`crate::convert`].

use napi_derive::napi;
use serde_json::Value;
use velesdb_memory::{
    EntityProfile, EntityRelation, Explanation, MemoryEdge, MemoryNode, Recollection,
    RememberedExtraction, UnrelateOutcome,
};

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

/// One typed edge touching an entity (output of `entity`).
///
/// Which end `targetId`/`target` name depends on the list it came from: in
/// `relations` it is the far end the edge points AT, in `relationsIn` it is
/// the far end the edge comes FROM.
#[napi(object)]
pub struct EntityRelationJs {
    /// The edge label the passage stated, e.g. `"father_of"`.
    pub predicate: String,
    /// Decimal-string id of the entity (or fact) on the far end.
    pub target_id: String,
    /// Stored content of the far end — for an entity hub, `Entity: <name>`.
    pub target: String,
}

impl From<EntityRelation> for EntityRelationJs {
    fn from(r: EntityRelation) -> Self {
        Self {
            predicate: r.predicate,
            target_id: id_to_string(r.target_id),
            target: r.target,
        }
    }
}

/// Everything the auto-built graph knows about one named entity (output of
/// `entity`). `found` separates "known entity, no attributes yet" from
/// "nothing has ever mentioned this name" — on a miss the other fields carry
/// their empty values and `name` still echoes the canonicalized query, so a
/// caller running several lookups can pair each answer with its question.
#[napi(object)]
pub struct EntityProfileJs {
    /// Whether an entity is known under that name at all.
    pub found: bool,
    /// Decimal-string, content-addressed id of the entity (`"0"` on a miss).
    pub id: String,
    /// Canonical (trimmed, lowercased) entity name — filled in hit or miss.
    pub name: String,
    /// Attributes learned about this entity, reserved keys stripped.
    pub attributes: Value,
    /// Typed edges leaving this entity (`mentions` scaffolding excluded).
    pub relations: Vec<EntityRelationJs>,
    /// Typed edges pointing AT this entity (`mentions` scaffolding excluded).
    /// Here each edge's `targetId`/`target` name the far end it comes FROM.
    ///
    /// Without these, a question is only answerable from one side: the graph
    /// holds `camille --sister of--> theo`, so reading Theo's outgoing edges
    /// never finds Camille. The edge exists, it simply leaves the other node.
    /// Nothing is inferred — the converse of a kinship label would need a
    /// gender the graph does not hold; this reports only what is stored.
    pub relations_in: Vec<EntityRelationJs>,
    /// Whether `relations` is a PARTIAL view — true when a response budget
    /// cut the outgoing side. A list holding exactly the cap is otherwise
    /// indistinguishable from a cut one (#1820).
    pub relations_truncated: bool,
    /// Whether `relationsIn` is a PARTIAL view — the incoming mirror of
    /// `relationsTruncated`.
    pub relations_in_truncated: bool,
}

impl EntityProfileJs {
    /// Wire form of a lookup for `queried`, hit or miss — mirroring the MCP
    /// `entity` tool's own `EntityProfileDto::from_lookup`, including the
    /// canonicalized name echoed back on a miss.
    pub fn from_lookup(queried: &str, profile: Option<EntityProfile>) -> Self {
        let Some(profile) = profile else {
            return Self {
                found: false,
                id: id_to_string(0),
                name: velesdb_memory::service::canonical_entity_name(queried),
                attributes: Value::Object(serde_json::Map::new()),
                relations: Vec::new(),
                relations_in: Vec::new(),
                relations_truncated: false,
                relations_in_truncated: false,
            };
        };
        Self {
            found: true,
            id: id_to_string(profile.id),
            name: profile.name,
            attributes: Value::Object(profile.attributes),
            relations: profile
                .relations
                .into_iter()
                .map(EntityRelationJs::from)
                .collect(),
            relations_in: profile
                .relations_in
                .into_iter()
                .map(EntityRelationJs::from)
                .collect(),
            relations_truncated: profile.relations_truncated,
            relations_in_truncated: profile.relations_in_truncated,
        }
    }
}

/// Outcome of `rememberExtracted` (output): the ids stored, and how many
/// facts were dropped for exceeding the embeddable cap.
///
/// An envelope rather than the bare id array this binding used to return,
/// because a shorter list cannot say WHY it is shorter: nothing distinguished
/// "the passage held three facts" from "it held twelve and nine were dropped
/// for their size". That is a silence about lost data, not a missing
/// convenience (issue #1692).
#[napi(object)]
pub struct RememberedExtractionJs {
    /// Decimal-string ids of the stored facts, in extraction order.
    pub ids: Vec<String>,
    /// How many extracted facts were skipped for exceeding the cap.
    pub skipped_over_cap: u32,
}

impl From<RememberedExtraction> for RememberedExtractionJs {
    fn from(outcome: RememberedExtraction) -> Self {
        Self {
            ids: outcome.ids.into_iter().map(id_to_string).collect(),
            // A passage yielding more than u32::MAX skipped facts is not
            // reachable through any call this binding can make; saturating
            // keeps the report from ever reading LOW if that ever changes.
            skipped_over_cap: u32::try_from(outcome.skipped_over_cap).unwrap_or(u32::MAX),
        }
    }
}

/// What `unrelate` actually removed (output). Idempotent by design: an edge
/// that was not there is reported as `found: false`, never as a rejection, so
/// a cleanup can be replayed. `removed` counts the edges genuinely deleted —
/// two facts can carry several parallel edges under the same label.
#[napi(object)]
pub struct UnrelateJs {
    /// Whether at least one matching edge existed and was removed.
    pub found: bool,
    /// How many matching edges were removed (parallel duplicates included).
    pub removed: u32,
}

impl From<UnrelateOutcome> for UnrelateJs {
    fn from(outcome: UnrelateOutcome) -> Self {
        Self {
            found: outcome.found,
            // The count is how many parallel edges joined the same two facts
            // under one label, so it is a handful in practice; saturating
            // rather than wrapping keeps the report from ever reading LOW,
            // and `found` carries the "something was removed" answer anyway.
            removed: u32::try_from(outcome.removed).unwrap_or(u32::MAX),
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
    /// Mechanical, low-noise heads-up over `decisions`, wire shape: every
    /// externalized fragment relevant enough to the query that the caller
    /// should double-check it was not needed.
    ///
    /// Not cosmetic — it is the signal that a compilation degraded something.
    /// A caller compiling under a tight budget saw none of it and believed
    /// their context complete. An EMPTY list is not a clean bill of health
    /// either: `warnings_for` only reports retrieved fragments above a
    /// relevance floor, so `decisions` remains the exhaustive record.
    pub warnings: Value,
}
