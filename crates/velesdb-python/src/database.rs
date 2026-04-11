//! `Database` — PyO3-exported database entry point for Python bindings.

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;

use crate::agent;
use crate::collection::Collection;
use crate::collection_helpers::core_err;
use crate::graph_collection::{PyGraphCollection, PyGraphSchema};
use crate::options::{AutoReindexOptions, HnswOptions, VelesConfigOptions};
use crate::utils::{self, parse_metric, parse_storage_mode};

use velesdb_core::collection::auto_reindex::AutoReindexManager;
use velesdb_core::{CollectionType, Database as CoreDatabase, GraphSchema};

/// VelesDB Database - the main entry point for interacting with VelesDB.
///
/// Example:
///     >>> db = velesdb.Database("./my_data")
///     >>> collections = db.list_collections()
#[pyclass]
pub struct Database {
    inner: Arc<CoreDatabase>,
    path: PathBuf,
}

/// Internal dispatch plan computed under the GIL by
/// [`Database::create_collection`] and consumed inside a
/// `py.allow_threads` closure.
///
/// Wave 3 Commit 10 introduces the typed-options surface: `Full`
/// carries fully-materialized [`HnswParams`] plus an explicit
/// `pq_rescore_oversampling` override so the closure can call
/// [`CoreDatabase::create_vector_collection_with_params`] in one step.
/// `Default` defers every parameter to the engine.
#[derive(Clone)]
enum CreatePlan {
    /// Use `create_vector_collection_with_params` with the given
    /// fully-materialized HNSW params and PQ rescore factor.
    Full {
        hnsw_params: velesdb_core::index::hnsw::HnswParams,
        pq_rescore_oversampling: Option<u32>,
    },
    /// Use `create_vector_collection_with_options` — engine picks
    /// every HNSW field.
    Default,
}

#[pymethods]
impl Database {
    /// Create or open a VelesDB database at the specified path.
    ///
    /// Args:
    ///     path: Directory path for database storage.
    ///     config: Optional typed configuration (limits, etc.) applied
    ///         at open time. See :class:`VelesConfigOptions`.
    ///
    /// Returns:
    ///     Database instance
    ///
    /// Example:
    ///     >>> db = velesdb.Database("./my_vectors")
    ///     >>> # With explicit limits:
    ///     >>> from velesdb import VelesConfigOptions, LimitsOptions
    ///     >>> cfg = VelesConfigOptions(limits=LimitsOptions(max_collections=50))
    ///     >>> db = velesdb.Database("./tenant1", config=cfg)
    #[new]
    #[pyo3(signature = (path, config = None))]
    fn new(py: Python<'_>, path: &str, config: Option<VelesConfigOptions>) -> PyResult<Self> {
        // Open the database off the GIL. Opening walks the WAL, rebuilds
        // any in-memory state, and mmaps vector/edge files — on a multi-
        // million-vector directory this easily reaches multi-second
        // latency, and holding the GIL for that long blocks every other
        // Python thread. PyO3 ≥0.20 allows `py: Python<'_>` as the first
        // parameter of a `#[new]` constructor, which is exactly what we
        // need to call `allow_threads` here.
        let path_buf = PathBuf::from(path);
        let path_clone = path_buf.clone();
        let db = py
            .allow_threads(move || match config {
                Some(cfg) => CoreDatabase::open_with_config(&path_clone, cfg.to_core()),
                None => CoreDatabase::open(&path_clone),
            })
            .map_err(core_err)?;
        Ok(Self {
            inner: Arc::new(db),
            path: path_buf,
        })
    }

