//! Anti-corruption marshalling between JS-facing types and `velesdb_memory`
//! domain types. This module (with [`crate::dto`]) is the only place that names
//! both worlds, so the dependency boundary is auditable by inspection.

use serde_json::Value;
use velesdb_memory::context::CompileRequest;
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

/// Input of `compileTranscript` — the shared
/// [`TranscriptCompileInput`](velesdb_memory::context::TranscriptCompileInput),
/// re-exported so the napi method keeps naming a local type.
pub use velesdb_memory::context::TranscriptCompileInput as CompileTranscriptInput;

/// The pure-Rust half of `compileTranscript`: segments `input.transcript`,
/// assembles the [`CompileRequest`] `compile_context` then compiles, and
/// serializes the segmentation audit trail.
///
/// The segmentation glue itself lives in `velesdb_memory`'s
/// [`transcript_bridge`](velesdb_memory::context::transcript_bridge) — it
/// used to be duplicated here and in the WASM binding, each doc comment
/// pointing at the other. What is left here is the napi-specific part: the
/// error translation and the JSON envelope.
///
/// # Errors
/// An `INVALID_INPUT` error for an empty transcript, a genuine budget/cap
/// breach, or a forced-format parse failure — all translated from
/// [`velesdb_memory::MemoryError`] by [`to_napi_err`].
pub fn build_transcript_compile_request(
    input: CompileTranscriptInput,
) -> napi::Result<(CompileRequest, Value)> {
    let (request, report) =
        velesdb_memory::context::build_transcript_compile_request(input).map_err(to_napi_err)?;
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
    let mut value = serde_json::to_value(compiled)
        .map_err(|err| compiled_internal(&format!("serialize: {err}")))?;
    stringify_id_fields(&mut value);
    let Value::Object(mut map) = value else {
        return Err(compiled_internal("not an object"));
    };
    compiled_envelope(&mut map)
}

/// The `[INTERNAL]` error of the compiled-context marshalling: every variant
/// signals a bug in this conversion (the wire JSON of a `CompiledContext` is
/// always a complete object), never bad caller input.
fn compiled_internal(what: &str) -> napi::Error {
    napi::Error::from_reason(format!("[INTERNAL] compiled context: {what}"))
}

/// Take a top-level field out of the serialized compiled context.
fn compiled_field(map: &mut serde_json::Map<String, Value>, key: &str) -> napi::Result<Value> {
    map.remove(key)
        .ok_or_else(|| compiled_internal(&format!("missing field {key}")))
}

/// [`compiled_field`] for the two fields the typed envelope declares as
/// `String` rather than raw wire JSON.
fn compiled_string_field(
    map: &mut serde_json::Map<String, Value>,
    key: &str,
) -> napi::Result<String> {
    match compiled_field(map, key)? {
        Value::String(text) => Ok(text),
        _ => Err(compiled_internal(&format!("{key} is not a string"))),
    }
}

/// Lift the top-level fields of an id-stringified compiled context into the
/// typed envelope.
///
/// This used to say that a field the envelope does not declare "stays behind
/// in `map` and is dropped — the envelope is the binding's contract, not a
/// mirror of the domain type". That reading is what cost Node
/// `compile_context.warnings` (issue #1691): the envelope IS meant to mirror
/// the domain type, and a hand-written mirror silently loses every field the
/// server later grows. So the contract is now the opposite one — every
/// top-level field is taken, and the drain below is what says so.
fn compiled_envelope(
    map: &mut serde_json::Map<String, Value>,
) -> napi::Result<crate::dto::CompiledContextJs> {
    // The two string fields are taken first, keeping the order in which a
    // malformed envelope surfaces its error unchanged.
    let content = compiled_string_field(map, "content")?;
    let risk = compiled_string_field(map, "risk")?;
    let envelope = crate::dto::CompiledContextJs {
        content,
        sections: compiled_field(map, "sections")?,
        decisions: compiled_field(map, "decisions")?,
        sources: compiled_field(map, "sources")?,
        retrieval_handles: compiled_field(map, "retrieval_handles")?,
        insights: compiled_field(map, "insights")?,
        risk,
        warnings: compiled_field(map, "warnings")?,
    };
    // `compiled_field` REMOVES, so an empty map means nothing was left behind.
    // A `debug_assert` and not an error on purpose: this fires for the
    // DEVELOPER who grew the server without growing the mirror, which is a
    // build-time mistake, and it must never turn a caller's successful
    // compilation into a failure in the release profile npm ships. The
    // release-profile blind spot is real and is exactly why the guard in
    // `binding_parity_bdd.rs` exists on top of it.
    debug_assert!(
        map.is_empty(),
        "the server grew {:?} on compile_context and this mirror dropped it — add the field to \
         CompiledContextJs",
        map.keys().collect::<Vec<_>>(),
    );
    Ok(envelope)
}
