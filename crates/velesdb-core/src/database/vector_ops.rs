//! Vector collection creation and retrieval operations.

use crate::collection::VectorCollection;
use crate::index::hnsw::HnswParams;
use crate::{CollectionType, DistanceMetric, Result, StorageMode};

use super::Database;

impl Database {
    /// Resolves the HNSW parameters for a collection about to be created.
    ///
    /// Implements the `[hnsw]` half of the configuration precedence chain
    /// (issue #2087). From strongest to weakest:
    ///
    /// 1. **Per-collection creation argument** — the `m` / `ef_construction`
    ///    passed to the constructor, or a whole `HnswParams` handed to
    ///    [`Database::create_vector_collection_with_params`], which does not
    ///    consult the config at all.
    /// 2. **`VelesConfig`'s `[hnsw]` section** — the deployment-wide default.
    /// 3. **Built-in engine default** — `HnswParams::auto(dimension)`.
    ///
    /// Per-query `WITH (...)` overrides sit above all three but never reach
    /// here: they tune `ef_search` at query time, whereas everything resolved
    /// in this function is graph *topology*, fixed when the index is built.
    ///
    /// Because the resolved params are persisted in the collection's
    /// `config.json`, a collection keeps the topology it was created with:
    /// editing `[hnsw]` later changes new collections only. Applying it to an
    /// existing one is a full index rebuild, which is `auto_reindex`'s job,
    /// not a config reload's.
    ///
    /// Resolution is layered here, in the only component that owns a
    /// `VelesConfig`, so a direct `VectorCollection::create` caller is
    /// unaffected by any file on disk: it receives a `HnswParams` value that
    /// is already an answer.
    ///
    /// That is a claim about *decisions*, not about imports.
    /// [`HnswParams::from_config`](crate::index::hnsw::HnswParams::from_config)
    /// names `config::HnswConfig` in its signature, exactly as
    /// `RuntimeLimits::from_config` names `LimitsConfig` — one pure mapping
    /// function per table, sitting beside the type it produces. What neither
    /// module does is *read* configuration: nothing below this function
    /// consults a `VelesConfig`, and the precedence chain exists only here.
    ///
    /// `storage_mode` is left at whatever `HnswParams::auto` produced: every
    /// constructor downstream overwrites it with the collection's own storage
    /// mode argument.
    ///
    /// Returns `None` when **no** level chose anything — neither argument, and
    /// an untouched `[hnsw]` section. That is not the same as returning
    /// `HnswParams::auto(dimension)`: a collection persists this value, and
    /// `hnsw_params: None` on disk means "nobody ever chose", exactly as
    /// `pq_rescore_oversampling: None` does a field away. Materializing a
    /// snapshot of today's auto-tuned defaults into that slot would build the
    /// identical index and destroy the distinction a later migration reads.
    /// Callers that need a concrete value regardless say so at the call site
    /// with `unwrap_or_else(|| HnswParams::auto(dimension))`.
    pub(super) fn resolve_hnsw_params(
        &self,
        dimension: usize,
        m: Option<usize>,
        ef_construction: Option<usize>,
    ) -> Option<HnswParams> {
        if m.is_none()
            && ef_construction.is_none()
            && self.config.hnsw.m.is_none()
            && self.config.hnsw.ef_construction.is_none()
        {
            return None;
        }

        // Level 2 first, then level 1 on top of it, so a per-field argument
        // overrides only the field it names.
        let mut params = HnswParams::from_config(dimension, &self.config.hnsw);
        if let Some(m) = m {
            params.max_connections = m;
        }
        if let Some(ef) = ef_construction {
            params.ef_construction = ef;
        }
        Some(params)
    }

    /// Creates a new vector collection.
    ///
    /// # Errors
    ///
    /// Returns an error if a collection with the same name already exists.
    pub fn create_vector_collection(
        &self,
        name: &str,
        dimension: usize,
        metric: DistanceMetric,
    ) -> Result<()> {
        self.create_vector_collection_with_options(name, dimension, metric, StorageMode::default())
    }

    /// Creates a new vector collection with custom storage options.
    ///
    /// # Errors
    ///
    /// Returns an error if a collection with the same name already exists
    /// or if the dimension exceeds the configured `max_dimensions` limit.
    pub fn create_vector_collection_with_options(
        &self,
        name: &str,
        dimension: usize,
        metric: DistanceMetric,
        storage_mode: StorageMode,
    ) -> Result<()> {
        self.ensure_collection_name_available(name)?;
        self.enforce_vector_dimension_limit(dimension)?;
        let path = self.data_dir.join(name);
        // #2087: no per-collection HNSW argument here, so the `[hnsw]` section
        // is the strongest level that applies. An untouched section resolves
        // to `None` and takes the original constructor, so this path is
        // byte-for-byte unchanged for anyone who did not configure `[hnsw]` —
        // including the `hnsw_params: None` it persists.
        let coll = match self.resolve_hnsw_params(dimension, None, None) {
            Some(params) => VectorCollection::create_with_hnsw_params(
                path,
                dimension,
                metric,
                storage_mode,
                params,
            )?,
            None => VectorCollection::create(path, name, dimension, metric, storage_mode)?,
        };
        self.register_vector_collection(name, &coll, dimension, metric, storage_mode);
        Ok(())
    }

