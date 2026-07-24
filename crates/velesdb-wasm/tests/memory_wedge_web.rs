//! wasm-bindgen tests for the memory-wedge binding: behaviours only
//! observable with a real JS runtime — i.e. how values marshal across the
//! wasm boundary. Logic-level coverage lives in the native suites
//! (`memory_service_tests.rs`, `memory_store_tests.rs`).

#![cfg(target_arch = "wasm32")]

use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::*;

use velesdb_wasm::WasmMemoryService;

/// Regression: the default `serde_wasm_bindgen` serializer turns a
/// `serde_json::Value::Object` into an ES2015 `Map`, on which property
/// access and `JSON.stringify` silently yield nothing — every metadata
/// read in the browser came back empty while the same code worked on the
/// Node binding. Metadata must marshal as a plain JS object.
#[wasm_bindgen_test]
fn recall_metadata_is_a_plain_js_object_not_a_map() {
    let svc = WasmMemoryService::new(16);
    let meta = js_sys::Object::new();
    js_sys::Reflect::set(&meta, &"project".into(), &"veles".into()).unwrap();
    svc.remember(
        "we chose parking_lot to avoid lock poisoning",
        JsValue::UNDEFINED,
        meta.into(),
        None,
    )
    .unwrap();

    let hits = svc
        .recall("parking_lot", Some(5), JsValue::UNDEFINED)
        .unwrap();
    let first = js_sys::Reflect::get(&hits, &0.into()).unwrap();
    let metadata = js_sys::Reflect::get(&first, &"metadata".into()).unwrap();

    assert!(
        !metadata.is_instance_of::<js_sys::Map>(),
        "metadata must be a plain object, not an ES2015 Map"
    );
    let project = js_sys::Reflect::get(&metadata, &"project".into()).unwrap();
    assert_eq!(
        project.as_string().as_deref(),
        Some("veles"),
        "metadata properties must be readable by plain property access"
    );
}

/// A `null` VALUE inside metadata must marshal as JS `null`, matching the
/// Node binding — with the serializer's default missing-as-undefined, the
/// key silently vanished from `JSON.stringify` output on WASM only.
#[wasm_bindgen_test]
fn recall_metadata_preserves_null_values() {
    let svc = WasmMemoryService::new(16);
    let meta = js_sys::Object::new();
    js_sys::Reflect::set(&meta, &"flag".into(), &JsValue::NULL).unwrap();
    svc.remember(
        "a fact carrying a null-valued metadata key",
        JsValue::UNDEFINED,
        meta.into(),
        None,
    )
    .unwrap();

    let hits = svc
        .recall("null-valued metadata", Some(5), JsValue::UNDEFINED)
        .unwrap();
    let first = js_sys::Reflect::get(&hits, &0.into()).unwrap();
    let metadata = js_sys::Reflect::get(&first, &"metadata".into()).unwrap();
    let flag = js_sys::Reflect::get(&metadata, &"flag".into()).unwrap();
    assert!(
        flag.is_null(),
        "a stored null value must round-trip as null, not undefined"
    );
}

/// The absent-metadata convention must survive the serializer change:
/// a fact with no caller metadata marshals `metadata` as `undefined`
/// (matching the Node binding), not `null` or an empty object.
#[wasm_bindgen_test]
fn recall_without_metadata_marshals_undefined() {
    let svc = WasmMemoryService::new(16);
    svc.remember(
        "a bare fact with no metadata",
        JsValue::UNDEFINED,
        JsValue::UNDEFINED,
        None,
    )
    .unwrap();

    let hits = svc
        .recall("bare fact", Some(5), JsValue::UNDEFINED)
        .unwrap();
    let first = js_sys::Reflect::get(&hits, &0.into()).unwrap();
    let metadata = js_sys::Reflect::get(&first, &"metadata".into()).unwrap();
    assert!(metadata.is_undefined(), "absent metadata must be undefined");
}

