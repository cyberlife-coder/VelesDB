//! Database facade and orchestration layer for collection lifecycle and query routing.
//!
//! This module is split into focused submodules:
//!
//! - [`collection_ops`] — Collection CRUD (create, delete, list, get)
//! - [`query_engine`] — `VelesQL` query execution, plan caching, DML
//! - [`persistence`] — Loading collections from disk at startup
//! - [`training`] — `TRAIN QUANTIZER` statement execution
//! - [`stats`] — Collection statistics (analyze, cache)
//! - [`database_helpers`] — DML value conversion and JOIN column store helpers

use crate::collection::{GraphCollection, MetadataCollection, VectorCollection};
use crate::observer::DatabaseObserver;
use crate::simd_dispatch;
#[allow(deprecated)]
use crate::{Collection, ColumnStore, Error, Result};

mod collection_ops;
mod persistence;
mod query_engine;
mod stats;
mod training;

#[cfg(feature = "persistence")]
mod database_helpers;

#[cfg(all(test, feature = "persistence"))]
mod collection_ops_tests;
#[cfg(all(test, feature = "persistence"))]
mod database_tests;
#[cfg(all(test, feature = "persistence"))]
mod query_engine_tests;

/// Database instance managing collections and storage.
///
/// # Lifecycle
///
/// `Database::open()` automatically loads all previously created collections from disk.
/// There is no need to call `load_collections()` separately.
///
/// # Extension (Premium)
///
/// Use [`Database::open_with_observer`] to inject a [`DatabaseObserver`] implementation
/// from `velesdb-premium` without modifying this crate.
#[cfg(feature = "persistence")]
pub struct Database {
    /// Path to the data directory
    data_dir: std::path::PathBuf,
    /// Exclusive file lock preventing multi-process corruption.
    ///
    /// The lock is held for the lifetime of the `Database` and released on `Drop`.
    /// The `_` prefix signals this field is kept for its RAII side effect.
    _lock_file: std::fs::File,
    /// Legacy registry (Collection god-object) — kept for backward compatibility during migration.
    #[allow(deprecated)]
    collections: parking_lot::RwLock<std::collections::HashMap<String, Collection>>,
    /// New registry: vector collections.
    vector_colls: parking_lot::RwLock<std::collections::HashMap<String, VectorCollection>>,
    /// New registry: graph collections.
    graph_colls: parking_lot::RwLock<std::collections::HashMap<String, GraphCollection>>,
    /// New registry: metadata-only collections.
    metadata_colls: parking_lot::RwLock<std::collections::HashMap<String, MetadataCollection>>,
    /// Cached collection statistics for CBO planning.
    collection_stats: parking_lot::RwLock<
        std::collections::HashMap<String, crate::collection::stats::CollectionStats>,
    >,
    /// Optional lifecycle observer (used by velesdb-premium for RBAC, audit, multi-tenant).
    observer: Option<std::sync::Arc<dyn DatabaseObserver>>,
    /// Monotonic DDL schema version counter (CACHE-01).
    ///
    /// Incremented on every create/drop collection operation.
    /// Used by `CompiledPlanCache` to invalidate cached query plans.
    schema_version: std::sync::atomic::AtomicU64,
    /// Compiled query plan cache (CACHE-02).
    ///
    /// Stores recently compiled `QueryPlan` instances keyed by `PlanKey`.
    /// Default sizing: L1 = 1K hot entries, L2 = 10K LRU entries.
    compiled_plan_cache: crate::cache::CompiledPlanCache,
}

#[cfg(feature = "persistence")]
impl Database {
    /// Opens or creates a database, **automatically loading all existing collections**.
    ///
    /// This replaces the previous `open()` + `load_collections()` two-step pattern.
    /// The new `open()` is a strict auto-load: all `config.json` directories under
    /// `path` are loaded on startup.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or accessed.
    pub fn open<P: AsRef<std::path::Path>>(path: P) -> Result<Self> {
        Self::open_impl(path, None)
    }

