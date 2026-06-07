//! Python bindings for AgentMemory (EPIC-010/US-005)
//!
//! Provides Pythonic access to VelesDB's agent memory subsystems:
//! - SemanticMemory: Long-term knowledge facts
//! - EpisodicMemory: Event timeline
//! - ProceduralMemory: Learned patterns

use pyo3::exceptions::{PyKeyError, PyRuntimeError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyString};
use std::collections::HashMap;
use std::sync::Arc;
use velesdb_core::agent::{
    AgentMemory as CoreAgentMemory, AgentMemoryError, EpisodicMemory as CoreEpisodicMemory,
    ProceduralMemory as CoreProceduralMemory, SemanticMemory as CoreSemanticMemory,
    DEFAULT_DIMENSION,
};
use velesdb_core::Database as CoreDatabase;

use crate::collection::query::convert_params;
use crate::collection_helpers::{core_err, search_results_to_multimodel_dicts};
use crate::exceptions::DimensionMismatchError;

/// Convert procedural memory matches to a Python list of dicts.
fn procedures_to_pylist(
    py: Python<'_>,
    results: Vec<velesdb_core::agent::ProcedureMatch>,
) -> PyResult<PyObject> {
    let list = pyo3::types::PyList::empty(py);
    for m in results {
        let dict = PyDict::new(py);
        let _ = dict.set_item(PyString::intern(py, "id"), m.id);
        let _ = dict.set_item(PyString::intern(py, "name"), &m.name);
        let _ = dict.set_item(PyString::intern(py, "steps"), &m.steps);
        let _ = dict.set_item(PyString::intern(py, "confidence"), m.confidence);
        let _ = dict.set_item(PyString::intern(py, "score"), m.score);
        list.append(dict)?;
    }
    Ok(list.into())
}

/// Convert episodic event tuples to a Python list of dicts.
fn events_to_pylist(py: Python<'_>, events: Vec<(u64, String, i64)>) -> PyResult<PyObject> {
    let list = pyo3::types::PyList::empty(py);
    for (id, description, timestamp) in events {
        let dict = PyDict::new(py);
        let _ = dict.set_item(PyString::intern(py, "id"), id);
        let _ = dict.set_item(PyString::intern(py, "description"), description);
        let _ = dict.set_item(PyString::intern(py, "timestamp"), timestamp);
        list.append(dict)?;
    }
    Ok(list.into())
}

/// Convert `AgentMemoryError` to the most specific Python exception.
///
/// `AgentMemoryError::DatabaseError` is a transparent wrapper around
/// `velesdb_core::Error`, so its inner variant is unwrapped and
/// delegated to [`core_err`] — this means a `VELES-002 CollectionNotFound`
/// raised from inside an agent memory operation surfaces as
/// `CollectionNotFoundError` in Python, not a flat `RuntimeError`.
///
/// Other `AgentMemoryError` variants carry agent-layer semantics that
/// do not have a direct core equivalent:
///
/// * `DimensionMismatch` → mapped to the typed `DimensionMismatchError`
///   (same class used by `core_err` for `VELES-004`).
/// * `NotFound` → mapped to the Python built-in `KeyError` — agents
///   call this when a memory entry (not a collection) is missing.
/// * `InitializationError`, `CollectionError`, `SnapshotError`,
///   `SnapshotIoError` → fall through to `PyRuntimeError` because they
///   wrap opaque agent-layer messages without a structured inner type.
fn to_py_err(e: AgentMemoryError) -> PyErr {
    match e {
        AgentMemoryError::DatabaseError(inner) => core_err(inner),
        AgentMemoryError::DimensionMismatch { expected, actual } => {
            DimensionMismatchError::new_err(format!("Expected {expected} dimensions, got {actual}"))
        }
        AgentMemoryError::NotFound(msg) => PyKeyError::new_err(msg),
        other => PyRuntimeError::new_err(other.to_string()),
    }
}

