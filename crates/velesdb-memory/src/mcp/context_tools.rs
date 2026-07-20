//! The context compiler's MCP tools — an *extension* of the one existing
//! server (never a second server): a second `#[tool_router]` block whose
//! router is combined with the main one in `McpServer::new`.
//!
//! Wire shapes reuse the domain types from [`crate::context`] directly
//! (`CompileRequest` *is* the tool input, `CompiledContext` the output) —
//! the only DTOs here are the thin request envelopes of the seven smaller
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
    fragment_id, segment_transcript, suggest_token_budget, CompilePolicy, CompileRequest,
    CompiledContext, ContextCompiler, ContextDecision, ContextFragment, ContextSavings, MediaRef,
    SegmentFormat, SegmentKind, SegmentationPolicy, SuggestedBudget, WorkingContext,
    WorkingContextSession,
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
    /// `true` when a working context was found under this exact project +
    /// session. Wire-additive alongside `working` (added V2a-1): a client
    /// that only reads `working` sees no change.
    pub found: bool,
    /// The previously saved working context, or `null` when nothing was ever
    /// saved under that project + session (a fresh start, not an error).
    pub working: Option<WorkingContext>,
    /// Other sessions saved under this SAME project, populated only when
    /// `found` is `false` — helps recover from a typo in `session` instead
    /// of silently starting fresh (e.g. `"task-1234"` saved,
    /// `"task-1235"` requested by mistake). Always empty when `found` is
    /// `true`.
    #[serde(default)]
    pub other_sessions: Vec<String>,
}

/// Input of the `list_working_contexts` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct ListWorkingContextsParams {
    /// Project facet to list saved working-context sessions for (same
    /// convention as `save_working_context`'s `project`).
    pub project: String,
}

/// Output of the `list_working_contexts` tool.
#[derive(Debug, serde::Serialize, JsonSchema)]
pub(super) struct ListWorkingContextsResult {
    /// Every session saved under this project, most-recently-saved first.
    /// Empty (not an error) when the project never saved anything.
    pub sessions: Vec<WorkingContextSession>,
}

/// Input of the `compile_transcript` tool.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct CompileTranscriptParams {
    /// What the agent is working on — drives relevance scoring, exactly like
    /// `compile_context`'s `query`.
    pub query: String,
    /// The raw transcript text (plain, marker-based, or JSONL). Exactly one
    /// of `transcript` or `path` must be set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<String>,
    /// Read the transcript from this absolute filesystem path instead of
    /// inline `transcript` — the same `VELESDB_MEMORY_INGEST_ROOTS`
    /// allowlist and security pipeline as a `compile_context` fragment's
    /// `path` (V2b-1), except capped at
    /// [`crate::limits::MAX_TRANSCRIPT_BYTES`] (8 MiB) instead of the
    /// ordinary 1 MiB fragment ceiling — the transcript is segmented into
    /// sub-1-MiB pieces immediately after this read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Hard token ceiling for the assembled content, same as
    /// `compile_context`'s `token_budget`.
    pub token_budget: u64,
    /// Project facet, recorded in provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Target model name, for cost insights.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_model: Option<String>,
    /// Per-request compile policy override, same as `compile_context`'s
    /// `policy`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<CompilePolicy>,
    /// Tuning knobs for the transcript segmentation step itself (format,
    /// merge threshold, system-turn caching). `None` uses
    /// [`SegmentationPolicy::default`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segmentation: Option<SegmentationPolicy>,
}

/// One entry of [`SegmentationReport::segments`] — the audit trail of how
/// `compile_transcript` cut the transcript up, independent of what
/// `compile_context` then did with the resulting fragments.
#[derive(Debug, serde::Serialize, serde::Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub(super) struct SegmentInfo {
    /// Position of this segment in `segmentation.segments`, in transcript
    /// order.
    pub index: usize,
    /// Which turn (0-based) this segment belongs to.
    pub turn: usize,
    /// The turn's role, when one was determined. `null` for a `plain`
    /// transcript with no matching marker at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// `"body"`, `"code"`, or `"log"`.
    pub kind: SegmentKind,
    /// Start byte offset (inclusive) in the original transcript. For a
    /// `jsonl` transcript this is the raw JSON line's span, not an offset
    /// into the decoded `content` (see [`SegmentationReport::segments`]'s
    /// struct docs for the one documented edge case where two segments can
    /// report the SAME range).
    pub byte_start: usize,
    /// End byte offset (exclusive) in the original transcript. Same caveat
    /// as `byte_start`.
    pub byte_end: usize,
    /// The id this segment's fragment carries into `context.decisions` —
    /// content-addressed, same formula as every other `compile_context`
    /// fragment with no caller-supplied `id`.
    pub fragment_id: u64,
}

