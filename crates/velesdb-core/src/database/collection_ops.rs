//! Collection CRUD dispatcher: create, delete, list, get, and diagnostics.
//!
//! Type-specific operations are in sibling modules:
//! - [`vector_ops`](super::vector_ops) — vector collection create/get
//! - [`graph_ops`](super::graph_ops) — graph collection create/get
//! - [`metadata_ops`](super::metadata_ops) — metadata-only collection create/get

use crate::collection::AnyCollection;
use crate::{CollectionType, DistanceMetric, Error, Result, StorageMode};

use super::Database;

impl Database {
    /// Ensures a collection name is valid, free in memory, and free on disk.
    ///
    /// Validates the name against path traversal and forbidden characters
    /// **before** any filesystem operation, then checks that no collection
    /// with the same name already exists in any registry or on disk, and
    /// finally enforces the `LimitsConfig::max_collections` cap so that
    /// callers are refused cleanly instead of filling the registry past
    /// the configured ceiling.
    pub(super) fn ensure_collection_name_available(&self, name: &str) -> Result<()> {
        crate::validation::validate_collection_name(name)?;

        if self.collection_exists_in_registry(name) {
            return Err(Error::CollectionExists(name.to_string()));
        }

        let collection_path = self.data_dir.join(name);
        if collection_path.exists() {
            return Err(Error::CollectionExists(name.to_string()));
        }

        // Wave 3 Commit 7 — enforce `LimitsConfig::max_collections`.
        //
        // Counted across every typed registry (vector + graph + metadata)
        // because the limit is tenant-wide, not per-type. Evaluated after
        // the name validation and duplicate checks so the typed error
        // precedence stays unchanged: invalid name and duplicate still
        // win over the cap — callers that want to detect "too many
        // collections" specifically rely on the `GuardRail` variant.
        let total_collections = self.vector_colls.read().len()
            + self.graph_colls.read().len()
            + self.metadata_colls.read().len();
        let cap = self.config.limits.max_collections;
        if total_collections >= cap {
            return Err(Error::GuardRail {
                kind: crate::error::GuardRailKind::Capacity,
                message: format!(
                    "max_collections limit reached ({total_collections} / {cap}); \
                 raise `limits.max_collections` in VelesConfig to create more"
                ),
            });
        }

        Ok(())
    }

    /// Pushes the live [`LimitsConfig`](crate::config::LimitsConfig) ingest/
    /// search caps into a collection (parity item E).
    ///
    /// Single helper reused by the vector / graph / metadata registration and
    /// disk-open paths so all three thread the same runtime limits into the
    /// `Collection`. The limits are **not** persisted to `config.json`: they
    /// are re-pushed on every open from the live `VelesConfig`.
    fn push_runtime_limits(&self, coll: &crate::collection::Collection) {
        coll.set_runtime_limits(crate::collection::RuntimeLimits::from_config(
            &self.config.limits,
        ));
        // `[search]` alongside `[limits]`, same contract: resolved here because
        // `Database` is the only component holding a `VelesConfig`, pushed on
        // every open, never persisted (#2087).
        coll.set_runtime_search_quality(self.config.search.resolved_quality());
    }

    /// Admits a collection into a typed registry: pushes the live runtime
    /// limits and guard-rails into it, then runs `insert`.
    ///
    /// The single path every create, startup-load and disk-open registration
    /// goes through, so no collection reaches a registry without the limits
    /// of [`Database::update_guardrails`]. `insert` runs under the
    /// `guardrail_limits` read lock — see that field for why the lock spans
    /// the insert.
    pub(super) fn admit_collection(
        &self,
        coll: &crate::collection::Collection,
        insert: impl FnOnce(),
    ) {
        self.push_runtime_limits(coll);
        let limits = self.guardrail_limits.read();
        coll.guard_rails().update_limits(&limits);
        insert();
        drop(limits);
    }

    /// Checks whether a collection name exists in any of the typed registries.
    fn collection_exists_in_registry(&self, name: &str) -> bool {
        self.vector_colls.read().contains_key(name)
            || self.graph_colls.read().contains_key(name)
            || self.metadata_colls.read().contains_key(name)
    }

