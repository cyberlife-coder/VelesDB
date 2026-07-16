//! The context compiler's MCP tools — an *extension* of the one existing
//! server (never a second server): a second `#[tool_router]` block whose
//! router is combined with the main one in `McpServer::new`.
//!
//! Wire shapes reuse the domain types from [`crate::context`] directly
//! (`CompileRequest` *is* the tool input, `CompiledContext` the output) —
//! the only DTOs here are the thin request envelopes of the three smaller
//! tools. Same conventions as every other tool: `spawn_blocking` around the
//! sync service, errors mapped through the transport-neutral category.

use std::sync::Arc;

use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::ErrorCode;
use rmcp::{tool, tool_router, ErrorData};
use schemars::JsonSchema;
use serde::Deserialize;

use super::{join_error, to_error, McpServer};
use crate::context::{
    CompilePolicy, CompileRequest, CompiledContext, ContextCompiler, ContextDecision,
    ContextSavings,
};

// --- Thin request envelopes --------------------------------------------------

/// Input of the `context_savings` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct ContextSavingsParams {
    /// Restrict the aggregation to this project facet.
    pub project: Option<String>,
}

/// Input of the `explain_compilation` tool.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct ExplainCompilationParams {
    /// The compile request to explain (compilation is deterministic, so
    /// re-submitting the request reproduces the exact decisions).
    pub request: CompileRequest,
    /// The fragment whose decision to return.
    pub fragment_id: u64,
}

/// Input of the `retrieve_context_source` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct RetrieveContextSourceParams {
    /// A `ctx://source/<hash>` handle from a compiled context.
    pub handle: String,
}

/// Output of the `retrieve_context_source` tool.
#[derive(Debug, serde::Serialize, JsonSchema)]
pub(super) struct RetrieveContextSourceResult {
    /// The handle that was resolved.
    pub handle: String,
    /// The original fragment content, byte for byte.
    pub content: String,
}

#[tool_router(router = context_tool_router, vis = "pub(super)")]
impl McpServer {
    #[tool(
        name = "compile_context",
        description = "Compile context fragments into a token-budgeted, provenance-audited prompt context — deterministically, with no LLM call. Duplicates are dropped, repeated log lines collapse, code/URLs/numbers/negative constraints survive verbatim, over-budget content becomes retrievable ctx://source/ handles instead of silently vanishing, and `memory_scope` pulls relevant stored memories into the result. Returns the assembled content plus one auditable decision per fragment (rule id, reason, risk), the sources, the retrieval handles, and token-savings insights."
    )]
    async fn compile_context(
        &self,
        Parameters(request): Parameters<CompileRequest>,
    ) -> Result<Json<CompiledContext>, ErrorData> {
        let service = Arc::clone(&self.service);
        let compiled = tokio::task::spawn_blocking(move || {
            service.compile_context(&ContextCompiler::new(CompilePolicy::default()), &request)
        })
        .await
        .map_err(join_error)?
        .map_err(to_error)?;
        Ok(Json(compiled))
    }

    #[tool(
        name = "context_savings",
        description = "Aggregate the token (and cost) savings of past compile_context calls, optionally per project. Figures are local estimates recorded per compilation (metadata only, never content); `truncated` reports when the sweep hit the recall cap."
    )]
    async fn context_savings(
        &self,
        Parameters(params): Parameters<ContextSavingsParams>,
    ) -> Result<Json<ContextSavings>, ErrorData> {
        let service = Arc::clone(&self.service);
        let savings =
            tokio::task::spawn_blocking(move || service.context_savings(params.project.as_deref()))
                .await
                .map_err(join_error)?
                .map_err(to_error)?;
        Ok(Json(savings))
    }

    #[tool(
        name = "explain_compilation",
        description = "Explain why one fragment of a compile_context request was preserved, abstracted, externalized, dropped, or cached. Compilation is deterministic, so the request is re-compiled (with event/source recording off) and the fragment's exact decision (rule id, reason, relevance, risk, handle) is returned — no server-side state needed. Caveat: with a memory_scope the re-compile recalls from CURRENT memory, so decisions about pulled memories reflect the memory as it is now, not as it was."
    )]
    async fn explain_compilation(
        &self,
        Parameters(params): Parameters<ExplainCompilationParams>,
    ) -> Result<Json<ContextDecision>, ErrorData> {
        let service = Arc::clone(&self.service);
        let ExplainCompilationParams {
            request,
            fragment_id,
        } = params;
        let compiled = tokio::task::spawn_blocking(move || {
            // Explanation must not re-record an event or re-store sources:
            // it is a read-only question about a deterministic function.
            let mut request = request;
            let mut policy = request.policy.take().unwrap_or_default();
            policy.record_events = false;
            policy.store_sources = false;
            request.policy = Some(policy);
            service.compile_context(&ContextCompiler::new(CompilePolicy::default()), &request)
        })
        .await
        .map_err(join_error)?
        .map_err(to_error)?;
        compiled
            .decisions
            .into_iter()
            .find(|decision| decision.fragment_id == fragment_id)
            .map(Json)
            .ok_or_else(|| {
                ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    format!("the request contains no fragment with id {fragment_id}"),
                    None,
                )
            })
    }

    #[tool(
        name = "retrieve_context_source",
        description = "Fetch back the exact original content behind a ctx://source/<hash> handle from a compiled context — what compile_context externalized or partially packed is recoverable, not lost."
    )]
    async fn retrieve_context_source(
        &self,
        Parameters(params): Parameters<RetrieveContextSourceParams>,
    ) -> Result<Json<RetrieveContextSourceResult>, ErrorData> {
        let service = Arc::clone(&self.service);
        let RetrieveContextSourceParams { handle } = params;
        let lookup = handle.clone();
        let content = tokio::task::spawn_blocking(move || service.retrieve_context_source(&lookup))
            .await
            .map_err(join_error)?
            .map_err(to_error)?;
        Ok(Json(RetrieveContextSourceResult { handle, content }))
    }
}

#[cfg(test)]
#[path = "context_tools_tests.rs"]
mod tests;
