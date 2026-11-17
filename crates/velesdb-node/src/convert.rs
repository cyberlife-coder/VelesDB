//! Anti-corruption marshalling between JS-facing types and `velesdb_memory`
//! domain types. This module (with [`crate::dto`]) is the only place that names
//! both worlds, so the dependency boundary is auditable by inspection.

use serde_json::Value;
use velesdb_memory::context::ingest::{resolve_transcript_path, IngestRoots};
use velesdb_memory::context::CompileRequest;
use velesdb_memory::{ColumnFilter, ColumnOp, FusionOptions, Link, MemoryError, Metadata};

use crate::dto::{ColumnFilterJs, FusionOptionsJs, LinkJs};
use crate::error::{internal, invalid_input, to_napi_err};

/// Format a `u64` id as a decimal string (JS `number` loses precision >2^53).
pub fn id_to_string(id: u64) -> String {
    id.to_string()
}

/// Parse a decimal-string id back to `u64`. Never panics; rejects floats/garbage.
pub fn parse_id(s: &str) -> napi::Result<u64> {
    s.parse::<u64>()
        .map_err(|_| invalid_input(format!("invalid id '{s}' (expected a decimal u64 string)")))
}

/// JSON data (read by [`crate::json_arg`]) → engine [`Metadata`].
/// `null`/absent → `None`; anything but an object is an error (callers must
/// pass a plain object for metadata and filters).
pub fn to_metadata(value: Option<Value>) -> napi::Result<Option<Metadata>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(map)) => Ok(Some(map)),
        Some(_) => Err(invalid_input("metadata/filter must be an object")),
    }
}

/// The largest whole number a JS `number` holds exactly (`2^53 - 1`).
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// Restores the integers napi's JSON conversion turned into floats, for the
/// values napi still converts itself (a column filter's `value`, a field of
/// a `#[napi(object)]`); the arguments [`crate::json_arg`] reads come out
/// right.
///
/// napi reads a JS number as an integer only within `u32` / `i32` range: a
/// whole number past it — a millisecond timestamp, a large id — arrives as
/// `5000000000.0`. Stored so, it reads back as a float from Python or Rust,
/// and misses a JSON equality against the integer every other surface stores
/// (`[5000000000] != [5000000000.0]`). Every whole number a JS number holds
/// exactly becomes an integer again, at any depth.
pub fn integral_numbers(value: Value) -> Value {
    match value {
        Value::Number(n) => Value::Number(match n.as_f64() {
            Some(f) if n.is_f64() => json_number(f).unwrap_or(n),
            _ => n,
        }),
        Value::Array(items) => Value::Array(items.into_iter().map(integral_numbers).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, item)| (key, integral_numbers(item)))
                .collect(),
        ),
        other => other,
    }
}

