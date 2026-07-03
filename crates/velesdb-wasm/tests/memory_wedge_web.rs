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