/// `compileContext` marshalling across the wasm boundary: the request goes
/// in as a plain JS object (fragment ids as decimal strings), the compiled
/// result comes back as a plain object (not a Map) with every id field as a
/// decimal string — u64::MAX must survive, which proves ids never pass
/// through a JS number.
#[wasm_bindgen_test]
fn compile_context_round_trips_with_string_ids() {
    let svc = WasmMemoryService::new(16);
    let request = js_sys::JSON::parse(
        r#"{
            "query": "state of the canary deploy",
            "token_budget": 500,
            "fragments": [
                {"id": "18446744073709551615", "content": "The canary is green: 2% traffic."},
                {"content": "Rollback runbook: kubectl rollout undo deployment/canary."}
            ]
        }"#,
    )
    .unwrap();

    let compiled = svc.compile_context(request).unwrap();
    assert!(
        !compiled.is_instance_of::<js_sys::Map>(),
        "compiled context must be a plain object"
    );

    let risk = js_sys::Reflect::get(&compiled, &"risk".into()).unwrap();
    assert_eq!(risk.as_string().as_deref(), Some("low"));
    let content = js_sys::Reflect::get(&compiled, &"content".into()).unwrap();
    let content = content.as_string().unwrap();
    assert!(content.contains("canary is green"));
    assert!(content.contains("Rollback runbook"));

    let decisions = js_sys::Reflect::get(&compiled, &"decisions".into()).unwrap();
    let first = js_sys::Reflect::get(&decisions, &0.into()).unwrap();
    let fragment_id = js_sys::Reflect::get(&first, &"fragment_id".into()).unwrap();
    let fragment_id = fragment_id
        .as_string()
        .expect("fragment_id must cross as a decimal string, not a number");
    assert_eq!(fragment_id, "18446744073709551615", "u64::MAX survives");
}

/// Determinism across the boundary: the same request compiles to the same
/// bytes twice (JSON-stringified equality of the full result).
#[wasm_bindgen_test]
fn compile_context_is_deterministic() {
    let svc = WasmMemoryService::new(16);
    let request = || {
        js_sys::JSON::parse(
            r#"{"query": "q", "token_budget": 400,
                "fragments": [{"content": "same line"}, {"content": "same line"}]}"#,
        )
        .unwrap()
    };
    let a = svc.compile_context(request()).unwrap();
    let b = svc.compile_context(request()).unwrap();
    let stringify = |v: &JsValue| js_sys::JSON::stringify(v).unwrap().as_string().unwrap();
    assert_eq!(stringify(&a), stringify(&b), "same input, same bytes");
}

/// `memory_scope` on the in-memory store: the tri-engine pull (fused recall
/// + PR2's importance blend, whose default weights are active) must work on
/// `WasmStore` — this pins the whole `recall_fused_scored` path in wasm.
#[wasm_bindgen_test]
fn compile_context_memory_scope_pulls_stored_memories() {
    let svc = WasmMemoryService::new(16);
    svc.remember(
        "the canary rollback runbook is kubectl rollout undo",
        JsValue::UNDEFINED,
        JsValue::UNDEFINED,
        None,
    )
    .unwrap();

    let request = js_sys::JSON::parse(
        r#"{
            "query": "canary rollback runbook",
            "token_budget": 800,
            "memory_scope": {"k": 3},
            "fragments": [{"content": "Current task: fix the canary deploy."}]
        }"#,
    )
    .unwrap();

    let compiled = svc.compile_context(request).unwrap();
    let content = js_sys::Reflect::get(&compiled, &"content".into()).unwrap();
    let content = content.as_string().unwrap();
    assert!(
        content.contains("rollback runbook"),
        "the scoped memory must be pulled into the compiled context, got: {content}"
    );
}

// --- media fragments (US-009, PR3) ------------------------------------------
//
// `WasmMemoryService::compile_context` takes the request as a plain JS
// object and returns the wire JSON as-is (`serde_wasm_bindgen`, no field
// remapping), so `fragments[].media` already crosses the boundary via the
// same passthrough `compile_context_round_trips_with_string_ids` pins for
// ids — nothing wasm-specific needed to be built for a fragment to CARRY
// media across. What was never exercised is that the media-aware rules
// (`media.atomic`, dedup, cost) actually run correctly on `WasmStore`
// (a different `MemoryStore` impl than the native one every other media
// test in this workspace runs against).
//
// `retrieveContextSource` (below, V2d-2/A4) resolves a media handle back to
// its bytes within a wasm session, mirroring the Node binding's own
// `retrieveContextSource` — in-memory only, so the handle resolves within
// the current session's `WasmStore`, never across a process/page reload.

/// A real, independently-decodable 1x1 transparent PNG (IHDR + IDAT + IEND),
/// fixed bytes never derived from the fragment's caption or any other
/// property under test.
const PNG_1X1_B64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

