//! The output DTOs the binding serialises back to JS.

use super::errors::CODE_INTERNAL;
use super::marshal::id_to_string;
use crate::wasm_error::structured_js_error;
use serde::Serialize;
use serde_json::Value;
use velesdb_memory::context::SegmentationReport;
use velesdb_memory::service::canonical_entity_name;
use velesdb_memory::{
    EntityProfile, EntityRelation, Explanation, MemoryEdge, MemoryNode, Recollection,
};
use wasm_bindgen::JsValue;

// --- Output DTOs ---------------------------------------------------------
//
// Plain `Serialize` structs converted via [`to_js`], over the crate's one
// JS serializer `crate::js_value::to_js_value` — not `#[wasm_bindgen(object)]`, since these are one-shot
// output values, not stateful classes. `id`/`from`/`to` are strings: a plain
// `u64` field would serialize as a JS `number` and lose precision above 2^53.

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RecollectionOut {
    pub(super) id: String,
    pub(super) score: f32,
    pub(super) content: String,
    /// Skipped when `None` so absent metadata reads as `undefined` in JS
    /// (the Node binding's convention) even though [`to_js`] serializes
    /// missing-as-null — that setting exists for `null` *values inside*
    /// the metadata map, not for this absent-field case.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) metadata: Option<Value>,
}

impl From<Recollection> for RecollectionOut {
    fn from(r: Recollection) -> Self {
        Self {
            id: id_to_string(r.id),
            score: r.score,
            content: r.content,
            metadata: r.metadata.map(Value::Object),
        }
    }
}

/// Result of [`WasmMemoryService::recall_fused_dated`](super::WasmMemoryService::recall_fused_dated): the recalled memories
/// plus a chronological, date-prefixed timeline and a "now" anchor.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DatedRecallOut {
    pub(super) memories: Vec<RecollectionOut>,
    pub(super) dated_context: String,
    /// `null` when no fact is dated — kept present (not skipped) so this matches
    /// the Node binding, where napi serializes `Option::None` as JS `null`.
    pub(super) now: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MemoryNodeOut {
    id: String,
    content: String,
    hop: usize,
}

impl From<MemoryNode> for MemoryNodeOut {
    fn from(n: MemoryNode) -> Self {
        Self {
            id: id_to_string(n.id),
            content: n.content,
            hop: n.hop,
        }
    }
}

#[derive(Serialize)]
pub(super) struct MemoryEdgeOut {
    from: String,
    to: String,
    relation: String,
}

impl From<MemoryEdge> for MemoryEdgeOut {
    fn from(e: MemoryEdge) -> Self {
        Self {
            from: id_to_string(e.from),
            to: id_to_string(e.to),
            relation: e.relation,
        }
    }
}

/// What [`WasmMemoryService::unrelate`](super::WasmMemoryService::unrelate) actually removed. Idempotent by
/// design: an edge that was not there is reported as `found: false`, never as
/// a thrown error, so a cleanup can be replayed. `removed` counts the edges
/// genuinely deleted — two facts can carry several parallel edges under the
/// same label.
#[derive(Serialize)]
pub(super) struct UnrelateOut {
    pub(super) found: bool,
    pub(super) removed: usize,
}

/// Outcome of [`WasmMemoryService::remember_extracted`](super::WasmMemoryService::remember_extracted): the ids stored, and
/// how many facts were dropped for exceeding the embeddable cap.
///
/// An envelope rather than a bare id array, because a shorter list is
/// otherwise indistinguishable from a passage that simply held fewer facts —
/// a silence about lost data.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RememberedExtractionOut {
    pub(super) ids: Vec<String>,
    pub(super) skipped_over_cap: usize,
}

/// One typed edge touching an entity (output of
/// [`WasmMemoryService::entity`](super::WasmMemoryService::entity)). `targetId` crosses as a decimal string
/// for the same reason every other id here does.
///
/// Which end `targetId`/`target` name depends on the list it came from: in
/// `relations` it is the far end the edge points AT, in `relationsIn` it is
/// the far end the edge comes FROM.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct EntityRelationOut {
    pub(super) predicate: String,
    pub(super) target_id: String,
    pub(super) target: String,
}

impl From<EntityRelation> for EntityRelationOut {
    fn from(r: EntityRelation) -> Self {
        Self {
            predicate: r.predicate,
            target_id: id_to_string(r.target_id),
            target: r.target,
        }
    }
}

