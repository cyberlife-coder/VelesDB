//! MCP transport: exposes the memory service as MCP tools over stdio.
//!
//! Only **memory semantics** are exposed (`remember / recall / relate / forget
//! / why`) — never raw database capabilities. See [`crate`] docs for the
//! license boundary this enforces.

use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::schema_for_input;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{ErrorCode, Implementation, JsonObject, ServerCapabilities, ServerInfo};
use rmcp::{tool, tool_handler, tool_router, ErrorData, ServerHandler};
use schemars::JsonSchema;

use crate::limits::{DEFAULT_WHY_HOPS, MAX_FACT_BYTES, MAX_RECALL_LIMIT, MAX_WHY_HOPS};
use crate::model::FusionOptions;
use crate::service::MemoryService;

/// Default number of memories returned by `recall`.
const DEFAULT_RECALL_LIMIT: usize = 10;

// The boxed embedder and the shared, runtime-attached extraction backend the
// server stores — imported for internal use only. The canonical public paths are
// `velesdb_memory::DynEmbedder` / `velesdb_memory::DynExtractor` (crate root).
use crate::embedder::DynEmbedder;
use crate::extract::DynExtractor;

// --- Tool parameter / result DTOs ------------------------------------------
//
// The request envelopes, small id-results, and the id-echoing wire wrappers
// (`RecollectionDto`, `ExplanationDto` — the `id_str` twins of issue #1468)
// live in their own module so this file stays focused on the server and tool
// wiring; the domain types in `crate::model` are unchanged.
/// The context compiler's eight tools — a second `#[tool_router]` block whose
/// router is combined with the main one below, extending the ONE server.
#[cfg(feature = "context")]
mod context_tools;

mod dto;
use dto::{
    ExplanationDto, FeedbackParams, FeedbackResult, ForgetParams, ForgetResult, RecallFusedParams,
    RecallFusedResult, RecallParams, RecallResult, RecallWhereParams, RelateParams, RelateResult,
    RememberExtractedParams, RememberExtractedResult, RememberParams, RememberResult, WhyParams,
};

/// Advertised-schema counterpart of [`crate::model::deserialize_id`]: `keys`
/// (e.g. `["from", "to"]`) widen from `integer` to `["integer", "string"]` so
/// a client generating requests from the schema can discover the
/// decimal-string form (issue #1468). Scoped per tool — each takes a
/// different set of id-named parameters (`relate`'s `from`/`to`,
/// `forget`/`feedback`'s `id`, `remember`'s nested `links[].target`) — unlike
/// the context compiler's `wire_safe_input_schema` (single hardcoded `"id"`
/// key, `context`-feature-gated). Both reuse the same underlying
/// [`crate::schema::widen_id_properties`] tree walk.
fn id_wire_input_schema<T: JsonSchema + std::any::Any>(keys: &[&str]) -> Arc<JsonObject> {
    let schema = schema_for_input::<Parameters<T>>().unwrap_or_else(|e| {
        panic!(
            "Invalid input schema for {}: {e}",
            std::any::type_name::<T>()
        )
    });
    let mut map = (*schema).clone();
    crate::schema::widen_id_properties(&mut map, keys);
    Arc::new(map)
}

// --- The server ------------------------------------------------------------

/// MCP server wrapping a [`MemoryService`].
#[derive(Clone)]
pub struct McpServer {
    service: Arc<MemoryService<DynEmbedder>>,
    /// Optional extraction backend powering `remember_extracted`. `None` unless
    /// a backend is attached via [`Self::with_extractor`]; the tool then reports
    /// extraction as unconfigured.
    extractor: Option<DynExtractor>,
    /// Default time-to-live (seconds) applied to `remember`d facts that don't
    /// specify their own `ttl_seconds`. `None` (the default) stores permanently.
    /// Set from `VELESDB_MEMORY_DEFAULT_TTL` by the binary.
    default_ttl: Option<u64>,
    /// Allowlisted filesystem roots for `path`-referenced context fragments
    /// (V2b-1). `None` (the default) disables path ingestion entirely — every
    /// `path` fragment fails with an explicit error. Set from
    /// `VELESDB_MEMORY_INGEST_ROOTS` by the binary via [`Self::with_ingest_roots`].
    #[cfg(all(feature = "context", not(target_arch = "wasm32")))]
    ingest_roots: Option<crate::context::IngestRoots>,
    tool_router: ToolRouter<McpServer>,
}