/// Python wrapper for AgentMemory.
///
/// Provides unified memory access for AI agents with three subsystems:
/// - semantic: Long-term knowledge storage
/// - episodic: Event timeline
/// - procedural: Learned patterns
///
/// Example:
///     >>> from velesdb import Database, AgentMemory
///     >>> db = Database("./agent_data")
///     >>> memory = AgentMemory(db)
///     >>> memory.semantic.store(1, "Paris is the capital of France", embedding)
#[pyclass]
pub struct AgentMemory {
    db: Arc<CoreDatabase>,
    /// Persistent core handle that owns the shared TTL registry, eviction
    /// config, and (optional) snapshot manager. TTL/eviction/snapshot and the
    /// VelesQL bridges route through this instance so state stays coherent.
    core: Arc<CoreAgentMemory>,
    dimension: usize,
}

#[pymethods]
impl AgentMemory {
    /// Create a new AgentMemory from a Database.
    ///
    /// Args:
    ///     db: Database instance
    ///     dimension: Embedding dimension (default: 384)
    ///     snapshot_dir: Optional directory to enable versioned snapshots
    ///     max_snapshots: Number of snapshots to retain (default: 10)
    ///
    /// Example:
    ///     >>> memory = AgentMemory(db)
    ///     >>> memory = AgentMemory(db, dimension=768)
    #[new]
    #[pyo3(signature = (db, dimension = None, snapshot_dir = None, max_snapshots = 10))]
    pub fn new(
        db: &crate::Database,
        dimension: Option<usize>,
        snapshot_dir: Option<String>,
        max_snapshots: usize,
    ) -> PyResult<Self> {
        let dim = dimension.unwrap_or(DEFAULT_DIMENSION);

        // PyO3 classes cannot hold lifetime parameters, so we open an
        // independent Database handle from the same path. Each handle has its
        // own in-memory registries but reads/writes the same on-disk data.
        let owned_db = db.open_shared().map_err(PyRuntimeError::new_err)?;

        // Initialize memory subsystems — this creates the underlying collections
        // if they do not already exist. The returned handle owns the shared TTL
        // registry used by set_*_ttl / auto_expire and the snapshot manager.
        let mut core =
            CoreAgentMemory::with_dimension(Arc::clone(&owned_db), dim).map_err(to_py_err)?;
        if let Some(dir) = snapshot_dir {
            core = core.with_snapshots(&dir, max_snapshots);
        }

        Ok(Self {
            db: owned_db,
            core: Arc::new(core),
            dimension: dim,
        })
    }

    /// Returns the semantic memory subsystem.
    #[getter]
    fn semantic(&self) -> PyResult<PySemanticMemory> {
        let inner = CoreSemanticMemory::new_from_db(Arc::clone(&self.db), self.dimension)
            .map_err(to_py_err)?;
        Ok(PySemanticMemory { inner })
    }

    /// Returns the episodic memory subsystem.
    #[getter]
    fn episodic(&self) -> PyResult<PyEpisodicMemory> {
        let inner = CoreEpisodicMemory::new_from_db(Arc::clone(&self.db), self.dimension)
            .map_err(to_py_err)?;
        Ok(PyEpisodicMemory { inner })
    }

    /// Returns the procedural memory subsystem.
    #[getter]
    fn procedural(&self) -> PyResult<PyProceduralMemory> {
        let inner = CoreProceduralMemory::new_from_db(Arc::clone(&self.db), self.dimension)
            .map_err(to_py_err)?;
        Ok(PyProceduralMemory { inner })
    }

    /// Returns the embedding dimension.
    #[getter]
    fn dimension(&self) -> usize {
        self.dimension
    }

    /// Sets a TTL (in seconds) for a semantic memory entry.
    #[pyo3(signature = (id, ttl_seconds))]
    fn set_semantic_ttl(&self, id: u64, ttl_seconds: u64) {
        self.core.set_semantic_ttl(id, ttl_seconds);
    }

