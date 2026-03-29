//! `IndexedDB` persistence for `VectorStore`.
//!
//! Provides async save/load operations for offline-first applications.

use crate::idb_helpers::{idb_factory, open_with_stores, wait_for_request};
use wasm_bindgen::prelude::*;
use web_sys::{IdbDatabase, IdbObjectStore, IdbTransactionMode};

const STORE_NAME: &str = "vectors";
const STORES: &[&str] = &[STORE_NAME];
const DATA_KEY: &str = "data";

/// Opens or creates an `IndexedDB` database.
async fn open_db(db_name: &str) -> Result<IdbDatabase, JsValue> {
    let factory = idb_factory()?;
    let request = factory.open(db_name)?;
    open_with_stores(&request, STORES, "Failed to access IndexedDB result").await
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
    let factory = idb_factory()?;
    let request = factory.delete_database(db_name)?;
    wait_for_request(&request).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    // Tests require wasm-bindgen-test and must be run with wasm-pack test
}