    /// Create a new vector collection.
    ///
    /// Args:
    ///     name: Collection name
    ///     dimension: Vector dimension (e.g., 768 for BERT embeddings)
    ///     metric: Distance metric — "cosine", "euclidean", "dot",
    ///             "hamming", or "jaccard" (default: "cosine")
    ///     storage_mode: Storage mode (default: "full"). Accepted values
    ///                   (case-insensitive, aliases in parentheses):
    ///                   - "full" ("f32"): Full f32 precision — best recall, 4 bytes/dim.
    ///                   - "sq8" ("int8"): 8-bit scalar quantization — 4x compression, ~1% recall loss.
    ///                   - "binary" ("bit"): 1-bit binary quantization — 32x compression,
    ///                     best for edge/IoT devices.
    ///                   - "pq" ("product_quantization"): Product Quantization — 8x-16x compression
    ///                     via trained codebooks (requires a training step before upserts).
    ///                   - "rabitq": RaBitQ — 1-bit with rotation + scalar correction,
    ///                     32x compression with ~1-2% recall loss.
    ///     hnsw: Optional :class:`HnswOptions` dataclass with typed HNSW
    ///           parameters. Replaces the v1.12 flat kwargs (`m=`,
    ///           `ef_construction=`, `expected_vectors=`) — see the
    ///           v1.13 CHANGELOG for the migration guide.
    ///     auto_reindex: Optional :class:`AutoReindexOptions` dataclass.
    ///           When provided, an :class:`AutoReindexManager` is
    ///           constructed from the options and attached to the
    ///           freshly-created collection as a runtime-only hook.
    ///           The attachment is not persisted — re-attach after
    ///           every `Database(path)` to restore the behavior.
    ///
    /// Returns:
    ///     Collection instance
    ///
    /// Example:
    ///     >>> # Simple creation
    ///     >>> collection = db.create_collection("documents", dimension=768)
    ///     >>> # With SQ8 quantization:
    ///     >>> quantized = db.create_collection(
    ///     ...     "embeddings", dimension=768, storage_mode="sq8"
    ///     ... )
    ///     >>> # With typed HNSW options:
    ///     >>> from velesdb import HnswOptions
    ///     >>> custom = db.create_collection(
    ///     ...     "docs",
    ///     ...     dimension=768,
    ///     ...     hnsw=HnswOptions(m=48, ef_construction=600),
    ///     ... )
    ///     >>> # Auto-tuned for expected dataset size:
    ///     >>> large = db.create_collection(
    ///     ...     "big",
    ///     ...     dimension=128,
    ///     ...     hnsw=HnswOptions.for_dataset_size(128, 1_000_000),
    ///     ... )
    ///     >>> # With auto-reindex divergence monitoring:
    ///     >>> from velesdb import AutoReindexOptions
    ///     >>> monitored = db.create_collection(
    ///     ...     "agents",
    ///     ...     dimension=384,
    ///     ...     auto_reindex=AutoReindexOptions(min_size_for_reindex=5_000),
    ///     ... )
    #[pyo3(signature = (
        name,
        dimension,
        metric = "cosine",
        storage_mode = "full",
        hnsw = None,
        auto_reindex = None,
    ))]
    #[allow(clippy::too_many_arguments)] // Reason: `py` is an injected PyO3 token, not a user-facing argument
    fn create_collection(
        &self,
        py: Python<'_>,
        name: &str,
        dimension: usize,
        metric: &str,
        storage_mode: &str,
        hnsw: Option<HnswOptions>,
        auto_reindex: Option<AutoReindexOptions>,
    ) -> PyResult<Collection> {
        let distance_metric = parse_metric(metric)?;
        let mode = parse_storage_mode(storage_mode)?;

        // Compute the dispatch plan under the GIL — cheap.
        let plan = if let Some(opts) = hnsw {
            CreatePlan::Full {
                hnsw_params: opts.to_hnsw_params(),
                pq_rescore_oversampling: opts.pq_rescore_oversampling,
            }
        } else {
            CreatePlan::Default
        };

        // Convert AutoReindexOptions to a manager under the GIL so the
        // closure doesn't have to touch Python types. `Arc<AutoReindexManager>`
        // is the runtime-only attachment handle (Commit 9).
        let reindex_manager: Option<Arc<AutoReindexManager>> = auto_reindex
            .map(|opts| Arc::new(AutoReindexManager::new(opts.to_core())));

        // Drop the GIL for the disk write + index init. Every string
        // argument must be cloned into an owned value because the
        // closure must be `'static + Send`.
        let name_owned = name.to_string();
        let inner = Arc::clone(&self.inner);
        let name_for_closure = name_owned.clone();
        py.allow_threads(move || match plan {
            CreatePlan::Full {
                hnsw_params,
                pq_rescore_oversampling,
            } => inner.create_vector_collection_with_params(
                &name_for_closure,
                dimension,
                distance_metric,
                mode,
                hnsw_params,
                pq_rescore_oversampling,
            ),
            CreatePlan::Default => inner.create_vector_collection_with_options(
                &name_for_closure,
                dimension,
                distance_metric,
                mode,
            ),
        })
        .map_err(core_err)?;

        // Registry lookup is O(1) on an in-memory map — keep it under
        // the GIL. If this miss fires, something raced against the
        // creation we just did, which is a core-level bug not a user
        // error.
        let collection = self
            .inner
            .get_vector_collection(&name_owned)
            .ok_or_else(|| PyRuntimeError::new_err("Collection not found after creation"))?;

        // Attach the AutoReindex manager to the newly-created collection
        // as a runtime-only hook. This is a no-op when auto_reindex was
        // not provided.
        if let Some(manager) = reindex_manager {
            collection.attach_auto_reindex(manager);
        }

        Ok(Collection::new(collection, name_owned))
    }

    /// Get an existing collection by name.
    ///
    /// Args:
    ///     name: Collection name
    ///
    /// Returns:
    ///     Collection instance or None if not found
    ///
    /// Example:
    ///     >>> collection = db.get_collection("documents")
    #[pyo3(signature = (name))]
    /// Get a collection by name.
    ///
    /// Returns the collection regardless of its type (vector, graph, or metadata).
    /// Returns None if the collection does not exist.
    fn get_collection(&self, name: &str) -> PyResult<Option<Collection>> {
        match self.inner.get_any_collection(name) {
            Some(any_coll) => {
                // F2.2 mitigation: Python SDK exposes a single Collection
                // type. Invoking vector-specific methods on a graph or
                // metadata collection returns empty results rather than
                // raising — the typed split is tracked as a post-seed
                // EPIC in docs/ARCHITECTURE.md.
                let vc = any_coll.as_vector_collection_unchecked();
                Ok(Some(Collection::new(vc, name.to_string())))
            }
            None => Ok(None),
        }
    }

    /// List all collection names in the database.
    ///
    /// Returns:
    ///     List of collection names
    ///
    /// Example:
    ///     >>> names = db.list_collections()
    ///     >>> print(names)  # ['documents', 'images']
    fn list_collections(&self) -> Vec<String> {
        self.inner.list_collections()
    }

    /// Delete a collection by name.
    ///
    /// Args:
    ///     name: Collection name to delete
    ///
    /// Example:
    ///     >>> db.delete_collection("old_collection")
    #[pyo3(signature = (name))]
    fn delete_collection(&self, py: Python<'_>, name: &str) -> PyResult<()> {
        // `delete_collection` walks the directory tree and unlinks every
        // file belonging to the collection — that is a `rm -rf`-class
        // operation and can take tens of milliseconds on hot paths,
        // several hundred milliseconds on cold ones. Release the GIL so
        // other Python threads keep running.
        let name_owned = name.to_string();
        let inner = Arc::clone(&self.inner);
        py.allow_threads(move || inner.delete_collection(&name_owned))
            .map_err(core_err)
    }

    /// Create a metadata-only collection (no vectors, no HNSW index).
    ///
    /// Metadata-only collections are optimized for storing reference data,
    /// catalogs, and other non-vector data. They support CRUD operations
    /// and VelesQL queries on payload, but NOT vector search.
    ///
    /// Args:
    ///     name: Collection name
    ///
    /// Returns:
    ///     Collection instance
    ///
    /// Example:
    ///     >>> products = db.create_metadata_collection("products")
    ///     >>> products.upsert_metadata([
    ///     ...     {"id": 1, "payload": {"name": "Widget", "price": 9.99}}
    ///     ... ])
    #[pyo3(signature = (name))]
    fn create_metadata_collection(&self, py: Python<'_>, name: &str) -> PyResult<Collection> {
        let name_owned = name.to_string();
        let inner = Arc::clone(&self.inner);
        let name_for_closure = name_owned.clone();
        py.allow_threads(move || inner.create_metadata_collection(&name_for_closure))
            .map_err(core_err)?;

        // Use get_any_collection to get the registered instance (not a disconnected copy).
        let collection = self
            .inner
            .get_any_collection(&name_owned)
            .map(velesdb_core::AnyCollection::as_vector_collection_unchecked)
            .ok_or_else(|| PyRuntimeError::new_err("Collection not found after creation"))?;

        Ok(Collection::new(collection, name_owned))
    }

    /// Create an AgentMemory instance for AI agent workflows.
    ///
    /// Args:
    ///     dimension: Embedding dimension (default: 384)
    ///
    /// Returns:
    ///     AgentMemory instance with semantic, episodic, and procedural subsystems
    ///
    /// Example:
    ///     >>> memory = db.agent_memory()
    ///     >>> memory.semantic.store(1, "Paris is in France", embedding)
    #[pyo3(signature = (dimension = None))]
    fn agent_memory(&self, dimension: Option<usize>) -> PyResult<agent::AgentMemory> {
        agent::AgentMemory::new(self, dimension)
    }

    /// Train product quantization on a collection.
    ///
    /// Builds PQ codebooks from existing vectors, enabling compressed
    /// storage and faster ADC-based search.
    ///
    /// Args:
    ///     collection_name: Name of the collection to train on.
    ///     m: Number of subspaces (dimension must be divisible by m). Default: 8.
    ///     k: Number of centroids per subspace. Default: 256.
    ///     opq: Whether to use Optimized Product Quantization. Default: False.
    ///
    /// Returns:
    ///     Status message from the training operation.
    ///
    /// Raises:
    ///     RuntimeError: If training fails (e.g., insufficient data, bad params).
    ///
    /// Example:
    ///     >>> db.train_pq("documents", m=8, k=256)
    ///     >>> db.train_pq("documents", m=16, k=128, opq=True)
    #[pyo3(signature = (collection_name, m=8, k=256, opq=false))]
    fn train_pq(&self, collection_name: &str, m: usize, k: usize, opq: bool) -> PyResult<String> {
        // Validate collection_name to prevent VelesQL injection via string interpolation.
        if !collection_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(PyValueError::new_err(format!(
                "Invalid collection name '{collection_name}': only ASCII letters, digits, \
                 and underscores are allowed"
            )));
        }

        let mut query = format!("TRAIN QUANTIZER ON {collection_name} WITH (m={m}, k={k}");
        if opq {
            query.push_str(", type=opq");
        }
        query.push(')');

        let parsed = velesdb_core::velesql::Parser::parse(&query).map_err(|e| {
            PyValueError::new_err(format!("Failed to construct TRAIN query: {}", e.message))
        })?;

        let empty_params = std::collections::HashMap::new();
        let results = self
            .inner
            .execute_query(&parsed, &empty_params)
            .map_err(core_err)?;

        Ok(format!("PQ training complete: {} results", results.len()))
    }

    /// Create a new persistent graph collection.
    ///
    /// Graph collections store typed relationships between nodes, with optional
    /// node embeddings for vector search.
    ///
    /// Args:
    ///     name: Collection name
    ///     dimension: Optional vector dimension for node embeddings (default: None)
    ///     metric: Distance metric - "cosine", "euclidean", "dot" (default: "cosine")
    ///     schema: Optional GraphSchema (default: schemaless)
    ///
    /// Returns:
    ///     GraphCollection instance
    ///
    /// Example:
    ///     >>> graph = db.create_graph_collection("knowledge")
    ///     >>> graph_with_emb = db.create_graph_collection("kg", dimension=768)
    #[pyo3(signature = (name, dimension=None, metric="cosine", schema=None))]
    fn create_graph_collection(
        &self,
        py: Python<'_>,
        name: &str,
        dimension: Option<usize>,
        metric: &str,
        schema: Option<PyGraphSchema>,
    ) -> PyResult<PyGraphCollection> {
        let distance_metric = parse_metric(metric)?;
        let graph_schema = schema
            .map(|s| s.inner().clone())
            .unwrap_or_else(GraphSchema::schemaless);
        let name_owned = name.to_string();
        let inner = Arc::clone(&self.inner);
        let name_for_closure = name_owned.clone();

        py.allow_threads(move || {
            inner.create_collection_typed(
                &name_for_closure,
                &CollectionType::Graph {
                    dimension,
                    metric: distance_metric,
                    schema: graph_schema,
                },
            )
        })
        .map_err(core_err)?;

        let coll = self
            .inner
            .get_graph_collection(&name_owned)
            .ok_or_else(|| PyRuntimeError::new_err("Graph collection not found after creation"))?;

        Ok(PyGraphCollection::new(coll, name_owned))
    }

    /// Execute a VelesQL query string (SELECT, DDL, or DML).
    ///
    /// Supports all VelesQL statements including:
    ///
    /// - ``SELECT … FROM … WHERE …``
    /// - ``CREATE [GRAPH|METADATA] COLLECTION …``
    /// - ``DROP COLLECTION [IF EXISTS] …``
    /// - ``INSERT EDGE INTO …``
    /// - ``DELETE FROM … WHERE …``
    /// - ``DELETE EDGE … FROM …``
    ///
    /// Args:
    ///     sql: VelesQL query string.
    ///     params: Optional parameter bindings (e.g., ``{"$v": [0.1, 0.2]}``).
    ///             Pass ``None`` or omit to run with no bindings.
    ///
    /// Returns:
    ///     List of result dicts for SELECT queries.
    ///     Each dict contains ``node_id``, ``vector_score``, ``graph_score``,
    ///     ``fused_score``, ``bindings``, ``column_data``, ``id``, ``score``,
    ///     and ``payload`` fields.
    ///     Returns an empty list for DDL/DML statements.
    ///
    /// Raises:
    ///     ValueError: If the SQL string fails to parse.
    ///     RuntimeError: If execution fails.
    ///
    /// # Cross-Collection MATCH
    ///
    /// For MATCH queries that span multiple collections, pass the
    /// ``_collection`` key in ``params`` to specify the primary collection
    /// (the one containing graph edges). Nodes annotated with ``@collection``
    /// in the MATCH pattern will have their payloads enriched from the named
    /// collection after traversal.
    ///
    /// Example:
    ///     >>> results = db.execute_query("SELECT * FROM docs LIMIT 5")
    ///     >>> db.execute_query("CREATE COLLECTION notes (dimension=128, metric=cosine)")
    ///     >>> db.execute_query(
    ///     ...     "SELECT * FROM docs WHERE vector NEAR $q LIMIT 10",
    ///     ...     params={"$q": [0.1, 0.2]},
    ///     ... )
    ///     >>> # Cross-collection MATCH: enrich from 'inventory' collection
    ///     >>> db.execute_query(
    ///     ...     "MATCH (p:Product)-[:STORED_IN]->(inv:Inventory@inventory) "
    ///     ...     "RETURN p.name, inv.price, inv.stock LIMIT 20",
    ///     ...     params={"_collection": "catalog_graph"},
    ///     ... )
    #[pyo3(signature = (sql, params = None))]
    fn execute_query(
        &self,
        py: Python<'_>,
        sql: &str,
        params: Option<std::collections::HashMap<String, PyObject>>,
    ) -> PyResult<Vec<PyObject>> {
        use crate::collection::query::{convert_params, parse_velesql};
        use crate::collection_helpers::search_results_to_multimodel_dicts;

        let parsed = parse_velesql(sql)?;
        let rust_params = convert_params(py, params)?;
        let inner = Arc::clone(&self.inner);
        let results = py
            .allow_threads(move || inner.execute_query(&parsed, &rust_params))
            .map_err(core_err)?;
        Ok(search_results_to_multimodel_dicts(py, results))
    }

    /// Get an existing graph collection by name.
    ///
    /// Args:
    ///     name: Collection name
    ///
    /// Returns:
    ///     GraphCollection instance or None if not found
    ///
    /// Example:
    ///     >>> graph = db.get_graph_collection("knowledge")
    #[pyo3(signature = (name))]
    fn get_graph_collection(&self, name: &str) -> PyResult<Option<PyGraphCollection>> {
        Ok(self
            .inner
            .get_graph_collection(name)
            .map(|c| PyGraphCollection::new(c, name.to_string())))
    }

    /// Analyze a collection, computing and persisting statistics.
    ///
    /// Computes row counts, size metrics, column cardinality, and index
    /// statistics, then caches them in memory and persists to disk.
    ///
    /// Args:
    ///     name: Collection name to analyze
    ///
    /// Returns:
    ///     dict with statistics (total_points, row_count, deleted_count,
    ///     total_size_bytes, avg_row_size_bytes, payload_size_bytes,
    ///     column_stats, index_stats, last_analyzed_epoch_ms, etc.)
    ///
    /// Raises:
    ///     RuntimeError: If the collection does not exist or analysis fails
    ///
    /// Example:
    ///     >>> stats = db.analyze_collection("documents")
    ///     >>> print(stats["total_points"])
    #[pyo3(signature = (name))]
    fn analyze_collection(&self, py: Python<'_>, name: &str) -> PyResult<PyObject> {
        // `analyze_collection` walks the column store and the index,
        // computing cardinality, size histograms, and graph stats. On
        // a ten-million-row collection it crosses the 1-second mark —
        // way past the "release the GIL" threshold.
        let name_owned = name.to_string();
        let inner = Arc::clone(&self.inner);
        let stats = py
            .allow_threads(move || inner.analyze_collection(&name_owned))
            .map_err(core_err)?;
        let json = serde_json::to_value(&stats)
            .map_err(|e| PyRuntimeError::new_err(format!("Serialization failed: {e}")))?;
        Ok(utils::json_to_python(py, &json))
    }

    /// Get cached collection statistics (or None if never analyzed).
    ///
    /// Returns previously computed statistics from cache or disk.
    /// Call ``analyze_collection`` first to generate fresh statistics.
    ///
    /// Args:
    ///     name: Collection name
    ///
    /// Returns:
    ///     dict with statistics or None if the collection has never been analyzed
    ///
    /// Raises:
    ///     RuntimeError: If on-disk stats exist but cannot be read
    ///
    /// Example:
    ///     >>> stats = db.get_collection_stats("documents")
    ///     >>> if stats is not None:
    ///     ...     print(stats["row_count"])
    #[pyo3(signature = (name))]
    fn get_collection_stats(&self, py: Python<'_>, name: &str) -> PyResult<Option<PyObject>> {
        // `get_collection_stats` reads the cached stats file from disk
        // when the in-memory cache is cold, so in the worst case it
        // performs a small I/O. Release the GIL so other Python threads
        // are not blocked on that read.
        let name_owned = name.to_string();
        let inner = Arc::clone(&self.inner);
        let maybe_stats = py
            .allow_threads(move || inner.get_collection_stats(&name_owned))
            .map_err(core_err)?;
        maybe_stats
            .map(|stats| {
                let json = serde_json::to_value(&stats).map_err(|e| {
                    PyRuntimeError::new_err(format!("stats serialization failed: {e}"))
                })?;
                Ok(utils::json_to_python(py, &json))
            })
            .transpose()
    }
}

impl Database {
    /// Get a reference to the inner CoreDatabase.
    pub fn inner(&self) -> &CoreDatabase {
        &self.inner
    }

    /// Get the database path.
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Return a shared `Arc<CoreDatabase>` handle to the already-opened database.
    ///
    /// Used by subsystems (e.g., AgentMemory) that need `Arc` ownership.
    /// The handle shares the same in-memory registries and file lock as the
    /// parent `Database`, avoiding VELES-031 re-entrant lock errors.
    pub fn open_shared(&self) -> std::result::Result<Arc<CoreDatabase>, String> {
        Ok(Arc::clone(&self.inner))
    }
}