    /// Sets a TTL (in seconds) for an episodic memory entry.
    #[pyo3(signature = (id, ttl_seconds))]
    fn set_episodic_ttl(&self, id: u64, ttl_seconds: u64) {
        self.core.set_episodic_ttl(id, ttl_seconds);
    }

    /// Sets a TTL (in seconds) for a procedural memory entry.
    #[pyo3(signature = (id, ttl_seconds))]
    fn set_procedural_ttl(&self, id: u64, ttl_seconds: u64) {
        self.core.set_procedural_ttl(id, ttl_seconds);
    }

    /// Expires entries past their TTL and consolidates old episodes.
    ///
    /// Returns:
    ///     Dict with 'semantic_expired', 'episodic_expired', 'procedural_expired',
    ///     'episodic_consolidated', 'procedural_evicted' counts.
    fn auto_expire(&self, py: Python<'_>) -> PyResult<PyObject> {
        let result = py.allow_threads(|| self.core.auto_expire().map_err(to_py_err))?;
        Ok(expire_result_to_dict(py, &result))
    }

    /// Evicts procedures whose confidence is below the threshold.
    ///
    /// Returns:
    ///     Number of procedures evicted.
    #[pyo3(signature = (min_confidence))]
    fn evict_low_confidence_procedures(
        &self,
        py: Python<'_>,
        min_confidence: f32,
    ) -> PyResult<usize> {
        py.allow_threads(|| {
            self.core
                .evict_low_confidence_procedures(min_confidence)
                .map_err(to_py_err)
        })
    }

    /// Creates a versioned snapshot of the current memory state.
    ///
    /// Requires `snapshot_dir` to have been provided at construction.
    ///
    /// Returns:
    ///     The version number of the created snapshot.
    fn snapshot(&self, py: Python<'_>) -> PyResult<u64> {
        py.allow_threads(|| self.core.snapshot().map_err(to_py_err))
    }

    /// Loads the most recent snapshot, restoring all memory subsystems.
    ///
    /// Returns:
    ///     The version number of the loaded snapshot.
    fn load_latest_snapshot(&self, py: Python<'_>) -> PyResult<u64> {
        py.allow_threads(|| self.core.load_latest_snapshot().map_err(to_py_err))
    }

    /// Loads a specific snapshot version, restoring all memory subsystems.
    #[pyo3(signature = (version))]
    fn load_snapshot_version(&self, py: Python<'_>, version: u64) -> PyResult<()> {
        py.allow_threads(|| self.core.load_snapshot_version(version).map_err(to_py_err))
    }

    /// Lists all available snapshot version numbers.
    fn list_snapshot_versions(&self, py: Python<'_>) -> PyResult<Vec<u64>> {
        py.allow_threads(|| self.core.list_snapshot_versions().map_err(to_py_err))
    }

    /// Executes a VelesQL query against the semantic memory collection.
    ///
    /// Args:
    ///     query_str: VelesQL query string (e.g. WHERE vector NEAR $v)
    ///     params: Optional query parameters (vectors as lists, scalars)
    ///
    /// Returns:
    ///     List of result dicts.
    #[pyo3(signature = (query_str, params = None))]
    fn query_semantic(
        &self,
        py: Python<'_>,
        query_str: &str,
        params: Option<HashMap<String, PyObject>>,
    ) -> PyResult<Vec<PyObject>> {
        self.run_memory_query(py, query_str, params, |c, sql, p| c.query_semantic(sql, p))
    }

    /// Executes a VelesQL query against the episodic memory collection.
    #[pyo3(signature = (query_str, params = None))]
    fn query_episodic(
        &self,
        py: Python<'_>,
        query_str: &str,
        params: Option<HashMap<String, PyObject>>,
    ) -> PyResult<Vec<PyObject>> {
        self.run_memory_query(py, query_str, params, |c, sql, p| c.query_episodic(sql, p))
    }

