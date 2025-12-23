//! `IndexedDB` persistence for `VectorStore`.
//!
//! Provides async save/load operations for offline-first applications.

use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Event, IdbDatabase, IdbObjectStore, IdbRequest, IdbTransactionMode};

const STORE_NAME: &str = "vectors";
const DATA_KEY: &str = "data";

/// Opens or creates an `IndexedDB` database.
async fn open_db(db_name: &str) -> Result<IdbDatabase, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("No window object"))?;
    let idb_factory = window
        .indexed_db()?
        .ok_or_else(|| JsValue::from_str("IndexedDB not available"))?;

    let request = idb_factory.open(db_name)?;

    // Set up upgrade handler to create object store
    let request_clone = request.clone();
    let onupgradeneeded = Closure::once(move |_event: Event| {
        let db: IdbDatabase = request_clone
            .result()
            .expect("Failed to get result")
            .unchecked_into();

        // Create object store if it doesn't exist
        let store_names = db.object_store_names();
        let mut found = false;
        for i in 0..store_names.length() {
            if let Some(name) = store_names.get(i) {
                if name == STORE_NAME {
                    found = true;
                    break;
                }
            }
        }
        if !found {
            db.create_object_store(STORE_NAME)
                .expect("Failed to create object store");
        }
    });
    request.set_onupgradeneeded(Some(onupgradeneeded.as_ref().unchecked_ref()));
    onupgradeneeded.forget();

    // Wait for request to complete
    let result = wait_for_request(&request).await?;
    Ok(result.unchecked_into())
}

/// Waits for an IDB request to complete.
async fn wait_for_request(request: &IdbRequest) -> Result<JsValue, JsValue> {
    let promise = js_sys::Promise::new(&mut |resolve, reject| {
        let resolve_clone = resolve.clone();
        let onsuccess = Closure::once(move |_event: Event| {
            resolve_clone.call0(&JsValue::UNDEFINED).unwrap();
        });
        request.set_onsuccess(Some(onsuccess.as_ref().unchecked_ref()));
        onsuccess.forget();

        let onerror = Closure::once(move |_event: Event| {
            reject
                .call1(&JsValue::UNDEFINED, &JsValue::from_str("Request failed"))
                .unwrap();
        });
        request.set_onerror(Some(onerror.as_ref().unchecked_ref()));
        onerror.forget();
    });

    JsFuture::from(promise).await?;
    request.result()
}

/// Saves bytes to `IndexedDB`.
pub async fn save_to_indexeddb(db_name: &str, data: &[u8]) -> Result<(), JsValue> {
    let db = open_db(db_name).await?;

    let transaction =
        db.transaction_with_str_and_mode(STORE_NAME, IdbTransactionMode::Readwrite)?;
    let store: IdbObjectStore = transaction.object_store(STORE_NAME)?;

    // Convert bytes to Uint8Array
    let array = js_sys::Uint8Array::from(data);
    let request = store.put_with_key(&array, &JsValue::from_str(DATA_KEY))?;

    wait_for_request(&request).await?;
    Ok(())
}

/// Loads bytes from `IndexedDB`.
pub async fn load_from_indexeddb(db_name: &str) -> Result<Vec<u8>, JsValue> {
    let db = open_db(db_name).await?;

    let transaction = db.transaction_with_str_and_mode(STORE_NAME, IdbTransactionMode::Readonly)?;
    let store: IdbObjectStore = transaction.object_store(STORE_NAME)?;

    let request = store.get(&JsValue::from_str(DATA_KEY))?;
    let result = wait_for_request(&request).await?;

    if result.is_undefined() || result.is_null() {
        return Err(JsValue::from_str("No data found in database"));
    }

    let array: js_sys::Uint8Array = result.unchecked_into();
    Ok(array.to_vec())
}

/// Deletes the `IndexedDB` database.
pub async fn delete_database(db_name: &str) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("No window object"))?;
    let idb_factory = window
        .indexed_db()?
        .ok_or_else(|| JsValue::from_str("IndexedDB not available"))?;

    let request = idb_factory.delete_database(db_name)?;
    wait_for_request(&request).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    // Tests require wasm-bindgen-test and must be run with wasm-pack test
}
