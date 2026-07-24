//! Anti-corruption marshalling between JS-facing types and `velesdb_memory`
//! domain types. This module (with [`crate::dto`]) is the only place that names
//! both worlds, so the dependency boundary is auditable by inspection.

use serde_json::Value;
use velesdb_memory::context::{
    fragment_id as ctx_fragment_id, segment_transcript, CompilePolicy, CompileRequest,
    SegmentFormat, SegmentKind, SegmentationPolicy,
};
use velesdb_memory::limits;
use velesdb_memory::{ColumnFilter, ColumnOp, FusionOptions, Link, Metadata};

use crate::dto::{ColumnFilterJs, FusionOptionsJs, LinkJs};
use crate::error::{invalid_input, to_napi_err};

/// Format a `u64` id as a decimal string (JS `number` loses precision >2^53).
pub fn id_to_string(id: u64) -> String {
    id.to_string()
}

/// Parse a decimal-string id back to `u64`. Never panics; rejects floats/garbage.
pub fn parse_id(s: &str) -> napi::Result<u64> {
    s.parse::<u64>()
        .map_err(|_| invalid_input(format!("invalid id '{s}' (expected a decimal u64 string)")))
}

/// JS object → engine [`Metadata`]. `null`/absent → `None`; a non-object is an
/// error (callers must pass a plain object for metadata and filters).
pub fn to_metadata(value: Option<Value>) -> napi::Result<Option<Metadata>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(map)) => Ok(Some(map)),
        Some(_) => Err(invalid_input("metadata/filter must be an object")),
    }
}

/// JS `[{target, relation}]` → engine `Vec<Link>`, parsing each id.
pub fn to_links(links: Option<Vec<LinkJs>>) -> napi::Result<Vec<Link>> {
    links
        .unwrap_or_default()
        .into_iter()
        .map(|l| {
            Ok(Link {
                target: parse_id(&l.target)?,
                relation: l.relation,
            })
        })
        .collect()
}