/// Regression this attrapes: `WasmStore`'s `MemoryStore` impl diverges from
/// the native file-backed one (different metadata batch/get paths) — a
/// media-specific bug there (e.g. the atomic-packing rule never matching, or
/// the media source never getting a handle) would show up only on wasm,
/// never in the native `context_memory_bdd` suite.
#[wasm_bindgen_test]
fn compile_context_with_media_fragment_decides_atomic_preserve_and_mints_a_handle() {
    let svc = WasmMemoryService::new(16);
    let request = js_sys::JSON::parse(&format!(
        r#"{{
            "query": "a screenshot of the failing build",
            "token_budget": 4000,
            "fragments": [
                {{"content": "the failing build, before the fix",
                  "media": {{"mime": "image/png", "bytes_b64": "{PNG_1X1_B64}"}}}}
            ]
        }}"#
    ))
    .unwrap();

    let compiled = svc.compile_context(request).unwrap();
    let decisions = js_sys::Reflect::get(&compiled, &"decisions".into()).unwrap();
    let first = js_sys::Reflect::get(&decisions, &0.into()).unwrap();
    let rule_id = js_sys::Reflect::get(&first, &"rule_id".into()).unwrap();
    assert_eq!(
        rule_id.as_string().as_deref(),
        Some("media.atomic"),
        "the media fragment must be decided by the atomic-packing rule"
    );
    let action = js_sys::Reflect::get(&first, &"action".into()).unwrap();
    assert_eq!(action.as_string().as_deref(), Some("preserve"));

    let sources = js_sys::Reflect::get(&compiled, &"sources".into()).unwrap();
    let source = js_sys::Reflect::get(&sources, &0.into()).unwrap();
    let handle = js_sys::Reflect::get(&source, &"handle".into())
        .unwrap()
        .as_string()
        .expect("the media fragment gets an addressable ctx://source/ handle");
    assert!(handle.starts_with("ctx://source/"));
}

/// Byte-determinism of media fragments across the wasm boundary — the same
/// media-carrying request compiles to the exact same bytes twice, extending
/// `compile_context_is_deterministic` (text-only) to a fragment whose
/// classification depends on the decoded image (dimensions, token cost).
/// Regression this attrapes: a non-deterministic path in the image
/// estimator or the media dedup hash (e.g. an uninitialized buffer, a
/// HashMap iteration order leak) would flap this test across runs while a
/// single-compile assertion could not tell determinism from luck.
#[wasm_bindgen_test]
fn compile_context_with_media_fragment_is_deterministic() {
    let svc = WasmMemoryService::new(16);
    let request = || {
        js_sys::JSON::parse(&format!(
            r#"{{"query": "q", "token_budget": 4000,
                "fragments": [{{"content": "same caption",
                  "media": {{"mime": "image/png", "bytes_b64": "{PNG_1X1_B64}"}}}}]}}"#
        ))
        .unwrap()
    };
    let a = svc.compile_context(request()).unwrap();
    let b = svc.compile_context(request()).unwrap();
    let stringify = |v: &JsValue| js_sys::JSON::stringify(v).unwrap().as_string().unwrap();
    assert_eq!(stringify(&a), stringify(&b), "same media input, same bytes");
}

// --- retrieveContextSource (V2d-2/A4) ---------------------------------------
// In-memory semantics: a handle minted by `compile_context` resolves only
// within the current session's `WasmStore` (no persistence in WASM) — same
// caveat `compile_context`'s own doc comment already states.

/// A text-only source resolves back to its exact original content, with no
/// `media` field on the wire (matches the Node binding's own contract).
#[wasm_bindgen_test]
fn retrieve_context_source_resolves_a_text_only_source() {
    let svc = WasmMemoryService::new(16);
    let content_text = "Never restart the primary during a rebalance.";
    let request = js_sys::JSON::parse(&format!(
        r#"{{"query": "q", "token_budget": 4000, "fragments": [{{"content": "{content_text}"}}]}}"#
    ))
    .unwrap();
    let compiled = svc.compile_context(request).unwrap();
    let sources = js_sys::Reflect::get(&compiled, &"sources".into()).unwrap();
    let source = js_sys::Reflect::get(&sources, &0.into()).unwrap();
    let handle = js_sys::Reflect::get(&source, &"handle".into())
        .unwrap()
        .as_string()
        .unwrap();
    assert!(handle.starts_with("ctx://source/"));

    let resolved = svc.retrieve_context_source(&handle).unwrap();
    assert!(
        !resolved.is_instance_of::<js_sys::Map>(),
        "resolved source must be a plain object"
    );
    let content = js_sys::Reflect::get(&resolved, &"content".into())
        .unwrap()
        .as_string()
        .unwrap();
    assert_eq!(content, content_text);
    let handle_field = js_sys::Reflect::get(&resolved, &"handle".into())
        .unwrap()
        .as_string()
        .unwrap();
    assert_eq!(
        handle_field, handle,
        "the resolved object echoes its own handle"
    );
    let media = js_sys::Reflect::get(&resolved, &"media".into()).unwrap();
    assert!(
        media.is_undefined(),
        "a text-only source carries no media field"
    );
}

