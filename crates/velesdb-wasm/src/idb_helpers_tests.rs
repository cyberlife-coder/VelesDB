//! Tests for `idb_helpers`: where the `IndexedDB` factory comes from.
//!
//! These run under `wasm-pack test --node`, whose global scope has no
//! `window` — the same shape as a Web Worker's `WorkerGlobalScope`, which
//! exposes `indexedDB` on the global object and has no `window` either.

use super::idb_factory;
use wasm_bindgen::JsValue;
use wasm_bindgen_test::wasm_bindgen_test;

fn eval(source: &str) {
    js_sys::eval(source).expect("test: eval");
}

fn message(err: &JsValue) -> String {
    err.as_string().unwrap_or_default()
}

/// The factory is read from the global scope, so a worker (no `window`,
/// `indexedDB` on its global) reaches `IndexedDB`; a global without one is
/// refused as "IndexedDB not available", not as a missing window.
#[wasm_bindgen_test]
fn the_factory_comes_from_the_global_scope_without_a_window() {
    eval("delete globalThis.indexedDB; delete globalThis.IDBFactory;");
    assert!(
        js_sys::eval("typeof window")
            .ok()
            .and_then(|v| v.as_string())
            == Some("undefined".to_string()),
        "precondition: the test global has no window"
    );
    let err = idb_factory().expect_err("no indexedDB on the global");
    assert_eq!(message(&err), "IndexedDB not available");

    eval("globalThis.IDBFactory = class IDBFactory {}; globalThis.indexedDB = new IDBFactory();");
    let found = idb_factory();
    eval("delete globalThis.indexedDB; delete globalThis.IDBFactory;");
    assert!(found.is_ok(), "{:?}", found.err().map(|e| message(&e)));
}

/// A global `indexedDB` that is not an `IDBFactory` is refused rather than
/// used as one.
#[wasm_bindgen_test]
fn a_global_indexed_db_that_is_not_a_factory_is_refused() {
    eval("delete globalThis.IDBFactory; globalThis.IDBFactory = class IDBFactory {}; globalThis.indexedDB = {};");
    let found = idb_factory();
    eval("delete globalThis.indexedDB; delete globalThis.IDBFactory;");
    let err = found.expect_err("a plain object is not a factory");
    assert_eq!(message(&err), "IndexedDB not available");
}
