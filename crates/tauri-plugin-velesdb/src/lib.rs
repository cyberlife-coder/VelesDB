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
pub mod helpers;
pub mod state;
pub mod types;

pub use error::{CommandError, Error, Result};
pub use state::VelesDbState;

// ============================================================================
// Simple In-Memory Index for Demo (VelesDbExt trait)
// ============================================================================

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Simple in-memory vector index for demo purposes.
/// For production, use the full plugin commands with persistent storage.
pub struct SimpleVectorIndex {
    vectors: HashMap<u64, Vec<f32>>,
    dimension: usize,
}

impl SimpleVectorIndex {
    /// Creates a new empty index with the given dimension.
    #[must_use]
    pub fn new(dimension: usize) -> Self {
        Self {
            vectors: HashMap::new(),
            dimension,
        }
    }

    /// Inserts a vector with the given ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the vector dimension doesn't match the index dimension.
    pub fn insert(&mut self, id: u64, vector: &[f32]) -> Result<()> {
        if vector.len() != self.dimension {
            return Err(Error::InvalidConfig(format!(
                "Vector dimension mismatch: expected {}, got {}",
                self.dimension,
                vector.len()
            )));
        }
        self.vectors.insert(id, vector.to_vec());
        Ok(())
    }

    /// Searches for the k most similar vectors.
    ///
    /// # Errors
    ///
    /// Returns an error if the query dimension doesn't match the index dimension.
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<(u64, f32)>> {
        if query.len() != self.dimension {
            return Err(Error::InvalidConfig(format!(
                "Query dimension mismatch: expected {}, got {}",
                self.dimension,
                query.len()
            )));
        }

        let mut scores: Vec<(u64, f32)> = self
            .vectors
            .iter()
            .map(|(id, vec)| {
                let score = cosine_similarity(query, vec);
                (*id, score)
            })
            .collect();

        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scores.truncate(k);
        Ok(scores)
    }

    /// Returns the number of vectors in the index.
    #[must_use]
    pub fn len(&self) -> usize {
        self.vectors.len()
    }

    /// Returns true if the index is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.vectors.is_empty()
    }

    /// Returns the dimension of vectors in this index.
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// Clears all vectors from the index.
    pub fn clear(&mut self) {
        self.vectors.clear();
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a > 0.0 && norm_b > 0.0 {
        dot / (norm_a * norm_b)
    } else {
        0.0
    }
}

/// State for the simple vector index (used by `VelesDbExt`).
pub struct SimpleIndexState(pub Arc<Mutex<SimpleVectorIndex>>);

/// Extension trait for easy access to `VelesDB` from Tauri `AppHandle`.
pub trait VelesDbExt<R: Runtime> {
    /// Returns a handle to the simple vector index.
    fn velesdb(&self) -> SimpleIndexHandle;
}

impl<R: Runtime, T: Manager<R>> VelesDbExt<R> for T {
    fn velesdb(&self) -> SimpleIndexHandle {
        let state = self
            .try_state::<SimpleIndexState>()
            .expect("SimpleIndexState not initialized. Did you call init()?");
        SimpleIndexHandle(Arc::clone(&state.0))
    }
}

/// Handle to interact with the simple vector index.
pub struct SimpleIndexHandle(Arc<Mutex<SimpleVectorIndex>>);

impl SimpleIndexHandle {
    /// Inserts a vector with the given ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the vector dimension doesn't match the index dimension.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn insert(&self, id: u64, vector: &[f32]) -> Result<()> {
        self.0.lock().unwrap().insert(id, vector)
    }

    /// Searches for the k most similar vectors.
    ///
    /// # Errors
    ///
    /// Returns an error if the query dimension doesn't match the index dimension.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<(u64, f32)>> {
        self.0.lock().unwrap().search(query, k)
    }

    /// Returns the number of vectors in the index.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }

    /// Returns true if the index is empty.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.lock().unwrap().is_empty()
    }

    /// Returns the dimension of vectors in this index.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.0.lock().unwrap().dimension()
    }

    /// Clears all vectors from the index.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
}