#[tool_router]
impl McpServer {
    /// Wrap a memory service as an MCP server.
    #[must_use]
    pub fn new(service: MemoryService<DynEmbedder>) -> Self {
        Self {
            service: Arc::new(service),
            extractor: None,
            default_ttl: None,
            #[cfg(all(feature = "context", not(target_arch = "wasm32")))]
            ingest_roots: None,
            tool_router: Self::combined_router(),
        }
    }

    /// The full tool router: the memory tools, plus the context compiler's
    /// tools when that feature is on. Combined here — rmcp routers add — so
    /// there is exactly ONE server whichever features are enabled.
    fn combined_router() -> ToolRouter<McpServer> {
        #[cfg(feature = "context")]
        {
            Self::tool_router() + Self::context_tool_router()
        }
        #[cfg(not(feature = "context"))]
        {
            Self::tool_router()
        }
    }

    /// Attach an extraction backend, enabling the `remember_extracted` tool.
    /// Without it the tool reports that extraction is not configured.
    #[must_use]
    pub fn with_extractor(mut self, extractor: DynExtractor) -> Self {
        self.extractor = Some(extractor);
        self
    }

    /// Apply a default TTL (seconds) to `remember`d facts that don't carry their
    /// own `ttl_seconds`. `0` is treated as "no default" (permanent).
    #[must_use]
    pub fn with_default_ttl(mut self, ttl_seconds: u64) -> Self {
        self.default_ttl = (ttl_seconds > 0).then_some(ttl_seconds);
        self
    }

    /// Enable path ingestion (V2b-1): `compile_context` and
    /// `explain_compilation` fragments carrying `path` are resolved against
    /// this allowlist before compilation. Without this (the default), every
    /// `path` fragment fails with an explicit "ingestion disabled" error —
    /// same pattern as [`Self::with_extractor`].
    #[cfg(all(feature = "context", not(target_arch = "wasm32")))]
    #[must_use]
    pub fn with_ingest_roots(mut self, roots: crate::context::IngestRoots) -> Self {
        self.ingest_roots = Some(roots);
        self
    }

