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
                    .ok_or(VelesError::Database {
                        message: "Failed to retrieve collection after creation".to_string(),
                    })?
            }
        };

        Ok(Self { collection })
    }

    /// Stores a knowledge fact with its embedding vector.
    ///
    /// The content text is persisted in the point payload as `{"content": ...}`
    /// so it survives a database reload.
    pub fn store(&self, id: u64, content: String, embedding: Vec<f32>) -> Result<(), VelesError> {
        let payload =
            serde_json::to_string(&serde_json::json!({ "content": content })).map_err(|e| {
                VelesError::Database {
                    message: format!("Failed to encode content payload: {e}"),
                }
            })?;
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
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_db() -> (TempDir, std::sync::Arc<VelesDatabase>) {
        let dir = TempDir::new().expect("test: create temp dir");
        let db = VelesDatabase::open(dir.path().to_string_lossy().to_string())
            .expect("test: open database");
        (dir, db)
    }

    #[test]
    fn test_semantic_memory_new() {
        let (_dir, db) = create_test_db();
        let memory = VelesSemanticMemory::new(&db, 4).expect("test: construct semantic memory");
        assert_eq!(memory.dimension(), 4);
        assert!(memory.is_empty().expect("test: is_empty on fresh memory"));
    }

    #[test]
    fn test_semantic_memory_store_and_query() {
        let (_dir, db) = create_test_db();
        let memory = VelesSemanticMemory::new(&db, 4).expect("test: construct semantic memory");

        memory
            .store(1, "Test content".to_string(), vec![0.1, 0.2, 0.3, 0.4])
            .expect("test: store knowledge fact");

        assert_eq!(memory.len().expect("test: read len"), 1);
        assert!(!memory.is_empty().expect("test: is_empty after store"));

        let results = memory
            .query(vec![0.1, 0.2, 0.3, 0.4], 5)
            .expect("test: query semantic memory");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, 1);
        assert_eq!(results[0].content, "Test content");
    }

    #[test]
    fn test_semantic_memory_delete() {
        let (_dir, db) = create_test_db();
        let memory = VelesSemanticMemory::new(&db, 4).expect("test: construct semantic memory");

        memory
            .store(1, "Content".to_string(), vec![0.1, 0.2, 0.3, 0.4])
            .expect("test: store knowledge fact");
        assert_eq!(memory.len().expect("test: read len"), 1);

        memory.delete(1).expect("test: delete knowledge fact");
        assert!(memory.is_empty().expect("test: is_empty after delete"));
    }

    #[test]
    fn test_semantic_memory_clear() {
        let (_dir, db) = create_test_db();
        let memory = VelesSemanticMemory::new(&db, 4).expect("test: construct semantic memory");

        memory
            .store(1, "First".to_string(), vec![0.1, 0.2, 0.3, 0.4])
            .expect("test: store first fact");
        memory
            .store(2, "Second".to_string(), vec![0.5, 0.6, 0.7, 0.8])
            .expect("test: store second fact");
        assert_eq!(memory.len().expect("test: read len"), 2);

        memory.clear().expect("test: clear knowledge facts");
        assert!(memory.is_empty().expect("test: is_empty after clear"));
    }

    #[test]
    fn test_semantic_memory_content_survives_reload() {
        let dir = TempDir::new().expect("test: create temp dir");
        let path = dir.path().to_string_lossy().to_string();

        // Store a fact, then drop the database handle entirely.
        {
            let db = VelesDatabase::open(path.clone()).expect("test: open database");
            let memory = VelesSemanticMemory::new(&db, 4).expect("test: construct semantic memory");
            memory
                .store(
                    7,
                    "Paris is the capital of France".to_string(),
                    vec![0.1, 0.2, 0.3, 0.4],
                )
                .expect("test: store knowledge fact");
        }

        // Re-open the database from disk and recover the content text.
        let db = VelesDatabase::open(path).expect("test: re-open database");
        let memory = VelesSemanticMemory::new(&db, 4).expect("test: re-construct semantic memory");

        let results = memory
            .query(vec![0.1, 0.2, 0.3, 0.4], 5)
            .expect("test: query after reload");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, 7);
        assert_eq!(results[0].content, "Paris is the capital of France");
    }
}
