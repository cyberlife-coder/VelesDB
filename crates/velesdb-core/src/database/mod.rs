//! Database facade and orchestration layer for collection lifecycle and query routing.
//!
//! This module is split into focused submodules:
//!
//! - [`collection_ops`] — Collection CRUD dispatcher (create, delete, list, get)
//! - [`vector_ops`] — Vector collection create/get
//! - [`graph_ops`] — Graph collection create/get
//! - [`metadata_ops`] — Metadata-only collection create/get
//! - [`query_engine`] — `VelesQL` query execution, plan caching, DML dispatch
//! - [`query_join`] — JOIN execution strategies (lookup, filtered, condition pushdown)
//! - [`dml_executor`] — DML mutations (INSERT EDGE, DELETE, DELETE EDGE, SELECT EDGES, INSERT NODE)
//! - [`persistence`] — Loading collections from disk at startup
//! - [`training`] — `TRAIN QUANTIZER` statement execution
//! - [`stats`] — Collection statistics (analyze, cache)
//! - [`database_helpers`] — DML value conversion and JOIN column store helpers

use crate::collection::{GraphCollection, MetadataCollection, VectorCollection};
use crate::observer::DatabaseObserver;
use crate::simd_dispatch;
use crate::{ColumnStore, Error, Result};

mod admin_executor;
mod collection_ops;
mod cross_collection;
mod ddl_executor;
mod dml_executor;
mod graph_ops;
mod introspection_executor;
mod join_pushdown;
mod metadata_ops;
mod persistence;
mod query_engine;
mod query_engine_dml;
mod query_join;
mod stats;
mod training;
mod vector_ops;

#[cfg(feature = "persistence")]
mod database_helpers;

#[cfg(all(test, feature = "persistence"))]
mod collection_ops_tests;
#[cfg(all(test, feature = "persistence"))]
mod database_helpers_tests;
#[cfg(all(test, feature = "persistence"))]
mod database_tests;
#[cfg(all(test, feature = "persistence"))]
mod ddl_executor_tests;
#[cfg(all(test, feature = "persistence"))]
mod graph_ops_tests;
#[cfg(all(test, feature = "persistence"))]
mod query_engine_tests;
#[cfg(all(test, feature = "persistence"))]
mod stats_tests;

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
    /// Root configuration applied to every subsystem.
    ///
    /// Stored as an `Arc` so `Database::config()` can hand out cheap,
    /// cloneable references without forcing the whole struct onto the
    /// heap or locking. The value is populated at construction time
    /// (`open`, `open_with_observer`, or `open_with_config`) and is
    /// immutable for the life of the `Database` — Wave 3 never needs
    /// to mutate the root config at runtime, and making it immutable
    /// rules out a large class of surprising behaviours.
    config: std::sync::Arc<crate::config::VelesConfig>,
    /// Typed registry: vector collections.
    vector_colls: parking_lot::RwLock<std::collections::HashMap<String, VectorCollection>>,
    /// Typed registry: graph collections.
    graph_colls: parking_lot::RwLock<std::collections::HashMap<String, GraphCollection>>,
    /// Typed registry: metadata-only collections.
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
    /// Uses the default [`VelesConfig`](crate::config::VelesConfig) — every
    /// subsystem behaves identically to the pre-Wave-3 version of this
    /// function, so existing callers keep their exact behaviour. Users
    /// that need to customise subsystem limits or WAL batching should
    /// call [`Database::open_with_config`] instead.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or accessed.
    pub fn open<P: AsRef<std::path::Path>>(path: P) -> Result<Self> {
        Self::open_impl(path, None, None)
    }

    /// Opens a database with an explicit [`VelesConfig`](crate::config::VelesConfig).
    ///
    /// Every subsystem that honours a config field (HNSW defaults, WAL
    /// batching, runtime limits, search quality) reads from the passed
    /// instance. A clone is stored inside the `Database` and retained
    /// for the lifetime of the handle so sub-systems can consult it
    /// without re-parsing a TOML file.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created, the lock
    /// cannot be acquired, or any already-present collection exceeds
    /// the limits declared in `config.limits` (see
    /// [`Database::open`] for the default-limit behaviour).
    pub fn open_with_config<P: AsRef<std::path::Path>>(
        path: P,
        config: crate::config::VelesConfig,
    ) -> Result<Self> {
        Self::open_impl(path, None, Some(config))
    }

    /// Opens a database with a [`DatabaseObserver`] (used by velesdb-premium).
    ///
    /// The observer receives lifecycle hooks for every collection operation,
    /// enabling RBAC, audit logging, multi-tenant routing, etc.
    ///
    /// Equivalent to [`Database::open`] plus the observer injection —
    /// applies the default [`VelesConfig`](crate::config::VelesConfig).
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or accessed.
    pub fn open_with_observer<P: AsRef<std::path::Path>>(
        path: P,
        observer: std::sync::Arc<dyn DatabaseObserver>,
    ) -> Result<Self> {
        Self::open_impl(path, Some(observer), None)
    }

    /// Opens a database with both an explicit [`VelesConfig`] and a
    /// [`DatabaseObserver`]. Used by the premium shell that layers
    /// RBAC/audit on top of a tenant-specific config file.
    ///
    /// # Errors
    ///
    /// Same as [`Database::open_with_config`].
    pub fn open_with_observer_and_config<P: AsRef<std::path::Path>>(
        path: P,
        observer: std::sync::Arc<dyn DatabaseObserver>,
        config: crate::config::VelesConfig,
    ) -> Result<Self> {
        Self::open_impl(path, Some(observer), Some(config))
    }

    fn open_impl<P: AsRef<std::path::Path>>(
        path: P,
        observer: Option<std::sync::Arc<dyn DatabaseObserver>>,
        config: Option<crate::config::VelesConfig>,
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
            config: std::sync::Arc::new(config.unwrap_or_default()),
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

    /// Returns a reference to the root [`VelesConfig`](crate::config::VelesConfig)
    /// that was supplied at construction (or the default if the database
    /// was opened via [`Database::open`]).
    ///
    /// Sub-systems (`vector_ops`, `query_engine`, `stats`, …) consult this
    /// through `database.config()` when they need to honour a user-supplied
    /// limit or toggle — the shared `Arc` makes the call free of locks
    /// and cheap to propagate to background threads.
    #[must_use]
    pub fn config(&self) -> &crate::config::VelesConfig {
        &self.config
    }

    /// Returns a cheap, cloneable handle to the root config.
    ///
    /// Use this when you need to move the config into a thread or
    /// long-lived closure that outlives the current `&self` borrow.
    #[must_use]
    pub fn config_arc(&self) -> std::sync::Arc<crate::config::VelesConfig> {
        std::sync::Arc::clone(&self.config)
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