    #[tool(
        name = "remember",
        description = "Store a fact in durable local memory. Optionally link it to existing memories (graph) and tag it with structured metadata like project/author/type/status/date (ColumnStore) for later filtering — metadata is capped at 64 KiB serialized. Set `ttl_seconds` to make the fact expire after a delay (a durable TTL that survives restarts); omit it for a permanent memory. Returns the fact's stable id. Ids exceed 2^53 — always relay them as strings (`id_str`); passing a JSON-number id read from a previous response will fail on float-lossy clients.",
        input_schema = id_wire_input_schema::<RememberParams>(&["target"])
    )]
    async fn remember(
        &self,
        Parameters(params): Parameters<RememberParams>,
    ) -> Result<Json<RememberResult>, ErrorData> {
        if params.fact.len() > MAX_FACT_BYTES {
            return Err(ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                format!("fact exceeds maximum size of {MAX_FACT_BYTES} bytes"),
                None,
            ));
        }
        let service = Arc::clone(&self.service);
        let RememberParams {
            fact,
            links,
            metadata,
            ttl_seconds,
        } = params;
        let ttl = ttl_seconds.or(self.default_ttl);
        let id = tokio::task::spawn_blocking(move || {
            service.remember_with_ttl(&fact, &links, metadata.as_ref(), ttl)
        })
        .await
        .map_err(join_error)?
        .map_err(to_error)?;
        Ok(Json(RememberResult {
            id,
            id_str: id.to_string(),
        }))
    }

    #[tool(
        name = "recall",
        description = "Recall memories semantically similar to a query (vector), most similar first. Optionally narrow to exact-match metadata via `filter` (ColumnStore), e.g. {\"project\":\"veles\",\"status\":\"resolved\"}. Ids exceed 2^53 — always relay them as strings (`id_str`); passing a JSON-number id read from a previous response will fail on float-lossy clients."
    )]
    async fn recall(
        &self,
        Parameters(params): Parameters<RecallParams>,
    ) -> Result<Json<RecallResult>, ErrorData> {
        let limit = params
            .limit
            .unwrap_or(DEFAULT_RECALL_LIMIT)
            .min(MAX_RECALL_LIMIT);
        let service = Arc::clone(&self.service);
        let RecallParams { query, filter, .. } = params;
        let memories =
            tokio::task::spawn_blocking(move || service.recall(&query, limit, filter.as_ref()))
                .await
                .map_err(join_error)?
                .map_err(to_error)?;
        Ok(Json(RecallResult::new(memories)))
    }

    #[tool(
        name = "recall_where",
        description = "Fused recall: semantically similar memories (vector) constrained by structured ColumnStore predicates over metadata — ranges and comparisons, not just equality. Each filter is {field, op (eq/ne/lt/le/gt/ge), value}, ANDed. Use for time-windowed or numeric-scoped recall, e.g. facts about a topic with `ts` in a date range. Comparisons are TYPE-STRICT, with no runtime coercion: a filter value of 20230601 (a JSON number) never matches a fact stored with metadata {\"ts\": \"20230601\"} (a JSON string) — same value, different JSON type, no match, no error. Store comparable values like dates NUMERICALLY at `remember` time (e.g. 20230601, not \"20230601\") so `recall_where` filters actually match them. Most similar first."
    )]
    async fn recall_where(
        &self,
        Parameters(params): Parameters<RecallWhereParams>,
    ) -> Result<Json<RecallResult>, ErrorData> {
        let limit = params
            .limit
            .unwrap_or(DEFAULT_RECALL_LIMIT)
            .min(MAX_RECALL_LIMIT);
        let service = Arc::clone(&self.service);
        let RecallWhereParams { query, filters, .. } = params;
        let memories =
            tokio::task::spawn_blocking(move || service.recall_where(&query, limit, &filters))
                .await
                .map_err(join_error)?
                .map_err(to_error)?;
        Ok(Json(RecallResult::new(memories)))
    }

    #[tool(
        name = "recall_fused",
        description = "Fused vector + graph recall: like `recall`, but also walks the graph from the top vector hit and folds any connected fact into the ranking — the tri-engine ranking (vector similarity + ColumnStore filter + graph reach) measured on multi-hop and temporal benchmarks. Reach for this when an answer needs a fact the query doesn't mention directly but a stored `relate`/extracted link connects (multi-hop reasoning, temporal chains). `hops`/`graph_boost` tune the graph reach; omit them for the proven defaults. Optionally narrow with an exact-match `filter`. Set `date_field` (the metadata key holding a YYYYMMDD date) to also get a `dated_context` timeline and a `now` anchor for temporal questions. Most relevant first."
    )]
    async fn recall_fused(
        &self,
        Parameters(params): Parameters<RecallFusedParams>,
    ) -> Result<Json<RecallFusedResult>, ErrorData> {
        let k = params
            .limit
            .unwrap_or(DEFAULT_RECALL_LIMIT)
            .min(MAX_RECALL_LIMIT);
        let opts = FusionOptions::from_knobs(params.hops, params.graph_boost, None);
        let service = Arc::clone(&self.service);
        let RecallFusedParams {
            query,
            filter,
            date_field,
            ..
        } = params;
        // With a date field, take the shared "recall then format" path so the
        // dated timeline stays identical to the Node/WASM bindings; without one,
        // plain fused recall (no timeline).
        let (memories, dated_context, now) = if let Some(field) = date_field {
            let (hits, ctx) = tokio::task::spawn_blocking(move || {
                service.recall_fused_dated(&query, k, filter.as_ref(), opts, &field)
            })
            .await
            .map_err(join_error)?
            .map_err(to_error)?;
            (hits, Some(ctx.timeline), ctx.now)
        } else {
            let hits = tokio::task::spawn_blocking(move || {
                service.recall_fused(&query, k, filter.as_ref(), opts)
            })
            .await
            .map_err(join_error)?
            .map_err(to_error)?;
            (hits, None, None)
        };
        Ok(Json(RecallFusedResult::new(memories, dated_context, now)))
    }

    #[tool(
        name = "feedback",
        description = "Reinforce a recalled memory with an outcome: `success=true` if the fact was useful, `false` if it was noise. This durably updates the fact's learned confidence, which `recall` uses to re-rank future results — over repeated feedback, useful facts drift up and noise drifts down, so the memory improves with use without retraining the model. Returns the fact's new confidence in [0,1].",
        input_schema = id_wire_input_schema::<FeedbackParams>(&["id"])
    )]
    async fn feedback(
        &self,
        Parameters(params): Parameters<FeedbackParams>,
    ) -> Result<Json<FeedbackResult>, ErrorData> {
        let service = Arc::clone(&self.service);
        let FeedbackParams { id, success } = params;
        let confidence = tokio::task::spawn_blocking(move || service.feedback(id, success))
            .await
            .map_err(join_error)?
            .map_err(to_error)?;
        Ok(Json(FeedbackResult {
            id,
            id_str: id.to_string(),
            confidence,
        }))
    }

    #[tool(
        name = "relate",
        description = "Create a typed, directional link between two memories (`from` → `to`) labeled by `relation`. These links are the graph edges that `why` and `recall_fused` later traverse to surface connected facts that share no words with the query — build the graph with `relate` so multi-hop reasoning works (e.g. link a decision to its cause, a fact to its source, a task to the person it concerns). Direction matters: traversal follows OUTGOING edges only, so point `from` at the memory you will later ask `why` about and `to` at its evidence (decision → cause, fact → source) — an edge pointing INTO a memory is invisible to `why(that memory)`. Idempotent per (from, relation, to). Returns the new edge id. Ids exceed 2^53 — always relay them as strings (`id_str`); passing a JSON-number id read from a previous response will fail on float-lossy clients.",
        input_schema = id_wire_input_schema::<RelateParams>(&["from", "to"])
    )]
    async fn relate(
        &self,
        Parameters(params): Parameters<RelateParams>,
    ) -> Result<Json<RelateResult>, ErrorData> {
        let service = Arc::clone(&self.service);
        let RelateParams { from, to, relation } = params;
        let edge_id = tokio::task::spawn_blocking(move || service.relate(from, to, &relation))
            .await
            .map_err(join_error)?
            .map_err(to_error)?;
        Ok(Json(RelateResult {
            edge_id,
            edge_id_str: edge_id.to_string(),
        }))
    }

    #[tool(
        name = "forget",
        description = "Permanently delete a memory by its `id` (as returned by `remember` or `recall`), removing the fact and its graph links. The deletion is durable and cannot be undone — use it to retract or correct stored knowledge. For automatic time-based expiry instead, set a TTL when calling `remember`. Returns the requested id plus `found`: `true` if a memory actually existed and was deleted, `false` if nothing was stored under that id (a stale id or a typo) — a no-op, not an error, but distinguishable from a real deletion.",
        input_schema = id_wire_input_schema::<ForgetParams>(&["id"])
    )]
    async fn forget(
        &self,
        Parameters(params): Parameters<ForgetParams>,
    ) -> Result<Json<ForgetResult>, ErrorData> {
        let service = Arc::clone(&self.service);
        let id = params.id;
        let found = tokio::task::spawn_blocking(move || service.forget(id))
            .await
            .map_err(join_error)?
            .map_err(to_error)?;
        Ok(Json(ForgetResult {
            id,
            id_str: id.to_string(),
            found,
        }))
    }

    #[tool(
        name = "why",
        description = "Explain a decision: find the best-matching memory (optionally scoped by a metadata `filter`, e.g. the current project) and return the connected subgraph of related memories reachable through typed links — fusing vector, ColumnStore, and graph to surface context a plain similarity search misses."
    )]
    async fn why(
        &self,
        Parameters(params): Parameters<WhyParams>,
    ) -> Result<Json<ExplanationDto>, ErrorData> {
        let max_hops = params
            .max_hops
            .unwrap_or(DEFAULT_WHY_HOPS)
            .min(MAX_WHY_HOPS);
        let service = Arc::clone(&self.service);
        let WhyParams {
            decision, filter, ..
        } = params;
        let explanation =
            tokio::task::spawn_blocking(move || service.why(&decision, max_hops, filter.as_ref()))
                .await
                .map_err(join_error)?
                .map_err(to_error)?;
        Ok(Json(ExplanationDto::from(explanation)))
    }

    #[tool(
        name = "remember_extracted",
        description = "Store a passage of raw text by extracting its atomic facts and auto-building the fact↔topic graph, so `why` can later connect them with no manual links. Requires the server to be started with an extraction backend (set VELESDB_MEMORY_EXTRACTOR; build with --features extract). Returns the stored facts' ids."
    )]
    async fn remember_extracted(
        &self,
        Parameters(params): Parameters<RememberExtractedParams>,
    ) -> Result<Json<RememberExtractedResult>, ErrorData> {
        if params.text.len() > MAX_FACT_BYTES {
            return Err(ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                format!("text exceeds maximum size of {MAX_FACT_BYTES} bytes"),
                None,
            ));
        }
        let Some(extractor) = self.extractor.clone() else {
            return Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                "extraction backend not configured: start the server with \
                 VELESDB_MEMORY_EXTRACTOR set (built with --features extract)",
                None,
            ));
        };
        // Extraction makes a blocking network call (up to the extractor's
        // timeout), so run it off the async worker pool to keep the stdio loop
        // responsive to other tool calls and cancellations.
        let service = Arc::clone(&self.service);
        let RememberExtractedParams { text, metadata } = params;
        let ids = tokio::task::spawn_blocking(move || {
            service.remember_extracted(&text, &extractor, metadata.as_ref())
        })
        .await
        .map_err(join_error)?
        .map_err(to_error)?;
        let ids_str = ids.iter().map(u64::to_string).collect();
        Ok(Json(RememberExtractedResult { ids, ids_str }))
    }
}

