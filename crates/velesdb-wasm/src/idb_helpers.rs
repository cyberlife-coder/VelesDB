//! Shared `IndexedDB` helpers used by both vector and graph persistence.
//!
//! Centralizes opening a database with its stores, awaiting a request, and
//! awaiting a transaction's commit, so that `persistence.rs` and
//! `graph_persistence.rs` share a single implementation.

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

/// Opens the database `name` at its current version and makes sure every
/// store in `stores` exists, upgrading it once if one is missing.
///
/// Opening at the current version (no version argument) never fails on a
/// database a newer layout already upgraded, and an upgrade fires only when
/// it is needed: `onupgradeneeded` runs for a database that does not exist
/// yet, and a database that exists without one of the stores (an older
/// layout) is reopened at the next version, which creates it. Opening once at
/// a fixed version, or without one, left such a database without its store.
pub async fn open_with_stores(
    name: &str,
    stores: &'static [&'static str],
    error_label: &str,
) -> Result<web_sys::IdbDatabase, JsValue> {
    let factory = idb_factory()?;
    let db = await_open(&factory.open(name)?, stores, error_label).await?;
    let existing = db.object_store_names();
    if stores.iter().all(|store| contains_store(&existing, store)) {
        return Ok(db);
    }
    let next = db.version() + 1.0;
    db.close();
    // Reason: an IndexedDB version is a small positive integer stored as f64.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let request = factory.open_with_u32(name, next as u32)?;
    await_open(&request, stores, error_label).await
}

/// Attaches an upgrade handler to `request` that creates every missing store,
/// then awaits the request and returns the opened database.
async fn await_open(
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

/// A future that resolves once `transaction` has committed and rejects if it
/// errors or aborts.
///
/// A request's `success` only means the request ran: the transaction can
/// still abort (a quota error, a later failing request), and nothing is
/// durable until it commits. A save awaits this before reporting success.
/// Create it right after the transaction, before its requests settle, so the
/// `complete` event cannot fire unobserved.
pub fn transaction_committed(
    transaction: &web_sys::IdbTransaction,
) -> impl std::future::Future<Output = Result<(), JsValue>> {
    let promise = js_sys::Promise::new(&mut |resolve, reject| {
        let oncomplete = Closure::once(move |_event: Event| {
            let _ = resolve.call0(&JsValue::UNDEFINED);
        });
        transaction.set_oncomplete(Some(oncomplete.as_ref().unchecked_ref()));
        // Standard WASM pattern: Closure::forget leaks into JS GC
        oncomplete.forget();

        let reject_abort = reject.clone();
        let onerror = Closure::once(move |_event: Event| {
            let _ = reject.call1(
                &JsValue::UNDEFINED,
                &JsValue::from_str("Transaction failed"),
            );
        });
        transaction.set_onerror(Some(onerror.as_ref().unchecked_ref()));
        onerror.forget();

        let onabort = Closure::once(move |_event: Event| {
            let _ = reject_abort.call1(
                &JsValue::UNDEFINED,
                &JsValue::from_str("Transaction aborted"),
            );
        });
        transaction.set_onabort(Some(onabort.as_ref().unchecked_ref()));
        onabort.forget();
    });
    async move {
        JsFuture::from(promise).await?;
        Ok(())
    }
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