    /// Creates a new vector collection with custom HNSW parameters.
    ///
    /// When `m` or `ef_construction` are `Some`, those values win over the
    /// configured `[hnsw]` section, which in turn wins over the
    /// dimension-based auto-tuned defaults from [`HnswParams::auto`] — see
    /// `Database::resolve_hnsw_params` for the full chain. The two arguments
    /// are resolved independently, so pinning one still takes the other from
    /// config.
    ///
    /// Shortcut for [`Database::create_vector_collection_with_params`] that
    /// only overrides `max_connections` and `ef_construction`.
    ///
    /// # Errors
    ///
    /// Returns an error if a collection with the same name already exists.
    pub fn create_vector_collection_with_hnsw(
        &self,
        name: &str,
        dimension: usize,
        metric: DistanceMetric,
        storage_mode: StorageMode,
        m: Option<usize>,
        ef_construction: Option<usize>,
    ) -> Result<()> {
        self.ensure_collection_name_available(name)?;
        self.enforce_vector_dimension_limit(dimension)?;
        let path = self.data_dir.join(name);
        // #2087: each argument is resolved on its own — a caller that pins
        // only `m` still picks up `ef_construction` from `[hnsw]`.
        //
        // Materialized unconditionally, unlike the path above: this
        // constructor already persisted `Some(HnswParams::auto(dimension))`
        // before the wiring, so keeping `None` here would be the behaviour
        // change rather than avoiding one.
        let params = self
            .resolve_hnsw_params(dimension, m, ef_construction)
            .unwrap_or_else(|| HnswParams::auto(dimension));
        let coll = VectorCollection::create_with_params(
            path,
            dimension,
            metric,
            storage_mode,
            params,
            None,
        )?;
        self.register_vector_collection(name, &coll, dimension, metric, storage_mode);
        Ok(())
    }

    /// Creates a new vector collection with a fully specified
    /// [`HnswParams`] and an explicit `pq_rescore_oversampling` override.
    ///
    /// This is the most expressive vector constructor exposed by
    /// `Database`: callers pass every HNSW parameter — `max_connections`,
    /// `ef_construction`, `max_elements`, `alpha`, storage mode — via a
    /// single value, and override the PQ rescore factor explicitly rather
    /// than implicitly falling back to the engine default of `Some(4)`.
    /// Passing `pq_rescore_oversampling = None` keeps the persisted config
    /// in "no explicit override" mode so later migrations can recompute
    /// the factor from dataset shape.
    ///
    /// The storage mode argument wins over `hnsw_params.storage_mode` if
    /// they disagree — the field on `HnswParams` is a legacy denormalised
    /// copy that the engine keeps in sync with the collection-level value.
    ///
    /// The configured `[hnsw]` section is **not** consulted: `hnsw_params` is
    /// already a complete answer, and silently merging a deployment default
    /// into a fully specified value would make the result depend on a file the
    /// caller did not mention. Callers wanting the config as a base should
    /// build from [`HnswParams::from_config`] and adjust from there.
    ///
    /// # Errors
    ///
    /// Returns an error if a collection with the same name already exists
    /// or if the underlying directory cannot be created.
    pub fn create_vector_collection_with_params(
        &self,
        name: &str,
        dimension: usize,
        metric: DistanceMetric,
        storage_mode: StorageMode,
        hnsw_params: HnswParams,
        pq_rescore_oversampling: Option<u32>,
    ) -> Result<()> {
        self.ensure_collection_name_available(name)?;
        self.enforce_vector_dimension_limit(dimension)?;
        let path = self.data_dir.join(name);
        let coll = VectorCollection::create_with_params(
            path,
            dimension,
            metric,
            storage_mode,
            hnsw_params,
            pq_rescore_oversampling,
        )?;
        self.register_vector_collection(name, &coll, dimension, metric, storage_mode);
        Ok(())
    }

    /// Registers a vector collection in the typed registry,
    /// notifies the observer, and bumps the schema version.
    fn register_vector_collection(
        &self,
        name: &str,
        coll: &VectorCollection,
        dimension: usize,
        metric: DistanceMetric,
        storage_mode: StorageMode,
    ) {
        // Parity item E: thread the live LimitsConfig caps into the collection
        // before it is shared, so direct Collection::upsert / search paths
        // (used by every SDK/REST handler) enforce the configured limits.
        self.push_runtime_limits(&coll.inner);

        self.vector_colls
            .write()
            .insert(name.to_string(), coll.clone());

        if let Some(ref obs) = self.observer {
            let kind = CollectionType::Vector {
                dimension,
                metric,
                storage_mode,
            };
            obs.on_collection_created(name, &kind);
        }

        self.schema_version
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Returns a `VectorCollection` by name.
    ///
    /// Checks the typed registry first.  If not found there, falls back to
    /// opening the collection directory from disk (e.g. for collections created
    /// via the legacy `create_collection` API that were not registered in the
    /// typed registry).  The opened instance is cached back into the registry
    /// so subsequent calls avoid the disk round-trip.
    ///
    /// Returns `None` if the collection does not exist on disk.
    #[must_use]
    pub fn get_vector_collection(&self, name: &str) -> Option<VectorCollection> {
        // Bound before the `if let`: the guard would otherwise stay alive for
        // the whole expression, and the disk fallback below takes `vector_colls`
        // for WRITE — one refactor away from a self-deadlock on a non-reentrant
        // `parking_lot` lock.
        let cached = self.vector_colls.read().get(name).cloned();
        if let Some(c) = cached {
            return Some(c);
        }
        self.open_vector_collection_from_disk(name)
    }

    /// Disk fallback for `get_vector_collection`.
    fn open_vector_collection_from_disk(&self, name: &str) -> Option<VectorCollection> {
        let cfg = self.read_collection_config(name)?;
        if cfg.graph_schema.is_some() || cfg.metadata_only {
            return None;
        }
        let coll = VectorCollection::open(self.data_dir.join(name)).ok()?;
        // Parity item E: re-push runtime limits on disk-open (not persisted).
        self.push_runtime_limits(&coll.inner);
        self.vector_colls
            .write()
            .insert(name.to_string(), coll.clone());
        Some(coll)
    }
}
