//! Semantic Memory - Long-term knowledge storage (US-002)
//!
//! Stores facts and knowledge as vectors with similarity search.
//! Each fact has an ID, content text, and embedding vector.

use crate::{Database, DistanceMetric, Point};
use parking_lot::RwLock;
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;

use super::error::AgentMemoryError;
use super::ttl::MemoryTtl;

pub struct SemanticMemory<'a> {
    collection_name: String,
    db: &'a Database,
    dimension: usize,
    ttl: Arc<MemoryTtl>,
    stored_ids: RwLock<HashSet<u64>>,
}

impl<'a> SemanticMemory<'a> {
    const COLLECTION_NAME: &'static str = "_semantic_memory";

    pub fn new_from_db(db: &'a Database, dimension: usize) -> Result<Self, AgentMemoryError> {
        Self::new(db, dimension, Arc::new(MemoryTtl::new()))
    }

    pub(crate) fn new(
        db: &'a Database,
        dimension: usize,
        ttl: Arc<MemoryTtl>,
    ) -> Result<Self, AgentMemoryError> {
        let collection_name = Self::COLLECTION_NAME.to_string();

        let stored_ids = RwLock::new(HashSet::new());

        let actual_dimension = if let Some(collection) = db.get_collection(&collection_name) {
            let existing_dim = collection.config().dimension;
            if existing_dim != dimension {
                return Err(AgentMemoryError::DimensionMismatch {
                    expected: existing_dim,
                    actual: dimension,
                });
            }

            let all_ids = collection.all_ids();
            let mut ids = stored_ids.write();
            for id in all_ids {
                ids.insert(id);
            }
            drop(ids);

            existing_dim
        } else {
            db.create_collection(&collection_name, dimension, DistanceMetric::Cosine)?;
            dimension
        };

        Ok(Self {
            collection_name,
            db,
            dimension: actual_dimension,
            ttl,
            stored_ids,
        })
    }

    #[must_use]
    pub fn collection_name(&self) -> &str {
        &self.collection_name
    }

    #[must_use]
    pub fn dimension(&self) -> usize {
        self.dimension
    }

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

        self.stored_ids.write().insert(id);
        Ok(())
    }

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

    pub fn delete(&self, id: u64) -> Result<(), AgentMemoryError> {
        let collection = self
            .db
            .get_collection(&self.collection_name)
            .ok_or_else(|| AgentMemoryError::CollectionError("Collection not found".to_string()))?;

        collection
            .delete(&[id])
            .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))?;

        self.stored_ids.write().remove(&id);
        self.ttl.remove(id);
        Ok(())
    }

    pub fn serialize(&self) -> Result<Vec<u8>, AgentMemoryError> {
        let collection = self
            .db
            .get_collection(&self.collection_name)
            .ok_or_else(|| AgentMemoryError::CollectionError("Collection not found".to_string()))?;

        let all_ids: Vec<u64> = self.stored_ids.read().iter().copied().collect();
        let points: Vec<_> = collection.get(&all_ids).into_iter().flatten().collect();

        let serialized =
            serde_json::to_vec(&points).map_err(|e| AgentMemoryError::IoError(e.to_string()))?;

        Ok(serialized)
    }

    pub fn deserialize(&self, data: &[u8]) -> Result<(), AgentMemoryError> {
        if data.is_empty() {
            return Ok(());
        }

        let points: Vec<Point> =
            serde_json::from_slice(data).map_err(|e| AgentMemoryError::IoError(e.to_string()))?;

        let collection = self
            .db
            .get_collection(&self.collection_name)
            .ok_or_else(|| AgentMemoryError::CollectionError("Collection not found".to_string()))?;

        let existing_ids = collection.all_ids();
        if !existing_ids.is_empty() {
            collection
                .delete(&existing_ids)
                .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))?;
        }

        {
            let mut ids = self.stored_ids.write();
            ids.clear();
            for point in &points {
                ids.insert(point.id);
            }
        }

        collection
            .upsert(points)
            .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))?;

        Ok(())
    }
}
