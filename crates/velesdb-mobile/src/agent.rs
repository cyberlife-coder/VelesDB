//! `AgentMemory` Mobile bindings (EPIC-016 US-003)
//!
//! Provides semantic memory for AI agents on iOS/Android.

use super::{VelesCollection, VelesDatabase, VelesError};

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
/// # Example (Swift)
///
/// ```swift
/// let memory = try VelesSemanticMemory(db: db, dimension: 384)
/// try memory.store(id: 1, content: "Paris is the capital of France", embedding: embedding)
/// let results = try memory.query(embedding: queryEmbedding, topK: 5)
/// ```
#[derive(uniffi::Object)]
pub struct VelesSemanticMemory {
    collection: VelesCollection,
    contents: std::sync::RwLock<std::collections::HashMap<u64, String>>,
}

#[uniffi::export]
impl VelesSemanticMemory {
    /// Creates a new `VelesSemanticMemory` with the given embedding dimension.
    #[uniffi::constructor]
    pub fn new(db: &VelesDatabase, dimension: u32) -> Result<Self, VelesError> {
        let collection_name = "_semantic_memory";

        // Try to get existing or create new collection
        let collection = match db.get_collection(collection_name.to_string()) {
            Ok(coll) => coll,
            Err(_) => db.create_collection(collection_name.to_string(), dimension, "cosine".to_string())?,
        };

        Ok(Self {
            collection,
            contents: std::sync::RwLock::new(std::collections::HashMap::new()),
        })
    }

    /// Stores a knowledge fact with its embedding vector.
    pub fn store(&self, id: u64, content: String, embedding: Vec<f32>) -> Result<(), VelesError> {
        self.collection.upsert(id, embedding, None)?;
        self.contents
            .write()
            .map_err(|e| VelesError::Database {
                message: format!("Lock error: {e}"),
            })?
            .insert(id, content);
        Ok(())
    }

    /// Queries semantic memory by similarity search.
    pub fn query(&self, embedding: Vec<f32>, top_k: u32) -> Result<Vec<SemanticResult>, VelesError> {
        let results = self.collection.search(embedding, top_k)?;

        let contents = self.contents.read().map_err(|e| VelesError::Database {
            message: format!("Lock error: {e}"),
        })?;

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
        let contents = self.contents.read().map_err(|e| VelesError::Database {
            message: format!("Lock error: {e}"),
        })?;
        Ok(contents.len() as u64)
    }

    /// Returns true if no knowledge facts are stored.
    pub fn is_empty(&self) -> Result<bool, VelesError> {
        Ok(self.len()? == 0)
    }

    /// Removes a knowledge fact by ID.
    pub fn remove(&self, id: u64) -> Result<bool, VelesError> {
        self.collection.delete(vec![id])?;
        let mut contents = self.contents.write().map_err(|e| VelesError::Database {
            message: format!("Lock error: {e}"),
        })?;
        Ok(contents.remove(&id).is_some())
    }

    /// Clears all knowledge facts.
    pub fn clear(&self) -> Result<(), VelesError> {
        let mut contents = self.contents.write().map_err(|e| VelesError::Database {
            message: format!("Lock error: {e}"),
        })?;

        // Get all IDs and delete them
        let ids: Vec<u64> = contents.keys().copied().collect();
        if !ids.is_empty() {
            self.collection.delete(ids)?;
        }
        contents.clear();
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

    fn create_test_db() -> (TempDir, VelesDatabase) {
        let dir = TempDir::new().unwrap();
        let db = VelesDatabase::open(dir.path().to_string_lossy().to_string()).unwrap();
        (dir, db)
    }

    #[test]
    fn test_semantic_memory_new() {
        let (_dir, db) = create_test_db();
        let memory = VelesSemanticMemory::new(&db, 4).unwrap();
        assert_eq!(memory.dimension(), 4);
        assert!(memory.is_empty().unwrap());
    }

    #[test]
    fn test_semantic_memory_store_and_query() {
        let (_dir, db) = create_test_db();
        let memory = VelesSemanticMemory::new(&db, 4).unwrap();

        memory
            .store(1, "Test content".to_string(), vec![0.1, 0.2, 0.3, 0.4])
            .unwrap();

        assert_eq!(memory.len().unwrap(), 1);
        assert!(!memory.is_empty().unwrap());

        let results = memory.query(vec![0.1, 0.2, 0.3, 0.4], 5).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, 1);
        assert_eq!(results[0].content, "Test content");
    }

    #[test]
    fn test_semantic_memory_remove() {
        let (_dir, db) = create_test_db();
        let memory = VelesSemanticMemory::new(&db, 4).unwrap();

        memory
            .store(1, "Content".to_string(), vec![0.1, 0.2, 0.3, 0.4])
            .unwrap();
        assert_eq!(memory.len().unwrap(), 1);

        let removed = memory.remove(1).unwrap();
        assert!(removed);
        assert!(memory.is_empty().unwrap());
    }
}
