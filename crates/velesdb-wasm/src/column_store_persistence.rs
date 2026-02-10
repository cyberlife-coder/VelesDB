//! IndexedDB persistence for `ColumnStoreWasm` (v3-08/Plan-03).
//!
//! Provides async save/load operations for column store data in browser apps.
//! Serializes schema + active rows as JSON blobs into IndexedDB.

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Event, IdbDatabase, IdbRequest, IdbTransactionMode};

use crate::column_store::ColumnStoreWasm;
use velesdb_core::column_store::{ColumnStore, ColumnType, ColumnValue};

const DB_NAME: &str = "velesdb_column_stores";
const DB_VERSION: u32 = 1;
const DATA_STORE: &str = "data";
const META_STORE: &str = "metadata";

/// Metadata for a persisted column store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnStoreMetadata {
    pub name: String,
    pub row_count: usize,
    pub column_count: usize,
    pub primary_key: Option<String>,
    pub created_at: f64,
    pub updated_at: f64,
    pub version: u32,
}

/// Serializable snapshot of a column store (schema + rows).
#[derive(Serialize, Deserialize)]
pub(crate) struct ColumnStoreSnapshot {
    schema: Vec<SchemaEntry>,
    primary_key: Option<String>,
    rows: Vec<serde_json::Map<String, serde_json::Value>>,
}

/// Schema entry for serialization.
#[derive(Serialize, Deserialize)]
pub(crate) struct SchemaEntry {
    name: String,
    #[serde(rename = "type")]
    col_type: String,
}

/// IndexedDB persistence manager for `ColumnStoreWasm`.
#[wasm_bindgen]
pub struct ColumnStorePersistence {
    db: Option<IdbDatabase>,
}

#[wasm_bindgen]
impl ColumnStorePersistence {
    /// Creates a new persistence instance (call `init()` to open database).
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self { db: None }
    }

    /// Initializes the database connection. Must be called before save/load.
    #[wasm_bindgen]
    pub async fn init(&mut self) -> Result<(), JsValue> {
        let db = open_column_store_db().await?;
        self.db = Some(db);
        Ok(())
    }

    /// Saves a column store to IndexedDB with the given name.
    #[wasm_bindgen]
    pub async fn save(&self, name: &str, store: &ColumnStoreWasm) -> Result<(), JsValue> {
        let db = self.db_ref()?;

        let store_names = js_sys::Array::new();
        store_names.push(&JsValue::from_str(DATA_STORE));
        store_names.push(&JsValue::from_str(META_STORE));

        let tx =
            db.transaction_with_str_sequence_and_mode(&store_names, IdbTransactionMode::Readwrite)?;

        let data_obj = tx.object_store(DATA_STORE)?;
        let meta_obj = tx.object_store(META_STORE)?;

        // Build snapshot from the ColumnStoreWasm
        let snapshot = store.export_snapshot();
        let snapshot_js = serde_wasm_bindgen::to_value(&snapshot)
            .map_err(|e| JsValue::from_str(&format!("Serialize error: {e}")))?;

        let req = data_obj.put_with_key(&snapshot_js, &JsValue::from_str(name))?;
        wait_for_request(&req).await?;

        // Save metadata
        let now = js_sys::Date::now();
        let metadata = ColumnStoreMetadata {
            name: name.to_string(),
            row_count: snapshot.rows.len(),
            column_count: snapshot.schema.len(),
            primary_key: snapshot.primary_key,
            created_at: now,
            updated_at: now,
            version: 1,
        };
        let meta_js = serde_wasm_bindgen::to_value(&metadata)
            .map_err(|e| JsValue::from_str(&format!("Serialize error: {e}")))?;
        let meta_req = meta_obj.put_with_key(&meta_js, &JsValue::from_str(name))?;
        wait_for_request(&meta_req).await?;

        Ok(())
    }

    /// Loads a column store from IndexedDB by name.
    #[wasm_bindgen]
    pub async fn load(&self, name: &str) -> Result<ColumnStoreWasm, JsValue> {
        let db = self.db_ref()?;

        let tx = db.transaction_with_str(DATA_STORE)?;
        let data_obj = tx.object_store(DATA_STORE)?;

        let req = data_obj.get(&JsValue::from_str(name))?;
        let result = wait_for_request(&req).await?;

        if result.is_undefined() || result.is_null() {
            return Err(JsValue::from_str(&format!(
                "Column store '{name}' not found"
            )));
        }

        let snapshot: ColumnStoreSnapshot = serde_wasm_bindgen::from_value(result)
            .map_err(|e| JsValue::from_str(&format!("Deserialize error: {e}")))?;

        Ok(ColumnStoreWasm::from_snapshot(snapshot))
    }

    /// Lists all saved column store names.
    #[wasm_bindgen]
    pub async fn list_stores(&self) -> Result<js_sys::Array, JsValue> {
        let db = self.db_ref()?;

        let tx = db.transaction_with_str(META_STORE)?;
        let store = tx.object_store(META_STORE)?;

        let req = store.get_all_keys()?;
        let result = wait_for_request(&req).await?;

        Ok(result.unchecked_into())
    }

    /// Gets metadata for a saved column store.
    #[wasm_bindgen]
    pub async fn get_metadata(&self, name: &str) -> Result<JsValue, JsValue> {
        let db = self.db_ref()?;

        let tx = db.transaction_with_str(META_STORE)?;
        let store = tx.object_store(META_STORE)?;

        let req = store.get(&JsValue::from_str(name))?;
        wait_for_request(&req).await
    }

    /// Deletes a saved column store by name.
    #[wasm_bindgen]
    pub async fn delete_store(&self, name: &str) -> Result<(), JsValue> {
        let db = self.db_ref()?;

        let store_names = js_sys::Array::new();
        store_names.push(&JsValue::from_str(DATA_STORE));
        store_names.push(&JsValue::from_str(META_STORE));

        let tx =
            db.transaction_with_str_sequence_and_mode(&store_names, IdbTransactionMode::Readwrite)?;

        let data_obj = tx.object_store(DATA_STORE)?;
        let req = data_obj.delete(&JsValue::from_str(name))?;
        wait_for_request(&req).await?;

        let meta_obj = tx.object_store(META_STORE)?;
        let meta_req = meta_obj.delete(&JsValue::from_str(name))?;
        wait_for_request(&meta_req).await?;

        Ok(())
    }
}