/// The segmentation audit trail returned alongside `context`.
#[derive(Debug, serde::Serialize, serde::Deserialize, JsonSchema)]
pub(super) struct SegmentationReport {
    /// `"plain"` or `"jsonl"` — the format actually used, never `"auto"`
    /// even when the request asked for it.
    pub format_detected: SegmentFormat,
    /// Every segment, in transcript order. **Documented edge case:** a
    /// single `jsonl` line whose decoded `content` alone exceeds
    /// [`crate::limits::MAX_FRAGMENT_BYTES`] (1 MiB) is re-split into
    /// several segments (`compile_context` still gets sub-1-MiB fragments),
    /// but every child segment reports the SAME `byte_start`/`byte_end` —
    /// the original JSON line's span — because a JSONL line's decoded text
    /// has no byte-aligned mapping back into the raw (JSON-escaped) source
    /// bytes (see `resplit_body` in `context::segment`). An extreme edge
    /// case (one transcript line over 1 MiB of decoded content); every
    /// other segment kind keeps a unique, non-overlapping range.
    pub segments: Vec<SegmentInfo>,
    /// How many segments [`SegmentationPolicy::min_segment_bytes`] merging
    /// eliminated.
    pub merged_segments: usize,
}

/// Output of the `compile_transcript` tool.
#[derive(Debug, serde::Serialize, serde::Deserialize, JsonSchema)]
pub(super) struct CompileTranscriptResult {
    /// The compiled context — byte-compatible with `compile_context`'s
    /// output.
    pub context: CompiledContext,
    /// How the transcript was cut into fragments before compilation.
    pub segmentation: SegmentationReport,
}

/// Input of the `suggest_budget` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct SuggestBudgetParams {
    /// The model name to look up in the static window table (e.g.
    /// `"claude-sonnet-4-5"`). Matched case-insensitively.
    pub target_model: String,
    /// Tokens to reserve for the response, subtracted from the model's
    /// window (default `0`) — mirrors
    /// [`CompilePolicy::response_reserve_tokens`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reserve_tokens: Option<u64>,
}

#[tool_router(router = context_tool_router, vis = "pub(super)")]
impl McpServer {
    /// Resolve every `path`-carrying fragment of `fragments` against this
    /// server's configured ingest roots (V2b-1), turning `path` into
    /// ordinary `content` in place before the request reaches the compiler
    /// — the adapter-side pre-pass `context::ingest` describes. A no-op
    /// when no fragment carries a `path`. Shared by `compile_context` and
    /// `explain_compilation`, the only two tools that accept a `path`
    /// fragment.
    #[cfg(not(target_arch = "wasm32"))]
    fn resolve_ingest(&self, fragments: &mut [ContextFragment]) -> Result<(), ErrorData> {
        crate::context::ingest::resolve_fragments(fragments, self.ingest_roots.as_ref())
            .map_err(to_error)
    }

    /// This crate never targets `wasm32` with the `mcp` feature on (the
    /// server pulls in `rmcp`/`tokio`), so this arm exists only to keep the
    /// call site uniform if that ever changes — a `path` fragment simply
    /// reports the same "ingestion disabled" error the pure compiler core
    /// would (see `context::validate`), since there is no adapter here to
    /// resolve it.
    #[cfg(target_arch = "wasm32")]
    fn resolve_ingest(&self, fragments: &mut [ContextFragment]) -> Result<(), ErrorData> {
        if fragments.iter().any(|f| f.path.is_some()) {
            return Err(to_error(crate::error::MemoryError::IngestDisabled));
        }
        Ok(())
    }