    /// Enforces `LimitsConfig::max_dimensions` on a prospective vector
    /// collection creation.
    ///
    /// Complements [`crate::validation::validate_dimension`] (the static
    /// `65_536` hard ceiling): the config-driven limit is typically tighter
    /// — 4096 by default — and is consulted here so the guard-rail can
    /// be relaxed per tenant via [`Database::open_with_config`] without
    /// touching the static constant.
    ///
    /// Dimension `0` is accepted because it is the sentinel used by
    /// metadata-only and graph-without-embeddings collections. Callers
    /// that need to reject zero should do so upstream via
    /// [`crate::validation::validate_dimension`].
    pub(super) fn enforce_vector_dimension_limit(&self, dimension: usize) -> Result<()> {
        if dimension == 0 {
            return Ok(());
        }
        let cap = self.config.limits.max_dimensions;
        if dimension > cap {
            return Err(Error::GuardRail {
                kind: crate::error::GuardRailKind::Capacity,
                message: format!(
                    "vector dimension {dimension} exceeds configured max_dimensions cap of {cap}; \
                 raise `limits.max_dimensions` in VelesConfig to allow larger vectors"
                ),
            });
        }
        Ok(())
    }

    /// Creates a new collection with the specified parameters.
    ///
    /// # Arguments
    ///
    /// * `name` - Unique name for the collection
    /// * `dimension` - Vector dimension (e.g., 768 for many embedding models)
    /// * `metric` - Distance metric to use for similarity calculations
    ///
    /// # Errors
    ///
    /// - Returns `Error::CollectionExists` if a collection with the same name already exists.
    /// - Returns an error if the directory cannot be created or storage initialization fails.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use velesdb_core::{Database, DistanceMetric};
    /// let db = Database::open("./data")?;
    /// db.create_collection("documents", 768, DistanceMetric::Cosine)?;
    /// # Ok::<(), velesdb_core::Error>(())
    /// ```
    pub fn create_collection(
        &self,
        name: &str,
        dimension: usize,
        metric: DistanceMetric,
    ) -> Result<()> {
        self.create_collection_with_options(name, dimension, metric, StorageMode::default())
    }

    /// Creates a new collection with custom storage options.
    ///
    /// # Errors
    ///
    /// Returns an error if a collection with the same name already exists.
    pub fn create_collection_with_options(
        &self,
        name: &str,
        dimension: usize,
        metric: DistanceMetric,
        storage_mode: StorageMode,
    ) -> Result<()> {
        self.create_vector_collection_with_options(name, dimension, metric, storage_mode)
    }

    /// Returns a type-erased collection handle by name.
    ///
    /// Checks vector → graph → metadata registries in order.
    /// Returns `None` if no collection with the given name exists.
    #[must_use]
    pub fn get_any_collection(&self, name: &str) -> Option<AnyCollection> {
        if let Some(c) = self.get_vector_collection(name) {
            return Some(AnyCollection::Vector(c));
        }
        if let Some(c) = self.get_graph_collection(name) {
            return Some(AnyCollection::Graph(c));
        }
        if let Some(c) = self.get_metadata_collection(name) {
            return Some(AnyCollection::Metadata(c));
        }
        None
    }

    /// Returns the write generation for a named collection, if it exists.
    #[must_use]
    pub fn collection_write_generation(&self, name: &str) -> Option<u64> {
        if let Some(vc) = self.vector_colls.read().get(name) {
            return Some(vc.inner.write_generation());
        }
        if let Some(gc) = self.graph_colls.read().get(name) {
            return Some(gc.inner.write_generation());
        }
        if let Some(mc) = self.metadata_colls.read().get(name) {
            return Some(mc.inner.write_generation());
        }
        None
    }

    /// Returns the set of payload field names covered by a secondary index
    /// for the named collection (issue #607). Empty set when the collection
    /// has no indexes or does not exist.
    ///
    /// Used by `Database::build_plan_with_stats` to thread the real
    /// indexed-field set into `QueryPlan::from_query_with_stats` so that
    /// `IndexLookup` plan nodes are generated in the EXPLAIN tree when a
    /// WHERE clause targets an indexed column.
    #[must_use]
    pub fn indexed_fields_for(&self, name: &str) -> std::collections::HashSet<String> {
        if let Some(vc) = self.vector_colls.read().get(name) {
            return vc.inner.indexed_field_names();
        }
        if let Some(gc) = self.graph_colls.read().get(name) {
            return gc.inner.indexed_field_names();
        }
        if let Some(mc) = self.metadata_colls.read().get(name) {
            return mc.inner.indexed_field_names();
        }
        std::collections::HashSet::new()
    }