    /// Executes a VelesQL query against the procedural memory collection.
    #[pyo3(signature = (query_str, params = None))]
    fn query_procedural(
        &self,
        py: Python<'_>,
        query_str: &str,
        params: Option<HashMap<String, PyObject>>,
    ) -> PyResult<Vec<PyObject>> {
        self.run_memory_query(py, query_str, params, |c, sql, p| {
            c.query_procedural(sql, p)
        })
    }

    fn __repr__(&self) -> String {
        format!("AgentMemory(dimension={})", self.dimension)
    }
}

impl AgentMemory {
    /// Shared driver for the three VelesQL memory bridges: converts params,
    /// runs the core query off the GIL, then builds result dicts.
    fn run_memory_query<F>(
        &self,
        py: Python<'_>,
        query_str: &str,
        params: Option<HashMap<String, PyObject>>,
        execute: F,
    ) -> PyResult<Vec<PyObject>>
    where
        F: FnOnce(
                &CoreAgentMemory,
                &str,
                &HashMap<String, serde_json::Value>,
            ) -> Result<Vec<velesdb_core::SearchResult>, AgentMemoryError>
            + Send,
    {
        let rust_params = convert_params(py, params)?;
        let core = Arc::clone(&self.core);
        let results =
            py.allow_threads(|| execute(&core, query_str, &rust_params).map_err(to_py_err))?;
        Ok(search_results_to_multimodel_dicts(py, results))
    }
}

/// Builds a Python dict from an `ExpireResult`.
fn expire_result_to_dict(py: Python<'_>, r: &velesdb_core::agent::ExpireResult) -> PyObject {
    let dict = PyDict::new(py);
    let _ = dict.set_item(PyString::intern(py, "semantic_expired"), r.semantic_expired);
    let _ = dict.set_item(PyString::intern(py, "episodic_expired"), r.episodic_expired);
    let _ = dict.set_item(
        PyString::intern(py, "procedural_expired"),
        r.procedural_expired,
    );
    let _ = dict.set_item(
        PyString::intern(py, "episodic_consolidated"),
        r.episodic_consolidated,
    );
    let _ = dict.set_item(
        PyString::intern(py, "procedural_evicted"),
        r.procedural_evicted,
    );
    dict.into()
}

/// Python wrapper for SemanticMemory.
///
/// Stores long-term knowledge facts with vector similarity search.
/// The core memory object is resolved once when this wrapper is created,
/// avoiding per-method registry lookups.
///
/// Example:
///     >>> memory.semantic.store(1, "The sky is blue", [0.1, 0.2, ...])
///     >>> results = memory.semantic.query([0.1, 0.2, ...], top_k=5)
#[pyclass]
pub struct PySemanticMemory {
    inner: CoreSemanticMemory,
}

#[pymethods]
impl PySemanticMemory {
    /// Store a knowledge fact with its embedding.
    ///
    /// Args:
    ///     id: Unique identifier for the fact
    ///     content: Text content of the knowledge
    ///     embedding: Vector representation (list of floats)
    ///
    /// Example:
    ///     >>> memory.semantic.store(1, "Paris is in France", embedding)
    #[pyo3(signature = (id, content, embedding))]
    fn store(&self, py: Python<'_>, id: u64, content: &str, embedding: Vec<f32>) -> PyResult<()> {
        let content_owned = content.to_string();
        py.allow_threads(|| {
            self.inner
                .store(id, &content_owned, &embedding)
                .map_err(to_py_err)
        })
    }

    /// Store a knowledge fact with its embedding and a TTL.
    ///
    /// The entry is automatically eligible for expiry once `ttl_seconds`
    /// have elapsed (enforced on query and by `AgentMemory.auto_expire`).
    ///
    /// Args:
    ///     id: Unique identifier for the fact
    ///     content: Text content of the knowledge
    ///     embedding: Vector representation (list of floats)
    ///     ttl_seconds: Time-to-live in seconds
    ///
    /// Example:
    ///     >>> memory.semantic.store_with_ttl(1, "ephemeral", embedding, 60)
    #[pyo3(signature = (id, content, embedding, ttl_seconds))]
    fn store_with_ttl(
        &self,
        py: Python<'_>,
        id: u64,
        content: &str,
        embedding: Vec<f32>,
        ttl_seconds: u64,
    ) -> PyResult<()> {
        let content_owned = content.to_string();
        py.allow_threads(|| {
            self.inner
                .store_with_ttl(id, &content_owned, &embedding, ttl_seconds)
                .map_err(to_py_err)
        })
    }