    /// Resolve a `compile_transcript` `path` field against this server's
    /// configured ingest roots (V2b-2) — the same security pipeline as
    /// [`Self::resolve_ingest`], but through
    /// [`crate::context::ingest::resolve_transcript_path`] so the byte cap
    /// is [`crate::limits::MAX_TRANSCRIPT_BYTES`], not the ordinary 1 MiB
    /// fragment ceiling.
    #[cfg(not(target_arch = "wasm32"))]
    fn resolve_transcript_path(&self, path: &str) -> Result<String, ErrorData> {
        let roots = self
            .ingest_roots
            .as_ref()
            .filter(|roots| roots.is_enabled())
            .ok_or_else(|| to_error(crate::error::MemoryError::IngestDisabled))?;
        crate::context::ingest::resolve_transcript_path(path, roots).map_err(to_error)
    }

    /// `mcp` never targets `wasm32` (see [`Self::resolve_ingest`]'s wasm
    /// arm) — kept for call-site uniformity.
    #[cfg(target_arch = "wasm32")]
    fn resolve_transcript_path(&self, _path: &str) -> Result<String, ErrorData> {
        Err(to_error(crate::error::MemoryError::IngestDisabled))
    }

    #[tool(
        name = "compile_context",
        description = "Compile context fragments into a token-budgeted, provenance-audited prompt context — deterministically, with no LLM call. Duplicates are dropped, repeated log lines collapse, code/URLs/numbers/negative constraints survive verbatim, over-budget content becomes retrievable ctx://source/ handles instead of silently vanishing, and `memory_scope` pulls relevant stored memories into the result. Each fragment's own `metadata` is capped at 64 KiB serialized. A fragment may set `path` (an absolute filesystem path) instead of inline `content` to ingest a file by reference — exactly one of `path`, `content`, or `media` per fragment; requires the server to be started with VELESDB_MEMORY_INGEST_ROOTS set to an allowlist of directories, and the resolved file must be plain UTF-8 text under 1 MiB. Returns the assembled content plus one auditable decision per fragment (rule id, reason, risk), the sources, the retrieval handles, token-savings insights, and `warnings` — a mechanical shortlist of externalized fragments relevant enough to the query that they are worth a second look, so checking `decisions` by hand is only needed when `warnings` is non-empty and still ambiguous. `policy.slim_response` (default false) empties `sections`/`decisions` from the response — keep it off when you need the audit trail, or re-compile without it later (compilation is deterministic). `policy.ids_as_strings` (default false) rewrites every id field of the response into a decimal string, for MCP clients without u64-safe JSON number parsing.",
        input_schema = wire_safe_input_schema::<CompileRequest>(),
        output_schema = wire_safe_output_schema::<CompiledContext>()
    )]
    async fn compile_context(
        &self,
        Parameters(mut request): Parameters<CompileRequest>,
    ) -> Result<Json<Value>, ErrorData> {
        self.resolve_ingest(&mut request.fragments)?;
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

    /// **Error taxonomy caveat (acted in PR #1500 review):** every
    /// segmentation failure — oversized fence, forced `jsonl` that fails to
    /// parse, too many fragments after merging, transcript over
    /// [`crate::limits::MAX_TRANSCRIPT_BYTES`] — surfaces as
    /// [`crate::error::MemoryError::ContextOverLimit`] with a
    /// segmentation-specific message, deliberately reusing
    /// `compile_context`'s existing `INVALID_PARAMS`-category taxonomy
    /// rather than minting a new variant for this PR. Revisit if the
    /// overloaded variant makes segmentation errors hard for a caller to
    /// distinguish from an ordinary budget/fragment-count error.
    #[tool(
        name = "compile_transcript",
        description = "One-call shortcut over compile_context for a raw agent-session transcript: deterministically segments it into turns (plain marker-based — System:/User:/Human:/Assistant:/AI:/Tool:/### User/### Assistant — or JSONL, one line per turn) and, within each turn, into code/log/body sub-segments (fenced code blocks stay atomic; runs of 8+ log-like lines collapse the same way abstract.log_dedup would), then compiles the result exactly like compile_context. Exactly one of `transcript` (inline) or `path` (an absolute filesystem path, same VELESDB_MEMORY_INGEST_ROOTS allowlist as compile_context's `path` fragments but capped at 8 MiB) must be set. `segmentation.format` forces plain or jsonl instead of auto-detecting; a forced jsonl format that fails to parse is a hard error, never a silent fallback. The first turn is tagged cache-eligible when it looks like a system prompt (segmentation.cache_system_turn, default true). Returns `context` (byte-compatible with compile_context's output) plus `segmentation` — the detected format and one audit entry (turn, role, kind, byte range, fragment_id) per segment, so a caller can see exactly how the transcript was cut before trusting the compiled result.",
        input_schema = wire_safe_input_schema::<CompileTranscriptParams>(),
        output_schema = wire_safe_output_schema::<CompileTranscriptResult>()
    )]
    async fn compile_transcript(
        &self,
        Parameters(params): Parameters<CompileTranscriptParams>,
    ) -> Result<Json<Value>, ErrorData> {
        let CompileTranscriptParams {
            query,
            transcript,
            path,
            token_budget,
            project,
            target_model,
            policy,
            segmentation,
        } = params;
        let transcript_text = match (transcript, path) {
            (Some(text), None) => text,
            (None, Some(path)) => self.resolve_transcript_path(&path)?,
            _ => {
                return Err(ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    "exactly one of `transcript` or `path` must be set".to_owned(),
                    None,
                ));
            }
        };
        // Checked AFTER resolving `path` (not folded into the match guard
        // above) so an inline empty string and a `path` that resolves to an
        // empty file are rejected identically — the ingest pipeline itself
        // happily reads a zero-byte file, so this is the one place that
        // catches "nothing to compile" regardless of source.
        if transcript_text.is_empty() {
            return Err(ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                "the transcript is empty — `transcript` must be non-empty text, or `path` must \
                 point to a non-empty file"
                    .to_owned(),
                None,
            ));
        }
        let segmentation_policy = segmentation.unwrap_or_default();
        let outcome =
            segment_transcript(&transcript_text, &segmentation_policy).map_err(to_error)?;
        let segments_info: Vec<SegmentInfo> = outcome
            .segments
            .iter()
            .enumerate()
            .map(|(index, segment)| SegmentInfo {
                index,
                turn: segment.turn,
                role: segment.role.clone(),
                kind: segment.kind,
                byte_start: segment.byte_start,
                byte_end: segment.byte_end,
                fragment_id: fragment_id(&segment.fragment.content),
            })
            .collect();
        let fragments: Vec<ContextFragment> = outcome
            .segments
            .into_iter()
            .map(|segment| segment.fragment)
            .collect();
        let ids_as_strings = policy.as_ref().is_some_and(|p| p.ids_as_strings);
        let request = CompileRequest {
            query,
            fragments,
            project,
            target_model,
            token_budget,
            memory_scope: None,
            policy,
        };
        let service = Arc::clone(&self.service);
        let compiled = tokio::task::spawn_blocking(move || {
            service.compile_context(&ContextCompiler::new(CompilePolicy::default()), &request)
        })
        .await
        .map_err(join_error)?
        .map_err(to_error)?;
        let result = CompileTranscriptResult {
            context: compiled,
            segmentation: SegmentationReport {
                format_detected: outcome.format_detected,
                segments: segments_info,
                merged_segments: outcome.merged_segments,
            },
        };
        Ok(Json(to_wire_value(&result, ids_as_strings)?))
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
        description = "Explain why one fragment of a compile_context request was preserved, abstracted, externalized, dropped, or cached. Compilation is deterministic, so the request is re-compiled (with event/source recording off) and the fragment's exact decision (rule id, reason, relevance, risk, handle) is returned — no server-side state needed. Caveat: with a memory_scope the re-compile recalls from CURRENT memory, so decisions about pulled memories reflect the memory as it is now, not as it was; a `path` fragment is likewise re-read from disk, so the decision reflects the file's CURRENT content, not necessarily what the original compile_context call saw. Pass `fragment_index` (0-based position in request.fragments) instead of relying on `fragment_id` alone when fragments are byte-identical — a shared content-addressed id otherwise always resolves to the deduplication survivor's decision. `policy.ids_as_strings` on the request rewrites the response's id fields into decimal strings, like compile_context.",
        input_schema = wire_safe_input_schema::<ExplainCompilationParams>(),
        output_schema = wire_safe_output_schema::<ContextDecision>()
    )]
    async fn explain_compilation(
        &self,
        Parameters(params): Parameters<ExplainCompilationParams>,
    ) -> Result<Json<Value>, ErrorData> {
        let service = Arc::clone(&self.service);
        let ExplainCompilationParams {
            mut request,
            fragment_id,
            fragment_index,
        } = params;
        self.resolve_ingest(&mut request.fragments)?;
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
        description = "Resume a session: load back the working context previously saved by save_working_context under the same project + session id — the goal, constraints, verified facts, open hypotheses, decisions, exact evidence, and pending actions a PRIOR session left off with. Call this at the START of a new session before doing anything else, so work continues instead of restarting. `found: false` (with `working: null`) means nothing was ever saved under that exact project + session — not an error, but check `other_sessions`: if it lists a similarly-named session, `session` was likely a typo, not a genuinely fresh start. Use `list_working_contexts` to browse a project's sessions up front."
    )]
    async fn load_working_context(
        &self,
        Parameters(params): Parameters<LoadWorkingContextParams>,
    ) -> Result<Json<LoadWorkingContextResult>, ErrorData> {
        let LoadWorkingContextParams { project, session } = params;
        let service = Arc::clone(&self.service);
        let lookup_project = project.clone();
        let working = tokio::task::spawn_blocking(move || {
            service.load_working_context(&lookup_project, &session)
        })
        .await
        .map_err(join_error)?
        .map_err(to_error)?;
        let found = working.is_some();
        let other_sessions = if found {
            Vec::new()
        } else {
            let service = Arc::clone(&self.service);
            tokio::task::spawn_blocking(move || service.list_working_contexts(&project))
                .await
                .map_err(join_error)?
                .map_err(to_error)?
                .into_iter()
                .map(|s| s.session)
                .collect()
        };
        Ok(Json(LoadWorkingContextResult {
            found,
            working,
            other_sessions,
        }))
    }

    #[tool(
        name = "list_working_contexts",
        description = "List every session saved under a project via save_working_context, most-recently-saved first — so an agent can discover what is resumable before guessing a session id at load_working_context, or recover from a typo. Empty (not an error) when the project never saved anything."
    )]
    async fn list_working_contexts(
        &self,
        Parameters(params): Parameters<ListWorkingContextsParams>,
    ) -> Result<Json<ListWorkingContextsResult>, ErrorData> {
        let service = Arc::clone(&self.service);
        let ListWorkingContextsParams { project } = params;
        let sessions = tokio::task::spawn_blocking(move || service.list_working_contexts(&project))
            .await
            .map_err(join_error)?
            .map_err(to_error)?;
        Ok(Json(ListWorkingContextsResult { sessions }))
    }

    #[tool(
        name = "suggest_budget",
        description = "Suggest a starting token_budget for compile_context, for a named target model — looked up in a static, committed model-name to context-window table (dated \"as of\", NEVER a network call). Pass `reserve_tokens` (default 0) to reserve room for the response, mirroring compile_context's own `policy.response_reserve_tokens`. `window`/`suggested_budget` come back null when the model is not in the table — an honest \"unknown\", never a guess; extend the table in a new release instead of relying on this for an unlisted model."
    )]
    async fn suggest_budget(
        &self,
        Parameters(params): Parameters<SuggestBudgetParams>,
    ) -> Result<Json<SuggestedBudget>, ErrorData> {
        let SuggestBudgetParams {
            target_model,
            reserve_tokens,
        } = params;
        Ok(Json(suggest_token_budget(
            &target_model,
            reserve_tokens.unwrap_or(0),
        )))
    }
}

#[cfg(test)]
#[path = "context_tools_tests.rs"]
mod tests;
