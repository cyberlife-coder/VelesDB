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

use rmcp::handler::server::tool::{schema_for_input, schema_for_output};
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{ErrorCode, JsonObject};
use rmcp::{tool, tool_router, ErrorData};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use super::{join_error, to_error, McpServer};
use crate::context::wire::{stringify_id_fields, ID_KEYS};
use crate::context::{
    CompilePolicy, CompileRequest, CompiledContext, ContextCompiler, ContextDecision,
    ContextSavings, MediaRef, WorkingContext,
};

/// Serialize `payload`, opt-in rewriting every id field into decimal-string
/// form ([`CompilePolicy::ids_as_strings`]) — the shared response-side half
/// of the wire-compat contract, reused by both `compile_context` and
/// `explain_compilation` so the id rewrite is expressed exactly once.
fn to_wire_value<T: serde::Serialize>(
    payload: &T,
    ids_as_strings: bool,
) -> Result<Value, ErrorData> {
    let mut value = serde_json::to_value(payload).map_err(|err| {
        ErrorData::internal_error(
            format!("Failed to serialize structured content: {err}"),
            None,
        )
    })?;
    if ids_as_strings {
        stringify_id_fields(&mut value);
    }
    Ok(value)
}

/// The advertised-schema half of the [`CompilePolicy::ids_as_strings`]
/// contract: the response may carry each [`ID_KEYS`] field as an integer OR
/// a decimal string, and the official MCP SDKs validate `structuredContent`
/// against the advertised `outputSchema` (spec 2025-06-18) — so those
/// fields must be typed `["integer", "string"]`, or every opted-in response
/// would fail client-side validation for exactly the clients the option
/// exists for.
fn wire_safe_output_schema<T: JsonSchema + std::any::Any>() -> Arc<JsonObject> {
    let schema = schema_for_output::<T>().unwrap_or_else(|e| {
        panic!(
            "Invalid output schema for {}: {e}",
            std::any::type_name::<T>()
        )
    });
    let mut map = (*schema).clone();
    crate::schema::widen_id_properties(&mut map, ID_KEYS);
    Arc::new(map)
}

/// Input-side counterpart: `fragments[].id` accepts an integer or a decimal
/// string ([`crate::context::wire::deserialize_optional_id`]), so the
/// advertised input schema announces both — a client generating requests
/// from the schema must be able to discover the string form. Scoped to the
/// `id` property only: `explain_compilation`'s own `fragment_id` parameter
/// deserializes as a strict `u64` and stays typed `integer`.
fn wire_safe_input_schema<T: JsonSchema + std::any::Any>() -> Arc<JsonObject> {
    let schema = schema_for_input::<Parameters<T>>().unwrap_or_else(|e| {
        panic!(
            "Invalid input schema for {}: {e}",
            std::any::type_name::<T>()
        )
    });
    let mut map = (*schema).clone();
    crate::schema::widen_id_properties(&mut map, &["id"]);
    Arc::new(map)
}

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
    /// The fragment whose decision to return. Looked up by matching
    /// `ContextDecision::fragment_id`, UNLESS `fragment_index` is also
    /// given (see there) — still required even then, since it is the only
    /// disambiguator when `fragment_index` is absent.
    pub fragment_id: u64,
    /// Optional, 0-based position of the fragment in `request.fragments`.
    /// When given, TAKES PRIORITY over `fragment_id` for locating the
    /// decision: `compile_context` records exactly one decision per input
    /// fragment, in order, so `decisions[fragment_index]` is unambiguous
    /// even when several fragments are byte-identical and therefore share
    /// the same content-addressed `fragment_id` — a plain `fragment_id`
    /// lookup always returns the FIRST such decision (the deduplication
    /// survivor), never a dropped twin's. Absent (the default): behavior is
    /// unchanged, the decision is found by `fragment_id` alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fragment_index: Option<usize>,
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
    /// The original media payload, when the fragment carried one (US-009,
    /// PR2). Absent for every text-only source — the exact pre-PR2 shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaRef>,
}

/// Input of the `save_working_context` tool.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct SaveWorkingContextParams {
    /// Project facet this working context belongs to (matches `remember`'s
    /// `project` metadata convention).
    pub project: String,
    /// Session identifier — pick something stable for the agent run you want
    /// to resume later (e.g. a conversation id).
    pub session: String,
    /// The distilled state to persist: goal, active constraints, verified
    /// facts, open hypotheses, decisions taken, exact evidence, and pending
    /// actions.
    pub working: WorkingContext,
}

/// Output of the `save_working_context` tool.
#[derive(Debug, serde::Serialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct SaveWorkingContextResult {
    /// Id of the stored system fact backing this working context.
    pub id: u64,
}

/// Input of the `load_working_context` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct LoadWorkingContextParams {
    /// Project facet the working context was saved under.
    pub project: String,
    /// Session identifier the working context was saved under.
    pub session: String,
}

/// Output of the `load_working_context` tool. An envelope (not a bare
/// `Option<WorkingContext>`): the MCP spec requires the output schema's
/// root to be an object, so a nullable root is rejected by rmcp.
#[derive(Debug, serde::Serialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct LoadWorkingContextResult {
    /// The previously saved working context, or `null` when nothing was ever
    /// saved under that project + session (a fresh start, not an error).
    pub working: Option<WorkingContext>,
}