    /// Query semantic memory by similarity.
    ///
    /// Args:
    ///     embedding: Query vector
    ///     top_k: Number of results to return (default: 10)
    ///
    /// Returns:
    ///     List of dicts with 'id', 'score', 'content' keys
    ///
    /// Example:
    ///     >>> results = memory.semantic.query(embedding, top_k=5)
    ///     >>> for r in results:
    ///     ...     print(f"{r['content']} (score: {r['score']:.3f})")
    #[pyo3(signature = (embedding, top_k = 10))]
    fn query(&self, py: Python<'_>, embedding: Vec<f32>, top_k: usize) -> PyResult<PyObject> {
        let results =
            py.allow_threads(|| self.inner.query(&embedding, top_k).map_err(to_py_err))?;

        // Phase 3: Build Python objects (GIL held)
        // set_item is infallible on fresh dicts with interned keys and basic Python types.
        let list = pyo3::types::PyList::empty(py);
        for (id, score, content) in results {
            let dict = PyDict::new(py);
            let _ = dict.set_item(PyString::intern(py, "id"), id);
            let _ = dict.set_item(PyString::intern(py, "score"), score);
            let _ = dict.set_item(PyString::intern(py, "content"), content);
            list.append(dict)?;
        }
        Ok(list.into())
    }

    /// Delete a knowledge fact by ID.
    ///
    /// Args:
    ///     id: ID of the fact to delete
    ///
    /// Example:
    ///     >>> memory.semantic.delete(1)
    #[pyo3(signature = (id,))]
    fn delete(&self, py: Python<'_>, id: u64) -> PyResult<()> {
        py.allow_threads(|| self.inner.delete(id).map_err(to_py_err))
    }

    /// Serializes all stored facts to a bytes blob for snapshotting.
    ///
    /// Returns:
    ///     A `bytes` object that can be passed back to `deserialize`.
    fn serialize(&self, py: Python<'_>) -> PyResult<PyObject> {
        let bytes = py.allow_threads(|| self.inner.serialize().map_err(to_py_err))?;
        Ok(pyo3::types::PyBytes::new(py, &bytes).into())
    }

    /// Replaces semantic memory state from a `serialize()` blob.
    ///
    /// Args:
    ///     data: Bytes previously produced by `serialize()`.
    #[pyo3(signature = (data))]
    fn deserialize(&self, py: Python<'_>, data: Vec<u8>) -> PyResult<()> {
        py.allow_threads(|| self.inner.deserialize(&data).map_err(to_py_err))
    }

    fn __repr__(&self) -> String {
        format!("SemanticMemory(dimension={})", self.inner.dimension())
    }
}

/// Python wrapper for EpisodicMemory.
///
/// Records events with timestamps and provides temporal/similarity queries.
/// The core memory object is resolved once when this wrapper is created,
/// avoiding per-method registry lookups.
///
/// Example:
///     >>> memory.episodic.record(1, "User asked about weather", timestamp=1234567890)
///     >>> events = memory.episodic.recent(limit=10)
#[pyclass]
pub struct PyEpisodicMemory {
    inner: CoreEpisodicMemory,
}

