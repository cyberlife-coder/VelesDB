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
    pub fn collection_name(&self) -> &str {
        &self.collection_name
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
    pub fn collection_name(&self) -> &str {
        &self.collection_name
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
    pub fn collection_name(&self) -> &str {
        &self.collection_name
    }
}