    /// Opens a database with a [`DatabaseObserver`] (used by velesdb-premium).
    ///
    /// The observer receives lifecycle hooks for every collection operation,
    /// enabling RBAC, audit logging, multi-tenant routing, etc.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or accessed.
    pub fn open_with_observer<P: AsRef<std::path::Path>>(
        path: P,
        observer: std::sync::Arc<dyn DatabaseObserver>,
    ) -> Result<Self> {
        Self::open_impl(path, Some(observer))
    }

    fn open_impl<P: AsRef<std::path::Path>>(
        path: P,
        observer: Option<std::sync::Arc<dyn DatabaseObserver>>,
    ) -> Result<Self> {
        let data_dir = path.as_ref().to_path_buf();
        std::fs::create_dir_all(&data_dir)?;

        // Acquire exclusive file lock to prevent multi-process corruption
        let lock_path = data_dir.join("velesdb.lock");
        let lock_file = std::fs::File::create(&lock_path)?;
        fs2::FileExt::try_lock_exclusive(&lock_file)
            .map_err(|_| Error::DatabaseLocked(data_dir.display().to_string()))?;

        // Log SIMD features detected at startup
        let features = simd_dispatch::simd_features_info();
        tracing::info!(
            avx512 = features.avx512f,
            avx2 = features.avx2,
            "SIMD features detected - direct dispatch enabled"
        );

        let db = Self {
            data_dir,
            _lock_file: lock_file,
            collections: parking_lot::RwLock::new(std::collections::HashMap::new()),
            vector_colls: parking_lot::RwLock::new(std::collections::HashMap::new()),
            graph_colls: parking_lot::RwLock::new(std::collections::HashMap::new()),
            metadata_colls: parking_lot::RwLock::new(std::collections::HashMap::new()),
            collection_stats: parking_lot::RwLock::new(std::collections::HashMap::new()),
            observer,
            schema_version: std::sync::atomic::AtomicU64::new(0),
            compiled_plan_cache: crate::cache::CompiledPlanCache::new(1_000, 10_000),
        };

        // Auto-load all existing collections from disk (replaces manual load_collections()).
        db.load_collections()?;

        Ok(db)
    }

    /// Returns the path to the data directory.
    #[must_use]
    pub fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }

    /// Returns the current DDL schema version counter.
    #[must_use]
    pub fn schema_version(&self) -> u64 {
        self.schema_version
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Returns a reference to the compiled query plan cache.
    #[must_use]
    pub fn plan_cache(&self) -> &crate::cache::CompiledPlanCache {
        &self.compiled_plan_cache
    }

    // =========================================================================
    // Observer notification helpers (called by server handlers after operations)
    // =========================================================================

    /// Notifies the observer that points were upserted into a collection.
    ///
    /// **Caller contract**: this method is NOT called automatically by
    /// [`Database`] internals. HTTP handlers and SDK bindings are responsible
    /// for calling it after a successful upsert, passing the number of points
    /// written. Forgetting to call it means the observer receives no upsert
    /// events for that operation.
    ///
    /// No-op when no observer is registered.
    pub fn notify_upsert(&self, collection: &str, point_count: usize) {
        if let Some(ref obs) = self.observer {
            obs.on_upsert(collection, point_count);
        }
    }

    /// Notifies the observer that a query was executed, with its duration.
    ///
    /// **Caller contract**: this method is NOT called automatically by
    /// [`Database::execute_query`]. Callers must measure the wall-clock
    /// duration themselves (e.g. `std::time::Instant::now()` before the call)
    /// and invoke this method afterwards with the elapsed microseconds.
    ///
    /// No-op when no observer is registered.
    pub fn notify_query(&self, collection: &str, duration_us: u64) {
        if let Some(ref obs) = self.observer {
            obs.on_query(collection, duration_us);
        }
    }
}
