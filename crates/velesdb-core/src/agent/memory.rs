//! AgentMemory - Unified memory interface for AI agents (EPIC-010/US-001)

use crate::Database;
use thiserror::Error;

/// Error type for AgentMemory operations
#[derive(Debug, Error)]
pub enum AgentMemoryError {
    /// Memory initialization failed.
    #[error("Failed to initialize memory: {0}")]
    InitializationError(String),

    /// Collection operation failed.
    #[error("Collection error: {0}")]
    CollectionError(String),

    /// Underlying database error.
    #[error("Database error: {0}")]
    DatabaseError(#[from] crate::error::Error),
}

/// Unified memory interface for AI agents.
///
/// Provides access to three memory subsystems:
/// - `semantic`: Long-term knowledge (vector-graph storage)
/// - `episodic`: Event timeline with temporal context
/// - `procedural`: Learned patterns and action sequences
///
/// Uses lifetime `'a` to borrow the Database without cloning.
pub struct AgentMemory<'a> {
    semantic: SemanticMemory<'a>,
    episodic: EpisodicMemory<'a>,
    procedural: ProceduralMemory<'a>,
}

impl<'a> AgentMemory<'a> {
    /// Creates a new AgentMemory instance from a Database.
    ///
    /// Initializes or connects to the three memory subsystem collections:
    /// - `_semantic_memory`: For knowledge facts
    /// - `_episodic_memory`: For event sequences
    /// - `_procedural_memory`: For learned procedures
    pub fn new(db: &'a Database) -> Result<Self, AgentMemoryError> {
        let semantic = SemanticMemory::new(db);
        let episodic = EpisodicMemory::new(db);
        let procedural = ProceduralMemory::new(db);

        Ok(Self {
            semantic,
            episodic,
            procedural,
        })
    }

    /// Returns a reference to the semantic memory subsystem.
    #[must_use]
    pub fn semantic(&self) -> &SemanticMemory<'a> {
        &self.semantic
    }

    /// Returns a reference to the episodic memory subsystem.
    #[must_use]
    pub fn episodic(&self) -> &EpisodicMemory<'a> {
        &self.episodic
    }

    /// Returns a reference to the procedural memory subsystem.
    #[must_use]
    pub fn procedural(&self) -> &ProceduralMemory<'a> {
        &self.procedural
    }
}

/// Semantic Memory - Long-term knowledge storage
///
/// Stores facts and knowledge as vectors with graph relationships.
/// Supports similarity search and knowledge graph traversal.
pub struct SemanticMemory<'a> {
    collection_name: String,
    #[allow(dead_code)]
    db: &'a Database,
}

impl<'a> SemanticMemory<'a> {
    fn new(db: &'a Database) -> Self {
        Self {
            collection_name: "_semantic_memory".to_string(),
            db,
        }
    }

    /// Returns the name of the underlying collection.
    #[must_use]
    pub fn collection_name(&self) -> &str {
        &self.collection_name
    }

    /// Stores a knowledge fact with its embedding vector.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier for this fact
    /// * `content` - Text content of the knowledge
    /// * `embedding` - Vector representation of the content
    ///
    /// # Note
    ///
    /// Full implementation in US-002.
    #[allow(clippy::unnecessary_wraps)]
    pub fn store(
        &self,
        _id: u64,
        _content: &str,
        _embedding: &[f32],
    ) -> Result<(), AgentMemoryError> {
        // TODO(US-002): Implement semantic memory storage
        Ok(())
    }

    /// Queries semantic memory by similarity search.
    ///
    /// # Arguments
    ///
    /// * `query_embedding` - Vector to search for
    /// * `k` - Number of results to return
    ///
    /// # Returns
    ///
    /// Vector of (id, score, content) tuples.
    ///
    /// # Note
    ///
    /// Full implementation in US-002.
    #[allow(clippy::unnecessary_wraps)]
    pub fn query(
        &self,
        _query_embedding: &[f32],
        _k: usize,
    ) -> Result<Vec<(u64, f32, String)>, AgentMemoryError> {
        // TODO(US-002): Implement semantic memory query
        Ok(Vec::new())
    }
}

