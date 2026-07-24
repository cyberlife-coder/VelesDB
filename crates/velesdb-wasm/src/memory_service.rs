//! WASM binding for `velesdb-memory`'s agent-memory wedge — `remember` /
//! `recall` / `recallWhere` / `recallFused` / `relate` / `forget` / `why` /
//! `compileContext` / `compileTranscript` / `explainCompilation` /
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
//! Two methods available on the Node/Python bindings are deliberately absent
//! here, both re-confirmed by issue #1547's audit:
//!
//! - `feedback` (RL Memory): [`MemoryService::feedback`] lives in the
//!   `persistence`-gated `reinforce` module
//!   (`crates/velesdb-memory/src/service.rs`'s own doc comment on that
//!   module: "a durable learned confidence is meaningless on the in-memory
//!   (WASM) backend"), so it is not even compiled into this crate — adding
//!   it would mean enabling `persistence` for the `wasm32` target, pulling
//!   in `NativeStore`/filesystem code this binding exists specifically to
//!   avoid. Not a "missing binding"; an intentional architectural boundary.
//! - `rememberExtracted`: it needs a generative model (`OllamaExtractor` is
//!   the only [`velesdb_memory::extract::Extractor`] impl in the crate),
//!   which would pull a network dependency into the WASM bundle by default.
//!   A JS-provided extractor callback is a natural v2 addition.
//!
//! `compileTranscript` accepts only an inline `transcript` string, never the
//! MCP tool's `path` field — this binding has no filesystem, so a `path`
//! input has nothing to resolve against.

use serde::Serialize;
use serde_json::Value;
use wasm_bindgen::prelude::*;

use velesdb_memory::context::{
    fragment_id as ctx_fragment_id, segment_transcript, suggest_token_budget, CompilePolicy,
    CompileRequest, ContextCompiler, SegmentFormat, SegmentKind, SegmentationPolicy,
    WorkingContext, WorkingContextSession,
};
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

/// Input of [`WasmMemoryService::compile_transcript`] — the same fields as
/// the MCP `compile_transcript` tool's request MINUS `path`: this binding
/// has no filesystem (see the module docs), so only an inline `transcript`
/// is accepted.
#[derive(serde::Deserialize)]
struct CompileTranscriptInput {
    query: String,
    transcript: String,
    token_budget: u64,
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    target_model: Option<String>,
    #[serde(default)]
    policy: Option<CompilePolicy>,
    #[serde(default)]
    segmentation: Option<SegmentationPolicy>,
}

/// One entry of [`SegmentationReportOut::segments`] — same shape as the MCP
/// `compile_transcript` tool's own (private) `SegmentInfo`, rebuilt here
/// rather than reused since that type is `pub(super)` to `velesdb_memory`'s
/// `mcp` module. `fragment_id` is already a decimal string, so no separate
/// stringify pass is needed for the segmentation half of the response.
#[derive(Debug, Serialize)]
struct SegmentInfoOut {
    index: usize,
    turn: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    kind: SegmentKind,
    byte_start: usize,
    byte_end: usize,
    fragment_id: String,
}

/// The segmentation audit trail returned alongside `context` by
/// [`WasmMemoryService::compile_transcript`].
#[derive(Debug, Serialize)]
struct SegmentationReportOut {
    format_detected: SegmentFormat,
    segments: Vec<SegmentInfoOut>,
    merged_segments: usize,
}

/// Output of [`WasmMemoryService::compile_transcript`]: the compiled context
/// (already id-stringified, byte-compatible with [`WasmMemoryService::compile_context`]'s
/// own output) plus how the transcript was cut into fragments before compilation.
#[derive(Serialize)]
struct CompileTranscriptOut {
    context: Value,
    segmentation: SegmentationReportOut,
}

/// The pure-Rust half of [`WasmMemoryService::compile_transcript`]: segments
/// `input.transcript` and assembles the [`CompileRequest`] /
/// [`SegmentationReportOut`] pair `compile_context` then compiles — split
/// out from the `#[wasm_bindgen]` method (which only marshals `JsValue` in
/// and out) specifically so this glue is testable from a native `cargo test`
/// (a `JsValue` cannot be constructed off `wasm32`; see
/// `memory_service_tests.rs`'s module docs). Returns
/// [`MemoryError::SegmentationError`] for an empty transcript — mirroring
/// the MCP `compile_transcript` tool's own empty-transcript guard, since
/// `segment_transcript` has no such check itself (an empty string is a
/// valid, if useless, zero-turn input to it) — or whatever error
/// `segment_transcript` itself returns (a genuine budget/cap breach, or a
/// forced-format parse failure).
fn build_transcript_compile_request(
    input: CompileTranscriptInput,
) -> Result<(CompileRequest, SegmentationReportOut), MemoryError> {
    if input.transcript.is_empty() {
        return Err(MemoryError::SegmentationError(
            "the transcript is empty — `transcript` must be non-empty text".to_owned(),
        ));
    }
    let segmentation_policy = input.segmentation.unwrap_or_default();
    let outcome = segment_transcript(&input.transcript, &segmentation_policy)?;
    let segments_info: Vec<SegmentInfoOut> = outcome
        .segments
        .iter()
        .enumerate()
        .map(|(index, segment)| SegmentInfoOut {
            index,
            turn: segment.turn,
            role: segment.role.clone(),
            kind: segment.kind,
            byte_start: segment.byte_start,
            byte_end: segment.byte_end,
            fragment_id: id_to_string(ctx_fragment_id(&segment.fragment.content)),
        })
        .collect();
    let format_detected = outcome.format_detected;
    let merged_segments = outcome.merged_segments;
    let fragments = outcome.segments.into_iter().map(|s| s.fragment).collect();
    let request = CompileRequest {
        query: input.query,
        fragments,
        project: input.project,
        target_model: input.target_model,
        token_budget: input.token_budget,
        memory_scope: None,
        policy: input.policy,
    };
    Ok((
        request,
        SegmentationReportOut {
            format_detected,
            segments: segments_info,
            merged_segments,
        },
    ))
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
        let savings = self
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
        let decision = self
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
        let budget = suggest_token_budget(target_model, reserve_tokens.unwrap_or(0));
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

    /// The working context previously saved under `project` + `session` —
    /// `null` in JS when there is none, the start-of-session mirror of
    /// [`Self::save_working_context`] (#1517, option 2).
    ///
    /// **In-memory semantics**: see [`Self::save_working_context`]'s doc
    /// comment — this only ever resolves what THIS session's [`WasmStore`]
    /// still holds; nothing persists across a page reload.
    #[wasm_bindgen(js_name = loadWorkingContext)]
    pub fn load_working_context(&self, project: &str, session: &str) -> Result<JsValue, JsValue> {
        let loaded = self
            .inner
            .load_working_context(project, session)
            .map_err(to_js_err)?;
        match loaded {
            Some(working) => {
                let mut value = serde_json::to_value(&working)
                    .map_err(|e| structured_js_error(CODE_INTERNAL, &format!("serialize: {e}")))?;
                stringify_id_fields(&mut value);
                to_js(&value)
            }
            None => Ok(JsValue::NULL),
        }
    }

    /// Every session ever saved under `project`'s working-context index,
    /// most-recently-saved first: resolves to `{sessions: [{session,
    /// saved_at}]}` — empty (never an error) when the project never saved
    /// anything (#1517, option 2).
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
