//! # tauri-plugin-velesdb
//!
//! A Tauri plugin for `VelesDB` - Vector search in desktop applications.
//!
//! This plugin provides seamless integration of `VelesDB`'s vector database
//! capabilities into Tauri desktop applications.
//!
//! ## Features
//!
//! - **Collection Management**: Create, list, and delete vector collections
//! - **Vector Operations**: Insert, update, and delete vectors with payloads
//! - **Vector Search**: Fast similarity search with multiple distance metrics
//! - **Text Search**: BM25 full-text search across payloads
//! - **Hybrid Search**: Combined vector + text search with RRF fusion
//! - **`VelesQL`**: SQL-like query language for advanced searches
//!
//! ## Usage
//!
//! ### Rust (Plugin Registration)
//!
//! ```rust,ignore
//! fn main() {
//!     tauri::Builder::default()
//!         .plugin(tauri_plugin_velesdb::init("./data"))
//!         .run(tauri::generate_context!())
//!         .expect("error while running tauri application");
//! }
//! ```
//!
//! ### JavaScript (Frontend)
//!
//! ```javascript
//! import { invoke } from '@tauri-apps/api/core';
//!
//! // Create a collection
//! await invoke('plugin:velesdb|create_collection', {
//!   request: { name: 'documents', dimension: 768, metric: 'cosine' }
//! });
//!
//! // Insert vectors
//! await invoke('plugin:velesdb|upsert', {
//!   request: {
//!     collection: 'documents',
//!     points: [{ id: 1, vector: [...], payload: { title: 'Doc' } }]
//!   }
//! });
//!
//! // Search
//! const results = await invoke('plugin:velesdb|search', {
//!   request: { collection: 'documents', vector: [...], topK: 10 }
//! });
//! ```

#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

use std::path::Path;

use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, Runtime,
};

pub mod commands;
pub mod error;
pub mod state;

pub use error::{CommandError, Error, Result};
pub use state::VelesDbState;

/// Initializes the `VelesDB` plugin with the specified data directory.
///
/// # Arguments
///
/// * `path` - Path to the database directory
///
/// # Returns
///
/// A configured `TauriPlugin` ready to be registered with the Tauri app.
///
/// # Example
///
/// ```rust,ignore
/// tauri::Builder::default()
///     .plugin(tauri_plugin_velesdb::init("./velesdb_data"))
///     .run(tauri::generate_context!())
///     .expect("error while running tauri application");
/// ```
#[must_use]
pub fn init<R: Runtime, P: AsRef<Path>>(path: P) -> TauriPlugin<R> {
    let db_path = path.as_ref().to_path_buf();

    Builder::new("velesdb")
        .invoke_handler(tauri::generate_handler![
            commands::create_collection,
            commands::delete_collection,
            commands::list_collections,
            commands::get_collection,
            commands::upsert,
            commands::get_points,
            commands::delete_points,
            commands::search,
            commands::batch_search,
            commands::text_search,
            commands::hybrid_search,
            commands::query,
        ])
        .setup(move |app, _api| {
            let state = VelesDbState::new(db_path.clone());
            app.manage(state);
            tracing::info!("VelesDB plugin initialized with path: {:?}", db_path);
            Ok(())
        })
        .build()
}

/// Initializes the `VelesDB` plugin with the default data directory.
///
/// Uses `./velesdb_data` as the default path.
///
/// # Returns
///
/// A configured `TauriPlugin` ready to be registered with the Tauri app.
#[must_use]
pub fn init_default<R: Runtime>() -> TauriPlugin<R> {
    init("./velesdb_data")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_velesdb_state_creation() {
        // Arrange
        let path = std::path::PathBuf::from("/tmp/test");

        // Act
        let state = VelesDbState::new(path.clone());

        // Assert
        assert_eq!(state.path(), &path);
    }
}