/// A media source round-trips byte-identical — the exact regression
/// `compile_context_with_media_fragment_decides_atomic_preserve_and_mints_a_handle`
/// flagged as uncovered before this binding existed.
#[wasm_bindgen_test]
fn retrieve_context_source_round_trips_a_media_source() {
    let svc = WasmMemoryService::new(16);
    let request = js_sys::JSON::parse(&format!(
        r#"{{
            "query": "a screenshot of the failing build",
            "token_budget": 4000,
            "fragments": [
                {{"content": "the failing build, before the fix",
                  "media": {{"mime": "image/png", "bytes_b64": "{PNG_1X1_B64}"}}}}
            ]
        }}"#
    ))
    .unwrap();
    let compiled = svc.compile_context(request).unwrap();
    let sources = js_sys::Reflect::get(&compiled, &"sources".into()).unwrap();
    let source = js_sys::Reflect::get(&sources, &0.into()).unwrap();
    let handle = js_sys::Reflect::get(&source, &"handle".into())
        .unwrap()
        .as_string()
        .unwrap();

    let resolved = svc.retrieve_context_source(&handle).unwrap();
    let media = js_sys::Reflect::get(&resolved, &"media".into()).unwrap();
    assert!(
        !media.is_undefined(),
        "a media fragment's source must carry media back"
    );
    let mime = js_sys::Reflect::get(&media, &"mime".into())
        .unwrap()
        .as_string()
        .unwrap();
    assert_eq!(mime, "image/png");
    let bytes_b64 = js_sys::Reflect::get(&media, &"bytes_b64".into())
        .unwrap()
        .as_string()
        .unwrap();
    assert_eq!(
        bytes_b64, PNG_1X1_B64,
        "media bytes round-trip byte-identical"
    );
}

/// An unknown handle rejects with a structured `{code: "NOT_FOUND"}` error,
/// mirroring the Node binding's contract, never panicking across the
/// boundary.
#[wasm_bindgen_test]
fn retrieve_context_source_unknown_handle_rejects_with_not_found() {
    let svc = WasmMemoryService::new(16);
    let err = svc
        .retrieve_context_source("ctx://source/999999999999999999")
        .unwrap_err();
    let code = js_sys::Reflect::get(&err, &"code".into())
        .unwrap()
        .as_string()
        .unwrap();
    assert_eq!(code, "NOT_FOUND");
}

// --- saveWorkingContext / loadWorkingContext / listWorkingContexts (#1517) --
//
// Issue #1517's decision (Option 2): expose these three on WASM/TS with a
// clearly documented INTRA-SESSION semantics — same caveat already pinned
// above for `compile_context`/`retrieveContextSource`: this binding's
// `WasmStore` is in-memory only, so a "saved" working context disappears on
// page reload. Useful to carry state between two calls within the SAME
// page load; not real cross-session persistence (no IndexedDB backend yet).