    /// Returns the live graph `CollectionStats` (node/edge counts, average
    /// degree, label selectivity) for the named collection, used by the
    /// `MatchQueryPlanner` to choose a traversal strategy in EXPLAIN of a
    /// MATCH query (backlog #14). `None` when the collection does not exist.
    #[must_use]
    pub(crate) fn match_stats_for(
        &self,
        name: &str,
    ) -> Option<crate::velesql::match_planner::MatchGraphStats> {
        if let Some(vc) = self.vector_colls.read().get(name) {
            return Some(vc.inner.compute_match_collection_stats());
        }
        if let Some(gc) = self.graph_colls.read().get(name) {
            return Some(gc.inner.compute_match_collection_stats());
        }
        if let Some(mc) = self.metadata_colls.read().get(name) {
            return Some(mc.inner.compute_match_collection_stats());
        }
        None
    }

    /// Returns the analyze generation for a named collection, if it exists
    /// (issue #608).
    ///
    /// Parallel to [`Self::collection_write_generation`], but tracks `ANALYZE`
    /// invocations instead of data mutations. Threaded into the compiled plan
    /// cache key so that an `ANALYZE` run alone invalidates cached plans whose
    /// cost estimates pre-date the fresh calibrated statistics.
    #[must_use]
    pub fn collection_analyze_generation(&self, name: &str) -> Option<u64> {
        if let Some(vc) = self.vector_colls.read().get(name) {
            return Some(vc.inner.analyze_generation());
        }
        if let Some(gc) = self.graph_colls.read().get(name) {
            return Some(gc.inner.analyze_generation());
        }
        if let Some(mc) = self.metadata_colls.read().get(name) {
            return Some(mc.inner.analyze_generation());
        }
        None
    }

    /// Renders every graph collection's metrics as a Prometheus exposition.
    ///
    /// One `GraphMetrics` exists per edge store, so the samples are tagged
    /// with the collection they came from and each family is declared once;
    /// see `collection::graph::graph_metrics_to_prometheus`. Returns an empty
    /// string when the database holds no graph collection.
    #[must_use]
    pub fn graph_metrics_prometheus(&self) -> String {
        let graph_colls = self.graph_colls.read();
        let mut sorted: Vec<_> = graph_colls
            .iter()
            .map(|(name, coll)| (name.as_str(), coll.metrics()))
            .collect();
        // Stable output: a scrape diff should reflect metric movement, not
        // HashMap iteration order.
        sorted.sort_by_key(|(name, _)| *name);
        let rendered = crate::collection::graph::graph_metrics_to_prometheus(&sorted);
        // `sorted` borrows names out of the guard, so it goes first.
        drop(sorted);
        drop(graph_colls);
        rendered
    }

    /// Sets the two storage gauges of `metrics` from the live registries, so
    /// a scrape reads the database as it is (EPIC-050):
    ///
    /// - `velesdb_documents_total` — points across every vector, graph and
    ///   metadata collection;
    /// - `velesdb_index_size_bytes` — the summed `indexes_memory_usage()` of
    ///   every collection (see
    ///   [`VectorCollection::indexes_memory_usage`](crate::VectorCollection::indexes_memory_usage)):
    ///   the estimated heap of its secondary, property and range indexes. The
    ///   HNSW graph and the BM25 index carry no size accounting and are not
    ///   counted.
    ///
    /// Gauges rather than counters, recomputed per call: `/metrics` calls
    /// this before every export.
    ///
    /// The registries are read only to clone the collection handles: the
    /// walk of every index runs with no registry lock held, so a scrape that
    /// waits on an index being backfilled cannot hold a registry reader that
    /// a collection create or delete — and every request queued behind that
    /// writer — would then wait on.
    pub fn refresh_storage_gauges(&self, metrics: &crate::metrics::OperationalMetrics) {
        // One registry guard at a time, each dropped at the end of its
        // statement.
        let mut collections: Vec<crate::collection::Collection> = self
            .vector_colls
            .read()
            .values()
            .map(|c| c.inner.clone())
            .collect();
        collections.extend(self.graph_colls.read().values().map(|c| c.inner.clone()));
        collections.extend(self.metadata_colls.read().values().map(|c| c.inner.clone()));
        let mut points = 0usize;
        let mut index_bytes = 0usize;
        for coll in &collections {
            points += coll.len();
            index_bytes += coll.indexes_memory_usage();
        }
        metrics.set_documents(u64::try_from(points).unwrap_or(u64::MAX));
        metrics.set_index_size(u64::try_from(index_bytes).unwrap_or(u64::MAX));
    }

