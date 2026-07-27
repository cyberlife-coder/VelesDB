// src-tauri/src/velesdb_setup.rs — the Rust half of the graph example.
//
// Why this file exists: `add_edge` refuses an edge whose endpoints have no
// stored node payload (#1442), and no IPC command upserts a node payload. So
// graph nodes have to be created on the Rust side, through the plugin's own
// managed state.
//
// Declare it from main.rs:
//     mod velesdb_setup;
// and register the command:
//     .invoke_handler(tauri::generate_handler![velesdb_setup::seed_graph_nodes])

use std::sync::Arc;

use tauri::{AppHandle, Manager};
use tauri_plugin_velesdb::{Error, VelesDbState};

/// Creates the node payloads the frontend's edges will point at.
///
/// `VelesDbState::with_db` hands you the raw `velesdb_core::Database` the
/// plugin opened — the same instance the IPC commands use, so anything written
/// here is immediately visible to the frontend.
///
/// The graph collection must already exist. Either the frontend calls
/// `create_graph_collection` first, or you create it here with the equivalent
/// core call before upserting nodes.
///
/// # Errors
///
/// Returns a message when the collection is missing or a node write fails; the
/// frontend receives it as the rejection value of `invoke`.
#[tauri::command]
pub async fn seed_graph_nodes(app: AppHandle, collection: String) -> Result<usize, String> {
    let state = app.state::<VelesDbState>();

    state
        .with_db(|db: Arc<velesdb_core::Database>| {
            let coll = db
                .get_graph_collection(&collection)
                .ok_or_else(|| Error::CollectionNotFound(collection.clone()))?;

            let nodes = [
                (100_u64, "Rust in Action", "book"),
                (200_u64, "Ownership and Borrowing", "chapter"),
                (300_u64, "Lifetimes", "chapter"),
            ];

            for (id, title, kind) in nodes {
                coll.upsert_node_payload(id, &serde_json::json!({ "title": title, "kind": kind }))
                    .map_err(Error::Database)?;
            }

            Ok(nodes.len())
        })
        .map_err(|e| format!("{e}"))
}

/// A search that never crosses the IPC boundary.
///
/// This is the pattern to reach for whenever the query vector is produced in
/// Rust: computing the embedding, searching, and returning only the result
/// keeps a 384-float array out of every round trip. It is what the RAG demo
/// does — see demos/tauri-rag-app.
///
/// # Errors
///
/// Returns a message when the collection is missing or the search fails.
#[tauri::command]
pub async fn count_matches(app: AppHandle, collection: String) -> Result<usize, String> {
    let state = app.state::<VelesDbState>();

    state
        .with_db(|db: Arc<velesdb_core::Database>| {
            let coll = db
                .get_vector_collection(&collection)
                .ok_or_else(|| Error::CollectionNotFound(collection.clone()))?;

            // In a real app this vector comes from your embedding model.
            coll.search(&[0.1_f32; 384], 5)
                .map(|r| r.len())
                .map_err(Error::Database)
        })
        .map_err(|e| format!("{e}"))
}