/// Save then load within the same session round-trips byte-identical,
/// including a `u64::MAX` id field (`decisions[].fragment_id`) — proves ids
/// never pass through a JS number on this path either, matching
/// `compile_context_round_trips_with_string_ids`.
#[wasm_bindgen_test]
fn save_then_load_working_context_round_trips_within_session() {
    let svc = WasmMemoryService::new(16);
    let working = js_sys::JSON::parse(
        r#"{
            "goal": "ship the canary fix",
            "active_constraints": [],
            "verified_facts": [],
            "open_hypotheses": [],
            "decisions": [
                {"fragment_id": "18446744073709551615", "rule_id": "media.atomic"}
            ],
            "exact_evidence": [],
            "pending_actions": ["roll back if error rate spikes"]
        }"#,
    )
    .unwrap();

    let id = svc
        .save_working_context("veles", "session-a", working.clone())
        .unwrap();
    assert!(!id.is_empty(), "save resolves to a non-empty fact id");

    let loaded = svc.load_working_context("veles", "session-a").unwrap();
    assert!(
        !loaded.is_instance_of::<js_sys::Map>(),
        "loaded working context must be a plain object"
    );
    let goal = js_sys::Reflect::get(&loaded, &"goal".into())
        .unwrap()
        .as_string()
        .unwrap();
    assert_eq!(goal, "ship the canary fix");

    let decisions = js_sys::Reflect::get(&loaded, &"decisions".into()).unwrap();
    let first = js_sys::Reflect::get(&decisions, &0.into()).unwrap();
    let fragment_id = js_sys::Reflect::get(&first, &"fragment_id".into())
        .unwrap()
        .as_string()
        .expect("fragment_id must cross back as a decimal string, not a number");
    assert_eq!(fragment_id, "18446744073709551615", "u64::MAX round-trips");

    let pending = js_sys::Reflect::get(&loaded, &"pending_actions".into()).unwrap();
    let first_action = js_sys::Reflect::get(&pending, &0.into())
        .unwrap()
        .as_string()
        .unwrap();
    assert_eq!(first_action, "roll back if error rate spikes");
}

/// Nothing saved under a project + session pair loads back as `null` in JS —
/// mirroring the Node binding's `loadWorkingContext` contract — never an
/// error, and never a forged/leaked value from an unrelated slot.
#[wasm_bindgen_test]
fn load_working_context_returns_null_when_nothing_saved() {
    let svc = WasmMemoryService::new(16);
    let loaded = svc
        .load_working_context("veles", "never-saved-session")
        .unwrap();
    assert!(
        loaded.is_null(),
        "an unsaved project+session pair must load back as null"
    );
}

/// `listWorkingContexts` surfaces every session saved under a project,
/// most-recently-saved first, and stays empty (not an error) for a project
/// that never saved anything.
#[wasm_bindgen_test]
fn list_working_contexts_lists_saved_sessions_for_a_project() {
    let svc = WasmMemoryService::new(16);
    let empty = js_sys::JSON::parse(r#"{}"#).unwrap();

    svc.save_working_context("veles", "session-a", empty.clone())
        .unwrap();
    svc.save_working_context("veles", "session-b", empty)
        .unwrap();

    let listed = svc.list_working_contexts("veles").unwrap();
    let sessions = js_sys::Reflect::get(&listed, &"sessions".into()).unwrap();
    let length = js_sys::Reflect::get(&sessions, &"length".into())
        .unwrap()
        .as_f64()
        .unwrap() as u32;
    assert_eq!(length, 2, "both saved sessions must be listed");

    let empty_project = svc.list_working_contexts("never-used-project").unwrap();
    let empty_sessions = js_sys::Reflect::get(&empty_project, &"sessions".into()).unwrap();
    let empty_length = js_sys::Reflect::get(&empty_sessions, &"length".into())
        .unwrap()
        .as_f64()
        .unwrap() as u32;
    assert_eq!(
        empty_length, 0,
        "a project that never saved anything lists no sessions"
    );
}

// --- compileTranscript / explainCompilation / contextSavings / suggestBudget
// (issue #1547) --------------------------------------------------------------

/// `compileTranscript` marshalling across the wasm boundary: a plain
/// marker-based transcript segments into turns, each becoming a compiled
/// fragment — `context` mirrors `compile_context`'s own shape (a plain
/// object, id fields as decimal strings) and `segmentation` reports the
/// detected format plus one audit entry per segment.
#[wasm_bindgen_test]
fn compile_transcript_segments_and_compiles_a_plain_transcript() {
    let svc = WasmMemoryService::new(16);
    let request = js_sys::JSON::parse(
        r#"{
            "query": "what broke the deploy",
            "transcript": "System: you are a helpful agent.\nUser: what broke the deploy?\nAssistant: ```rust\nlet x = 42;\n```\n",
            "token_budget": 5000
        }"#,
    )
    .unwrap();

    let result = svc.compile_transcript(request).unwrap();
    assert!(
        !result.is_instance_of::<js_sys::Map>(),
        "compile_transcript result must be a plain object"
    );

    let context = js_sys::Reflect::get(&result, &"context".into()).unwrap();
    let content = js_sys::Reflect::get(&context, &"content".into())
        .unwrap()
        .as_string()
        .unwrap();
    assert!(
        content.contains("let x = 42;"),
        "the fenced code segment must survive verbatim, got: {content}"
    );

    let segmentation = js_sys::Reflect::get(&result, &"segmentation".into()).unwrap();
    let format_detected = js_sys::Reflect::get(&segmentation, &"format_detected".into())
        .unwrap()
        .as_string()
        .unwrap();
    assert_eq!(format_detected, "plain");

    let segments = js_sys::Reflect::get(&segmentation, &"segments".into()).unwrap();
    let first_segment = js_sys::Reflect::get(&segments, &0.into()).unwrap();
    let fragment_id = js_sys::Reflect::get(&first_segment, &"fragment_id".into())
        .unwrap()
        .as_string()
        .expect("segment fragment_id must cross as a decimal string, not a number");
    assert!(fragment_id.parse::<u64>().is_ok());
}

