//! Python bindings for AgentMemory (EPIC-010/US-005)
//!
//! Provides Pythonic access to VelesDB's agent memory subsystems:
//! - SemanticMemory: Long-term knowledge facts
//! - EpisodicMemory: Event timeline
//! - ProceduralMemory: Learned patterns

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::sync::Arc;
use velesdb_core::agent::{
    AgentMemory as CoreAgentMemory, AgentMemoryError, EpisodicMemory as CoreEpisodicMemory,
    ProceduralMemory as CoreProceduralMemory, SemanticMemory as CoreSemanticMemory,
    DEFAULT_DIMENSION,
};
use velesdb_core::Database as CoreDatabase;

/// Convert AgentMemoryError to PyErr
fn to_py_err(e: AgentMemoryError) -> PyErr {
    PyRuntimeError::new_err(format!("{e}"))
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
    dimension: usize,
}

#[pymethods]
impl AgentMemory {
    /// Create a new AgentMemory from a Database.
    ///
    /// Args:
    ///     db: Database instance
    ///     dimension: Embedding dimension (default: 384)
    ///
    /// Example:
    ///     >>> memory = AgentMemory(db)
    ///     >>> memory = AgentMemory(db, dimension=768)
    #[new]
    #[pyo3(signature = (db, dimension = None))]
    pub fn new(db: &crate::Database, dimension: Option<usize>) -> PyResult<Self> {
        let dim = dimension.unwrap_or(DEFAULT_DIMENSION);

        // Initialize memory subsystems by creating a temporary CoreAgentMemory
        // This ensures collections are created
        let core_db = db.inner();
        CoreAgentMemory::with_dimension(core_db, dim).map_err(to_py_err)?;

        // Reopen database to get an owned instance
        // This is necessary because PyO3 classes can't have lifetime parameters
        let owned_db = CoreDatabase::open(db.path())
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to reopen database: {e}")))?;

        Ok(Self {
            db: Arc::new(owned_db),
            dimension: dim,
        })
    }

    /// Returns the semantic memory subsystem.
    #[getter]
    fn semantic(&self) -> PyResult<PySemanticMemory> {
        Ok(PySemanticMemory {
            db: Arc::clone(&self.db),
            dimension: self.dimension,
        })
    }

    /// Returns the episodic memory subsystem.
    #[getter]
    fn episodic(&self) -> PyResult<PyEpisodicMemory> {
        Ok(PyEpisodicMemory {
            db: Arc::clone(&self.db),
            dimension: self.dimension,
        })
    }

    /// Returns the procedural memory subsystem.
    #[getter]
    fn procedural(&self) -> PyResult<PyProceduralMemory> {
        Ok(PyProceduralMemory {
            db: Arc::clone(&self.db),
            dimension: self.dimension,
        })
    }

    /// Returns the embedding dimension.
    #[getter]
    fn dimension(&self) -> usize {
        self.dimension
    }

    fn __repr__(&self) -> String {
        format!("AgentMemory(dimension={})", self.dimension)
    }
}

/// Python wrapper for SemanticMemory.
///
/// Stores long-term knowledge facts with vector similarity search.
///
/// Example:
///     >>> memory.semantic.store(1, "The sky is blue", [0.1, 0.2, ...])
///     >>> results = memory.semantic.query([0.1, 0.2, ...], top_k=5)
#[pyclass]
pub struct PySemanticMemory {
    db: Arc<CoreDatabase>,
    dimension: usize,
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
    fn store(&self, id: u64, content: &str, embedding: Vec<f32>) -> PyResult<()> {
        let memory = self.get_core_memory()?;
        memory.store(id, content, &embedding).map_err(to_py_err)
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
        let memory = self.get_core_memory()?;
        let results = memory.query(&embedding, top_k).map_err(to_py_err)?;

        let list = pyo3::types::PyList::empty(py);
        for (id, score, content) in results {
            let dict = PyDict::new(py);
            dict.set_item("id", id)?;
            dict.set_item("score", score)?;
            dict.set_item("content", content)?;
            list.append(dict)?;
        }
        Ok(list.into())
    }

    fn __repr__(&self) -> String {
        format!("SemanticMemory(dimension={})", self.dimension)
    }
}

