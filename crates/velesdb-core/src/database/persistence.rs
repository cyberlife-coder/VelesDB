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
            if let Some(name) = self.loadable_collection_name(&entry) {
                if self.try_load_single_collection(&entry.path(), &name) {
                    loaded_count += 1;
                }
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

    /// Returns the collection name if the directory entry is a loadable collection.
    ///
    /// A directory is loadable when it contains `config.json` and is not
    /// already registered in the legacy collections map.
    fn loadable_collection_name(&self, entry: &std::fs::DirEntry) -> Option<String> {
        let path = entry.path();
        if !path.is_dir() {
            return None;
        }
        if !path.join("config.json").exists() {
            return None;
        }
        let name = path.file_name()?.to_str().unwrap_or("unknown").to_string();
        if self.collections.read().contains_key(&name) {
            return None;
        }
        Some(name)
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
        self.try_open_and_register(path, name, "graph", |p| {
            GraphCollection::open(p).map(|c| (c.inner.clone(), TypedColl::Graph(c)))
        })
    }

    /// Loads a metadata collection from disk, registering it in both registries.
    fn load_metadata_collection(&self, path: &std::path::Path, name: &str) -> bool {
        self.try_open_and_register(path, name, "metadata", |p| {
            MetadataCollection::open(p).map(|c| (c.inner.clone(), TypedColl::Metadata(c)))
        })
    }

    /// Loads a vector collection from disk, registering it in both registries.
    fn load_vector_collection(&self, path: &std::path::Path, name: &str) -> bool {
        self.try_open_and_register(path, name, "vector", |p| {
            VectorCollection::open(p).map(|c| (c.inner.clone(), TypedColl::Vector(c)))
        })
    }

    /// Opens a collection from disk and registers it in the legacy + typed registries.
    ///
    /// The `open_fn` closure returns `(inner Collection clone, TypedColl variant)`.
    /// Returns `true` on success, `false` on failure (logged as warning).
    #[allow(deprecated)]
    fn try_open_and_register(
        &self,
        path: &std::path::Path,
        name: &str,
        kind: &str,
        open_fn: impl FnOnce(std::path::PathBuf) -> crate::Result<(crate::Collection, TypedColl)>,
    ) -> bool {
        match open_fn(path.to_path_buf()) {
            Ok((inner, typed)) => {
                self.collections.write().insert(name.to_string(), inner);
                typed.insert_into(
                    &self.vector_colls,
                    &self.graph_colls,
                    &self.metadata_colls,
                    name,
                );
                true
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    name = %path.display(),
                    "Failed to load {kind} collection"
                );
                false
            }
        }
    }

    /// Flushes all WALs across the typed collection registries.
    ///
    /// Best-effort: logs warnings for individual flush failures but continues
    /// flushing remaining collections. Returns the count of failures.
    ///
    /// The legacy `collections` registry is **not** iterated because it shares
    /// the same `Arc`'d inner storage as the typed registries. Flushing both
    /// would double-flush every collection, causing redundant I/O and
    /// potentially double-counting failures.
    pub fn flush_all(&self) -> usize {
        let mut failures: usize = 0;

        failures += flush_registry(&self.vector_colls, "vector");
        failures += flush_registry(&self.graph_colls, "graph");
        failures += flush_registry(&self.metadata_colls, "metadata");

        failures
    }
}

/// Discriminated union for the three typed collection registries.
///
/// Used by [`Database::try_open_and_register`] to route a freshly opened
/// collection into the correct registry without duplicating match arms.
enum TypedColl {
    Vector(VectorCollection),
    Graph(GraphCollection),
    Metadata(MetadataCollection),
}

impl TypedColl {
    fn insert_into(
        self,
        vectors: &parking_lot::RwLock<std::collections::HashMap<String, VectorCollection>>,
        graphs: &parking_lot::RwLock<std::collections::HashMap<String, GraphCollection>>,
        metadata: &parking_lot::RwLock<std::collections::HashMap<String, MetadataCollection>>,
        name: &str,
    ) {
        match self {
            Self::Vector(c) => {
                vectors.write().insert(name.to_string(), c);
            }
            Self::Graph(c) => {
                graphs.write().insert(name.to_string(), c);
            }
            Self::Metadata(c) => {
                metadata.write().insert(name.to_string(), c);
            }
        }
    }
}

/// Flushes all collections in a registry, logging failures. Returns failure count.
fn flush_registry<T: Flushable>(
    registry: &parking_lot::RwLock<std::collections::HashMap<String, T>>,
    kind: &str,
) -> usize {
    let mut failures = 0;
    for (name, coll) in registry.read().iter() {
        if let Err(e) = coll.flush() {
            tracing::warn!(
                error = %e,
                collection = %name,
                "Failed to flush {kind} collection"
            );
            failures += 1;
        }
    }
    failures
}

/// Internal trait for deduplicating `flush_all` iteration across collection types.
trait Flushable {
    fn flush(&self) -> crate::Result<()>;
}

impl Flushable for VectorCollection {
    fn flush(&self) -> crate::Result<()> {
        self.flush()
    }
}

impl Flushable for GraphCollection {
    fn flush(&self) -> crate::Result<()> {
        self.flush()
    }
}

impl Flushable for MetadataCollection {
    fn flush(&self) -> crate::Result<()> {
        self.flush()
    }
}
