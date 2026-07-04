//! MCP transport: exposes the memory service as MCP tools over stdio.
//!
//! Only **memory semantics** are exposed (`remember / recall / relate / forget
//! / why`) — never raw database capabilities. See [`crate`] docs for the
//! license boundary this enforces.

use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{ErrorCode, Implementation, ServerCapabilities, ServerInfo};
use rmcp::{tool, tool_handler, tool_router, ErrorData, ServerHandler};

use crate::limits::{DEFAULT_WHY_HOPS, MAX_FACT_BYTES, MAX_RECALL_LIMIT, MAX_WHY_HOPS};
use crate::model::{Explanation, FusionOptions};
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
// The request envelopes and small id-results live in their own module so this
// file stays focused on the server and tool wiring; output shapes reuse the
// domain types from `crate::model` directly (no duplicate wire/domain struct).
mod dto;
use dto::{
    ForgetParams, ForgetResult, RecallFusedParams, RecallFusedResult, RecallParams, RecallResult,
    RecallWhereParams, RelateParams, RelateResult, RememberExtractedParams,
    RememberExtractedResult, RememberParams, RememberResult, WhyParams,
};

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
            tool_router: Self::tool_router(),
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

    #[tool(
        name = "remember",
        description = "Store a fact in durable local memory. Optionally link it to existing memories (graph) and tag it with structured metadata like project/author/type/status/date (ColumnStore) for later filtering. Set `ttl_seconds` to make the fact expire after a delay (a durable TTL that survives restarts); omit it for a permanent memory. Returns the fact's stable id."
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
        Ok(Json(RememberResult { id }))
    }

    #[tool(
        name = "recall",
        description = "Recall memories semantically similar to a query (vector), most similar first. Optionally narrow to exact-match metadata via `filter` (ColumnStore), e.g. {\"project\":\"veles\",\"status\":\"resolved\"}."
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
        Ok(Json(RecallResult { memories }))
    }

    #[tool(
        name = "recall_where",
        description = "Fused recall: semantically similar memories (vector) constrained by structured ColumnStore predicates over metadata — ranges and comparisons, not just equality. Each filter is {field, op (eq/ne/lt/le/gt/ge), value}, ANDed. Use for time-windowed or numeric-scoped recall, e.g. facts about a topic with `ts` in a date range. Most similar first."
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
        Ok(Json(RecallResult { memories }))
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
        let memories = tokio::task::spawn_blocking(move || {
            service.recall_fused(&query, k, filter.as_ref(), opts)
        })
        .await
        .map_err(join_error)?
        .map_err(to_error)?;
        // When the caller names a date field, also ship the dated timeline it
        // would otherwise have to format itself in a prompt.
        let (dated_context, now) = match date_field {
            Some(field) => {
                let ctx = crate::dated_context::format_dated_context(&memories, &field);
                (Some(ctx.timeline), ctx.now)
            }
            None => (None, None),
        };
        Ok(Json(RecallFusedResult {
            memories,
            dated_context,
            now,
        }))
    }

    #[tool(
        name = "relate",
        description = "Create a typed link from one memory to another. Returns the edge id."
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
        Ok(Json(RelateResult { edge_id }))
    }

    #[tool(name = "forget", description = "Delete a memory by id.")]
    async fn forget(
        &self,
        Parameters(params): Parameters<ForgetParams>,
    ) -> Result<Json<ForgetResult>, ErrorData> {
        let service = Arc::clone(&self.service);
        let id = params.id;
        tokio::task::spawn_blocking(move || service.forget(id))
            .await
            .map_err(join_error)?
            .map_err(to_error)?;
        Ok(Json(ForgetResult { id }))
    }

    #[tool(
        name = "why",
        description = "Explain a decision: find the best-matching memory (optionally scoped by a metadata `filter`, e.g. the current project) and return the connected subgraph of related memories reachable through typed links — fusing vector, ColumnStore, and graph to surface context a plain similarity search misses."
    )]
    async fn why(
        &self,
        Parameters(params): Parameters<WhyParams>,
    ) -> Result<Json<Explanation>, ErrorData> {
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
        Ok(Json(explanation))
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
        Ok(Json(RememberExtractedResult { ids }))
    }
}

/// `#[tool_handler]` generates `call_tool` / `list_tools` from the router;
/// `get_info` is overridden so the server identifies itself as `velesdb-memory`
/// (the macro default falls back to rmcp's own identity). Per-tool guidance
/// lives in each `#[tool(description = …)]`.
#[tool_handler(router = self.tool_router)]
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.server_info = Implementation::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some(
            "Local-first memory for AI agents: remember facts, recall them semantically, \
             relate them, forget them, and ask why a decision was made (connected subgraph)."
                .to_owned(),
        );
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
