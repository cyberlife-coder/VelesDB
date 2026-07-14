//! `Database` — PyO3-exported database entry point for Python bindings.

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;

use crate::agent;
use crate::collection::{Collection, CollectionKind};
use crate::collection_helpers::core_err;
use crate::graph_collection::{PyGraphCollection, PyGraphSchema};
use crate::observer::PyObserver;
use crate::options::{AutoReindexOptions, HnswOptions, VelesConfigOptions};
use crate::utils::{self, parse_metric, parse_storage_mode};

use velesdb_core::collection::auto_reindex::AutoReindexManager;
use velesdb_core::{CollectionType, Database as CoreDatabase, DatabaseObserver, GraphSchema};

/// The full set of guardrail fields. `update_guardrails` is an explicit full
/// replacement (matching the Mobile/Tauri typed structs), so the supplied dict
/// must contain exactly these keys: omitting one would silently reset it to its
/// default, and a typo'd key would silently reset every field.
const GUARDRAIL_FIELDS: [&str; 7] = [
    "max_depth",
    "max_cardinality",
    "memory_limit_bytes",
    "timeout_ms",
    "rate_limit_qps",
    "circuit_failure_threshold",
    "circuit_recovery_seconds",
];

/// Validates that `obj` contains exactly the guardrail fields — no unknown keys
/// and none missing — so a partial or misspelled dict is rejected loudly rather
/// than silently resetting limits to their defaults.
fn validate_guardrail_keys(obj: &serde_json::Map<String, serde_json::Value>) -> PyResult<()> {
    for key in obj.keys() {
        if !GUARDRAIL_FIELDS.contains(&key.as_str()) {
            return Err(PyValueError::new_err(format!(
                "unknown guardrail field: '{key}'"
            )));
        }
    }
    for field in GUARDRAIL_FIELDS {
        if !obj.contains_key(field) {
            return Err(PyValueError::new_err(format!(
                "missing guardrail field: '{field}' (update_guardrails is a full replacement; provide all fields)"
            )));
        }
    }
    Ok(())
}

