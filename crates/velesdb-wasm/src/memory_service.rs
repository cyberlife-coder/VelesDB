//! WASM binding for `velesdb-memory`'s agent-memory wedge — `remember` /
//! `recall` / `recallWhere` / `recallFused` / `relate` / `unrelate` /
//! `forget` / `why` /
//! `entity` / `compileContext` / `compileTranscript` / `explainCompilation` /
//! `contextSavings` / `suggestBudget` / `retrieveContextSource` /
//! `saveWorkingContext` / `loadWorkingContext` / `listWorkingContexts`,
//! backed entirely in-memory ([`WasmStore`]): no filesystem, no network, no
//! `persistence` feature (`Cargo.toml` pulls `velesdb-memory` with
//! `default-features = false, features = ["context"]` only).
//!
//! Mirrors the Node/Python bindings' surface and conventions (decimal-string
//! ids, `{code, message}` structured errors), deliberately diverging from
//! this crate's own `VectorStore`/`SemanticMemory` (which marshal ids as raw
//! `u64`/`BigInt`) — this surface's callers move between the Node, Python,
//! and WASM bindings of the *same* `MemoryService`, so id representation
//! consistency across those three matters more than matching this crate's
//! internal convention.
//!
//! Synchronous, not `Promise`-returning: every operation here is pure
//! in-memory work (no I/O to await), matching this crate's own
//! `SemanticMemory`/`VectorStore` bindings rather than Node's async-by-default
//! convention (which exists there to keep CPU work off Node's event loop —
//! not a concern in a single-threaded WASM heap).
//!
//! One method available on the Node/Python bindings is deliberately absent
//! here, re-confirmed by issue #1547's audit:
//!
//! - `feedback` (RL Memory): [`MemoryService::feedback`] lives in the
//!   `persistence`-gated `reinforce` module
//!   (`crates/velesdb-memory/src/service.rs`'s own doc comment on that
//!   module: "a durable learned confidence is meaningless on the in-memory
//!   (WASM) backend"), so it is not even compiled into this crate — adding
//!   it would mean enabling `persistence` for the `wasm32` target, pulling
//!   in `NativeStore`/filesystem code this binding exists specifically to
//!   avoid. Not a "missing binding"; an intentional architectural boundary.
//!
//! `rememberExtracted` used to be the second, on the grounds that
//! `OllamaExtractor` was the crate's only [`velesdb_memory::extract::Extractor`]
//! impl and would drag a network call into the bundle. That reason stopped
//! being true when [`velesdb_memory::OutlineExtractor`] landed — deterministic
//! and dependency-free — so the method is exposed here with that backend, and
//! ollama refused by name (issue #1692).
//!
//! `compileTranscript` accepts only an inline `transcript` string, never the
//! MCP tool's `path` field — this binding has no filesystem, so a `path`
//! input has nothing to resolve against.

use serde::Serialize;
use serde_json::Value;
use wasm_bindgen::prelude::*;

use velesdb_memory::context::{
    suggest_token_budget, CompilePolicy, CompileRequest, CompiledContext, ContextCompiler,
    ContextDecision, ContextSavings, LoadedWorkingContext, SegmentationReport, SuggestedBudget,
    WorkingContext, WorkingContextSession,
};
use velesdb_memory::service::canonical_entity_name;
use velesdb_memory::{
    ColumnFilter, ColumnOp, EntityProfile, EntityRelation, ErrorCategory, Explanation,
    FusionOptions, HashEmbedder, MemoryEdge, MemoryError, MemoryNode, MemoryService, Metadata,
    OutlineExtractor, Recollection,
};

use crate::memory_store::WasmStore;
use crate::wasm_error::structured_js_error;

const CODE_INVALID_INPUT: &str = "INVALID_INPUT";
const CODE_NOT_FOUND: &str = "NOT_FOUND";
const CODE_INTERNAL: &str = "INTERNAL";
const CODE_UNSUPPORTED: &str = "UNSUPPORTED";

// --- Errors ------------------------------------------------------------

fn category_code(e: &MemoryError) -> &'static str {
    known_category_code(e.category()).unwrap_or(CODE_INTERNAL)
}

/// The explicit half of the mapping, split from the fallback so the coverage
/// test over [`velesdb_memory::ErrorCategory::ALL`] can tell them apart:
/// `ErrorCategory` is `non_exhaustive`, so a new category no longer fails this
/// match at compile time — the test does instead. The runtime fallback
/// (`INTERNAL`, the coarsest bucket) only ever serves a binding built against
/// a newer `velesdb-memory` than this file.
fn known_category_code(category: ErrorCategory) -> Option<&'static str> {
    Some(match category {
        ErrorCategory::InvalidInput => CODE_INVALID_INPUT,
        ErrorCategory::NotFound => CODE_NOT_FOUND,
        ErrorCategory::Internal => CODE_INTERNAL,
        ErrorCategory::Unsupported => CODE_UNSUPPORTED,
        _ => return None,
    })
}

fn to_js_err(e: MemoryError) -> JsValue {
    structured_js_error(category_code(&e), &e.to_string())
}

fn invalid_input(msg: impl AsRef<str>) -> JsValue {
    structured_js_error(CODE_INVALID_INPUT, msg.as_ref())
}

// --- Id / metadata / filter marshalling ---------------------------------

fn id_to_string(id: u64) -> String {
    id.to_string()
}