/// Episodic Memory - Event timeline storage
///
/// Records events with timestamps and contextual information.
/// Supports temporal queries and event sequence retrieval.
pub struct EpisodicMemory<'a> {
    collection_name: String,
    #[allow(dead_code)]
    db: &'a Database,
}

impl<'a> EpisodicMemory<'a> {
    fn new(db: &'a Database) -> Self {
        Self {
            collection_name: "_episodic_memory".to_string(),
            db,
        }
    }

    /// Returns the name of the underlying collection.
    #[must_use]
    pub fn collection_name(&self) -> &str {
        &self.collection_name
    }

    /// Records an event in episodic memory.
    ///
    /// # Arguments
    ///
    /// * `event_id` - Unique identifier for this event
    /// * `description` - Text description of the event
    /// * `timestamp` - Unix timestamp of the event
    /// * `embedding` - Optional vector representation
    ///
    /// # Note
    ///
    /// Full implementation in US-003.
    #[allow(clippy::unnecessary_wraps)]
    pub fn record(
        &self,
        _event_id: u64,
        _description: &str,
        _timestamp: i64,
        _embedding: Option<&[f32]>,
    ) -> Result<(), AgentMemoryError> {
        // TODO(US-003): Implement episodic memory storage
        Ok(())
    }

    /// Retrieves recent events from episodic memory.
    ///
    /// # Arguments
    ///
    /// * `limit` - Maximum number of events to return
    /// * `since_timestamp` - Optional filter for events after this time
    ///
    /// # Returns
    ///
    /// Vector of (event_id, description, timestamp) tuples.
    ///
    /// # Note
    ///
    /// Full implementation in US-003.
    #[allow(clippy::unnecessary_wraps)]
    pub fn recent(
        &self,
        _limit: usize,
        _since_timestamp: Option<i64>,
    ) -> Result<Vec<(u64, String, i64)>, AgentMemoryError> {
        // TODO(US-003): Implement episodic memory retrieval
        Ok(Vec::new())
    }
}

/// Procedural Memory - Learned patterns storage
///
/// Stores action sequences and learned procedures.
/// Supports pattern matching and procedure retrieval.
pub struct ProceduralMemory<'a> {
    collection_name: String,
    #[allow(dead_code)]
    db: &'a Database,
}

impl<'a> ProceduralMemory<'a> {
    fn new(db: &'a Database) -> Self {
        Self {
            collection_name: "_procedural_memory".to_string(),
            db,
        }
    }

    /// Returns the name of the underlying collection.
    #[must_use]
    pub fn collection_name(&self) -> &str {
        &self.collection_name
    }

    /// Learns a new procedure/pattern.
    ///
    /// # Arguments
    ///
    /// * `procedure_id` - Unique identifier for this procedure
    /// * `name` - Human-readable name
    /// * `steps` - Sequence of action steps
    /// * `embedding` - Optional vector representation
    ///
    /// # Note
    ///
    /// Full implementation in US-004.
    #[allow(clippy::unnecessary_wraps)]
    pub fn learn(
        &self,
        _procedure_id: u64,
        _name: &str,
        _steps: &[String],
        _embedding: Option<&[f32]>,
    ) -> Result<(), AgentMemoryError> {
        // TODO(US-004): Implement procedural memory storage
        Ok(())
    }

    /// Retrieves a procedure by similarity or name.
    ///
    /// # Arguments
    ///
    /// * `query_embedding` - Optional vector to search for similar procedures
    /// * `name_filter` - Optional name prefix filter
    /// * `k` - Maximum number of results
    ///
    /// # Returns
    ///
    /// Vector of (procedure_id, name, steps) tuples.
    ///
    /// # Note
    ///
    /// Full implementation in US-004.
    #[allow(clippy::unnecessary_wraps)]
    pub fn recall(
        &self,
        _query_embedding: Option<&[f32]>,
        _name_filter: Option<&str>,
        _k: usize,
    ) -> Result<Vec<(u64, String, Vec<String>)>, AgentMemoryError> {
        // TODO(US-004): Implement procedural memory retrieval
        Ok(Vec::new())
    }
}
