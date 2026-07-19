//! WASM binding for `velesdb-memory`'s agent-memory wedge — `remember` /
//! `recall` / `recallWhere` / `recallFused` / `relate` / `forget` / `why`,
//! backed entirely in-memory ([`WasmStore`]): no filesystem, no network, no
//! `persistence` feature.
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
//! `rememberExtracted` is omitted in this first cut: it needs a generative
//! model, which would pull a network dependency into the WASM bundle by
//! default. A JS-provided extractor callback is a natural v2 addition.

use serde::Serialize;
use serde_json::Value;
use wasm_bindgen::prelude::*;

use velesdb_memory::context::{CompilePolicy, CompileRequest, ContextCompiler};
use velesdb_memory::{
    ColumnFilter, ColumnOp, Explanation, FusionOptions, HashEmbedder, MemoryEdge, MemoryError,
    MemoryNode, MemoryService, Metadata, Recollection,
};

use crate::memory_store::WasmStore;

const CODE_INVALID_INPUT: &str = "INVALID_INPUT";
const CODE_NOT_FOUND: &str = "NOT_FOUND";
const CODE_INTERNAL: &str = "INTERNAL";

// --- Errors ------------------------------------------------------------

use crate::wasm_error::structured_js_error;

fn category_code(e: &MemoryError) -> &'static str {
    use velesdb_memory::ErrorCategory;
    match e.category() {
        ErrorCategory::InvalidInput => CODE_INVALID_INPUT,
        ErrorCategory::NotFound => CODE_NOT_FOUND,
        ErrorCategory::Internal => CODE_INTERNAL,
    }
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

#[derive(Serialize)]
struct ExplanationOut {
    nodes: Vec<MemoryNodeOut>,
    edges: Vec<MemoryEdgeOut>,
}

impl From<Explanation> for ExplanationOut {
    fn from(e: Explanation) -> Self {
        Self {
            nodes: e.nodes.into_iter().map(MemoryNodeOut::from).collect(),
            edges: e.edges.into_iter().map(MemoryEdgeOut::from).collect(),
        }
    }
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
    /// many seconds (omit, or `0`, for a permanent memory).
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

    /// Delete a memory by id. Returns whether a memory actually existed
    /// under that id and was deleted — `false` means nothing was stored
    /// there (a stale id or a typo), not a second successful deletion.
    #[wasm_bindgen(js_name = forget)]
    pub fn forget(&self, id: &str) -> Result<bool, JsValue> {
        let id = parse_id(id)?;
        self.inner.forget(id).map_err(to_js_err)
    }

    /// Explain a decision: the best-matching memory plus its connected
    /// subgraph. Resolves to `{nodes, edges}`. `maxHops` (default 2) is
    /// capped at 10.
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
        let compiled = self
            .inner
            .compile_context(&ContextCompiler::new(CompilePolicy::default()), &request)
            .map_err(to_js_err)?;
        let mut value = serde_json::to_value(&compiled)
            .map_err(|e| structured_js_error(CODE_INTERNAL, &format!("serialize: {e}")))?;
        stringify_id_fields(&mut value);
        to_js(&value)
    }
}

#[cfg(test)]
#[path = "memory_service_tests.rs"]
mod tests;