fn parse_id(s: &str) -> Result<u64, JsValue> {
    s.parse::<u64>()
        .map_err(|_| invalid_input(format!("invalid id '{s}' (expected a decimal u64 string)")))
}

/// Recursively rewrite every `context` id field (see
/// [`velesdb_memory::context::wire::ID_KEYS`]) of an outgoing JSON tree into
/// its decimal-string form. Shared with the Node binding via
/// `velesdb_memory::context::wire`, not duplicated here.
fn stringify_id_fields(value: &mut Value) {
    velesdb_memory::context::wire::stringify_id_fields(value);
}

/// The inverse of [`stringify_id_fields`]: recursively rewrite every
/// `context` id field given in the binding's decimal-string form back into
/// the numeric form the domain types deserialize (used by
/// [`WasmMemoryService::save_working_context`], the same helper the Node
/// binding applies before deserializing a `WorkingContext`). Shared with the
/// Node binding via `velesdb_memory::context::wire`, not duplicated here.
fn parse_id_fields(value: &mut Value) -> Result<(), JsValue> {
    velesdb_memory::context::wire::parse_id_fields(value).map_err(invalid_input)
}

/// Accept `fragments[].id` in decimal-string form (the Node binding's
/// contract, mirrored) by rewriting it to the numeric wire form.
fn parse_fragment_id_strings(request: &mut Value) -> Result<(), JsValue> {
    velesdb_memory::context::wire::parse_fragment_id_strings(request).map_err(invalid_input)
}

/// `undefined`/`null` → `None`; a plain object → `Some(Metadata)`; anything
/// else is a caller error.
fn to_metadata(value: JsValue) -> Result<Option<Metadata>, JsValue> {
    if value.is_undefined() || value.is_null() {
        return Ok(None);
    }
    let parsed: Value = serde_wasm_bindgen::from_value(value)
        .map_err(|e| invalid_input(format!("invalid metadata/filter: {e}")))?;
    match parsed {
        Value::Object(map) => Ok(Some(map)),
        _ => Err(invalid_input("metadata/filter must be an object")),
    }
}

#[derive(serde::Deserialize)]
struct LinkInput {
    target: String,
    relation: String,
}

fn to_links(value: JsValue) -> Result<Vec<velesdb_memory::Link>, JsValue> {
    if value.is_undefined() || value.is_null() {
        return Ok(Vec::new());
    }
    let inputs: Vec<LinkInput> = serde_wasm_bindgen::from_value(value)
        .map_err(|e| invalid_input(format!("invalid links: {e}")))?;
    inputs
        .into_iter()
        .map(|l| {
            Ok(velesdb_memory::Link {
                target: parse_id(&l.target)?,
                relation: l.relation,
            })
        })
        .collect()
}

#[derive(serde::Deserialize)]
struct ColumnFilterInput {
    field: String,
    op: String,
    value: Value,
}

fn parse_op(op: &str) -> Result<ColumnOp, JsValue> {
    match op {
        "eq" => Ok(ColumnOp::Eq),
        "ne" => Ok(ColumnOp::Ne),
        "lt" => Ok(ColumnOp::Lt),
        "le" => Ok(ColumnOp::Le),
        "gt" => Ok(ColumnOp::Gt),
        "ge" => Ok(ColumnOp::Ge),
        other => Err(invalid_input(format!(
            "invalid op '{other}' (expected eq|ne|lt|le|gt|ge)"
        ))),
    }
}

fn to_filters(value: JsValue) -> Result<Vec<ColumnFilter>, JsValue> {
    let inputs: Vec<ColumnFilterInput> = serde_wasm_bindgen::from_value(value)
        .map_err(|e| invalid_input(format!("invalid filters: {e}")))?;
    inputs
        .into_iter()
        .map(|f| {
            Ok(ColumnFilter {
                field: f.field,
                op: parse_op(&f.op)?,
                value: f.value,
            })
        })
        .collect()
}

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct FusionOptionsInput {
    hops: Option<usize>,
    graph_boost: Option<f64>,
    pool: Option<usize>,
}

fn to_fusion_options(value: JsValue) -> Result<FusionOptions, JsValue> {
    let defaults = FusionOptions::default();
    if value.is_undefined() || value.is_null() {
        return Ok(defaults);
    }
    let input: FusionOptionsInput = serde_wasm_bindgen::from_value(value)
        .map_err(|e| invalid_input(format!("invalid fusion options: {e}")))?;
    Ok(FusionOptions {
        hops: velesdb_memory::limits::clamp_hops(input.hops.unwrap_or(defaults.hops)),
        graph_boost: input.graph_boost.unwrap_or(defaults.graph_boost),
        pool: input.pool.or(defaults.pool),
    })
}