/// An empty `transcript` rejects with a structured `{code: "INVALID_INPUT"}`
/// error rather than a silent, useless empty compile.
#[wasm_bindgen_test]
fn compile_transcript_rejects_an_empty_transcript() {
    let svc = WasmMemoryService::new(16);
    let request =
        js_sys::JSON::parse(r#"{"query": "q", "transcript": "", "token_budget": 1000}"#).unwrap();
    let err = svc.compile_transcript(request).unwrap_err();
    let code = js_sys::Reflect::get(&err, &"code".into())
        .unwrap()
        .as_string()
        .unwrap();
    assert_eq!(code, "INVALID_INPUT");
}

/// `contextSavings` aggregates a `compileTranscript` call exactly like a
/// `compileContext` one — both go through the same memory-bridge event
/// recording.
#[wasm_bindgen_test]
fn context_savings_aggregates_after_a_compile_transcript_call() {
    let svc = WasmMemoryService::new(16);
    let request = js_sys::JSON::parse(
        r#"{"query": "q", "transcript": "User: hi\nAssistant: hello\n", "token_budget": 1000}"#,
    )
    .unwrap();
    svc.compile_transcript(request).unwrap();

    let savings = svc.context_savings(None).unwrap();
    let events = js_sys::Reflect::get(&savings, &"events".into())
        .unwrap()
        .as_f64()
        .unwrap();
    assert_eq!(events, 1.0, "the compile_transcript call must be counted");
}

/// `explainCompilation` returns the decision for the fragment named by
/// `compileContext`'s own output, with every id field as a decimal string.
#[wasm_bindgen_test]
fn explain_compilation_returns_the_matching_decision() {
    let svc = WasmMemoryService::new(16);
    let request = js_sys::JSON::parse(
        r#"{"query": "q", "token_budget": 5000, "fragments": [{"content": "the deploy pipeline runs clippy before tests"}]}"#,
    )
    .unwrap();

    let compiled = svc.compile_context(request.clone()).unwrap();
    let decisions = js_sys::Reflect::get(&compiled, &"decisions".into()).unwrap();
    let first = js_sys::Reflect::get(&decisions, &0.into()).unwrap();
    let fragment_id = js_sys::Reflect::get(&first, &"fragment_id".into())
        .unwrap()
        .as_string()
        .unwrap();

    let decision = svc
        .explain_compilation(request, &fragment_id, None)
        .unwrap();
    let decided_fragment_id = js_sys::Reflect::get(&decision, &"fragment_id".into())
        .unwrap()
        .as_string()
        .unwrap();
    assert_eq!(decided_fragment_id, fragment_id);
}

/// `suggestBudget` looks up a known model in the static table and reports
/// `null` for an unknown one — never a guess.
#[wasm_bindgen_test]
fn suggest_budget_looks_up_known_and_unknown_models() {
    let svc = WasmMemoryService::new(16);

    let known = svc.suggest_budget("claude-sonnet-4-5", None).unwrap();
    let window = js_sys::Reflect::get(&known, &"window".into()).unwrap();
    assert!(
        window.as_f64().is_some(),
        "a known model must report a numeric window"
    );

    let unknown = svc.suggest_budget("not-a-real-model-xyz", None).unwrap();
    let window = js_sys::Reflect::get(&unknown, &"window".into()).unwrap();
    assert!(
        window.is_null(),
        "an unknown model must report window: null, never a guess"
    );
}
