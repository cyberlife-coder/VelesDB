//! wasm-bindgen tests for the shape of every record `VectorStore` and the
//! graph types hand back to JavaScript (#2332).
//!
//! The default `serde_wasm_bindgen` serializer turns a `serde_json::Value`
//! object (and any Rust map) into an ES2015 `Map`, on which `record.id` is
//! `undefined` and `JSON.stringify` yields `{}`. Every record must arrive as a
//! plain object readable by property access. Only a real JS runtime can see
//! the difference, which is why these live here and not in the native suites.

#![cfg(target_arch = "wasm32")]

use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::*;

use velesdb_wasm::{GraphEdge, GraphNode, VectorStore};

/// Reads `key` off `value` by plain property access, as JS code would.
fn prop(value: &JsValue, key: &str) -> JsValue {
    js_sys::Reflect::get(value, &key.into()).expect("Reflect.get on an object")
}

/// The first element of a JS array result.
fn first(results: &JsValue) -> JsValue {
    js_sys::Reflect::get(results, &0.into()).expect("result is an array")
}

/// Asserts `value` is a plain object, not an ES2015 `Map`.
fn assert_plain_object(value: &JsValue, what: &str) {
    assert!(
        !value.is_instance_of::<js_sys::Map>(),
        "{what} must be a plain object, not an ES2015 Map"
    );
    assert!(value.is_object(), "{what} must be an object");
}

/// A two-vector store whose record 1 carries `{title: "hello rust"}` and whose
/// record 2 carries no payload at all.
fn store_with_payloads() -> VectorStore {
    let mut store = VectorStore::new(2, "cosine").unwrap();
    let payload = js_sys::Object::new();
    js_sys::Reflect::set(&payload, &"title".into(), &"hello rust".into()).unwrap();
    store
        .insert_with_payload(1, &[1.0, 0.0], payload.into())
        .unwrap();
    store.insert(2, &[0.0, 1.0]).unwrap();
    store
}

/// Asserts a plain-object record for id 1 carrying the payload above.
fn assert_record(record: &JsValue, what: &str) {
    assert_plain_object(record, what);
    assert_eq!(prop(record, "id").as_f64(), Some(1.0), "{what}.id");
    let payload = prop(record, "payload");
    assert_plain_object(&payload, &format!("{what}.payload"));
    assert_eq!(
        prop(&payload, "title").as_string().as_deref(),
        Some("hello rust"),
        "{what}.payload.title"
    );
}

/// [`assert_record`], plus a positive numeric `score`.
fn assert_scored_record(record: &JsValue, what: &str) {
    assert_record(record, what);
    let score = prop(record, "score").as_f64();
    assert!(
        score.is_some_and(|s| s > 0.0),
        "{what}.score must be a positive number, got {score:?}"
    );
}

#[wasm_bindgen_test]
fn hybrid_search_records_are_plain_objects() {
    let store = store_with_payloads();
    let results = store
        .hybrid_search(&[1.0, 0.0], "hello", 2, Some(0.5))
        .unwrap();
    assert_scored_record(&first(&results), "hybrid_search record");
}

#[wasm_bindgen_test]
fn search_with_filter_records_are_plain_objects() {
    let store = store_with_payloads();
    let filter: JsValue =
        js_sys::JSON::parse(r#"{"condition":{"type":"eq","field":"title","value":"hello rust"}}"#)
            .unwrap();
    let results = store.search_with_filter(&[1.0, 0.0], 2, filter).unwrap();
    assert_scored_record(&first(&results), "search_with_filter record");
}

/// `text_search` selects by case-insensitive substring and ranks nothing, so
/// its records carry `score: null` — never a number a caller could threshold
/// or sort on — and come back in insertion order.
#[wasm_bindgen_test]
fn text_search_records_are_plain_objects_without_a_score() {
    let mut store = store_with_payloads();
    let payload = js_sys::Object::new();
    js_sys::Reflect::set(&payload, &"title".into(), &"HELLO again".into()).unwrap();
    store
        .insert_with_payload(3, &[1.0, 1.0], payload.into())
        .unwrap();

    let results = store.text_search("hello", 5, None).unwrap();

    let record = first(&results);
    assert_record(&record, "text_search record");
    assert!(
        prop(&record, "score").is_null(),
        "text_search score is null"
    );
    let ids: Vec<f64> = js_sys::Array::from(&results)
        .iter()
        .map(|r| prop(&r, "id").as_f64().unwrap())
        .collect();
    assert_eq!(ids, vec![1.0, 3.0], "every match, in insertion order");
}

/// An absent payload arrives as `null`, as `JSON.stringify` would keep it.
#[wasm_bindgen_test]
fn absent_payload_is_null() {
    let store = store_with_payloads();
    let results = store
        .hybrid_search(&[0.0, 1.0], "nothing matches", 1, Some(1.0))
        .unwrap();
    let record = first(&results);
    assert_eq!(prop(&record, "id").as_f64(), Some(2.0));
    assert!(prop(&record, "payload").is_null(), "absent payload is null");
}

#[wasm_bindgen_test]
fn get_returns_a_plain_object() {
    let store = store_with_payloads();
    let point = store.get(1).unwrap();
    assert_plain_object(&point, "get() point");
    assert_eq!(prop(&point, "id").as_f64(), Some(1.0));
    assert!(js_sys::Array::is_array(&prop(&point, "vector")));
    let payload = prop(&point, "payload");
    assert_plain_object(&payload, "get() payload");
    assert_eq!(
        prop(&payload, "title").as_string().as_deref(),
        Some("hello rust")
    );
}

#[wasm_bindgen_test]
fn query_bindings_are_a_plain_object() {
    let store = store_with_payloads();
    let results = store.query(&[1.0, 0.0], 1).unwrap();
    let record = first(&results);
    assert_plain_object(&record, "query() record");
    assert_eq!(prop(&record, "nodeId").as_f64(), Some(1.0));
    let bindings = prop(&record, "bindings");
    assert_plain_object(&bindings, "query() bindings");
    assert_eq!(
        prop(&bindings, "title").as_string().as_deref(),
        Some("hello rust")
    );
}

#[wasm_bindgen_test]
fn graph_node_to_json_properties_are_a_plain_object() {
    let mut node = GraphNode::new(7, "Person");
    node.set_string_property("name", "Ada");
    let json = node.to_json().unwrap();
    assert_plain_object(&json, "GraphNode.to_json()");
    let properties = prop(&json, "properties");
    assert_plain_object(&properties, "GraphNode.to_json().properties");
    assert_eq!(
        prop(&properties, "name").as_string().as_deref(),
        Some("Ada")
    );
}

#[wasm_bindgen_test]
fn graph_edge_to_json_properties_are_a_plain_object() {
    let mut edge = GraphEdge::new(1, 7, 8, "KNOWS").unwrap();
    edge.set_number_property("since", 2020.0);
    let json = edge.to_json().unwrap();
    let properties = prop(&json, "properties");
    assert_plain_object(&properties, "GraphEdge.to_json().properties");
    assert_eq!(prop(&properties, "since").as_f64(), Some(2020.0));
}
