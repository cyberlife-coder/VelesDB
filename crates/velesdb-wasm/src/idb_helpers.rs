//! Shared `IndexedDB` helpers used by both vector and graph persistence.
//!
//! Centralizes the `wait_for_request` and `contains_store` utilities so that
//! `persistence.rs` and `graph_persistence.rs` share a single implementation.

use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Event, IdbRequest};

/// Waits for an `IDBRequest` to complete, resolving with its result.
///
/// Wraps the callback-based `IDBRequest` API in a `Future` by creating a
/// `Promise` that resolves on `onsuccess` and rejects on `onerror`.
pub async fn wait_for_request(request: &IdbRequest) -> Result<JsValue, JsValue> {
    let promise = js_sys::Promise::new(&mut |resolve, reject| {
        let resolve_clone = resolve.clone();
        let onsuccess = Closure::once(move |_event: Event| {
            let _ = resolve_clone.call0(&JsValue::UNDEFINED);
        });
        request.set_onsuccess(Some(onsuccess.as_ref().unchecked_ref()));
        // Standard WASM pattern: Closure::forget leaks into JS GC (unavoidable without wasm_bindgen_futures)
        onsuccess.forget();

        let onerror = Closure::once(move |_event: Event| {
            let _ = reject.call1(&JsValue::UNDEFINED, &JsValue::from_str("Request failed"));
        });
        request.set_onerror(Some(onerror.as_ref().unchecked_ref()));
        // Standard WASM pattern: Closure::forget leaks into JS GC (unavoidable without wasm_bindgen_futures)
        onerror.forget();
    });

    JsFuture::from(promise).await?;
    request.result()
}

/// Checks if a `DOMStringList` contains the given store name.
pub fn contains_store(store_names: &web_sys::DomStringList, name: &str) -> bool {
    for i in 0..store_names.length() {
        if let Some(n) = store_names.get(i) {
            if n == name {
                return true;
            }
        }
    }
    false
}

/// Attaches an upgrade handler to the given request that ensures the listed
/// stores exist, then awaits the request and returns the opened database.
///
/// Consolidates the repeated `onupgradeneeded` + `wait_for_request` pattern
/// used by both vector and graph persistence.
pub async fn open_with_stores(
    request: &web_sys::IdbOpenDbRequest,
    stores: &'static [&'static str],
    error_label: &str,
) -> Result<web_sys::IdbDatabase, JsValue> {
    let request_clone = request.clone();
    let label = error_label.to_string();
    let onupgradeneeded = Closure::once(move |_event: Event| {
        let result = match request_clone.result() {
            Ok(r) => r,
            Err(err) => {
                web_sys::console::error_2(&JsValue::from_str(&label), &err);
                return;
            }
        };
        let db: web_sys::IdbDatabase = result.unchecked_into();
        ensure_stores(&db, stores);
    });
    request.set_onupgradeneeded(Some(onupgradeneeded.as_ref().unchecked_ref()));
    // Standard WASM pattern: Closure::forget leaks into JS GC
    onupgradeneeded.forget();

    let result = wait_for_request(request).await?;
    Ok(result.unchecked_into())
}

/// Gets the `IdbFactory` from the current window context.
///
/// Shared by both vector and graph persistence open functions.
pub fn idb_factory() -> Result<web_sys::IdbFactory, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("No window object"))?;
    window
        .indexed_db()?
        .ok_or_else(|| JsValue::from_str("IndexedDB not available"))
}

/// Ensures that every store name in `names` exists in the given `IdbDatabase`.
///
/// Creates missing object stores and logs errors for any that fail.
pub fn ensure_stores(db: &web_sys::IdbDatabase, names: &[&str]) {
    let existing = db.object_store_names();
    for &name in names {
        if !contains_store(&existing, name) {
            if let Err(err) = db.create_object_store(name) {
                web_sys::console::error_2(
                    &JsValue::from_str(&format!("Failed to create store '{name}'")),
                    &err,
                );
            }
        }
    }
}