// --- Output DTOs ---------------------------------------------------------
//
// Plain `Serialize` structs converted via `serde_wasm_bindgen::to_value`
// (this crate's established pattern for JS-facing output, e.g. `agent.rs`'s
// `SemanticResult`) — not `#[wasm_bindgen(object)]`, since these are one-shot
// output values, not stateful classes. `id`/`from`/`to` are strings: a plain
// `u64` field would serialize as a JS `number` and lose precision above 2^53.

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecollectionOut {
    id: String,
    score: f32,
    content: String,
    /// Skipped when `None` so absent metadata reads as `undefined` in JS
    /// (the Node binding's convention) even though [`to_js`] serializes
    /// missing-as-null — that setting exists for `null` *values inside*
    /// the metadata map, not for this absent-field case.
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<Value>,
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

/// Result of [`WasmMemoryService::recall_fused_dated`]: the recalled memories
/// plus a chronological, date-prefixed timeline and a "now" anchor.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DatedRecallOut {
    memories: Vec<RecollectionOut>,
    dated_context: String,
    /// `null` when no fact is dated — kept present (not skipped) so this matches
    /// the Node binding, where napi serializes `Option::None` as JS `null`.
    now: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MemoryNodeOut {
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
struct MemoryEdgeOut {
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

/// What [`WasmMemoryService::unrelate`] actually removed. Idempotent by
/// design: an edge that was not there is reported as `found: false`, never as
/// a thrown error, so a cleanup can be replayed. `removed` counts the edges
/// genuinely deleted — two facts can carry several parallel edges under the
/// same label.
#[derive(Serialize)]
struct UnrelateOut {
    found: bool,
    removed: usize,
}

/// Outcome of [`WasmMemoryService::remember_extracted`]: the ids stored, and
/// how many facts were dropped for exceeding the embeddable cap.
///
/// An envelope rather than a bare id array, because a shorter list is
/// otherwise indistinguishable from a passage that simply held fewer facts —
/// a silence about lost data.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RememberedExtractionOut {
    ids: Vec<String>,
    skipped_over_cap: usize,
}

/// One typed edge touching an entity (output of
/// [`WasmMemoryService::entity`]). `targetId` crosses as a decimal string
/// for the same reason every other id here does.
///
/// Which end `targetId`/`target` name depends on the list it came from: in
/// `relations` it is the far end the edge points AT, in `relationsIn` it is
/// the far end the edge comes FROM.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EntityRelationOut {
    predicate: String,
    target_id: String,
    target: String,
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
/// [`WasmMemoryService::entity`]). `found` separates "known entity, no
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
struct EntityProfileOut {
    found: bool,
    id: String,
    name: String,
    attributes: Value,
    /// Typed edges leaving this entity (bipartite scaffolding excluded).
    relations: Vec<EntityRelationOut>,
    /// Typed edges pointing AT this entity (bipartite scaffolding excluded).
    /// Here each edge's `targetId`/`target` name the far end it comes FROM.
    ///
    /// Without these, a question is only answerable from one side: the graph
    /// holds `camille --sister of--> theo`, so asking what Theo's outgoing
    /// edges say never finds Camille.
    relations_in: Vec<EntityRelationOut>,
    /// Whether `relations` is a PARTIAL view — true when a response budget
    /// cut the outgoing side. A list holding exactly the cap is otherwise
    /// indistinguishable from a cut one (#1820).
    relations_truncated: bool,
    /// Whether `relationsIn` is a PARTIAL view — the incoming mirror of
    /// `relationsTruncated`.
    relations_in_truncated: bool,
}

impl EntityProfileOut {
    /// Wire form of a lookup for `queried`, hit or miss — mirroring the MCP
    /// `entity` tool's own `EntityProfileDto::from_lookup`, canonicalized
    /// name echo included.
    fn from_lookup(queried: &str, profile: Option<EntityProfile>) -> Self {
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
struct ExplanationOut {
    nodes: Vec<MemoryNodeOut>,
    edges: Vec<MemoryEdgeOut>,
    /// Whether a width budget cut the walk — a subgraph sitting exactly at
    /// a cap is otherwise indistinguishable from a complete one (#1820).
    truncated: bool,
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

/// Input of [`WasmMemoryService::compile_transcript`] — the shared
/// [`TranscriptCompileInput`](velesdb_memory::context::TranscriptCompileInput),
/// aliased so this module keeps naming a local type. Same fields as the MCP
/// `compile_transcript` tool's request MINUS `path`: this binding has no
/// filesystem (see the module docs), so only an inline `transcript` is
/// accepted.
use velesdb_memory::context::TranscriptCompileInput as CompileTranscriptInput;

/// The segmentation glue this binding used to carry itself, now relayed from
/// `velesdb_memory`'s
/// [`transcript_bridge`](velesdb_memory::context::transcript_bridge) — the
/// Node binding carried a byte-for-byte twin of it, each doc comment pointing
/// at the other. Re-exported under the historical name so the tests, which
/// exercise it directly (a `JsValue` cannot be constructed off `wasm32`; see
/// `memory_service_tests.rs`'s module docs), keep naming it.
use velesdb_memory::context::build_transcript_compile_request;

/// Output of [`WasmMemoryService::compile_transcript`]: the compiled context
/// (already id-stringified, byte-compatible with [`WasmMemoryService::compile_context`]'s
/// own output) plus how the transcript was cut into fragments before compilation.
#[derive(Serialize)]
struct CompileTranscriptOut {
    context: Value,
    segmentation: SegmentationReport,
}

fn to_js(value: &impl Serialize) -> Result<JsValue, JsValue> {
    // `serialize_maps_as_objects`: `RecollectionOut.metadata` is a
    // `serde_json::Value::Object`, which the DEFAULT serializer turns into an
    // ES2015 `Map` — property access and `JSON.stringify` on it silently
    // yield nothing, breaking the documented `Record<string, unknown>` shape
    // and Node-binding parity.
    //
    // `serialize_missing_as_null`: a `Value::Null` INSIDE metadata (a caller
    // stored `{flag: null}`) must marshal as JS `null`, exactly like the
    // Node binding — the default (`undefined`) makes `JSON.stringify` drop
    // the key on WASM only. Absent metadata still reads as `undefined`:
    // that field is `skip_serializing_if`-omitted, never serialized as a
    // `None` this setting could turn into `null`.
    let serializer = serde_wasm_bindgen::Serializer::new()
        .serialize_maps_as_objects(true)
        .serialize_missing_as_null(true);
    value
        .serialize(&serializer)
        .map_err(|e| structured_js_error(CODE_INTERNAL, &e.to_string()))
}

// --- The binding ---------------------------------------------------------

/// Local-first agent memory with the `why()` graph wedge, running entirely
/// in the browser. Uses the offline, zero-dependency `HashEmbedder` — the
/// only embedder that makes sense with no filesystem and no network.
#[wasm_bindgen(js_name = MemoryService)]
pub struct WasmMemoryService {
    inner: MemoryService<HashEmbedder, WasmStore>,
}

#[wasm_bindgen(js_class = MemoryService)]
impl WasmMemoryService {
    /// Create a new, empty in-memory store sized for `dimension`-dimensional
    /// embeddings.
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new(dimension: usize) -> WasmMemoryService {
        let store = WasmStore::new(dimension);
        let embedder = HashEmbedder::new(dimension);
        Self {
            inner: MemoryService::with_store(store, embedder),
        }
    }

    /// Store a fact; resolves to its decimal-string id. `links` is an array
    /// of `{target, relation}` edges to existing memories; `metadata` is an
    /// optional plain object; `ttlSeconds` makes the fact expire after that
    /// many seconds. Omit it for a permanent memory, including when re-storing
    /// a fact that already had an expiry — only what this call supplies is
    /// applied. An explicit `0` is REFUSED, because a caller writing `0` means
    /// "expire now", not "never".
    #[wasm_bindgen(js_name = remember)]
    pub fn remember(
        &self,
        fact: &str,
        links: JsValue,
        metadata: JsValue,
        ttl_seconds: Option<u64>,
    ) -> Result<String, JsValue> {
        if fact.len() > velesdb_memory::limits::MAX_FACT_BYTES {
            return Err(invalid_input(format!(
                "fact exceeds {} bytes ({} given)",
                velesdb_memory::limits::MAX_FACT_BYTES,
                fact.len()
            )));
        }
        let links = to_links(links)?;
        let metadata = to_metadata(metadata)?;
        self.inner
            .remember_with_ttl(fact, &links, metadata.as_ref(), ttl_seconds)
            .map(id_to_string)
            .map_err(to_js_err)
    }

    /// Recall up to `k` (default 10, capped) memories similar to `query`,
    /// optionally narrowed by an exact-match metadata `filter`.
    #[wasm_bindgen(js_name = recall)]
    pub fn recall(
        &self,
        query: &str,
        k: Option<usize>,
        filter: JsValue,
    ) -> Result<JsValue, JsValue> {
        let k = velesdb_memory::limits::clamp_recall_limit(k.unwrap_or(10));
        let filter = to_metadata(filter)?;
        let hits = self
            .inner
            .recall(query, k, filter.as_ref())
            .map_err(to_js_err)?;
        to_js(
            &hits
                .into_iter()
                .map(RecollectionOut::from)
                .collect::<Vec<_>>(),
        )
    }

    /// Fused vector + `ColumnStore` recall: like [`Self::recall`] but
    /// `filters` support ranges/comparisons (`gt`, `le`, …).
    ///
    /// Returns your own stored facts ONLY: entity hubs and the context compiler's
    /// artefacts (stored sources, compilation events, working contexts and
    /// their index) are internal scaffolding and never come back, whatever the
    /// predicate — including a `ne` one, which matches facts lacking the field
    /// entirely.
    #[wasm_bindgen(js_name = recallWhere)]
    pub fn recall_where(
        &self,
        query: &str,
        filters: JsValue,
        k: Option<usize>,
    ) -> Result<JsValue, JsValue> {
        let k = velesdb_memory::limits::clamp_recall_limit(k.unwrap_or(10));
        let filters = to_filters(filters)?;
        let hits = self
            .inner
            .recall_where(query, k, &filters)
            .map_err(to_js_err)?;
        to_js(
            &hits
                .into_iter()
                .map(RecollectionOut::from)
                .collect::<Vec<_>>(),
        )
    }

    /// Fused vector + graph recall: like [`Self::recall`], but also walks
    /// the graph from the top vector hit and promotes any fact it reaches
    /// into the ranking. `opts` is optional (`{hops?, graphBoost?, pool?}`).
    #[wasm_bindgen(js_name = recallFused)]
    pub fn recall_fused(
        &self,
        query: &str,
        k: Option<usize>,
        filter: JsValue,
        opts: JsValue,
    ) -> Result<JsValue, JsValue> {
        let k = velesdb_memory::limits::clamp_recall_limit(k.unwrap_or(10));
        let filter = to_metadata(filter)?;
        let opts = to_fusion_options(opts)?;
        let hits = self
            .inner
            .recall_fused(query, k, filter.as_ref(), opts)
            .map_err(to_js_err)?;
        to_js(
            &hits
                .into_iter()
                .map(RecollectionOut::from)
                .collect::<Vec<_>>(),
        )
    }

    /// Fused recall plus a dated timeline: like [`Self::recall_fused`], but
    /// reads each fact's date from the `dateField` metadata key (a `YYYYMMDD`
    /// integer) and returns `{memories, datedContext, now}` — the memories, a
    /// chronological date-prefixed timeline, and a "now" anchor for temporal
    /// reasoning. A separate method (not a flag on `recallFused`) so the
    /// published `recallFused` array return type is unchanged.
    #[wasm_bindgen(js_name = recallFusedDated)]
    pub fn recall_fused_dated(
        &self,
        query: &str,
        date_field: &str,
        k: Option<usize>,
        filter: JsValue,
        opts: JsValue,
    ) -> Result<JsValue, JsValue> {
        let k = velesdb_memory::limits::clamp_recall_limit(k.unwrap_or(10));
        let filter = to_metadata(filter)?;
        let opts = to_fusion_options(opts)?;
        let (hits, ctx) = self
            .inner
            .recall_fused_dated(query, k, filter.as_ref(), opts, date_field)
            .map_err(to_js_err)?;
        to_js(&DatedRecallOut {
            memories: hits.into_iter().map(RecollectionOut::from).collect(),
            dated_context: ctx.timeline,
            now: ctx.now,
        })
    }

    /// Create a typed edge `from -> to`. Resolves to the edge's
    /// decimal-string id.
    #[wasm_bindgen(js_name = relate)]
    pub fn relate(&self, from: &str, to: &str, relation: &str) -> Result<String, JsValue> {
        let from = parse_id(from)?;
        let to = parse_id(to)?;
        self.inner
            .relate(from, to, relation)
            .map(id_to_string)
            .map_err(to_js_err)
    }

    /// Remove the typed edge(s) `from -relation-> to` — [`Self::relate`]'s
    /// exact undo, so a mistaken edge no longer costs the facts at its
    /// endpoints. Only the edge goes: both memories, and any entity hub, are
    /// untouched.
    ///
    /// Returns `{found, removed}`. Idempotent: removing an absent edge answers
    /// `found: false` instead of throwing, so a cleanup can be replayed. It
    /// refuses exactly what `relate` refuses (empty relation, `from` equal to
    /// `to`), and deliberately does NOT require the endpoints to still exist —
    /// the edge of a forgotten fact is already gone.
    #[wasm_bindgen(js_name = unrelate)]
    pub fn unrelate(&self, from: &str, to: &str, relation: &str) -> Result<JsValue, JsValue> {
        let from = parse_id(from)?;
        let to = parse_id(to)?;
        let outcome = self.inner.unrelate(from, to, relation).map_err(to_js_err)?;
        to_js(&UnrelateOut {
            found: outcome.found,
            removed: outcome.removed,
        })
    }

    /// Delete a memory by id. Returns whether a memory actually existed
    /// under that id and was deleted — `false` means nothing was stored
    /// there (a stale id or a typo), not a second successful deletion.
    #[wasm_bindgen(js_name = forget)]
    pub fn forget(&self, id: &str) -> Result<bool, JsValue> {
        let id = parse_id(id)?;
        self.inner.forget(id).map_err(to_js_err)
    }

    /// Explain a decision: the best-matching memory plus its connected
    /// subgraph. Resolves to `{nodes, edges, truncated}` — `truncated` is
    /// `true` when a width budget cut the walk, since a subgraph sitting
    /// exactly at a cap is otherwise indistinguishable from a complete one.
    /// `maxHops` (default 2) is capped at 10.
    #[wasm_bindgen(js_name = why)]
    pub fn why(
        &self,
        decision: &str,
        max_hops: Option<usize>,
        filter: JsValue,
    ) -> Result<JsValue, JsValue> {
        let max_hops = velesdb_memory::limits::clamp_hops(
            max_hops.unwrap_or(velesdb_memory::limits::DEFAULT_WHY_HOPS),
        );
        let filter = to_metadata(filter)?;
        let explanation = self
            .inner
            .why(decision, max_hops, filter.as_ref())
            .map_err(to_js_err)?;
        to_js(&ExplanationOut::from(explanation))
    }

    /// Look up everything the memory graph knows about a NAMED ENTITY (a
    /// person, a place, an organisation): the attributes merged onto its hub
    /// and the typed edges leaving it. Answers a question ABOUT a thing
    /// ("how old is X", "who is X's father") rather than about the sentences
    /// mentioning it, which is all [`Self::recall`] can return — entity hubs
    /// are deliberately invisible to recall, so without this the attributes
    /// the graph carries would be unreachable.
    ///
    /// `name` is matched case-insensitively (the id is content-addressed, so
    /// it is stable). Returns
    /// `{found, id, name, attributes, relations, relationsIn,
    /// relationsTruncated, relationsInTruncated}`, each edge
    /// `{predicate, targetId, target}`. `found: false` means nothing has ever
    /// mentioned that name; `name` still echoes the query canonicalized, so
    /// several lookups can be told apart. The two `*Truncated` booleans say
    /// when a response budget cut the matching side — a list holding exactly
    /// the cap is otherwise indistinguishable from a cut one.
    ///
    /// `relations` are the typed edges LEAVING the entity, `relationsIn`
    /// those pointing AT it — each naming, in `targetId`/`target`, the far
    /// end it comes FROM. Without the second list a question is only
    /// answerable from one side: the graph holds
    /// `camille --sister of--> theo`, so reading Theo's outgoing edges never
    /// finds Camille.
    ///
    /// Entity hubs are created exclusively by extraction, so this answered
    /// `found: false` for every name until `rememberExtracted` landed on this
    /// binding alongside it.
    #[wasm_bindgen(js_name = entity)]
    pub fn entity(&self, name: &str) -> Result<JsValue, JsValue> {
        let profile = self.inner.entity_profile(name).map_err(to_js_err)?;
        to_js(&EntityProfileOut::from_lookup(name, profile))
    }

    /// Extract atomic facts from `text` and wire the described entity graph,
    /// with no manual `relate()`. Resolves to
    /// `{ids, skippedOverCap}` — the stored ids as decimal strings, and how
    /// many facts were dropped for exceeding the embeddable cap.
    ///
    /// `extractor` names the backend, defaulting to `"outline"` — the
    /// deterministic, network-free one, which reads the structure the passage
    /// STATES instead of inferring it (one directive per line: `edge:`,
    /// `attr:`, `fact:`, see
    /// [`velesdb_memory::OutlineExtractor`]). `"ollama"` is deliberately
    /// absent here and refused by name rather than silently ignored: it is a
    /// network call, and keeping it out of the bundle is the reason this
    /// binding exists.
    ///
    /// This is the WRITE side of `entity`. Entity hubs are born only of
    /// extraction, so before this method the read side answered `found:
    /// false` for every name on this binding.
    #[wasm_bindgen(js_name = rememberExtracted)]
    pub fn remember_extracted(
        &self,
        text: &str,
        metadata: JsValue,
        extractor: Option<String>,
    ) -> Result<JsValue, JsValue> {
        let kind = extractor.unwrap_or_else(|| "outline".to_owned());
        if kind != "outline" {
            return Err(invalid_input(format!(
                "unknown extractor '{kind}' (this binding offers 'outline' only: a generative \
                 backend would put a network call in the WASM bundle)"
            )));
        }
        let metadata = to_metadata(metadata)?;
        let outcome = self
            .inner
            .remember_extracted(text, &OutlineExtractor, metadata.as_ref())
            .map_err(to_js_err)?;
        to_js(&RememberedExtractionOut {
            ids: outcome.ids.into_iter().map(id_to_string).collect(),
            skipped_over_cap: outcome.skipped_over_cap,
        })
    }

    /// Compile context fragments into a token-budgeted, provenance-audited
    /// prompt context — deterministic, no LLM, byte-identical to the native
    /// compiler on the same input (same core code). Request and result use
    /// the MCP `compile_context` wire shape, with this binding's id contract:
    /// every id field crosses as a decimal string.
    ///
    /// In-memory semantics: externalized sources and savings events live in
    /// this session's [`WasmStore`] — `ctx://source/` handles resolve only
    /// within the current browser session (no persistence in WASM).
    #[wasm_bindgen(js_name = compileContext)]
    pub fn compile_context(&self, request: JsValue) -> Result<JsValue, JsValue> {
        let mut request: Value = serde_wasm_bindgen::from_value(request)
            .map_err(|e| invalid_input(format!("invalid compile request: {e}")))?;
        parse_fragment_id_strings(&mut request)?;
        let request: CompileRequest = serde_json::from_value(request)
            .map_err(|e| invalid_input(format!("invalid compile request: {e}")))?;
        // Annotated, not inferred: this binding relays the SERVER'S own
        // `CompiledContext` untouched, and naming the type is what makes that
        // checkable — `tests/binding_parity_bdd.rs` reads it to know the
        // shape cannot lose a field, now or when the type grows one.
        let compiled: CompiledContext = self
            .inner
            .compile_context(&ContextCompiler::new(CompilePolicy::default()), &request)
            .map_err(to_js_err)?;
        let mut value = serde_json::to_value(&compiled)
            .map_err(|e| structured_js_error(CODE_INTERNAL, &format!("serialize: {e}")))?;
        stringify_id_fields(&mut value);
        to_js(&value)
    }

    /// One-call shortcut over [`Self::compile_context`] for a raw
    /// agent-session transcript: deterministically segments it into turns
    /// (plain marker-based — `System:`/`User:`/`Human:`/`Assistant:`/`AI:`/
    /// `Tool:`/`### User`/`### Assistant` — or JSONL, one line per turn) and,
    /// within each turn, into code/log/body sub-segments (fenced code blocks
    /// stay atomic; runs of 8+ log-like lines collapse the same way
    /// `abstract.log_dedup` would), then compiles the result exactly like
    /// [`Self::compile_context`]. Mirrors the MCP `compile_transcript`
    /// tool's `transcript` (inline) input — the tool's `path` field is NOT
    /// supported here (no filesystem in WASM; see the module docs). Returns
    /// `{context, segmentation}`: `context` is byte-compatible with
    /// [`Self::compile_context`]'s own output; `segmentation` is the
    /// detected format plus one audit entry (turn, role, kind, byte range,
    /// `fragment_id` — already a decimal string) per segment, so a caller
    /// can see exactly how the transcript was cut before trusting the
    /// compiled result.
    ///
    /// In-memory semantics: same as [`Self::compile_context`] — externalized
    /// sources and savings events live only in this session's [`WasmStore`].
    #[wasm_bindgen(js_name = compileTranscript)]
    pub fn compile_transcript(&self, request: JsValue) -> Result<JsValue, JsValue> {
        let input: CompileTranscriptInput = serde_wasm_bindgen::from_value(request)
            .map_err(|e| invalid_input(format!("invalid compile_transcript request: {e}")))?;
        let (request, segmentation) = build_transcript_compile_request(input).map_err(to_js_err)?;
        let compiled = self
            .inner
            .compile_context(&ContextCompiler::new(CompilePolicy::default()), &request)
            .map_err(to_js_err)?;
        let mut context_value = serde_json::to_value(&compiled)
            .map_err(|e| structured_js_error(CODE_INTERNAL, &format!("serialize: {e}")))?;
        stringify_id_fields(&mut context_value);
        to_js(&CompileTranscriptOut {
            context: context_value,
            segmentation,
        })
    }

    /// Aggregate the token (and cost) savings of past
    /// [`Self::compile_context`] / [`Self::compile_transcript`] calls,
    /// optionally narrowed to one `project`. Same JSON shape as the MCP
    /// `context_savings` tool and the Node binding's `contextSavings`. Pure
    /// delegation to `velesdb_memory`'s bridge — zero logic in the binding.
    ///
    /// In-memory semantics: like [`Self::compile_context`], the aggregated
    /// events live only in this session's [`WasmStore`].
    #[wasm_bindgen(js_name = contextSavings)]
    pub fn context_savings(&self, project: Option<String>) -> Result<JsValue, JsValue> {
        // Type annotated for the same reason as `compile_context`: the
        // relayed shape is the server's own `ContextSavings`.
        let savings: ContextSavings = self
            .inner
            .context_savings(project.as_deref())
            .map_err(to_js_err)?;
        to_js(&savings)
    }

    /// Explain why one fragment of a [`Self::compile_context`] request was
    /// preserved, abstracted, externalized, dropped, or cached. Compilation
    /// is deterministic, so `request` is re-compiled (event/source recording
    /// forced off) and the matching decision is returned — no server-side
    /// state needed. Same request/response shape as the MCP
    /// `explain_compilation` tool and the Node binding's
    /// `explainCompilation`: `fragmentIndex` (0-based position in
    /// `request.fragments`), when given, TAKES PRIORITY over `fragmentId`
    /// for locating the decision — see the MCP tool's own docs for the full
    /// disambiguation rationale (byte-identical fragments share a
    /// content-addressed id). Id fields on the returned decision cross as
    /// decimal strings, like [`Self::compile_context`].
    #[wasm_bindgen(js_name = explainCompilation)]
    pub fn explain_compilation(
        &self,
        request: JsValue,
        fragment_id: &str,
        fragment_index: Option<usize>,
    ) -> Result<JsValue, JsValue> {
        let mut request: Value = serde_wasm_bindgen::from_value(request)
            .map_err(|e| invalid_input(format!("invalid compile request: {e}")))?;
        parse_fragment_id_strings(&mut request)?;
        let request: CompileRequest = serde_json::from_value(request)
            .map_err(|e| invalid_input(format!("invalid compile request: {e}")))?;
        let fragment_id = parse_id(fragment_id)?;
        // Type annotated for the same reason as `compile_context`: the
        // relayed shape is the server's own `ContextDecision`.
        let decision: ContextDecision = self
            .inner
            .explain_compilation(&request, fragment_id, fragment_index)
            .map_err(to_js_err)?;
        let mut value = serde_json::to_value(&decision)
            .map_err(|e| structured_js_error(CODE_INTERNAL, &format!("serialize: {e}")))?;
        stringify_id_fields(&mut value);
        to_js(&value)
    }

    /// Suggest a starting `tokenBudget` for [`Self::compile_context`] /
    /// [`Self::compile_transcript`], for a named target model — looked up in
    /// a static, committed model-name to context-window table (dated "as
    /// of", NEVER a network call). Pass `reserveTokens` (default 0) to
    /// reserve room for the response, mirroring `compile_context`'s own
    /// `policy.response_reserve_tokens`. `window`/`suggested_budget` come
    /// back `null` when the model is not in the table — an honest "unknown",
    /// never a guess.
    #[wasm_bindgen(js_name = suggestBudget)]
    pub fn suggest_budget(
        &self,
        target_model: &str,
        reserve_tokens: Option<u64>,
    ) -> Result<JsValue, JsValue> {
        // Type annotated for the same reason as `compile_context`: the
        // relayed shape is the server's own `SuggestedBudget`.
        let budget: SuggestedBudget =
            suggest_token_budget(target_model, reserve_tokens.unwrap_or(0));
        to_js(&budget)
    }

    /// Fetch back the exact original content — and media, when the fragment
    /// carried one — behind a `ctx://source/<hash>` handle from a
    /// [`compile_context`](Self::compile_context) result: what was
    /// externalized or partially packed is recoverable, not lost. Same wire
    /// shape as the Node binding's `retrieveContextSource`: `{handle,
    /// content, media?}`, `media` present only for a source whose fragment
    /// carried one.
    ///
    /// In-memory semantics: the handle resolves only within this session's
    /// [`WasmStore`] — see [`Self::compile_context`]'s doc comment.
    #[wasm_bindgen(js_name = retrieveContextSource)]
    pub fn retrieve_context_source(&self, handle: &str) -> Result<JsValue, JsValue> {
        let source = self
            .inner
            .retrieve_context_source(handle)
            .map_err(to_js_err)?;
        let value = serde_json::to_value(&source)
            .map_err(|e| structured_js_error(CODE_INTERNAL, &format!("serialize: {e}")))?;
        let Value::Object(mut map) = value else {
            return Err(structured_js_error(
                CODE_INTERNAL,
                "context source is not an object",
            ));
        };
        map.insert("handle".to_owned(), Value::String(handle.to_owned()));
        to_js(&Value::Object(map))
    }

    /// Persist the agent's distilled working state under `project` +
    /// `session` (idempotent upsert: saving again replaces the previous
    /// state), for later resumption (#1517, option 2). Same wire shape as
    /// the Node binding's `saveWorkingContext` — the request's own field
    /// names (`goal`, `active_constraints`, `decisions`, …), decimal-string
    /// ids — pure delegation to `velesdb_memory`'s bridge, no reshaping.
    /// Resolves to the stored fact id as a decimal string.
    ///
    /// **In-memory semantics**: like [`Self::compile_context`], this is
    /// backed entirely by this session's [`WasmStore`] — there is no
    /// filesystem or IndexedDB persistence behind this binding. A "saved"
    /// working context disappears the moment the page (or worker) that
    /// created this `MemoryService` instance is gone. This is useful to
    /// carry state between two calls made within the SAME page load (e.g.
    /// across two `compileContext` calls), not to resume a session after a
    /// reload — that would need a real browser-storage backend, which does
    /// not exist yet.
    #[wasm_bindgen(js_name = saveWorkingContext)]
    pub fn save_working_context(
        &self,
        project: &str,
        session: &str,
        working: JsValue,
    ) -> Result<String, JsValue> {
        let mut working: Value = serde_wasm_bindgen::from_value(working)
            .map_err(|e| invalid_input(format!("invalid working context: {e}")))?;
        parse_id_fields(&mut working)?;
        let working: WorkingContext = serde_json::from_value(working)
            .map_err(|e| invalid_input(format!("invalid working context: {e}")))?;
        self.inner
            .save_working_context(project, session, &working)
            .map(id_to_string)
            .map_err(to_js_err)
    }

    /// The resumption envelope for `project` + `session` — the
    /// start-of-session mirror of [`Self::save_working_context`] (#1517,
    /// option 2), same shape as the MCP `load_working_context` tool:
    /// `{found, working, other_sessions}`.
    ///
    /// **BREAKING (0.12.0)**: this used to resolve the bare working context
    /// (or `null`), which collapsed two different answers into one — a
    /// project that never saved anything, and a typo in `session` that
    /// missed a session which does exist. `other_sessions` is what tells
    /// them apart, and it is filled in on a HIT too: a typo landing on
    /// another REAL session returns `found: true`, the case a caller can
    /// least detect on its own. Read `.working` for the previous return
    /// value.
    ///
    /// **In-memory semantics**: see [`Self::save_working_context`]'s doc
    /// comment — this only ever resolves what THIS session's [`WasmStore`]
    /// still holds; nothing persists across a page reload.
    #[wasm_bindgen(js_name = loadWorkingContext)]
    pub fn load_working_context(&self, project: &str, session: &str) -> Result<JsValue, JsValue> {
        // Annotated, not inferred: `binding_parity_bdd` reads this type name
        // to prove the binding relays the SERVER's own envelope rather than a
        // shape it recomposed by hand — and the compiler makes that proof
        // real. The doc comment above describes the envelope; only this
        // enforces it.
        let loaded: LoadedWorkingContext = self
            .inner
            .resume_working_context(project, session)
            .map_err(to_js_err)?;
        let mut value = serde_json::to_value(&loaded)
            .map_err(|e| structured_js_error(CODE_INTERNAL, &format!("serialize: {e}")))?;
        // Applied at the ROOT of the envelope, not to `working` alone: the
        // walk descends by KEY NAME, so it still reaches
        // `working.decisions[].fragment_id` and
        // `working.exact_evidence[].fragment_id` one level deeper.
        stringify_id_fields(&mut value);
        to_js(&value)
    }

    /// Every session ever saved under `project`'s working-context index,
    /// most-recently-saved first (by the index's write order: `saved_at` is
    /// always `0` here, wasm has no clock): resolves to `{sessions: [{session,
    /// saved_at}]}`, empty (never an error) when nothing was saved (#1517).
    ///
    /// **In-memory semantics**: see [`Self::save_working_context`]'s doc
    /// comment — reflects only what this session's [`WasmStore`] currently
    /// holds, never a cross-session/browser-restart view.
    #[wasm_bindgen(js_name = listWorkingContexts)]
    pub fn list_working_contexts(&self, project: &str) -> Result<JsValue, JsValue> {
        let sessions = self
            .inner
            .list_working_contexts(project)
            .map_err(to_js_err)?;
        let value = serde_json::to_value(SessionsOut { sessions })
            .map_err(|e| structured_js_error(CODE_INTERNAL, &format!("serialize: {e}")))?;
        to_js(&value)
    }
}

/// Wire envelope for [`WasmMemoryService::list_working_contexts`]: same
/// shape as the MCP `list_working_contexts` tool's result
/// (`{sessions: [...]}`), field names left as `WorkingContextSession`
/// serializes them (`session`, `saved_at`) — no camelCase remapping, for the
/// same reason `compile_context`/`retrieveContextSource` don't reshape their
/// output either.
#[derive(Serialize)]
struct SessionsOut {
    sessions: Vec<WorkingContextSession>,
}

#[cfg(test)]
#[path = "memory_service_tests.rs"]
mod tests;