/// A JS number as JSON: a whole number a JS `number` holds exactly (within
/// `±(2^53 - 1)`) as an integer, any other finite number as a float, and
/// `None` for `NaN` and the infinities, which JSON cannot hold.
pub fn json_number(f: f64) -> Option<serde_json::Number> {
    if f.fract() == 0.0 && f.abs() <= MAX_SAFE_INTEGER {
        // Reason: a whole number within ±(2^53 - 1), checked just above.
        #[allow(clippy::cast_possible_truncation)]
        return Some(serde_json::Number::from(f as i64));
    }
    serde_json::Number::from_f64(f)
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

/// Parse the lowercase operator token through `ColumnOp`'s own serde grammar
/// (`#[serde(rename_all = "lowercase")]`), so an operator added to
/// `velesdb-memory` is accepted here the day it exists upstream — a hand-copy
/// of the rename table is how the binding drifted before (#2095).
fn parse_op(op: &str) -> napi::Result<ColumnOp> {
    serde_json::from_value(Value::from(op))
        .map_err(|_| invalid_input(format!("invalid op '{op}' (expected eq|ne|lt|le|gt|ge)")))
}

/// JS `[{field, op, value}]` → engine `Vec<ColumnFilter>`.
pub fn to_filters(filters: Vec<ColumnFilterJs>) -> napi::Result<Vec<ColumnFilter>> {
    filters
        .into_iter()
        .map(|f| {
            Ok(ColumnFilter {
                field: f.field,
                op: parse_op(&f.op)?,
                value: integral_numbers(f.value),
            })
        })
        .collect()
}

/// JS `{hops?, graphBoost?, pool?}` → engine [`FusionOptions`]. Routed
/// through [`FusionOptions::from_knobs`] — the same constructor the MCP tool
/// and the `PyO3` binding use — so the defaults and the `DoS` clamps
/// (`MAX_WHY_HOPS`, `MAX_RECALL_LIMIT`) live in exactly one place and a knob
/// added upstream reaches this binding the day it exists. A hand-written
/// mirror here is how the two would drift.
///
/// # Errors
///
/// `INVALID_INPUT` when `hops` or `pool` is not a whole number in
/// `0..=u32::MAX` ([`crate::guards::count`]).
pub fn to_fusion_options(opts: Option<FusionOptionsJs>) -> napi::Result<FusionOptions> {
    let Some(o) = opts else {
        return Ok(FusionOptions::default());
    };
    Ok(FusionOptions::from_knobs(
        crate::guards::count("hops", o.hops)?.map(|h| h as usize),
        o.graph_boost,
        crate::guards::count("pool", o.pool)?.map(|p| p as usize),
    ))
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

/// Marshal a resolved `ctx://source/<hash>` lookup into the binding's
/// `{handle, content, media?}` wire shape (US-009, PR3) — the same envelope
/// the MCP `retrieve_context_source` tool returns, built here since
/// [`velesdb_memory::context::ContextSource`] itself carries no `handle`
/// (the caller already has it; the service only resolves content + media).
pub fn to_retrieve_source_js(
    handle: &str,
    source: &velesdb_memory::context::ContextSource,
) -> napi::Result<Value> {
    let Value::Object(fields) = serde_json::to_value(source)
        .map_err(|err| internal(format!("context source serialization: {err}")))?
    else {
        return Err(internal("context source is not an object"));
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
        .map_err(|err| internal(format!("segmentation report serialization: {err}")))?;
    Ok((request, segmentation_value))
}

/// Transcript-or-path resolution for `compileTranscript`: `path` is a
/// transcript-level field, not part of the bridge's input struct, so it is
/// pulled out here and the text fed back as `transcript`, by the rule the
/// MCP tool applies ([`velesdb_memory::context::resolve_transcript_source`]):
/// exactly one of the two, never empty. A field set to `null` is absent, as
/// it is on the MCP tool; a field of another type is `INVALID_INPUT`. A
/// `path` with ingestion disabled fails closed (`IngestDisabled`, mapped by
/// [`to_napi_err`]); the containment check runs at the sink
/// ([`velesdb_memory::context::ingest`]), never here.
pub fn resolve_transcript_input(
    request: Value,
    roots: Option<&IngestRoots>,
) -> napi::Result<Value> {
    // A non-object request is handed through unchanged: the bridge's own
    // deserialization rejects it with INVALID_INPUT.
    let mut map = match request {
        Value::Object(map) => map,
        other => return Ok(other),
    };
    let transcript = optional_string(map.remove("transcript"), "transcript")?;
    let path = optional_string(map.remove("path"), "path")?;
    let text =
        velesdb_memory::context::resolve_transcript_source(transcript, path.as_deref(), |path| {
            let roots = roots
                .filter(|roots| roots.is_enabled())
                .ok_or(MemoryError::IngestDisabled)?;
            resolve_transcript_path(path, roots)
        })
        .map_err(to_napi_err)?;
    map.insert("transcript".to_owned(), Value::String(text));
    Ok(Value::Object(map))
}

/// A request field read as JS gives it: absent or `null` is `None`, a string
/// is `Some`, anything else is `INVALID_INPUT` naming the field.
fn optional_string(value: Option<Value>, field: &str) -> napi::Result<Option<String>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(_) => Err(invalid_input(format!("`{field}` must be a string"))),
    }
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

#[cfg(test)]
#[path = "convert_tests.rs"]
mod convert_tests;
