//! AgentMemory - Unified memory interface for AI agents (EPIC-010)
//!
//! Provides three memory subsystems for AI agents:
//! - **SemanticMemory**: Long-term knowledge facts with vector similarity search
//! - **EpisodicMemory**: Event timeline with temporal and similarity queries
//! - **ProceduralMemory**: Learned patterns with confidence scoring

use crate::{Database, DistanceMetric, Point};
use serde_json::json;
use thiserror::Error;

/// Default embedding dimension for memory collections.
pub const DEFAULT_DIMENSION: usize = 384;

/// A matched procedure from procedural memory recall.
#[derive(Debug, Clone)]
pub struct ProcedureMatch {
    /// Procedure ID.
    pub id: u64,
    /// Procedure name.
    pub name: String,
    /// Action steps.
    pub steps: Vec<String>,
    /// Confidence score (0.0 - 1.0).
    pub confidence: f32,
    /// Similarity score from search.
    pub score: f32,
}

/// Error type for AgentMemory operations
#[derive(Debug, Error)]
pub enum AgentMemoryError {
    /// Memory initialization failed.
    #[error("Failed to initialize memory: {0}")]
    InitializationError(String),

    /// Collection operation failed.
    #[error("Collection error: {0}")]
    CollectionError(String),

    /// Item not found.
    #[error("Item not found: {0}")]
    NotFound(String),

    /// Invalid embedding dimension.
    #[error("Invalid embedding dimension: expected {expected}, got {actual}")]
    DimensionMismatch {
        /// Expected dimension.
        expected: usize,
        /// Actual dimension provided.
        actual: usize,
    },

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
    ///
    /// Uses default dimension (384) for embeddings. Use `with_dimension` for custom sizes.
    pub fn new(db: &'a Database) -> Result<Self, AgentMemoryError> {
        Self::with_dimension(db, DEFAULT_DIMENSION)
    }

    /// Creates a new AgentMemory with custom embedding dimension.
    pub fn with_dimension(db: &'a Database, dimension: usize) -> Result<Self, AgentMemoryError> {
        let semantic = SemanticMemory::new(db, dimension)?;
        let episodic = EpisodicMemory::new(db, dimension)?;
        let procedural = ProceduralMemory::new(db, dimension)?;

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

/// Semantic Memory - Long-term knowledge storage (US-002)
///
/// Stores facts and knowledge as vectors with similarity search.
/// Each fact has an ID, content text, and embedding vector.
pub struct SemanticMemory<'a> {
    collection_name: String,
    db: &'a Database,
    dimension: usize,
}

impl<'a> SemanticMemory<'a> {
    const COLLECTION_NAME: &'static str = "_semantic_memory";

    /// Creates a new SemanticMemory from a Database reference.
    ///
    /// This is the public constructor for use in Python bindings.
    pub fn new_from_db(db: &'a Database, dimension: usize) -> Result<Self, AgentMemoryError> {
        Self::new(db, dimension)
    }

    fn new(db: &'a Database, dimension: usize) -> Result<Self, AgentMemoryError> {
        let collection_name = Self::COLLECTION_NAME.to_string();

        // Create collection if it doesn't exist, or validate dimension matches
        let actual_dimension = if let Some(collection) = db.get_collection(&collection_name) {
            let existing_dim = collection.config().dimension;
            if existing_dim != dimension {
                return Err(AgentMemoryError::DimensionMismatch {
                    expected: existing_dim,
                    actual: dimension,
                });
            }
            existing_dim
        } else {
            db.create_collection(&collection_name, dimension, DistanceMetric::Cosine)?;
            dimension
        };

        Ok(Self {
            collection_name,
            db,
            dimension: actual_dimension,
        })
    }

    /// Returns the name of the underlying collection.
    #[must_use]
    pub fn collection_name(&self) -> &str {
        &self.collection_name
    }