/// Opens the core database, branching across the optional config and observer.
///
/// Extracted from [`Database::new`] so the constructor stays within the
/// cyclomatic-complexity budget: the four config × observer combinations would
/// otherwise inflate `new`. Runs off the GIL (called inside `allow_threads`).
fn open_core(
    path: &std::path::Path,
    config: Option<velesdb_core::config::VelesConfig>,
    observer: Option<Arc<dyn DatabaseObserver>>,
) -> velesdb_core::Result<CoreDatabase> {
    match (config, observer) {
        (Some(cfg), Some(obs)) => CoreDatabase::open_with_observer_and_config(path, obs, cfg),
        (Some(cfg), None) => CoreDatabase::open_with_config(path, cfg),
        (None, Some(obs)) => CoreDatabase::open_with_observer(path, obs),
        (None, None) => CoreDatabase::open(path),
    }
}

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
    ///     observer: Optional callable invoked on collection lifecycle
    ///         events and before every gated read. Called as
    ///         ``observer(event, **fields)`` where ``event`` is one of
    ///         ``"collection_created"``, ``"collection_deleted"``,
    ///         ``"upsert"``, ``"query"``, or ``"query_request"``:
    ///
    ///         - ``collection_created`` → ``name``, ``kind``
    ///           (``"vector"``/``"metadata"``/``"graph"``/``"unknown"``)
    ///         - ``collection_deleted`` → ``name``
    ///         - ``upsert`` → ``collection``, ``point_count``
    ///         - ``query`` → ``collection``, ``duration_us`` (after execution)
    ///         - ``query_request`` → ``collection``, ``operation``,
    ///           ``principal``, ``tenant`` (before execution — **veto point**)
    ///
    ///         The observer is immutable once the database is opened
    ///         (there is no post-open setter). For the *notify* events the
    ///         return value is ignored and a raised exception is swallowed so
    ///         it never breaks a core operation. For ``"query_request"`` the
    ///         callback governs the read: return ``False`` or a string reason to
    ///         **deny** it (raising a query error), ``None``/``True`` to allow,
    ///         or a ``dict`` to **allow with a narrowing scope**. The dict MUST
    ///         carry an enforceable ``"filter"`` (a VelesQL WHERE-predicate
    ///         string such as ``"tenant = 'acme'"``, AND-composed into the
    ///         read); ``"tenant"`` is an optional audit hint OSS does not narrow
    ///         by. A dict with a missing/empty/tenant-only or unparseable
    ///         ``"filter"`` denies (fail closed), so a ``{"tenant": t}`` return
    ///         can never masquerade as scoping. ``query_request`` fires for VelesQL
    ///         ``SELECT``/``MATCH`` and for the Python direct-search API
    ///         (``search``/``search_request``/``text_search``/``hybrid_search``
    ///         and their variants), with ``operation`` one of ``"select"``,
    ///         ``"vector_search"``, ``"text_search"``, ``"hybrid_search"``,
    ///         ``"graph_traversal"``. A callback that ignores ``query_request``
    ///         (returns ``None``) allows every read, so existing notify-only
    ///         observers are unaffected.
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
    ///     >>> # With a lifecycle observer:
    ///     >>> events = []
    ///     >>> db = velesdb.Database(
    ///     ...     "./audited",
    ///     ...     observer=lambda event, **f: events.append((event, f)),
    ///     ... )
    #[new]
    #[pyo3(signature = (path, config = None, observer = None))]
    fn new(
        py: Python<'_>,
        path: &str,
        config: Option<VelesConfigOptions>,
        observer: Option<Py<PyAny>>,
    ) -> PyResult<Self> {
        // Open the database off the GIL. Opening walks the WAL, rebuilds
        // any in-memory state, and mmaps vector/edge files — on a multi-
        // million-vector directory this easily reaches multi-second
        // latency, and holding the GIL for that long blocks every other
        // Python thread. PyO3 ≥0.20 allows `py: Python<'_>` as the first
        // parameter of a `#[new]` constructor, which is exactly what we
        // need to call `allow_threads` here.
        //
        // The observer's `Py<PyAny>` is `Send`, so it can be captured by the
        // `'static` open closure; building the `PyObserver` stays off-GIL.
        let path_buf = PathBuf::from(path);
        let path_clone = path_buf.clone();
        let core_config = config.map(|cfg| cfg.to_core());
        let core_observer: Option<Arc<dyn DatabaseObserver>> =
            observer.map(|cb| Arc::new(PyObserver::new(cb)) as Arc<dyn DatabaseObserver>);
        let db = py
            .detach(move || open_core(&path_clone, core_config, core_observer))
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
                hnsw_params: opts.to_hnsw_params()?,
                pq_rescore_oversampling: opts.pq_rescore_oversampling,
            }
        } else {
            CreatePlan::Default
        };

        // Convert AutoReindexOptions to a manager under the GIL so the
        // closure doesn't have to touch Python types. `Arc<AutoReindexManager>`
        // is the runtime-only attachment handle (Commit 9).
        let reindex_manager: Option<Arc<AutoReindexManager>> =
            auto_reindex.map(|opts| Arc::new(AutoReindexManager::new(opts.to_core())));

        // Drop the GIL for the disk write + index init. Every string
        // argument must be cloned into an owned value because the
        // closure must be `'static + Send`.
        let name_owned = name.to_string();
        let inner = Arc::clone(&self.inner);
        let name_for_closure = name_owned.clone();
        py.detach(move || match plan {
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

        Ok(Collection::new(
            collection,
            Arc::clone(&self.inner),
            name_owned,
        ))
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
                // The Python SDK exposes a single `Collection` facade over all
                // variants. Capture the real kind so vector-only methods fail
                // loud on graph/metadata collections (F2.2) instead of silently
                // returning empty results.
                let kind = if any_coll.is_graph() {
                    CollectionKind::Graph
                } else if any_coll.is_metadata() {
                    CollectionKind::Metadata
                } else {
                    CollectionKind::Vector
                };
                // SAFETY: `any_coll` came from `get_any_collection` (Some), so the
                // underlying `AnyCollection` is registered and valid.
                // - any_coll is a valid, registered handle returned by
                //   `get_any_collection`; it is never a disconnected copy.
                // - The coerced vector facade only exercises the shared surface
                //   for non-Vector kinds; `kind` is captured above and
                //   `Collection::ensure_vector` rejects vector-only ops on
                //   graph/metadata collections (fails loud, not UB).
                // Reason: single-Collection Python ergonomic facade (mirrors the
                // conforming block in `create_metadata_collection` below).
                let vc = unsafe { any_coll.into_vector_unchecked() };
                Ok(Some(Collection::new_with_kind(
                    vc,
                    Arc::clone(&self.inner),
                    name.to_string(),
                    kind,
                )))
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

    /// Alias for `list_collections`.
    ///
    /// Kept for compatibility with older documentation and examples that used
    /// `get_collections()` for the same operation.
    fn get_collections(&self) -> Vec<String> {
        self.list_collections()
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
        py.detach(move || inner.delete_collection(&name_owned))
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
        py.detach(move || inner.create_metadata_collection(&name_for_closure))
            .map_err(core_err)?;

        // Use get_any_collection to get the registered instance (not a disconnected copy).
        let any = self
            .inner
            .get_any_collection(&name_owned)
            .ok_or_else(|| PyRuntimeError::new_err("Collection not found after creation"))?;
        // SAFETY: Python SDK wraps the freshly-created metadata collection in the
        // single Collection facade (mirrors `get_collection` above).
        // - any was just registered by `create_metadata_collection` and retrieved
        //   via `get_any_collection`, so it is a valid registered handle.
        // - Only the shared surface is exercised on metadata variants.
        // Reason: single-Collection Python ergonomic facade.
        let collection = unsafe { any.into_vector_unchecked() };

        Ok(Collection::new_with_kind(
            collection,
            Arc::clone(&self.inner),
            name_owned,
            CollectionKind::Metadata,
        ))
    }

    /// Create an AgentMemory instance for AI agent workflows.
    ///
    /// Args:
    ///     dimension: Embedding dimension (default: 384)
    ///     snapshot_dir: Optional directory to enable versioned snapshots
    ///     max_snapshots: Number of snapshots to retain (default: 10)
    ///
    /// Returns:
    ///     AgentMemory instance with semantic, episodic, and procedural subsystems
    ///
    /// Example:
    ///     >>> memory = db.agent_memory()
    ///     >>> memory.semantic.store(1, "Paris is in France", embedding)
    #[pyo3(signature = (dimension = None, snapshot_dir = None, max_snapshots = 10))]
    fn agent_memory(
        &self,
        dimension: Option<usize>,
        snapshot_dir: Option<String>,
        max_snapshots: usize,
    ) -> PyResult<agent::AgentMemory> {
        agent::AgentMemory::new(self, dimension, snapshot_dir, max_snapshots)
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

        py.detach(move || {
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

        Ok(PyGraphCollection::new(
            coll,
            Arc::clone(&self.inner),
            name_owned,
        ))
    }

    /// Execute a VelesQL query string (SELECT, DDL, or DML).
    ///
    /// Supports all VelesQL statements including:
    ///
    /// - ``SELECT … FROM … WHERE …``
    /// - ``CREATE [GRAPH|METADATA] COLLECTION …``
    /// - ``CREATE INDEX ON <collection> (<field>)`` / ``DROP INDEX …``
    /// - ``ALTER COLLECTION <name> SET (auto_reindex = true|false)``
    ///   (see :py:meth:`set_auto_reindex` for a typed helper)
    /// - ``DROP COLLECTION [IF EXISTS] …``
    /// - ``INSERT EDGE INTO …``
    /// - ``DELETE FROM … WHERE …``
    /// - ``DELETE EDGE … FROM …``
    ///
    /// Hybrid fusion note:
    ///     Typed hybrid dense+sparse search (``search_request`` with both
    ///     ``vector`` and ``sparse_vector``) uses Reciprocal Rank Fusion
    ///     (RRF, k=60) by default. To choose another strategy, pass a
    ///     :py:class:`FusionStrategy` via ``SearchOptions(fusion=...)`` /
    ///     ``with_fusion(...)``, or run raw VelesQL with a
    ///     ``USING FUSION(...)`` clause through this method.
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
        params: Option<std::collections::HashMap<String, Py<PyAny>>>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        use crate::collection::query::{convert_params, parse_velesql};
        use crate::collection_helpers::search_results_to_multimodel_dicts;

        let parsed = parse_velesql(sql)?;
        let rust_params = convert_params(py, params)?;
        let inner = Arc::clone(&self.inner);
        let results = py
            .detach(move || inner.execute_query(&parsed, &rust_params))
            .map_err(core_err)?;
        Ok(search_results_to_multimodel_dicts(py, results))
    }

    /// Toggle automatic re-indexing on a collection at runtime.
    ///
    /// Routes a validated ``ALTER COLLECTION <name> SET (auto_reindex = …)``
    /// statement through the VelesQL DDL executor and persists the change so
    /// it survives a restart. This is the typed counterpart to running the
    /// raw ALTER statement via :py:meth:`execute_query`.
    ///
    /// Args:
    ///     name: Collection name.
    ///     enabled: ``True`` to enable auto-reindex, ``False`` to disable it.
    ///
    /// Raises:
    ///     ValueError: If the collection name fails to parse.
    ///     RuntimeError: If the collection does not exist or the change fails.
    ///
    /// Example:
    ///     >>> db.set_auto_reindex("documents", True)
    ///     >>> db.get_collection("documents").info()["auto_reindex"]
    ///     True
    #[pyo3(signature = (name, enabled))]
    fn set_auto_reindex(&self, py: Python<'_>, name: &str, enabled: bool) -> PyResult<()> {
        use crate::collection::query::parse_velesql;

        // Backtick-quote the identifier so names with hyphens/spaces parse,
        // doubling any embedded backtick per the grammar's escaping rule.
        let quoted = name.replace('`', "``");
        let sql = format!("ALTER COLLECTION `{quoted}` SET (auto_reindex = {enabled})");
        let parsed = parse_velesql(&sql)?;
        let inner = Arc::clone(&self.inner);
        py.detach(move || inner.execute_query(&parsed, &std::collections::HashMap::new()))
            .map_err(core_err)?;
        Ok(())
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
            .map(|c| PyGraphCollection::new(c, Arc::clone(&self.inner), name.to_string())))
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
    fn analyze_collection(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        // `analyze_collection` walks the column store and the index,
        // computing cardinality, size histograms, and graph stats. On
        // a ten-million-row collection it crosses the 1-second mark —
        // way past the "release the GIL" threshold.
        let name_owned = name.to_string();
        let inner = Arc::clone(&self.inner);
        let stats = py
            .detach(move || inner.analyze_collection(&name_owned))
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
    fn get_collection_stats(&self, py: Python<'_>, name: &str) -> PyResult<Option<Py<PyAny>>> {
        // `get_collection_stats` reads the cached stats file from disk
        // when the in-memory cache is cold, so in the worst case it
        // performs a small I/O. Release the GIL so other Python threads
        // are not blocked on that read.
        let name_owned = name.to_string();
        let inner = Arc::clone(&self.inner);
        let maybe_stats = py
            .detach(move || inner.get_collection_stats(&name_owned))
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

    /// Get health diagnostics for a collection.
    ///
    /// Reports index readiness without relying on the REST server — useful
    /// for embedded health checks.
    ///
    /// Args:
    ///     name: Collection name
    ///
    /// Returns:
    ///     dict with keys ``has_vectors``, ``search_ready``,
    ///     ``dimension_configured``, ``point_count``, ``index_health``
    ///     ("healthy" | "empty" | "needs_rebuild"), and
    ///     ``index_health_detail`` (only when a rebuild is needed).
    ///
    /// Raises:
    ///     RuntimeError: If the collection does not exist.
    #[pyo3(signature = (name))]
    fn collection_diagnostics(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        use velesdb_core::collection::IndexHealth;
        let name_owned = name.to_string();
        let inner = Arc::clone(&self.inner);
        let diag = py
            .detach(move || inner.collection_diagnostics(&name_owned))
            .map_err(core_err)?;

        let dict = pyo3::types::PyDict::new(py);
        dict.set_item("has_vectors", diag.has_vectors)?;
        dict.set_item("search_ready", diag.search_ready)?;
        dict.set_item("dimension_configured", diag.dimension_configured)?;
        dict.set_item("point_count", diag.point_count)?;
        let (health, detail) = match &diag.index_health {
            IndexHealth::Healthy => ("healthy", None),
            IndexHealth::Empty => ("empty", None),
            IndexHealth::NeedsRebuild(reason) => ("needs_rebuild", Some(reason.clone())),
            _ => ("unknown", None),
        };
        dict.set_item("index_health", health)?;
        if let Some(detail) = detail {
            dict.set_item("index_health_detail", detail)?;
        }
        Ok(dict.into())
    }

    /// Update query guardrail limits for every collection in this database.
    ///
    /// This is a **full replacement**, not a partial patch (matching the
    /// Mobile/Tauri bindings): the dict must contain *all* guardrail fields.
    /// A missing or misspelled key raises ``ValueError`` rather than silently
    /// resetting limits to their defaults. To change one field, read the
    /// current limits first (``Collection.guard_rails()``), mutate, and submit
    /// the complete dict.
    ///
    /// Args:
    ///     limits: dict with exactly these keys: ``max_depth``,
    ///         ``max_cardinality``, ``memory_limit_bytes``, ``timeout_ms``,
    ///         ``rate_limit_qps``, ``circuit_failure_threshold``,
    ///         ``circuit_recovery_seconds``.
    ///
    /// Raises:
    ///     ValueError: If a field is missing, unknown, or has an invalid value.
    #[pyo3(signature = (limits))]
    fn update_guardrails(&self, py: Python<'_>, limits: Py<PyAny>) -> PyResult<()> {
        let json = utils::python_to_json(py, &limits)?;
        let obj = json
            .as_object()
            .ok_or_else(|| PyValueError::new_err("guardrails must be a dict"))?;
        validate_guardrail_keys(obj)?;
        let parsed: velesdb_core::guardrails::QueryLimits = serde_json::from_value(json)
            .map_err(|e| PyValueError::new_err(format!("invalid guardrails: {e}")))?;
        py.detach(|| self.inner.update_guardrails(&parsed));
        Ok(())
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