impl ColumnStorePersistence {
    fn db_ref(&self) -> Result<&IdbDatabase, JsValue> {
        self.db
            .as_ref()
            .ok_or_else(|| JsValue::from_str("Database not initialized. Call init() first."))
    }
}

impl Default for ColumnStorePersistence {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Export/import helpers on ColumnStoreWasm
// =============================================================================

impl ColumnStoreWasm {
    /// Exports the column store as a serializable snapshot.
    pub(crate) fn export_snapshot(&self) -> ColumnStoreSnapshot {
        let inner = self.inner_ref();
        let schema = self.export_schema();
        let pk = inner.primary_key_column().map(String::from);
        let rows = export_rows(inner);

        ColumnStoreSnapshot {
            schema,
            primary_key: pk,
            rows,
        }
    }

    /// Reconstructs a `ColumnStoreWasm` from a snapshot.
    pub(crate) fn from_snapshot(snap: ColumnStoreSnapshot) -> Self {
        let mut schema_tuples: Vec<(String, ColumnType)> = Vec::with_capacity(snap.schema.len());

        for entry in &snap.schema {
            let ct = match entry.col_type.as_str() {
                "int" => ColumnType::Int,
                "float" => ColumnType::Float,
                "bool" => ColumnType::Bool,
                // Reason: "string" and unknown types both default to String for forward compat
                _ => ColumnType::String,
            };
            schema_tuples.push((entry.name.clone(), ct));
        }

        let fields: Vec<(&str, ColumnType)> = schema_tuples
            .iter()
            .map(|(n, t)| (n.as_str(), *t))
            .collect();

        let store = if let Some(ref pk) = snap.primary_key {
            ColumnStore::with_primary_key(&fields, pk).unwrap_or_else(|_| {
                // Reason: fallback if PK column missing/invalid after schema change
                ColumnStore::with_schema(&fields)
            })
        } else {
            ColumnStore::with_schema(&fields)
        };

        let mut wasm = Self::from_raw(store, schema_tuples);

        // Re-insert all rows
        for row in &snap.rows {
            let values = wasm.json_map_to_values(row);
            let refs: Vec<(&str, ColumnValue)> = values
                .iter()
                .map(|(k, v)| (k.as_str(), v.clone()))
                .collect();
            let _ = wasm.inner_mut().insert_row(&refs);
        }

        wasm
    }

    fn export_schema(&self) -> Vec<SchemaEntry> {
        self.schema_ref()
            .iter()
            .map(|(name, ct)| SchemaEntry {
                name: name.clone(),
                col_type: match ct {
                    ColumnType::Int => "int".to_string(),
                    ColumnType::Float => "float".to_string(),
                    ColumnType::String => "string".to_string(),
                    ColumnType::Bool => "bool".to_string(),
                },
            })
            .collect()
    }
}

/// Exports all active rows as JSON maps.
fn export_rows(store: &ColumnStore) -> Vec<serde_json::Map<String, serde_json::Value>> {
    let mut rows = Vec::new();
    let col_names: Vec<String> = store.column_names().map(String::from).collect();

    for row_idx in 0..store.row_count() {
        if store.is_deleted(row_idx) {
            continue;
        }

        let mut row = serde_json::Map::new();
        for col_name in &col_names {
            if let Some(val) = store.get_value_as_json(col_name, row_idx) {
                row.insert(col_name.clone(), val);
            }
        }
        rows.push(row);
    }

    rows
}

// =============================================================================
// IndexedDB helpers (same pattern as graph_persistence.rs)
// =============================================================================

async fn open_column_store_db() -> Result<IdbDatabase, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("No window object"))?;
    let idb_factory = window
        .indexed_db()?
        .ok_or_else(|| JsValue::from_str("IndexedDB not available"))?;

    let request = idb_factory.open_with_u32(DB_NAME, DB_VERSION)?;

    let request_clone = request.clone();
    let onupgradeneeded = Closure::once(move |_event: Event| {
        let db: IdbDatabase = request_clone
            .result()
            .expect("Failed to get result")
            .unchecked_into();

        let store_names = db.object_store_names();

        if !contains_store(&store_names, DATA_STORE) {
            db.create_object_store(DATA_STORE)
                .expect("Failed to create data store");
        }

        if !contains_store(&store_names, META_STORE) {
            db.create_object_store(META_STORE)
                .expect("Failed to create metadata store");
        }
    });
    request.set_onupgradeneeded(Some(onupgradeneeded.as_ref().unchecked_ref()));
    onupgradeneeded.forget();

    let result = wait_for_request(&request).await?;
    Ok(result.unchecked_into())
}

fn contains_store(store_names: &web_sys::DomStringList, name: &str) -> bool {
    for i in 0..store_names.length() {
        if let Some(n) = store_names.get(i) {
            if n == name {
                return true;
            }
        }
    }
    false
}

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

#[cfg(test)]
mod tests {
    // Reason: IndexedDB tests require a real browser — validated via Playwright
}