    /// Returns the embedding dimension.
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// Stores a knowledge fact with its embedding vector.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier for this fact
    /// * `content` - Text content of the knowledge
    /// * `embedding` - Vector representation of the content
    ///
    /// # Errors
    ///
    /// Returns error if embedding dimension doesn't match or collection operation fails.
    pub fn store(&self, id: u64, content: &str, embedding: &[f32]) -> Result<(), AgentMemoryError> {
        if embedding.len() != self.dimension {
            return Err(AgentMemoryError::DimensionMismatch {
                expected: self.dimension,
                actual: embedding.len(),
            });
        }

        let collection = self
            .db
            .get_collection(&self.collection_name)
            .ok_or_else(|| AgentMemoryError::CollectionError("Collection not found".to_string()))?;

        let point = Point::new(id, embedding.to_vec(), Some(json!({"content": content})));
        collection
            .upsert(vec![point])
            .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))?;

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
    /// Vector of (id, score, content) tuples ordered by similarity.
    pub fn query(
        &self,
        query_embedding: &[f32],
        k: usize,
    ) -> Result<Vec<(u64, f32, String)>, AgentMemoryError> {
        if query_embedding.len() != self.dimension {
            return Err(AgentMemoryError::DimensionMismatch {
                expected: self.dimension,
                actual: query_embedding.len(),
            });
        }

        let collection = self
            .db
            .get_collection(&self.collection_name)
            .ok_or_else(|| AgentMemoryError::CollectionError("Collection not found".to_string()))?;

        let results = collection
            .search(query_embedding, k)
            .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))?;

        Ok(results
            .into_iter()
            .map(|r| {
                let content = r
                    .point
                    .payload
                    .as_ref()
                    .and_then(|p| p.get("content"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                (r.point.id, r.score, content)
            })
            .collect())
    }
}

/// Episodic Memory - Event timeline storage (US-003)
///
/// Records events with timestamps and contextual information.
/// Supports temporal queries and similarity-based retrieval.
pub struct EpisodicMemory<'a> {
    collection_name: String,
    db: &'a Database,
    dimension: usize,
}

impl<'a> EpisodicMemory<'a> {
    const COLLECTION_NAME: &'static str = "_episodic_memory";

    /// Creates a new EpisodicMemory from a Database reference.
    ///
    /// This is the public constructor for use in Python bindings.
    pub fn new_from_db(db: &'a Database, dimension: usize) -> Result<Self, AgentMemoryError> {
        Self::new(db, dimension)
    }

    fn new(db: &'a Database, dimension: usize) -> Result<Self, AgentMemoryError> {
        let collection_name = Self::COLLECTION_NAME.to_string();

        // Create collection if it doesn't exist, or validate dimension matches
        let actual_dimension = if let Some(collection) = db.get_collection(&collection_name) {
            let existing_dim = collection.config().dimension;
            if existing_dim != dimension {
                return Err(AgentMemoryError::DimensionMismatch {
                    expected: existing_dim,
                    actual: dimension,
                });
            }
            existing_dim
        } else {
            db.create_collection(&collection_name, dimension, DistanceMetric::Cosine)?;
            dimension
        };

        Ok(Self {
            collection_name,
            db,
            dimension: actual_dimension,
        })
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
    /// * `embedding` - Optional vector representation for similarity search
    ///
    /// # Errors
    ///
    /// Returns error if embedding dimension doesn't match or collection operation fails.
    pub fn record(
        &self,
        event_id: u64,
        description: &str,
        timestamp: i64,
        embedding: Option<&[f32]>,
    ) -> Result<(), AgentMemoryError> {
        // Validate embedding dimension if provided
        if let Some(emb) = embedding {
            if emb.len() != self.dimension {
                return Err(AgentMemoryError::DimensionMismatch {
                    expected: self.dimension,
                    actual: emb.len(),
                });
            }
        }

        let collection = self
            .db
            .get_collection(&self.collection_name)
            .ok_or_else(|| AgentMemoryError::CollectionError("Collection not found".to_string()))?;

        // Use zero vector if no embedding provided (allows temporal-only queries)
        let vector = embedding.map_or_else(|| vec![0.0; self.dimension], <[f32]>::to_vec);

        let point = Point::new(
            event_id,
            vector,
            Some(json!({
                "description": description,
                "timestamp": timestamp
            })),
        );

        collection
            .upsert(vec![point])
            .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))?;

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
    /// Vector of (event_id, description, timestamp) tuples ordered by timestamp descending.
    pub fn recent(
        &self,
        limit: usize,
        since_timestamp: Option<i64>,
    ) -> Result<Vec<(u64, String, i64)>, AgentMemoryError> {
        let collection = self
            .db
            .get_collection(&self.collection_name)
            .ok_or_else(|| AgentMemoryError::CollectionError("Collection not found".to_string()))?;

        // Get points by scanning reasonable ID range
        // Note: For large datasets, this should use a proper index
        let all_ids: Vec<u64> = (0..10000).collect();
        let points = collection.get(&all_ids);

        let mut events: Vec<(u64, String, i64)> = points
            .into_iter()
            .flatten() // Filter out None values
            .filter_map(|p| {
                let payload = p.payload.as_ref()?;
                let desc = payload.get("description")?.as_str()?.to_string();
                let ts = payload.get("timestamp")?.as_i64()?;

                // Apply timestamp filter
                if let Some(since) = since_timestamp {
                    if ts <= since {
                        return None;
                    }
                }

                Some((p.id, desc, ts))
            })
            .collect();

        // Sort by timestamp descending (most recent first)
        events.sort_by(|a, b| b.2.cmp(&a.2));
        events.truncate(limit);

        Ok(events)
    }

