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
    /// A `ttl_seconds` of `0` means "expire immediately": rather than persisting
    /// a live point that then occupies an index slot until the next
    /// `auto_expire`, the point is eagerly removed (and any pre-existing point
    /// for `id` deleted). The embedding is still dimension-validated so callers
    /// get the same error contract as a real store.
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
        if ttl_seconds == 0 {
            memory_helpers::validate_dimension(self.dimension, embedding.len())?;
            return self.delete(id);
        }
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
                let content = extract_content(&r.point);
                (r.point.id, r.score, content)
            })
            .collect())
    }

    /// Stores multiple semantic memory points in one batch.
    ///
    /// Each tuple is `(id, content, embedding)`. All embeddings are
    /// dimension-validated before any write occurs.
    ///
    /// # Errors
    ///
    /// Returns an error when any embedding dimension is invalid, collection
    /// access fails, or persistence fails.
    pub fn store_batch(&self, facts: &[(u64, &str, &[f32])]) -> Result<(), AgentMemoryError> {
        let mut points = Vec::with_capacity(facts.len());
        for (id, content, embedding) in facts {
            memory_helpers::validate_dimension(self.dimension, embedding.len())?;
            points.push(Point::new(
                *id,
                embedding.to_vec(),
                Some(json!({ "content": content })),
            ));
        }

        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;
        memory_helpers::upsert_points(&collection, points)?;

        let mut ids = self.stored_ids.write();
        for (id, _, _) in facts {
            ids.insert(*id);
        }
        Ok(())
    }

    /// Retrieves a fact's content and embedding by id.
    ///
    /// Returns `None` when the id is unknown or has expired.
    ///
    /// # Errors
    ///
    /// Returns an error when collection access fails.
    pub fn get(&self, id: u64) -> Result<Option<(String, Vec<f32>)>, AgentMemoryError> {
        if self.ttl.is_expired(id) {
            return Ok(None);
        }
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;
        let Some(point) = collection.get(&[id]).into_iter().flatten().next() else {
            return Ok(None);
        };
        Ok(Some((extract_content(&point), point.vector.clone())))
    }

    /// Lists all live (non-expired) tracked facts as `(id, content)` pairs.
    ///
    /// # Errors
    ///
    /// Returns an error when collection access fails.
    pub fn list_all(&self) -> Result<Vec<(u64, String)>, AgentMemoryError> {
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;
        let all_ids: Vec<u64> = self.stored_ids.read().iter().copied().collect();

        Ok(collection
            .get(&all_ids)
            .into_iter()
            .flatten()
            .filter(|p| !self.ttl.is_expired(p.id))
            .map(|p| (p.id, extract_content(&p)))
            .collect())
    }

    /// Returns the number of tracked facts.
    #[must_use]
    pub fn count(&self) -> usize {
        self.stored_ids.read().len()
    }

    /// Returns `true` when no facts are tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.stored_ids.read().is_empty()
    }

    /// Removes all facts and their tracking entries.
    ///
    /// # Errors
    ///
    /// Returns an error when collection access or deletion fails.
    pub fn clear(&self) -> Result<(), AgentMemoryError> {
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;
        let ids: Vec<u64> = self.stored_ids.read().iter().copied().collect();
        if !ids.is_empty() {
            memory_helpers::delete_from_collection(&collection, &ids)?;
        }
        for id in &ids {
            self.ttl.remove(*id);
        }
        self.stored_ids.write().clear();
        Ok(())
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
    /// # TTL limitation
    ///
    /// The returned bytes contain only the stored points (id, embedding,
    /// content) and intentionally **omit TTL state**. TTL is tracked in a single
    /// `MemoryTtl` map shared across the semantic, episodic, and procedural
    /// subsystems (see [`AgentMemory`](crate::agent::AgentMemory)), so it cannot
    /// be partitioned per subsystem here. TTL is persisted and restored globally
    /// by [`AgentMemory::snapshot`](crate::agent::AgentMemory::snapshot) /
    /// `restore_state`. Calling [`Self::deserialize`] in isolation therefore
    /// restores facts but not their expiry; use the snapshot manager for a full
    /// round-trip including TTL.
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

/// Extracts the `content` string from a point's payload, or `""` when absent.
fn extract_content(point: &Point) -> String {
    point
        .payload
        .as_ref()
        .and_then(|p| p.get("content"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string()
}