    /// Renders the process-wide MATCH query metrics as a Prometheus
    /// exposition (EPIC-050 US-002).
    ///
    /// Unlike graph metrics, MATCH metrics are not tagged per collection:
    /// `MATCH` queries currently record into one global collector regardless
    /// of which collection they touch (`match_metrics::global_match_metrics`).
    #[must_use]
    pub fn match_metrics_prometheus(&self) -> String {
        crate::collection::search::query::match_metrics::global_match_metrics().to_prometheus()
    }

    /// Lists all collection names in the database.
    ///
    /// Includes collections created via any typed API (vector, graph, metadata).
    pub fn list_collections(&self) -> Vec<String> {
        let vector_colls = self.vector_colls.read();
        let graph_colls = self.graph_colls.read();
        let metadata_colls = self.metadata_colls.read();

        let mut names: std::collections::HashSet<String> = vector_colls.keys().cloned().collect();
        for k in graph_colls.keys() {
            names.insert(k.clone());
        }
        for k in metadata_colls.keys() {
            names.insert(k.clone());
        }
        // Every key is cloned, so the three maps are read as one consistent
        // snapshot and then released together before the sort below.
        drop(metadata_colls);
        drop(graph_colls);
        drop(vector_colls);
        let mut result: Vec<String> = names.into_iter().collect();
        result.sort();
        result
    }

    /// Deletes a collection by name.
    ///
    /// # Errors
    ///
    /// Returns an error if the name is invalid or the collection does not
    /// exist in any registry.
    pub fn delete_collection(&self, name: &str) -> Result<()> {
        crate::validation::validate_collection_name(name)?;

        if !self.collection_exists_in_registry(name) {
            return Err(Error::CollectionNotFound(name.to_string()));
        }

        let collection_path = self.data_dir.join(name);
        if collection_path.exists() {
            std::fs::remove_dir_all(&collection_path)?;
        }

        self.remove_from_all_registries(name);

        if let Some(ref obs) = self.observer {
            obs.on_collection_deleted(name);
        }

        self.schema_version
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        Ok(())
    }

    /// Removes a collection from all registries and stats cache.
    fn remove_from_all_registries(&self, name: &str) {
        self.vector_colls.write().remove(name);
        self.graph_colls.write().remove(name);
        self.metadata_colls.write().remove(name);
        self.collection_stats.write().remove(name);
        self.join_store_cache.write().remove(name);
    }

    /// Creates a new collection with a specific type (Vector, Graph, or `MetadataOnly`).
    ///
    /// # Errors
    ///
    /// Returns an error if a collection with the same name already exists.
    pub fn create_collection_typed(
        &self,
        name: &str,
        collection_type: &CollectionType,
    ) -> Result<()> {
        match collection_type {
            CollectionType::Vector {
                dimension,
                metric,
                storage_mode,
            } => {
                self.create_vector_collection_with_options(name, *dimension, *metric, *storage_mode)
            }
            CollectionType::MetadataOnly => self.create_metadata_collection(name),
            CollectionType::Graph {
                dimension,
                metric,
                schema,
            } => self.create_graph_collection_from_type(name, *dimension, *metric, schema),
        }
    }

    /// Reads and parses `config.json` from a collection directory.
    ///
    /// Returns `None` if the name is invalid, the config file does not exist,
    /// or the config cannot be parsed.
    pub(super) fn read_collection_config(
        &self,
        name: &str,
    ) -> Option<crate::collection::CollectionConfig> {
        if crate::validation::validate_collection_name(name).is_err() {
            return None;
        }
        let path = self.data_dir.join(name);
        let config_path = path.join("config.json");
        if !config_path.exists() {
            return None;
        }
        let data = std::fs::read_to_string(&config_path).ok()?;
        serde_json::from_str(&data).ok()
    }