/// `#[tool_handler]` generates `call_tool` / `list_tools` from the router;
/// `get_info` is overridden so the server identifies itself as `velesdb-memory`
/// (the macro default falls back to rmcp's own identity). Per-tool guidance
/// lives in each `#[tool(description = …)]`.
/// The server's one-shot vitrine to a connecting agent (V2a-1 quick win):
/// must cover every tool family, not just memory — a `#[cfg(feature =
/// "context")]` variant since the context-compiler tools only exist in that
/// build.
#[cfg(feature = "context")]
const SERVER_INSTRUCTIONS: &str = "Local-first memory and context engineering for AI agents, three tool families: (1) durable memory — remember, recall, recall_fused, recall_where, relate, forget, feedback, and why — explainable (why returns the evidence trail) and self-improving (feedback re-ranks future recall); (2) the deterministic context compiler — compile_context, compile_transcript, explain_compilation, retrieve_context_source, context_savings, and suggest_budget — token-budgets and audits prompt context with no LLM call, ever; (3) cross-session working-context resumption — save_working_context, load_working_context, and list_working_contexts. compile_context/explain_compilation fragments accept a `path` instead of inline `content` to ingest a file by reference — disabled unless the server is started with VELESDB_MEMORY_INGEST_ROOTS set to an allowlist of directories (compile_transcript's own `path` field uses the same allowlist). compile_transcript is a one-call shortcut over compile_context for a raw agent-session transcript: it segments plain or JSONL text into turns before compiling, so an agent no longer needs to segment a transcript by hand. Nothing ever leaves the machine.";

