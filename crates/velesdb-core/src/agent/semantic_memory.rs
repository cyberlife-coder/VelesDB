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
        let collection_name = Self::COLLECTION_NAME.to_string();
        let actual_dimension =
            memory_helpers::open_or_create_collection(&db, &collection_name, dimension)?;
        let stored_ids = RwLock::new(memory_helpers::load_stored_ids(&db, &collection_name));

        Ok(Self {
            collection_name,
            db,
            dimension: actual_dimension,
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
    #[allow(deprecated)]
    pub fn query(
        &self,
        query_embedding: &[f32],
        k: usize,
    ) -> Result<Vec<(u64, f32, String)>, AgentMemoryError> {
        memory_helpers::validate_dimension(self.dimension, query_embedding.len())?;

        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;
        let results = memory_helpers::search_collection(&collection, query_embedding, k)?;

        Ok(results
            .into_iter()
            .filter(|r| !self.ttl.is_expired(r.point.id))
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
    #[allow(deprecated)]
    pub fn delete(&self, id: u64) -> Result<(), AgentMemoryError> {
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;
        memory_helpers::delete_from_collection(&collection, &[id])?;

        self.stored_ids.write().remove(&id);
        self.ttl.remove(id);
        Ok(())
    }

    /// Serializes semantic memory points for snapshot persistence.
    ///
    /// # Errors
    ///
    /// Returns an error when collection access or JSON encoding fails.
    #[allow(deprecated)]
    pub fn serialize(&self) -> Result<Vec<u8>, AgentMemoryError> {
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;
        let all_ids: Vec<u64> = self.stored_ids.read().iter().copied().collect();
        memory_helpers::serialize_points(&collection, &all_ids)
    }

    /// Replaces semantic memory state from snapshot bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when JSON decoding fails, collection access fails,
    /// or persistence operations fail.
    #[allow(deprecated)]
    pub fn deserialize(&self, data: &[u8]) -> Result<(), AgentMemoryError> {
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;
        if let Some(points) = memory_helpers::deserialize_into_collection(data, &collection)? {
            memory_helpers::rebuild_stored_ids(&self.stored_ids, &points);
        }
        Ok(())
    }
}
