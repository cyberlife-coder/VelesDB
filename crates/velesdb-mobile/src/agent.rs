//! `AgentMemory` Mobile bindings (EPIC-016 US-003)
//!
//! Provides semantic memory for AI agents on iOS/Android.

use super::{DistanceMetric, VelesCollection, VelesDatabase, VelesError, VelesPoint};

/// Result from semantic memory query.
#[derive(Debug, Clone, uniffi::Record)]
pub struct SemanticResult {
    /// Knowledge fact ID.
    pub id: u64,
    /// Similarity score.
    pub score: f32,
    /// Knowledge content text.
    pub content: String,
}

/// Semantic Memory for AI agents on mobile.
///
/// Stores knowledge facts as vectors with similarity search.
///
/// Fact content text is persisted in the point payload (mirroring the core
/// `SemanticMemory`), so content survives a database reload.
///
/// # Example (Swift)
///
/// ```swift
/// let memory = try VelesSemanticMemory(db: db, dimension: 384)
/// try memory.store(id: 1, content: "Paris is the capital of France", embedding: embedding)
/// let results = try memory.query(embedding: queryEmbedding, topK: 5)
/// ```
#[derive(uniffi::Object)]
pub struct VelesSemanticMemory {
    collection: std::sync::Arc<VelesCollection>,
}

impl VelesSemanticMemory {
    /// Extracts the `content` text from a stored point's JSON payload.
    fn content_from_payload(payload: Option<&String>) -> String {
        payload
            .and_then(|p| serde_json::from_str::<serde_json::Value>(p).ok())
            .as_ref()
            .and_then(|v| v.get("content"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string()
    }
}

#[uniffi::export]
impl VelesSemanticMemory {
    /// Creates a new `VelesSemanticMemory` with the given embedding dimension.
    #[uniffi::constructor]
    pub fn new(db: &VelesDatabase, dimension: u32) -> Result<Self, VelesError> {
        let collection_name = "_semantic_memory";

        // Try to get existing or create new collection
        let collection = match db.get_collection(collection_name.to_string())? {
            Some(coll) => coll,
            None => {
                db.create_collection(
                    collection_name.to_string(),
                    dimension,
                    DistanceMetric::Cosine,
                )?;
                db.get_collection(collection_name.to_string())?
                    .ok_or(VelesError::database(
                        "Failed to retrieve collection after creation".to_string(),
                    ))?
            }
        };

        Ok(Self { collection })
    }

    /// Stores a knowledge fact with its embedding vector.
    ///
    /// The content text is persisted in the point payload as `{"content": ...}`
    /// so it survives a database reload.
    pub fn store(&self, id: u64, content: String, embedding: Vec<f32>) -> Result<(), VelesError> {
        let payload = serde_json::to_string(&serde_json::json!({ "content": content }))
            .map_err(|e| VelesError::database(format!("Failed to encode content payload: {e}")))?;
        let point = VelesPoint {
            id,
            vector: embedding,
            payload: Some(payload),
        };
        self.collection.upsert(point)?;
        Ok(())
    }

    /// Queries semantic memory by similarity search.
    ///
    /// Content text is read back from each matched point's payload.
    pub fn query(
        &self,
        embedding: Vec<f32>,
        top_k: u32,
    ) -> Result<Vec<SemanticResult>, VelesError> {
        let results = self.collection.search(embedding, top_k)?;

        let ids: Vec<u64> = results.iter().map(|r| r.id).collect();
        let contents: std::collections::HashMap<u64, String> = self
            .collection
            .get(ids)
            .into_iter()
            .map(|p| (p.id, Self::content_from_payload(p.payload.as_ref())))
            .collect();

        Ok(results
            .into_iter()
            .map(|r| SemanticResult {
                id: r.id,
                score: r.score,
                content: contents.get(&r.id).cloned().unwrap_or_default(),
            })
            .collect())
    }

    /// Returns the number of stored knowledge facts.
    pub fn len(&self) -> Result<u64, VelesError> {
        Ok(self.collection.count())
    }

    /// Returns true if no knowledge facts are stored.
    pub fn is_empty(&self) -> Result<bool, VelesError> {
        Ok(self.len()? == 0)
    }

    /// Deletes a knowledge fact by ID.
    pub fn delete(&self, id: u64) -> Result<(), VelesError> {
        self.collection.delete(id)
    }

    /// Removes a knowledge fact by ID.
    ///
    /// Deprecated alias for [`Self::delete`], kept for backward compatibility
    /// and naming parity with prior mobile releases.
    pub fn remove(&self, id: u64) -> Result<(), VelesError> {
        self.delete(id)
    }

    /// Removes all stored knowledge facts.
    ///
    /// Best-effort: individual delete failures are non-fatal so the operation
    /// clears as much as possible.
    pub fn clear(&self) -> Result<(), VelesError> {
        for id in self.collection.all_ids() {
            let _ = self.collection.delete(id);
        }
        Ok(())
    }

    /// Returns the embedding dimension.
    pub fn dimension(&self) -> u32 {
        self.collection.dimension()
    }
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