/// Initializes the `VelesDB` plugin with the default settings.
///
/// Uses `./velesdb_data` as the default path for persistence.
/// This is the simplest way to get started.
///
/// # Example
///
/// ```rust,ignore
/// tauri::Builder::default()
///     .plugin(tauri_plugin_velesdb::init())
///     .run(tauri::generate_context!())
///     .expect("error while running tauri application");
/// ```
#[must_use]
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    init_with_path("./velesdb_data")
}

/// Initializes the `VelesDB` plugin with a custom data directory.
///
/// # Arguments
///
/// * `path` - Path to the database directory
///
/// # Example
///
/// ```rust,ignore
/// tauri::Builder::default()
///     .plugin(tauri_plugin_velesdb::init_with_path("./my_data"))
///     .run(tauri::generate_context!())
///     .expect("error while running tauri application");
/// ```
#[must_use]
pub fn init_with_path<R: Runtime, P: AsRef<Path>>(path: P) -> TauriPlugin<R> {
    let db_path = path.as_ref().to_path_buf();

    Builder::new("velesdb")
        .invoke_handler(tauri::generate_handler![
            commands::create_collection,
            commands::create_metadata_collection,
            commands::delete_collection,
            commands::list_collections,
            commands::get_collection,
            commands::upsert,
            commands::upsert_metadata,
            commands::get_points,
            commands::delete_points,
            commands::search,
            commands::batch_search,
            commands::text_search,
            commands::hybrid_search,
            commands::multi_query_search,
            commands::query,
            commands::is_empty,
            commands::flush,
        ])
        .setup(move |app, _api| {
            let state = VelesDbState::new(db_path.clone());
            app.manage(state);
            // Initialize simple in-memory index for VelesDbExt trait (384 dimensions for AllMiniLML6V2)
            let simple_index = SimpleIndexState(Arc::new(Mutex::new(SimpleVectorIndex::new(384))));
            app.manage(simple_index);
            tracing::info!("VelesDB plugin initialized with path: {:?}", db_path);
            Ok(())
        })
        .build()
}

/// Alias for `init()` for backward compatibility.
#[must_use]
pub fn init_default<R: Runtime>() -> TauriPlugin<R> {
    init()
}

/// Initializes the `VelesDB` plugin using the platform's app data directory.
///
/// This is the recommended way to initialize the plugin for production apps.
/// Data is stored in the standard location for each platform:
/// - **Windows**: `%APPDATA%\<app_name>\velesdb\`
/// - **macOS**: `~/Library/Application Support/<app_name>/velesdb/`
/// - **Linux**: `~/.local/share/<app_name>/velesdb/`
///
/// # Arguments
///
/// * `app_name` - Your application's name (used in the path)
///
/// # Example
///
/// ```rust,ignore
/// tauri::Builder::default()
///     .plugin(tauri_plugin_velesdb::init_with_app_data("my-app"))
///     .run(tauri::generate_context!())
///     .expect("error while running tauri application");
/// ```
///
/// # Panics
///
/// Panics if the app data directory cannot be determined for the platform.
#[must_use]
pub fn init_with_app_data<R: Runtime>(app_name: &str) -> TauriPlugin<R> {
    let app_data_dir = get_app_data_dir(app_name);
    init_with_path(app_data_dir)
}

/// Returns the platform-specific app data directory for `VelesDB`.
///
/// # Arguments
///
/// * `app_name` - Your application's name
///
/// # Returns
///
/// Path to `<app_data>/<app_name>/velesdb/`
///
/// # Panics
///
/// Panics if the app data directory cannot be determined.
#[must_use]
pub fn get_app_data_dir(app_name: &str) -> std::path::PathBuf {
    let base_dir = dirs::data_dir()
        .or_else(dirs::config_dir)
        .expect("Could not determine app data directory for this platform");

    base_dir.join(app_name).join("velesdb")
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

    #[test]
    fn test_get_app_data_dir_structure() {
        // Act
        let path = get_app_data_dir("test-app");

        // Assert - path should end with test-app/velesdb
        assert!(path.ends_with("test-app/velesdb") || path.ends_with("test-app\\velesdb"));
        assert!(path.to_string_lossy().contains("test-app"));
    }

    #[test]
    fn test_get_app_data_dir_different_apps() {
        // Act
        let path1 = get_app_data_dir("app1");
        let path2 = get_app_data_dir("app2");

        // Assert - different apps should have different paths
        assert_ne!(path1, path2);
    }
}