#[pymethods]
impl PyEpisodicMemory {
    /// Record an event in episodic memory.
    ///
    /// Args:
    ///     event_id: Unique identifier
    ///     description: Event description
    ///     timestamp: Unix timestamp
    ///     embedding: Optional embedding for similarity search
    ///
    /// Example:
    ///     >>> import time
    ///     >>> memory.episodic.record(1, "User login", int(time.time()))
    #[pyo3(signature = (event_id, description, timestamp, embedding = None))]
    fn record(
        &self,
        py: Python<'_>,
        event_id: u64,
        description: &str,
        timestamp: i64,
        embedding: Option<Vec<f32>>,
    ) -> PyResult<()> {
        let description_owned = description.to_string();
        py.allow_threads(|| {
            let emb_ref = embedding.as_deref();
            self.inner
                .record(event_id, &description_owned, timestamp, emb_ref)
                .map_err(to_py_err)
        })
    }

    /// Get recent events from episodic memory.
    ///
    /// Args:
    ///     limit: Maximum number of events (default: 10)
    ///     since: Only return events after this timestamp
    ///
    /// Returns:
    ///     List of dicts with 'id', 'description', 'timestamp' keys
    ///
    /// Example:
    ///     >>> events = memory.episodic.recent(limit=5)
    #[pyo3(signature = (limit = 10, since = None))]
    fn recent(&self, py: Python<'_>, limit: usize, since: Option<i64>) -> PyResult<PyObject> {
        let results = py.allow_threads(|| self.inner.recent(limit, since).map_err(to_py_err))?;
        events_to_pylist(py, results)
    }

    /// Find similar events by embedding.
    ///
    /// Args:
    ///     embedding: Query vector
    ///     top_k: Number of results (default: 10)
    ///
    /// Returns:
    ///     List of dicts with 'id', 'description', 'timestamp', 'score' keys
    #[pyo3(signature = (embedding, top_k = 10))]
    fn recall_similar(
        &self,
        py: Python<'_>,
        embedding: Vec<f32>,
        top_k: usize,
    ) -> PyResult<PyObject> {
        let results = py.allow_threads(|| {
            self.inner
                .recall_similar(&embedding, top_k)
                .map_err(to_py_err)
        })?;

        let list = pyo3::types::PyList::empty(py);
        for (id, description, timestamp, score) in results {
            let dict = PyDict::new(py);
            let _ = dict.set_item(PyString::intern(py, "id"), id);
            let _ = dict.set_item(PyString::intern(py, "description"), description);
            let _ = dict.set_item(PyString::intern(py, "timestamp"), timestamp);
            let _ = dict.set_item(PyString::intern(py, "score"), score);
            list.append(dict)?;
        }
        Ok(list.into())
    }

    /// Get events older than a given timestamp.
    ///
    /// Args:
    ///     before: Unix timestamp threshold
    ///     limit: Maximum number of events (default: 10)
    ///
    /// Returns:
    ///     List of dicts with 'id', 'description', 'timestamp' keys
    ///
    /// Example:
    ///     >>> old_events = memory.episodic.older_than(before=yesterday, limit=20)
    #[pyo3(signature = (before, limit = 10))]
    fn older_than(&self, py: Python<'_>, before: i64, limit: usize) -> PyResult<PyObject> {
        let results =
            py.allow_threads(|| self.inner.older_than(before, limit).map_err(to_py_err))?;
        events_to_pylist(py, results)
    }

    /// Delete an event by ID.
    ///
    /// Args:
    ///     event_id: ID of the event to delete
    ///
    /// Example:
    ///     >>> memory.episodic.delete(1)
    #[pyo3(signature = (event_id,))]
    fn delete(&self, py: Python<'_>, event_id: u64) -> PyResult<()> {
        py.allow_threads(|| self.inner.delete(event_id).map_err(to_py_err))
    }

    fn __repr__(&self) -> String {
        format!("EpisodicMemory(dimension={})", self.inner.dimension())
    }
}

/// Python wrapper for ProceduralMemory.
///
/// Stores learned patterns with confidence scoring and reinforcement.
/// The core memory object is resolved once when this wrapper is created,
/// avoiding per-method registry lookups.
///
/// Example:
///     >>> memory.procedural.learn(1, "greet_user", ["say hello", "ask name"], confidence=0.8)
///     >>> patterns = memory.procedural.recall(embedding, min_confidence=0.5)
#[pyclass]
pub struct PyProceduralMemory {
    inner: CoreProceduralMemory,
}

