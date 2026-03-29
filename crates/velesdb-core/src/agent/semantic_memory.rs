//! Semantic Memory - Long-term knowledge storage (US-002)
//!
//! Stores facts and knowledge as vectors with similarity search.
//! Each fact has an ID, content text, and embedding vector.

use crate::{Database, Point};
use parking_lot::RwLock;
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;

use super::error::AgentMemoryError;
use super::memory_helpers;
use super::ttl::MemoryTtl;

/// Long-term semantic memory for storing knowledge facts with vector similarity search.
///
/// Each fact is stored as an embedding vector with associated text content.
/// Supports TTL-based expiration and snapshot serialization.
pub struct SemanticMemory {
    collection_name: String,
    db: Arc<Database>,
    dimension: usize,
    ttl: Arc<MemoryTtl>,
    stored_ids: RwLock<HashSet<u64>>,
}

impl SemanticMemory {
    const COLLECTION_NAME: &'static str = "_semantic_memory";

    /// Creates or opens semantic memory.
    ///
    /// # Errors
    ///
    /// Returns an error when collection creation/opening fails or dimensions mismatch.
    pub fn new_from_db(db: Arc<Database>, dimension: usize) -> Result<Self, AgentMemoryError> {
        Self::new(db, dimension, Arc::new(MemoryTtl::new()))
    }

    pub(crate) fn new(
        db: Arc<Database>,
        dimension: usize,
        ttl: Arc<MemoryTtl>,
    ) -> Result<Self, AgentMemoryError> {
        let (collection_name, dimension, stored_ids) =
            memory_helpers::init_tracked_memory(&db, Self::COLLECTION_NAME, dimension)?;

        Ok(Self {
            collection_name,
            db,
            dimension,
            ttl,
            stored_ids,
        })
    }

    /// Returns the name of the underlying `VelesDB` collection.
    #[must_use]
    pub fn collection_name(&self) -> &str {
        &self.collection_name
    }

    /// Returns the embedding dimension for this collection.
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// Stores a semantic memory point.
    ///
    /// # Errors
    ///
    /// Returns an error when embedding dimension is invalid, collection access fails,
    /// or persistence fails.
    #[allow(deprecated)]
    pub fn store(&self, id: u64, content: &str, embedding: &[f32]) -> Result<(), AgentMemoryError> {
        memory_helpers::validate_dimension(self.dimension, embedding.len())?;

        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;
        let point = Point::new(id, embedding.to_vec(), Some(json!({"content": content})));
        memory_helpers::upsert_points(&collection, vec![point])?;

        self.stored_ids.write().insert(id);
        Ok(())
    }

    /// Stores a semantic memory point and assigns a TTL.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::store`].
    pub fn store_with_ttl(
        &self,
        id: u64,
        content: &str,
        embedding: &[f32],
        ttl_seconds: u64,
    ) -> Result<(), AgentMemoryError> {
        self.store(id, content, embedding)?;
        self.ttl.set_ttl(id, ttl_seconds);
        Ok(())
    }

    /// Queries semantic memory by vector similarity.
    ///
    /// # Errors
    ///
    /// Returns an error when embedding dimension is invalid, collection access fails,
    /// or vector search fails.
    pub fn query(
        &self,
        query_embedding: &[f32],
        k: usize,
    ) -> Result<Vec<(u64, f32, String)>, AgentMemoryError> {
        let results = memory_helpers::search_filtered(
            &self.db,
            &self.collection_name,
            self.dimension,
            query_embedding,
            k,
            &self.ttl,
        )?;

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

    /// Deletes a semantic memory point by id.
    ///
    /// # Errors
    ///
    /// Returns an error when collection access or deletion fails.
    pub fn delete(&self, id: u64) -> Result<(), AgentMemoryError> {
        memory_helpers::delete_tracked_point(
            &self.db,
            &self.collection_name,
            id,
            &self.stored_ids,
            &self.ttl,
        )
    }

    /// Serializes semantic memory points for snapshot persistence.
    ///
    /// # Errors
    ///
    /// Returns an error when collection access or JSON encoding fails.
    pub fn serialize(&self) -> Result<Vec<u8>, AgentMemoryError> {
        memory_helpers::serialize_tracked_points(&self.db, &self.collection_name, &self.stored_ids)
    }

    /// Replaces semantic memory state from snapshot bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when JSON decoding fails, collection access fails,
    /// or persistence operations fail.
    pub fn deserialize(&self, data: &[u8]) -> Result<(), AgentMemoryError> {
        memory_helpers::deserialize_tracked_points(
            &self.db,
            &self.collection_name,
            data,
            &self.stored_ids,
        )
    }
}