    /// Returns the live query guard-rails: the last value
    /// [`Self::update_guardrails`] applied, or `QueryLimits::default()`.
    #[must_use]
    pub fn guardrail_limits(&self) -> crate::guardrails::QueryLimits {
        self.guardrail_limits.read().clone()
    }

    /// Replaces the query guard-rails of the whole database (full replacement).
    ///
    /// Every field reaches every collection — the registered ones now, and
    /// every one created or opened afterwards, which starts from this value:
    /// the per-query snapshot (`max_depth`, `max_cardinality`,
    /// `memory_limit_bytes`, `timeout_ms`), the rate limiter
    /// (`rate_limit_qps`) and the circuit breaker
    /// (`circuit_failure_threshold`, `circuit_recovery_seconds`). Token
    /// buckets and breaker state survive the update (see
    /// [`GuardRails::update_limits`](crate::guardrails::GuardRails::update_limits)).
    /// Not persisted: a reopened database starts from the defaults.
    ///
    /// Applies `limits` unchecked, as it always has. Prefer
    /// [`Self::update_guardrails_with`], which refuses limits that fail
    /// [`QueryLimits::validate`](crate::guardrails::QueryLimits::validate) —
    /// `PUT /guardrails` and the Python, Mobile and Tauri bindings go through
    /// it.
    #[expect(clippy::significant_drop_tightening)] // Reason: the write guard must span the propagation, or a collection registered in between would start from the old limits
    pub fn update_guardrails(&self, limits: &crate::guardrails::QueryLimits) {
        let mut live = self.guardrail_limits.write();
        live.clone_from(limits);
        self.propagate_guardrails(&live);
    }

    /// Edits the live query guard-rails and propagates the result as one
    /// atomic step; returns the limits now in force.
    ///
    /// The partial-update form of [`Self::update_guardrails`] (`PUT
    /// /guardrails` changes only the fields its body names): two concurrent
    /// edits of different fields both land, where a read-then-replace pair
    /// would lose one. `edit` runs on a copy, which must pass
    /// [`QueryLimits::validate`](crate::guardrails::QueryLimits::validate)
    /// before anything changes. It runs under the guard-rail lock, so it must
    /// not call back into this database (`parking_lot` locks are not
    /// reentrant).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] if the edited limits fail validation; the
    /// live limits are then unchanged.
    pub fn update_guardrails_with(
        &self,
        edit: impl FnOnce(&mut crate::guardrails::QueryLimits),
    ) -> Result<crate::guardrails::QueryLimits> {
        let mut live = self.guardrail_limits.write();
        let mut next = live.clone();
        edit(&mut next);
        next.validate()?;
        *live = next;
        self.propagate_guardrails(&live);
        Ok(live.clone())
    }

    /// Pushes `limits` to every registered collection. Called with the
    /// guard-rail write lock held, so a collection registered concurrently
    /// (`admit_collection`, which reads that lock) either is visited here or
    /// starts from `limits`.
    fn propagate_guardrails(&self, limits: &crate::guardrails::QueryLimits) {
        for vc in self.vector_colls.read().values() {
            vc.guard_rails().update_limits(limits);
        }
        for gc in self.graph_colls.read().values() {
            gc.inner.guard_rails().update_limits(limits);
        }
        for mc in self.metadata_colls.read().values() {
            mc.inner.guard_rails().update_limits(limits);
        }
    }

    /// Returns diagnostics for a named collection.
    ///
    /// # Errors
    ///
    /// Returns `Error::CollectionNotFound` if the collection does not exist.
    pub fn collection_diagnostics(
        &self,
        name: &str,
    ) -> Result<crate::collection::CollectionDiagnostics> {
        if let Some(c) = self.get_vector_collection(name) {
            return Ok(c.diagnostics());
        }
        if let Some(c) = self.get_graph_collection(name) {
            return Ok(c.diagnostics());
        }
        if let Some(c) = self.get_metadata_collection(name) {
            return Ok(c.diagnostics());
        }
        Err(Error::CollectionNotFound(name.to_string()))
    }
}