/// Parse the lowercase operator token (mirrors `ColumnOp`'s serde rename).
fn parse_op(op: &str) -> napi::Result<ColumnOp> {
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

/// JS `[{field, op, value}]` → engine `Vec<ColumnFilter>`.
pub fn to_filters(filters: Vec<ColumnFilterJs>) -> napi::Result<Vec<ColumnFilter>> {
    filters
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

/// JS `{hops?, graphBoost?, pool?}` → engine [`FusionOptions`]. An omitted
/// object, or an omitted field within it, falls back to
/// [`FusionOptions::default`]'s proven value. `hops` and `pool` are each
/// capped at their shared `DoS` limit ([`limits::MAX_WHY_HOPS`],
/// [`limits::MAX_RECALL_LIMIT`]) — `pool` feeds the same oversampled vector
/// search `k`/`hops` do, so an uncapped caller-supplied value is exactly as
/// much of an unbounded-scan risk as an uncapped `k` or `hops` would be.
pub fn to_fusion_options(opts: Option<FusionOptionsJs>) -> FusionOptions {
    let defaults = FusionOptions::default();
    let Some(opts) = opts else {
        return defaults;
    };
    FusionOptions {
        hops: limits::clamp_hops(opts.hops.map_or(defaults.hops, |h| h as usize)),
        graph_boost: opts.graph_boost.unwrap_or(defaults.graph_boost),
        pool: opts
            .pool
            .map(|p| limits::clamp_recall_limit(p as usize))
            .or(defaults.pool),
    }
}

/// Recursively rewrite every `context` id field (see
/// [`velesdb_memory::context::wire::ID_KEYS`]) of a serialized
/// `CompiledContext` into its decimal-string form — the same id contract as
/// every other method of this binding, applied to a whole tree at once so
/// the domain type needs no JS-specific duplicate. Shared with the WASM
/// binding via `velesdb_memory::context::wire`, not duplicated here.
pub fn stringify_id_fields(value: &mut Value) {
    velesdb_memory::context::wire::stringify_id_fields(value);
}

/// The inverse of [`stringify_id_fields`]: recursively rewrite every
/// `context` id field given in the binding's decimal-string form back into
/// the numeric form the domain types deserialize.
pub fn parse_id_fields(value: &mut Value) -> napi::Result<()> {
    velesdb_memory::context::wire::parse_id_fields(value).map_err(invalid_input)
}

/// Accept `fragments[].id` in the binding's decimal-string form by rewriting
/// it to the numeric form the domain type deserializes.
pub fn parse_fragment_id_strings(request: &mut Value) -> napi::Result<()> {
    velesdb_memory::context::wire::parse_fragment_id_strings(request).map_err(invalid_input)
}

/// Marshal a resolved `ctx://source/<hash>` lookup into the binding's
/// `{handle, content, media?}` wire shape (US-009, PR3) — the same envelope
/// the MCP `retrieve_context_source` tool returns, built here since
/// [`velesdb_memory::context::ContextSource`] itself carries no `handle`
/// (the caller already has it; the service only resolves content + media).
pub fn to_retrieve_source_js(
    handle: &str,
    source: &velesdb_memory::context::ContextSource,
) -> napi::Result<Value> {
    let internal =
        |what: &str| napi::Error::from_reason(format!("[INTERNAL] context source: {what}"));
    let Value::Object(fields) =
        serde_json::to_value(source).map_err(|err| internal(&format!("serialize: {err}")))?
    else {
        return Err(internal("not an object"));
    };
    let mut map = serde_json::Map::new();
    map.insert("handle".to_owned(), Value::String(handle.to_owned()));
    map.extend(fields);
    Ok(Value::Object(map))
}

/// Input of `compileTranscript` — the same fields as the MCP
/// `compile_transcript` tool's request MINUS `path`: this binding has no
/// ingest-roots configuration surface (unlike the MCP server), so only an
/// inline `transcript` is accepted. Mirrors the WASM binding's own
/// `CompileTranscriptInput`.
#[derive(serde::Deserialize)]
pub struct CompileTranscriptInput {
    pub query: String,
    pub transcript: String,
    pub token_budget: u64,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub target_model: Option<String>,
    #[serde(default)]
    pub policy: Option<CompilePolicy>,
    #[serde(default)]
    pub segmentation: Option<SegmentationPolicy>,
}

/// One entry of the `segmentation.segments` array `compileTranscript`
/// returns — same shape as the MCP `compile_transcript` tool's own (private)
/// `SegmentInfo`. `fragment_id` is already a decimal string, so no separate
/// [`stringify_id_fields`] pass is needed for the segmentation half of the
/// response.
#[derive(serde::Serialize)]
struct SegmentInfoJs {
    index: usize,
    turn: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    kind: SegmentKind,
    byte_start: usize,
    byte_end: usize,
    fragment_id: String,
}

/// The segmentation audit trail `compileTranscript` returns alongside
/// `context`.
#[derive(serde::Serialize)]
struct SegmentationReportJs {
    format_detected: SegmentFormat,
    segments: Vec<SegmentInfoJs>,
    merged_segments: usize,
}

/// The pure-Rust half of `compileTranscript`: segments `input.transcript`
/// and assembles the [`CompileRequest`] `compile_context` then compiles,
/// plus the JSON-ready segmentation audit trail. Split out from the napi
/// method into a plain function — synchronously testable with no napi/JS
/// runtime needed (unlike the WASM binding, `napi::Error` is an ordinary
/// Rust value off any target, so this needs no further split for
/// testability). Mirrors the WASM binding's own
/// `build_transcript_compile_request`.
///
/// # Errors
/// An `INVALID_INPUT` error for an empty transcript (mirrors the MCP
/// `compile_transcript` tool's own guard — `segment_transcript` has no such
/// check itself, since an empty string is a valid, if useless, zero-turn
/// input to it); otherwise whatever `segment_transcript` itself returns (a
/// genuine budget/cap breach, or a forced-format parse failure), translated
/// by [`to_napi_err`].
pub fn build_transcript_compile_request(
    input: CompileTranscriptInput,
) -> napi::Result<(CompileRequest, Value)> {
    if input.transcript.is_empty() {
        return Err(invalid_input(
            "the transcript is empty — `transcript` must be non-empty text",
        ));
    }
    let segmentation_policy = input.segmentation.unwrap_or_default();
    let outcome =
        segment_transcript(&input.transcript, &segmentation_policy).map_err(to_napi_err)?;
    let segments_info: Vec<SegmentInfoJs> = outcome
        .segments
        .iter()
        .enumerate()
        .map(|(index, segment)| SegmentInfoJs {
            index,
            turn: segment.turn,
            role: segment.role.clone(),
            kind: segment.kind,
            byte_start: segment.byte_start,
            byte_end: segment.byte_end,
            fragment_id: id_to_string(ctx_fragment_id(&segment.fragment.content)),
        })
        .collect();
    let report = SegmentationReportJs {
        format_detected: outcome.format_detected,
        segments: segments_info,
        merged_segments: outcome.merged_segments,
    };
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
    let segmentation_value = serde_json::to_value(&report)
        .map_err(|err| invalid_input(format!("segmentation report serialization: {err}")))?;
    Ok((request, segmentation_value))
}

/// Marshal a compiled context into its JS shape: serialize to the wire JSON,
/// stringify every id field, then lift the top-level fields into the typed
/// [`CompiledContextJs`] envelope. Pure conversion — no compile logic.
pub fn to_compiled_js(
    compiled: &velesdb_memory::context::CompiledContext,
) -> napi::Result<crate::dto::CompiledContextJs> {
    let internal =
        |what: &str| napi::Error::from_reason(format!("[INTERNAL] compiled context: {what}"));
    let mut value =
        serde_json::to_value(compiled).map_err(|err| internal(&format!("serialize: {err}")))?;
    stringify_id_fields(&mut value);
    let Value::Object(mut map) = value else {
        return Err(internal("not an object"));
    };
    let field = |map: &mut serde_json::Map<String, Value>, key: &str| {
        map.remove(key)
            .ok_or_else(|| internal(&format!("missing field {key}")))
    };
    let content = match field(&mut map, "content")? {
        Value::String(text) => text,
        _ => return Err(internal("content is not a string")),
    };
    let risk = match field(&mut map, "risk")? {
        Value::String(level) => level,
        _ => return Err(internal("risk is not a string")),
    };
    Ok(crate::dto::CompiledContextJs {
        content,
        sections: field(&mut map, "sections")?,
        decisions: field(&mut map, "decisions")?,
        sources: field(&mut map, "sources")?,
        retrieval_handles: field(&mut map, "retrieval_handles")?,
        insights: field(&mut map, "insights")?,
        risk,
    })
}