#[tool_router(router = context_tool_router, vis = "pub(super)")]
impl McpServer {
    #[tool(
        name = "compile_context",
        description = "Compile context fragments into a token-budgeted, provenance-audited prompt context — deterministically, with no LLM call. Duplicates are dropped, repeated log lines collapse, code/URLs/numbers/negative constraints survive verbatim, over-budget content becomes retrievable ctx://source/ handles instead of silently vanishing, and `memory_scope` pulls relevant stored memories into the result. Each fragment's own `metadata` is capped at 64 KiB serialized. Returns the assembled content plus one auditable decision per fragment (rule id, reason, risk), the sources, the retrieval handles, and token-savings insights. `policy.ids_as_strings` (default false) rewrites every id field of the response into a decimal string, for MCP clients without u64-safe JSON number parsing.",
        input_schema = wire_safe_input_schema::<CompileRequest>(),
        output_schema = wire_safe_output_schema::<CompiledContext>()
    )]
    async fn compile_context(
        &self,
        Parameters(request): Parameters<CompileRequest>,
    ) -> Result<Json<Value>, ErrorData> {
        let ids_as_strings = request.policy.as_ref().is_some_and(|p| p.ids_as_strings);
        let service = Arc::clone(&self.service);
        let compiled = tokio::task::spawn_blocking(move || {
            service.compile_context(&ContextCompiler::new(CompilePolicy::default()), &request)
        })
        .await
        .map_err(join_error)?
        .map_err(to_error)?;
        Ok(Json(to_wire_value(&compiled, ids_as_strings)?))
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
        description = "Explain why one fragment of a compile_context request was preserved, abstracted, externalized, dropped, or cached. Compilation is deterministic, so the request is re-compiled (with event/source recording off) and the fragment's exact decision (rule id, reason, relevance, risk, handle) is returned — no server-side state needed. Caveat: with a memory_scope the re-compile recalls from CURRENT memory, so decisions about pulled memories reflect the memory as it is now, not as it was. Pass `fragment_index` (0-based position in request.fragments) instead of relying on `fragment_id` alone when fragments are byte-identical — a shared content-addressed id otherwise always resolves to the deduplication survivor's decision. `policy.ids_as_strings` on the request rewrites the response's id fields into decimal strings, like compile_context.",
        input_schema = wire_safe_input_schema::<ExplainCompilationParams>(),
        output_schema = wire_safe_output_schema::<ContextDecision>()
    )]
    async fn explain_compilation(
        &self,
        Parameters(params): Parameters<ExplainCompilationParams>,
    ) -> Result<Json<Value>, ErrorData> {
        let service = Arc::clone(&self.service);
        let ExplainCompilationParams {
            request,
            fragment_id,
            fragment_index,
        } = params;
        if let Some(index) = fragment_index {
            let fragment_count = request.fragments.len();
            if index >= fragment_count {
                return Err(ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    format!(
                        "fragment_index {index} is out of bounds: request.fragments has {fragment_count} entries"
                    ),
                    None,
                ));
            }
        }
        let ids_as_strings = request.policy.as_ref().is_some_and(|p| p.ids_as_strings);
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
        let decision = if let Some(index) = fragment_index {
            // Bounds were already validated against `request.fragments`
            // above; memory-pulled fragments only ever append, so
            // `compiled.decisions` is at least as long.
            compiled.decisions.into_iter().nth(index)
        } else {
            compiled
                .decisions
                .into_iter()
                .find(|decision| decision.fragment_id == fragment_id)
        };
        let decision = decision.ok_or_else(|| {
            ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                format!("the request contains no fragment with id {fragment_id}"),
                None,
            )
        })?;
        Ok(Json(to_wire_value(&decision, ids_as_strings)?))
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
        let source = tokio::task::spawn_blocking(move || service.retrieve_context_source(&lookup))
            .await
            .map_err(join_error)?
            .map_err(to_error)?;
        Ok(Json(RetrieveContextSourceResult {
            handle,
            content: source.content,
            media: source.media,
        }))
    }

    #[tool(
        name = "save_working_context",
        description = "Persist this session's distilled working state (goal, active constraints, verified facts, open hypotheses, decisions, exact evidence, pending actions) under a project + session id — so a LATER session (a fresh agent run, a new conversation, a resumed process) can pick up exactly where this one left off instead of re-deriving context from scratch. Call this near the end of a session, or whenever the working state changes meaningfully. Saving again under the same project+session replaces the previous state (idempotent upsert). Serialized size is capped at 1 MiB. Returns the stored fact's id."
    )]
    async fn save_working_context(
        &self,
        Parameters(params): Parameters<SaveWorkingContextParams>,
    ) -> Result<Json<SaveWorkingContextResult>, ErrorData> {
        let service = Arc::clone(&self.service);
        let SaveWorkingContextParams {
            project,
            session,
            working,
        } = params;
        let id = tokio::task::spawn_blocking(move || {
            service.save_working_context(&project, &session, &working)
        })
        .await
        .map_err(join_error)?
        .map_err(to_error)?;
        Ok(Json(SaveWorkingContextResult { id }))
    }

    #[tool(
        name = "load_working_context",
        description = "Resume a session: load back the working context previously saved by save_working_context under the same project + session id — the goal, constraints, verified facts, open hypotheses, decisions, exact evidence, and pending actions a PRIOR session left off with. Call this at the START of a new session before doing anything else, so work continues instead of restarting. Returns null when nothing was ever saved under that project + session (a fresh start, not an error)."
    )]
    async fn load_working_context(
        &self,
        Parameters(params): Parameters<LoadWorkingContextParams>,
    ) -> Result<Json<LoadWorkingContextResult>, ErrorData> {
        let service = Arc::clone(&self.service);
        let LoadWorkingContextParams { project, session } = params;
        let working =
            tokio::task::spawn_blocking(move || service.load_working_context(&project, &session))
                .await
                .map_err(join_error)?
                .map_err(to_error)?;
        Ok(Json(LoadWorkingContextResult { working }))
    }
}

#[cfg(test)]
#[path = "context_tools_tests.rs"]
mod tests;
