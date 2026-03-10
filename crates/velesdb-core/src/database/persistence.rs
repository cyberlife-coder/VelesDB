//! Collection loading from disk at database startup.

use crate::collection::{GraphCollection, MetadataCollection, VectorCollection};
use crate::Result;

use super::Database;

impl Database {
    /// Loads existing collections from disk.
    ///
    /// # Deprecation note
    ///
    /// **This method is called automatically by [`Database::open`].**
    /// There is no need to call it manually. It is kept public only for
    /// backward compatibility with code that relied on the old two-step pattern.
    ///
    /// # Errors
    ///
    /// Returns an error if collection directories cannot be read.
    pub fn load_collections(&self) -> Result<()> {
        let mut loaded_count: usize = 0;

        for entry in std::fs::read_dir(&self.data_dir)? {
            let entry = entry?;
            let path = entry.path();

            if !path.is_dir() {
                continue;
            }
            let config_path = path.join("config.json");
            if !config_path.exists() {
                continue;
            }

            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            // Skip names already present in the legacy registry.
            if self.collections.read().contains_key(&name) {
                continue;
            }

            if self.try_load_single_collection(&path, &name) {
                loaded_count += 1;
            }
        }

        // Bump schema_version if at least one collection was loaded from disk (C-3).
        //
        // This ensures that any plan key built before load_collections() ran
        // (schema_version = 0) will never match a key built after it
        // (schema_version >= 1), preventing the plan cache from serving a stale
        // plan for a collection that was not yet visible in the registry.
        if loaded_count > 0 {
            self.schema_version
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        Ok(())
    }

    /// Attempts to load a single collection directory, returning `true` on success.
    fn try_load_single_collection(&self, path: &std::path::Path, name: &str) -> bool {
        let config_path = path.join("config.json");

        // Read config to determine the concrete type before opening.
        let cfg_data = match std::fs::read_to_string(&config_path) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(error = %e, name, "Cannot read config.json — skipping");
                return false;
            }
        };
        let cfg = match serde_json::from_str::<crate::collection::CollectionConfig>(&cfg_data) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, name, "Cannot parse config.json — skipping");
                return false;
            }
        };

        if cfg.graph_schema.is_some() {
            self.load_graph_collection(path, name)
        } else if cfg.metadata_only {
            self.load_metadata_collection(path, name)
        } else {
            self.load_vector_collection(path, name)
        }
    }

    /// Loads a graph collection from disk, registering it in both registries.
    fn load_graph_collection(&self, path: &std::path::Path, name: &str) -> bool {
        match GraphCollection::open(path.to_path_buf()) {
            Ok(coll) => {
                self.collections
                    .write()
                    .insert(name.to_string(), coll.inner.clone());
                self.graph_colls.write().insert(name.to_string(), coll);
                true
            }
            Err(e) => {
                tracing::warn!(error = %e, name = %path.display(), "Failed to load graph collection");
                false
            }
        }
    }

    /// Loads a metadata collection from disk, registering it in both registries.
    fn load_metadata_collection(&self, path: &std::path::Path, name: &str) -> bool {
        match MetadataCollection::open(path.to_path_buf()) {
            Ok(coll) => {
                self.collections
                    .write()
                    .insert(name.to_string(), coll.inner.clone());
                self.metadata_colls.write().insert(name.to_string(), coll);
                true
            }
            Err(e) => {
                tracing::warn!(error = %e, name = %path.display(), "Failed to load metadata collection");
                false
            }
        }
    }

    /// Loads a vector collection from disk, registering it in both registries.
    fn load_vector_collection(&self, path: &std::path::Path, name: &str) -> bool {
        match VectorCollection::open(path.to_path_buf()) {
            Ok(coll) => {
                self.collections
                    .write()
                    .insert(name.to_string(), coll.inner.clone());
                self.vector_colls.write().insert(name.to_string(), coll);
                true
            }
            Err(e) => {
                tracing::warn!(error = %e, name = %path.display(), "Failed to load vector collection");
                false
            }
        }
    }
}
