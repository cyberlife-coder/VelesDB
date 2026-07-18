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