#[cfg(not(feature = "context"))]
const SERVER_INSTRUCTIONS: &str = "Local-first memory for AI agents: remember facts, recall them \
     semantically, relate them, forget them, and ask why a decision was made (connected subgraph).";

#[tool_handler(router = self.tool_router)]
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.server_info = Implementation::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some(SERVER_INSTRUCTIONS.to_owned());
        info
    }
}

/// Map a `spawn_blocking` join failure (a panicked or cancelled tool task) to an
/// MCP error. Every tool body runs on the blocking pool, so they all funnel
/// through this on the (rare) task-failure path.
///
/// Takes the error by value so it can be used as `.map_err(join_error)`.
#[allow(clippy::needless_pass_by_value)]
fn join_error(join: tokio::task::JoinError) -> ErrorData {
    ErrorData::new(
        ErrorCode::INTERNAL_ERROR,
        format!("memory task failed: {join}"),
        None,
    )
}

/// Map a domain error to an MCP error.
///
/// Map a [`MemoryError`](crate::error::MemoryError) onto a JSON-RPC error,
/// driven by its transport-neutral [`ErrorCategory`](crate::error::ErrorCategory)
/// so the MCP taxonomy can never drift from the bindings'. Client-input errors
/// become `invalid_params` (-32602); genuine faults `internal_error` (-32603).
/// JSON-RPC defines no "not found" code, so a missing id is reported as
/// `invalid_params` (a bad id is, from the protocol's view, a bad parameter).
///
/// Takes the error by value so it can be used as `.map_err(to_error)` at every
/// call site without a per-site closure.
#[allow(clippy::needless_pass_by_value)]
fn to_error(err: crate::error::MemoryError) -> ErrorData {
    use crate::error::ErrorCategory;
    let code = match err.category() {
        ErrorCategory::InvalidInput | ErrorCategory::NotFound => ErrorCode::INVALID_PARAMS,
        ErrorCategory::Internal => ErrorCode::INTERNAL_ERROR,
    };
    ErrorData::new(code, err.to_string(), None)
}

#[cfg(test)]
#[path = "mcp/server_tests.rs"]
mod tests;
