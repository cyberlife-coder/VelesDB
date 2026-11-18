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

/// A read-write transaction that commits only through [`Self::commit`].
///
/// A request's `success` only means the request ran: nothing is durable
/// until the transaction commits, and it can still abort (a quota error, a
/// later failing request). Two things follow, both held here once:
///
/// - The outcome is observed from the start: `complete`, `error` and `abort`
///   feed a future created with the transaction, so an early return never
///   leaves a rejected promise unhandled.
/// - Dropped without `commit` — any `?` on an error path — the transaction
///   aborts, so the writes it already made (a save's deletion of the
///   previous rows) are rolled back instead of committing on their own.
pub struct WriteTransaction {
    transaction: web_sys::IdbTransaction,
    outcome: Option<JsFuture>,
}

impl WriteTransaction {
    /// Opens a read-write transaction over `stores` of `db`.
    ///
    /// # Errors
    ///
    /// The `JsValue` IndexedDB raises when the transaction cannot start.
    pub fn begin(db: &web_sys::IdbDatabase, stores: &js_sys::Array) -> Result<Self, JsValue> {
        let transaction = db.transaction_with_str_sequence_and_mode(
            stores,
            web_sys::IdbTransactionMode::Readwrite,
        )?;
        let outcome = JsFuture::from(outcome_promise(&transaction));
        Ok(Self {
            transaction,
            outcome: Some(outcome),
        })
    }

    /// The object store `name` inside this transaction.
    ///
    /// # Errors
    ///
    /// The `JsValue` IndexedDB raises for a store outside the transaction.
    pub fn store(&self, name: &str) -> Result<web_sys::IdbObjectStore, JsValue> {
        self.transaction.object_store(name)
    }

    /// Waits for the transaction to commit.
    ///
    /// # Errors
    ///
    /// `"Transaction failed"` / `"Transaction aborted"` when it does not.
    pub async fn commit(mut self) -> Result<(), JsValue> {
        match self.outcome.take() {
            Some(outcome) => outcome.await.map(|_| ()),
            None => Ok(()),
        }
    }
}

impl Drop for WriteTransaction {
    fn drop(&mut self) {
        if self.outcome.is_some() {
            // Not committed: an error path left early. Roll back what was
            // written. Aborting a transaction that already finished throws,
            // and there is nothing left to undo then.
            let _ = self.transaction.abort();
        }
    }
}

/// A promise that resolves when `transaction` commits and rejects when it
/// errors or aborts.
fn outcome_promise(transaction: &web_sys::IdbTransaction) -> js_sys::Promise {
    js_sys::Promise::new(&mut |resolve, reject| {
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
    })
}

/// Gets the `IdbFactory` from the global scope.
///
/// Both a `Window` and a Web Worker's `WorkerGlobalScope` expose `indexedDB`
/// on the global object, so reading it there serves both; a worker has no
/// `window`. A throwing getter (e.g. an opaque origin) propagates its error.
///
/// Shared by both vector and graph persistence open functions.
pub fn idb_factory() -> Result<web_sys::IdbFactory, JsValue> {
    js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("indexedDB"))?
        .dyn_into::<web_sys::IdbFactory>()
        .map_err(|_| JsValue::from_str("IndexedDB not available"))
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

#[cfg(all(test, target_arch = "wasm32"))]
#[path = "idb_helpers_tests.rs"]
mod tests;