impl PySemanticMemory {
    fn get_core_memory(&self) -> PyResult<CoreSemanticMemory<'_>> {
        // SAFETY: We own the Arc, so the pointer is valid for the lifetime of self
        let db_ref = unsafe { &*Arc::as_ptr(&self.db) };
        CoreSemanticMemory::new_from_db(db_ref, self.dimension).map_err(to_py_err)
    }
}

/// Python wrapper for EpisodicMemory.
///
/// Records events with timestamps and provides temporal/similarity queries.
///
/// Example:
///     >>> memory.episodic.record(1, "User asked about weather", timestamp=1234567890)
///     >>> events = memory.episodic.recent(limit=10)
#[pyclass]
pub struct PyEpisodicMemory {
    db: Arc<CoreDatabase>,
    dimension: usize,
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
        event_id: u64,
        description: &str,
        timestamp: i64,
        embedding: Option<Vec<f32>>,
    ) -> PyResult<()> {
        let memory = self.get_core_memory()?;
        let emb_ref = embedding.as_deref();
        memory
            .record(event_id, description, timestamp, emb_ref)
            .map_err(to_py_err)
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
        let memory = self.get_core_memory()?;
        let results = memory.recent(limit, since).map_err(to_py_err)?;

        let list = pyo3::types::PyList::empty(py);
        for (id, description, timestamp) in results {
            let dict = PyDict::new(py);
            dict.set_item("id", id)?;
            dict.set_item("description", description)?;
            dict.set_item("timestamp", timestamp)?;
            list.append(dict)?;
        }
        Ok(list.into())
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
        let memory = self.get_core_memory()?;
        let results = memory
            .recall_similar(&embedding, top_k)
            .map_err(to_py_err)?;

        let list = pyo3::types::PyList::empty(py);
        for (id, description, timestamp, score) in results {
            let dict = PyDict::new(py);
            dict.set_item("id", id)?;
            dict.set_item("description", description)?;
            dict.set_item("timestamp", timestamp)?;
            dict.set_item("score", score)?;
            list.append(dict)?;
        }
        Ok(list.into())
    }

    fn __repr__(&self) -> String {
        format!("EpisodicMemory(dimension={})", self.dimension)
    }
}

impl PyEpisodicMemory {
    fn get_core_memory(&self) -> PyResult<CoreEpisodicMemory<'_>> {
        let db_ref = unsafe { &*Arc::as_ptr(&self.db) };
        CoreEpisodicMemory::new_from_db(db_ref, self.dimension).map_err(to_py_err)
    }
}

/// Python wrapper for ProceduralMemory.
///
/// Stores learned patterns with confidence scoring and reinforcement.
///
/// Example:
///     >>> memory.procedural.learn(1, "greet_user", ["say hello", "ask name"], confidence=0.8)
///     >>> patterns = memory.procedural.recall(embedding, min_confidence=0.5)
#[pyclass]
pub struct PyProceduralMemory {
    db: Arc<CoreDatabase>,
    dimension: usize,
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
        procedure_id: u64,
        name: &str,
        steps: Vec<String>,
        embedding: Option<Vec<f32>>,
        confidence: f32,
    ) -> PyResult<()> {
        let memory = self.get_core_memory()?;
        let emb_ref = embedding.as_deref();
        memory
            .learn(procedure_id, name, &steps, emb_ref, confidence)
            .map_err(to_py_err)
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
        let memory = self.get_core_memory()?;
        let results = memory
            .recall(&embedding, top_k, min_confidence)
            .map_err(to_py_err)?;

        let list = pyo3::types::PyList::empty(py);
        for m in results {
            let dict = PyDict::new(py);
            dict.set_item("id", m.id)?;
            dict.set_item("name", &m.name)?;
            dict.set_item("steps", &m.steps)?;
            dict.set_item("confidence", m.confidence)?;
            dict.set_item("score", m.score)?;
            list.append(dict)?;
        }
        Ok(list.into())
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
    fn reinforce(&self, procedure_id: u64, success: bool) -> PyResult<()> {
        let memory = self.get_core_memory()?;
        memory.reinforce(procedure_id, success).map_err(to_py_err)
    }

    fn __repr__(&self) -> String {
        format!("ProceduralMemory(dimension={})", self.dimension)
    }
}

impl PyProceduralMemory {
    fn get_core_memory(&self) -> PyResult<CoreProceduralMemory<'_>> {
        let db_ref = unsafe { &*Arc::as_ptr(&self.db) };
        CoreProceduralMemory::new_from_db(db_ref, self.dimension).map_err(to_py_err)
    }
}