/// Everything the auto-built graph knows about one named entity (output of
/// [`WasmMemoryService::entity`](super::WasmMemoryService::entity)). `found` separates "known entity, no
/// attributes yet" from "nothing has ever mentioned this name"; on a miss
/// the other fields carry their empty values and `name` still echoes the
/// canonicalized query, so several lookups can be told apart.
///
/// `rename_all` is a no-op on the five single-word fields that predate
/// `relationsIn` — it is here so the one multi-word field crosses in the same
/// case as `EntityRelationOut::targetId` sitting inside it, instead of a
/// snake_case key next to a camelCase one in the very same object.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct EntityProfileOut {
    pub(super) found: bool,
    pub(super) id: String,
    pub(super) name: String,
    pub(super) attributes: Value,
    /// Typed edges leaving this entity (bipartite scaffolding excluded).
    pub(super) relations: Vec<EntityRelationOut>,
    /// Typed edges pointing AT this entity (bipartite scaffolding excluded).
    /// Here each edge's `targetId`/`target` name the far end it comes FROM.
    ///
    /// Without these, a question is only answerable from one side: the graph
    /// holds `camille --sister of--> theo`, so asking what Theo's outgoing
    /// edges say never finds Camille.
    pub(super) relations_in: Vec<EntityRelationOut>,
    /// Whether `relations` is a PARTIAL view — true when a response budget
    /// cut the outgoing side. A list holding exactly the cap is otherwise
    /// indistinguishable from a cut one (#1820).
    pub(super) relations_truncated: bool,
    /// Whether `relationsIn` is a PARTIAL view — the incoming mirror of
    /// `relationsTruncated`.
    pub(super) relations_in_truncated: bool,
}

impl EntityProfileOut {
    /// Wire form of a lookup for `queried`, hit or miss — mirroring the MCP
    /// `entity` tool's own `EntityProfileDto::from_lookup`, canonicalized
    /// name echo included.
    pub(super) fn from_lookup(queried: &str, profile: Option<EntityProfile>) -> Self {
        let Some(profile) = profile else {
            return Self {
                found: false,
                id: id_to_string(0),
                name: canonical_entity_name(queried),
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
                .map(EntityRelationOut::from)
                .collect(),
            relations_in: profile
                .relations_in
                .into_iter()
                .map(EntityRelationOut::from)
                .collect(),
            relations_truncated: profile.relations_truncated,
            relations_in_truncated: profile.relations_in_truncated,
        }
    }
}

#[derive(Serialize)]
pub(super) struct ExplanationOut {
    pub(super) nodes: Vec<MemoryNodeOut>,
    pub(super) edges: Vec<MemoryEdgeOut>,
    /// Whether a width budget cut the walk — a subgraph sitting exactly at
    /// a cap is otherwise indistinguishable from a complete one (#1820).
    pub(super) truncated: bool,
}

impl From<Explanation> for ExplanationOut {
    fn from(e: Explanation) -> Self {
        Self {
            nodes: e.nodes.into_iter().map(MemoryNodeOut::from).collect(),
            edges: e.edges.into_iter().map(MemoryEdgeOut::from).collect(),
            truncated: e.truncated,
        }
    }
}

/// Output of [`WasmMemoryService::compile_transcript`](super::WasmMemoryService::compile_transcript): the compiled context
/// (already id-stringified, byte-compatible with [`WasmMemoryService::compile_context`](super::WasmMemoryService::compile_context)'s
/// own output) plus how the transcript was cut into fragments before compilation.
#[derive(Serialize)]
pub(super) struct CompileTranscriptOut {
    pub(super) context: Value,
    pub(super) segmentation: SegmentationReport,
}

pub(super) fn to_js(value: &impl Serialize) -> Result<JsValue, JsValue> {
    // `crate::js_value::to_js_value` emits plain objects and `null`:
    // `RecollectionOut.metadata` is a `serde_json::Value::Object`, which the
    // DEFAULT serializer turns into an ES2015 `Map` — property access and
    // `JSON.stringify` on it silently yield nothing, breaking the documented
    // `Record<string, unknown>` shape and Node-binding parity. A
    // `Value::Null` INSIDE metadata (a caller stored `{flag: null}`) marshals
    // as JS `null`, exactly like the Node binding. Absent metadata still
    // reads as `undefined`: that field is `skip_serializing_if`-omitted,
    // never serialized as a `None` this setting could turn into `null`.
    crate::js_value::to_js_value(value)
        .map_err(|e| structured_js_error(CODE_INTERNAL, &e.to_string()))
}