#[pymethods]
impl PyProceduralMemory {
    /// Learn a new procedure/pattern.
    ///
    /// Args:
    ///     procedure_id: Unique identifier
    ///     name: Human-readable name
    ///     steps: List of action steps
    ///     embedding: Optional embedding for similarity matching
    ///     confidence: Initial confidence (0.0-1.0, default: 0.5)
    ///
    /// Example:
    ///     >>> memory.procedural.learn(1, "greet", ["wave", "say hi"], confidence=0.8)
    #[pyo3(signature = (procedure_id, name, steps, embedding = None, confidence = 0.5))]
    fn learn(
        &self,
        py: Python<'_>,
        procedure_id: u64,
        name: &str,
        steps: Vec<String>,
        embedding: Option<Vec<f32>>,
        confidence: f32,
    ) -> PyResult<()> {
        let name_owned = name.to_string();
        py.allow_threads(|| {
            let emb_ref = embedding.as_deref();
            self.inner
                .learn(procedure_id, &name_owned, &steps, emb_ref, confidence)
                .map_err(to_py_err)
        })
    }

    /// Recall procedures by similarity.
    ///
    /// Args:
    ///     embedding: Query vector
    ///     top_k: Number of results (default: 10)
    ///     min_confidence: Minimum confidence threshold (default: 0.0)
    ///
    /// Returns:
    ///     List of dicts with 'id', 'name', 'steps', 'confidence', 'score' keys
    ///
    /// Example:
    ///     >>> patterns = memory.procedural.recall(embedding, min_confidence=0.7)
    #[pyo3(signature = (embedding, top_k = 10, min_confidence = 0.0))]
    fn recall(
        &self,
        py: Python<'_>,
        embedding: Vec<f32>,
        top_k: usize,
        min_confidence: f32,
    ) -> PyResult<PyObject> {
        let results = py.allow_threads(|| {
            self.inner
                .recall(&embedding, top_k, min_confidence)
                .map_err(to_py_err)
        })?;
        procedures_to_pylist(py, results)
    }

    /// Reinforce a procedure based on success/failure.
    ///
    /// Updates confidence: +0.1 on success, -0.05 on failure.
    ///
    /// Args:
    ///     procedure_id: ID of the procedure to reinforce
    ///     success: True if the procedure succeeded, False otherwise
    ///
    /// Example:
    ///     >>> memory.procedural.reinforce(1, success=True)
    #[pyo3(signature = (procedure_id, success))]
    fn reinforce(&self, py: Python<'_>, procedure_id: u64, success: bool) -> PyResult<()> {
        py.allow_threads(|| {
            self.inner
                .reinforce(procedure_id, success)
                .map_err(to_py_err)
        })
    }

    /// List all stored procedures.
    ///
    /// Returns:
    ///     List of dicts with 'id', 'name', 'steps', 'confidence', 'score' keys
    ///
    /// Example:
    ///     >>> all_procs = memory.procedural.list_all()
    fn list_all(&self, py: Python<'_>) -> PyResult<PyObject> {
        let results = py.allow_threads(|| self.inner.list_all().map_err(to_py_err))?;
        procedures_to_pylist(py, results)
    }

    /// Delete a procedure by ID.
    ///
    /// Args:
    ///     procedure_id: ID of the procedure to delete
    ///
    /// Example:
    ///     >>> memory.procedural.delete(1)
    #[pyo3(signature = (procedure_id,))]
    fn delete(&self, py: Python<'_>, procedure_id: u64) -> PyResult<()> {
        py.allow_threads(|| self.inner.delete(procedure_id).map_err(to_py_err))
    }

    fn __repr__(&self) -> String {
        format!("ProceduralMemory(dimension={})", self.inner.dimension())
    }
}