    /// Retrieves events similar to a query embedding.
    ///
    /// # Arguments
    ///
    /// * `query_embedding` - Vector to search for similar events
    /// * `k` - Number of results to return
    ///
    /// # Returns
    ///
    /// Vector of (event_id, description, timestamp, score) tuples.
    pub fn recall_similar(
        &self,
        query_embedding: &[f32],
        k: usize,
    ) -> Result<Vec<(u64, String, i64, f32)>, AgentMemoryError> {
        if query_embedding.len() != self.dimension {
            return Err(AgentMemoryError::DimensionMismatch {
                expected: self.dimension,
                actual: query_embedding.len(),
            });
        }

        let collection = self
            .db
            .get_collection(&self.collection_name)
            .ok_or_else(|| AgentMemoryError::CollectionError("Collection not found".to_string()))?;

        let results = collection
            .search(query_embedding, k)
            .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))?;

        Ok(results
            .into_iter()
            .filter_map(|r| {
                let payload = r.point.payload.as_ref()?;
                let desc = payload.get("description")?.as_str()?.to_string();
                let ts = payload.get("timestamp")?.as_i64()?;
                Some((r.point.id, desc, ts, r.score))
            })
            .collect())
    }
}

/// Procedural Memory - Learned patterns storage (US-004)
///
/// Stores action sequences and learned procedures with confidence scoring.
/// Supports pattern matching by similarity and reinforcement learning.
pub struct ProceduralMemory<'a> {
    collection_name: String,
    db: &'a Database,
    dimension: usize,
}

impl<'a> ProceduralMemory<'a> {
    const COLLECTION_NAME: &'static str = "_procedural_memory";

    /// Creates a new ProceduralMemory from a Database reference.
    ///
    /// This is the public constructor for use in Python bindings.
    pub fn new_from_db(db: &'a Database, dimension: usize) -> Result<Self, AgentMemoryError> {
        Self::new(db, dimension)
    }

    fn new(db: &'a Database, dimension: usize) -> Result<Self, AgentMemoryError> {
        let collection_name = Self::COLLECTION_NAME.to_string();

        // Create collection if it doesn't exist, or validate dimension matches
        let actual_dimension = if let Some(collection) = db.get_collection(&collection_name) {
            let existing_dim = collection.config().dimension;
            if existing_dim != dimension {
                return Err(AgentMemoryError::DimensionMismatch {
                    expected: existing_dim,
                    actual: dimension,
                });
            }
            existing_dim
        } else {
            db.create_collection(&collection_name, dimension, DistanceMetric::Cosine)?;
            dimension
        };

        Ok(Self {
            collection_name,
            db,
            dimension: actual_dimension,
        })
    }

    /// Returns the name of the underlying collection.
    #[must_use]
    pub fn collection_name(&self) -> &str {
        &self.collection_name
    }

    /// Learns a new procedure/pattern with optional embedding.
    ///
    /// # Arguments
    ///
    /// * `procedure_id` - Unique identifier for this procedure
    /// * `name` - Human-readable name
    /// * `steps` - Sequence of action steps (JSON array)
    /// * `embedding` - Optional vector for similarity matching
    /// * `confidence` - Initial confidence score (0.0 - 1.0)
    ///
    /// # Errors
    ///
    /// Returns error if embedding dimension doesn't match or collection operation fails.
    pub fn learn(
        &self,
        procedure_id: u64,
        name: &str,
        steps: &[String],
        embedding: Option<&[f32]>,
        confidence: f32,
    ) -> Result<(), AgentMemoryError> {
        // Validate embedding dimension if provided
        if let Some(emb) = embedding {
            if emb.len() != self.dimension {
                return Err(AgentMemoryError::DimensionMismatch {
                    expected: self.dimension,
                    actual: emb.len(),
                });
            }
        }

        let collection = self
            .db
            .get_collection(&self.collection_name)
            .ok_or_else(|| AgentMemoryError::CollectionError("Collection not found".to_string()))?;

        // Use zero vector if no embedding provided
        let vector = embedding.map_or_else(|| vec![0.0; self.dimension], <[f32]>::to_vec);

        let point = Point::new(
            procedure_id,
            vector,
            Some(json!({
                "name": name,
                "steps": steps,
                "confidence": confidence,
                "usage_count": 0
            })),
        );

        collection
            .upsert(vec![point])
            .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))?;

        Ok(())
    }

    /// Retrieves procedures by similarity search.
    ///
    /// # Arguments
    ///
    /// * `query_embedding` - Vector to search for similar procedures
    /// * `k` - Maximum number of results
    /// * `min_confidence` - Minimum confidence threshold (0.0 - 1.0)
    ///
    /// # Returns
    ///
    /// Vector of (procedure_id, name, steps, confidence, score) tuples.
    pub fn recall(
        &self,
        query_embedding: &[f32],
        k: usize,
        min_confidence: f32,
    ) -> Result<Vec<ProcedureMatch>, AgentMemoryError> {
        if query_embedding.len() != self.dimension {
            return Err(AgentMemoryError::DimensionMismatch {
                expected: self.dimension,
                actual: query_embedding.len(),
            });
        }

        let collection = self
            .db
            .get_collection(&self.collection_name)
            .ok_or_else(|| AgentMemoryError::CollectionError("Collection not found".to_string()))?;

        let results = collection
            .search(query_embedding, k)
            .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))?;

        Ok(results
            .into_iter()
            .filter_map(|r| {
                let payload = r.point.payload.as_ref()?;
                let name = payload.get("name")?.as_str()?.to_string();
                let steps: Vec<String> = payload
                    .get("steps")?
                    .as_array()?
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
                let confidence = payload.get("confidence")?.as_f64()? as f32;

                // Filter by minimum confidence
                if confidence < min_confidence {
                    return None;
                }

                Some(ProcedureMatch {
                    id: r.point.id,
                    name,
                    steps,
                    confidence,
                    score: r.score,
                })
            })
            .collect())
    }

    /// Reinforces a procedure based on success/failure feedback.
    ///
    /// Updates confidence: +0.1 on success, -0.05 on failure (clamped to 0.0-1.0).
    /// Also increments usage count.
    pub fn reinforce(&self, procedure_id: u64, success: bool) -> Result<(), AgentMemoryError> {
        let collection = self
            .db
            .get_collection(&self.collection_name)
            .ok_or_else(|| AgentMemoryError::CollectionError("Collection not found".to_string()))?;

        // Get current procedure
        let points = collection.get(&[procedure_id]);
        let point = points
            .into_iter()
            .flatten()
            .next()
            .ok_or_else(|| AgentMemoryError::NotFound(format!("Procedure {procedure_id}")))?;

        let payload = point
            .payload
            .as_ref()
            .ok_or_else(|| AgentMemoryError::CollectionError("Missing payload".to_string()))?;

        // Extract current values
        let name = payload
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let steps: Vec<String> = payload
            .get("steps")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let old_confidence = payload
            .get("confidence")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.5) as f32;
        let usage_count = payload
            .get("usage_count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);

        // Update confidence
        let new_confidence = if success {
            (old_confidence + 0.1).min(1.0)
        } else {
            (old_confidence - 0.05).max(0.0)
        };

        // Update point with new values
        let updated_point = Point::new(
            procedure_id,
            point.vector.clone(),
            Some(json!({
                "name": name,
                "steps": steps,
                "confidence": new_confidence,
                "usage_count": usage_count + 1
            })),
        );

        collection
            .upsert(vec![updated_point])
            .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))?;

        Ok(())
    }
}
