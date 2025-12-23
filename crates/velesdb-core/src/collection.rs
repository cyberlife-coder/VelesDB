//! Collection management for `VelesDB`.

use crate::distance::DistanceMetric;
use crate::error::{Error, Result};
use crate::index::{Bm25Index, HnswIndex, VectorIndex};
use crate::point::{Point, SearchResult};
use crate::storage::{LogPayloadStorage, MmapStorage, PayloadStorage, VectorStorage};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

/// Metadata for a collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionConfig {
    /// Name of the collection.
    pub name: String,

    /// Vector dimension.
    pub dimension: usize,

    /// Distance metric.
    pub metric: DistanceMetric,

    /// Number of points in the collection.
    pub point_count: usize,
}

/// A collection of vectors with associated metadata.
#[derive(Clone)]
pub struct Collection {
    /// Path to the collection data.
    path: PathBuf,

    /// Collection configuration.
    config: Arc<RwLock<CollectionConfig>>,

    /// Vector storage (on-disk, memory-mapped).
    vector_storage: Arc<RwLock<MmapStorage>>,

    /// Payload storage (on-disk, log-structured).
    payload_storage: Arc<RwLock<LogPayloadStorage>>,

    /// HNSW index for fast approximate nearest neighbor search.
    index: Arc<HnswIndex>,

    /// BM25 index for full-text search.
    text_index: Arc<Bm25Index>,
}

impl Collection {
    /// Creates a new collection at the specified path.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the config cannot be saved.
    pub fn create(path: PathBuf, dimension: usize, metric: DistanceMetric) -> Result<Self> {
        std::fs::create_dir_all(&path)?;

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let config = CollectionConfig {
            name,
            dimension,
            metric,
            point_count: 0,
        };

        // Initialize persistent storages
        let vector_storage = Arc::new(RwLock::new(
            MmapStorage::new(&path, dimension).map_err(Error::Io)?,
        ));

        let payload_storage = Arc::new(RwLock::new(
            LogPayloadStorage::new(&path).map_err(Error::Io)?,
        ));

        // Create HNSW index
        let index = Arc::new(HnswIndex::new(dimension, metric));

        // Create BM25 index for full-text search
        let text_index = Arc::new(Bm25Index::new());

        let collection = Self {
            path,
            config: Arc::new(RwLock::new(config)),
            vector_storage,
            payload_storage,
            index,
            text_index,
        };

        collection.save_config()?;

        Ok(collection)
    }

    /// Opens an existing collection from the specified path.
    ///
    /// # Errors
    ///
    /// Returns an error if the config file cannot be read or parsed.
    pub fn open(path: PathBuf) -> Result<Self> {
        let config_path = path.join("config.json");
        let config_data = std::fs::read_to_string(&config_path)?;
        let config: CollectionConfig =
            serde_json::from_str(&config_data).map_err(|e| Error::Serialization(e.to_string()))?;

        // Open persistent storages
        let vector_storage = Arc::new(RwLock::new(
            MmapStorage::new(&path, config.dimension).map_err(Error::Io)?,
        ));

        let payload_storage = Arc::new(RwLock::new(
            LogPayloadStorage::new(&path).map_err(Error::Io)?,
        ));

        // Load HNSW index if it exists, otherwise create new (empty)
        let index = if path.join("hnsw.bin").exists() {
            Arc::new(HnswIndex::load(&path, config.dimension, config.metric).map_err(Error::Io)?)
        } else {
            Arc::new(HnswIndex::new(config.dimension, config.metric))
        };

        // Create and rebuild BM25 index from existing payloads
        let text_index = Arc::new(Bm25Index::new());

        // Rebuild BM25 index from persisted payloads
        {
            let storage = payload_storage.read();
            let ids = storage.ids();
            for id in ids {
                if let Ok(Some(payload)) = storage.retrieve(id) {
                    let text = Self::extract_text_from_payload(&payload);
                    if !text.is_empty() {
                        text_index.add_document(id, &text);
                    }
                }
            }
        }

        Ok(Self {
            path,
            config: Arc::new(RwLock::new(config)),
            vector_storage,
            payload_storage,
            index,
            text_index,
        })
    }

    /// Returns the collection configuration.
    #[must_use]
    pub fn config(&self) -> CollectionConfig {
        self.config.read().clone()
    }

    /// Inserts or updates points in the collection.
    ///
    /// # Errors
    ///
    /// Returns an error if any point has a mismatched dimension.
    pub fn upsert(&self, points: Vec<Point>) -> Result<()> {
        let config = self.config.read();
        let dimension = config.dimension;
        drop(config);

        // Validate dimensions first
        for point in &points {
            if point.dimension() != dimension {
                return Err(Error::DimensionMismatch {
                    expected: dimension,
                    actual: point.dimension(),
                });
            }
        }

        let mut vector_storage = self.vector_storage.write();
        let mut payload_storage = self.payload_storage.write();

        for point in points {
            // 1. Store Vector
            vector_storage
                .store(point.id, &point.vector)
                .map_err(Error::Io)?;

            // 2. Store Payload (if present)
            if let Some(payload) = &point.payload {
                payload_storage
                    .store(point.id, payload)
                    .map_err(Error::Io)?;
            } else {
                // If payload is None, check if we need to delete existing payload?
                // For now, let's assume upsert with None doesn't clear payload unless explicit.
                // Or consistency: Point represents full state. If None, maybe we should delete?
                // Let's stick to: if None, do nothing (keep existing) or delete?
                // Typically upsert replaces. Let's say if None, we delete potential existing payload to be consistent.
                let _ = payload_storage.delete(point.id); // Ignore error if not found
            }

            // 3. Update Vector Index
            // Note: HnswIndex.insert() skips if ID already exists (no updates supported)
            // For true upsert semantics, we'd need to remove then re-insert
            self.index.insert(point.id, &point.vector);

            // 4. Update BM25 Text Index
            if let Some(payload) = &point.payload {
                let text = Self::extract_text_from_payload(payload);
                if !text.is_empty() {
                    self.text_index.add_document(point.id, &text);
                }
            } else {
                // Remove from text index if payload was cleared
                self.text_index.remove_document(point.id);
            }
        }

        // Update point count
        let mut config = self.config.write();
        config.point_count = vector_storage.len();

        // Auto-flush for durability (MVP choice: consistent but slower)
        // In prod, this might be backgrounded or explicit.
        vector_storage.flush().map_err(Error::Io)?;
        payload_storage.flush().map_err(Error::Io)?;
        self.index.save(&self.path).map_err(Error::Io)?;

        Ok(())
    }

    /// Bulk insert optimized for high-throughput import.
    ///
    /// # Performance
    ///
    /// This method is optimized for bulk loading:
    /// - Uses parallel HNSW insertion (rayon)
    /// - Single flush at the end (not per-point)
    /// - ~2-3x faster than regular `upsert()` for large batches
    ///
    /// # Errors
    ///
    /// Returns an error if any point has a mismatched dimension.
    pub fn upsert_bulk(&self, points: &[Point]) -> Result<usize> {
        if points.is_empty() {
            return Ok(0);
        }

        let config = self.config.read();
        let dimension = config.dimension;
        drop(config);

        // Validate dimensions first
        for point in points {
            if point.dimension() != dimension {
                return Err(Error::DimensionMismatch {
                    expected: dimension,
                    actual: point.dimension(),
                });
            }
        }

        // Perf: Collect vectors for parallel HNSW insertion (needed for clone anyway)
        let vectors_for_hnsw: Vec<(u64, Vec<f32>)> =
            points.iter().map(|p| (p.id, p.vector.clone())).collect();

        // Perf: Single batch WAL write + contiguous mmap write
        // Use references from vectors_for_hnsw to avoid double allocation
        let vectors_for_storage: Vec<(u64, &[f32])> = vectors_for_hnsw
            .iter()
            .map(|(id, v)| (*id, v.as_slice()))
            .collect();

        let mut vector_storage = self.vector_storage.write();
        vector_storage
            .store_batch(&vectors_for_storage)
            .map_err(Error::Io)?;
        drop(vector_storage);

        // Store payloads and update BM25 (still sequential for now)
        let mut payload_storage = self.payload_storage.write();
        for point in points {
            if let Some(payload) = &point.payload {
                payload_storage
                    .store(point.id, payload)
                    .map_err(Error::Io)?;

                // Update BM25 text index
                let text = Self::extract_text_from_payload(payload);
                if !text.is_empty() {
                    self.text_index.add_document(point.id, &text);
                }
            }
        }
        drop(payload_storage);

        // Perf: Parallel HNSW insertion (CPU bound - benefits from parallelism)
        let inserted = self.index.insert_batch_parallel(vectors_for_hnsw);
        self.index.set_searching_mode();

        // Update point count
        let mut config = self.config.write();
        config.point_count = self.vector_storage.read().len();
        drop(config);

        // Single flush at the end (not per-point)
        self.vector_storage.write().flush().map_err(Error::Io)?;
        self.payload_storage.write().flush().map_err(Error::Io)?;
        self.index.save(&self.path).map_err(Error::Io)?;

        Ok(inserted)
    }

    /// Retrieves points by their IDs.
    #[must_use]
    pub fn get(&self, ids: &[u64]) -> Vec<Option<Point>> {
        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        ids.iter()
            .map(|&id| {
                // Retrieve vector
                let vector = vector_storage.retrieve(id).ok().flatten()?;

                // Retrieve payload
                let payload = payload_storage.retrieve(id).ok().flatten();

                Some(Point {
                    id,
                    vector,
                    payload,
                })
            })
            .collect()
    }

    /// Deletes points by their IDs.
    ///
    /// # Errors
    ///
    /// Returns an error if storage operations fail.
    pub fn delete(&self, ids: &[u64]) -> Result<()> {
        let mut vector_storage = self.vector_storage.write();
        let mut payload_storage = self.payload_storage.write();

        for &id in ids {
            vector_storage.delete(id).map_err(Error::Io)?;
            payload_storage.delete(id).map_err(Error::Io)?;
            self.index.remove(id);
        }

        let mut config = self.config.write();
        config.point_count = vector_storage.len();

        Ok(())
    }

    /// Searches for the k nearest neighbors of the query vector.
    ///
    /// Uses HNSW index for fast approximate nearest neighbor search.
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector dimension doesn't match the collection.
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<SearchResult>> {
        let config = self.config.read();

        if query.len() != config.dimension {
            return Err(Error::DimensionMismatch {
                expected: config.dimension,
                actual: query.len(),
            });
        }
        drop(config);

        // Use HNSW index for fast ANN search
        let index_results = self.index.search(query, k);

        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        // Map index results to SearchResult with full point data
        let results: Vec<SearchResult> = index_results
            .into_iter()
            .filter_map(|(id, score)| {
                // We need to fetch vector and payload
                let vector = vector_storage.retrieve(id).ok().flatten()?;
                let payload = payload_storage.retrieve(id).ok().flatten();

                let point = Point {
                    id,
                    vector,
                    payload,
                };

                Some(SearchResult::new(point, score))
            })
            .collect();

        Ok(results)
    }

    /// Returns the number of points in the collection.
    #[must_use]
    pub fn len(&self) -> usize {
        self.vector_storage.read().len()
    }

    /// Returns true if the collection is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.vector_storage.read().is_empty()
    }

    /// Saves the collection configuration and index to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if storage operations fail.
    pub fn flush(&self) -> Result<()> {
        self.save_config()?;
        self.vector_storage.write().flush().map_err(Error::Io)?;
        self.payload_storage.write().flush().map_err(Error::Io)?;
        self.index.save(&self.path).map_err(Error::Io)?;
        Ok(())
    }

    /// Saves the collection configuration to disk.
    fn save_config(&self) -> Result<()> {
        let config = self.config.read();
        let config_path = self.path.join("config.json");
        let config_data = serde_json::to_string_pretty(&*config)
            .map_err(|e| Error::Serialization(e.to_string()))?;
        std::fs::write(config_path, config_data)?;
        Ok(())
    }

    /// Performs full-text search using BM25.
    ///
    /// # Arguments
    ///
    /// * `query` - Text query to search for
    /// * `k` - Maximum number of results to return
    ///
    /// # Returns
    ///
    /// Vector of search results sorted by BM25 score (descending).
    #[must_use]
    pub fn text_search(&self, query: &str, k: usize) -> Vec<SearchResult> {
        let bm25_results = self.text_index.search(query, k);

        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        bm25_results
            .into_iter()
            .filter_map(|(id, score)| {
                let vector = vector_storage.retrieve(id).ok().flatten()?;
                let payload = payload_storage.retrieve(id).ok().flatten();

                let point = Point {
                    id,
                    vector,
                    payload,
                };

                Some(SearchResult::new(point, score))
            })
            .collect()
    }

    /// Performs hybrid search combining vector similarity and full-text search.
    ///
    /// Uses Reciprocal Rank Fusion (RRF) to combine results from both searches.
    ///
    /// # Arguments
    ///
    /// * `vector_query` - Query vector for similarity search
    /// * `text_query` - Text query for BM25 search
    /// * `k` - Maximum number of results to return
    /// * `vector_weight` - Weight for vector results (0.0-1.0, default 0.5)
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector dimension doesn't match.
    pub fn hybrid_search(
        &self,
        vector_query: &[f32],
        text_query: &str,
        k: usize,
        vector_weight: Option<f32>,
    ) -> Result<Vec<SearchResult>> {
        let config = self.config.read();
        if vector_query.len() != config.dimension {
            return Err(Error::DimensionMismatch {
                expected: config.dimension,
                actual: vector_query.len(),
            });
        }
        drop(config);

        let weight = vector_weight.unwrap_or(0.5).clamp(0.0, 1.0);
        let text_weight = 1.0 - weight;

        // Get vector search results (more than k to allow for fusion)
        let vector_results = self.index.search(vector_query, k * 2);

        // Get BM25 text search results
        let text_results = self.text_index.search(text_query, k * 2);

        // Perf: Apply RRF (Reciprocal Rank Fusion) with FxHashMap for faster hashing
        // RRF score = 1 / (rank + 60) - the constant 60 is standard
        let mut fused_scores: rustc_hash::FxHashMap<u64, f32> = rustc_hash::FxHashMap::default();

        // Add vector scores with RRF
        #[allow(clippy::cast_precision_loss)]
        for (rank, (id, _)) in vector_results.iter().enumerate() {
            let rrf_score = weight / (rank as f32 + 60.0);
            *fused_scores.entry(*id).or_insert(0.0) += rrf_score;
        }

        // Add text scores with RRF
        #[allow(clippy::cast_precision_loss)]
        for (rank, (id, _)) in text_results.iter().enumerate() {
            let rrf_score = text_weight / (rank as f32 + 60.0);
            *fused_scores.entry(*id).or_insert(0.0) += rrf_score;
        }

        // Perf: Use partial sort for top-k instead of full sort
        let mut scored_ids: Vec<_> = fused_scores.into_iter().collect();
        if scored_ids.len() > k {
            scored_ids.select_nth_unstable_by(k, |a, b| {
                b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
            });
            scored_ids.truncate(k);
            scored_ids.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        } else {
            scored_ids.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        }

        // Fetch full point data
        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        let results: Vec<SearchResult> = scored_ids
            .into_iter()
            .filter_map(|(id, score)| {
                let vector = vector_storage.retrieve(id).ok().flatten()?;
                let payload = payload_storage.retrieve(id).ok().flatten();

                let point = Point {
                    id,
                    vector,
                    payload,
                };

                Some(SearchResult::new(point, score))
            })
            .collect();

        Ok(results)
    }

    /// Extracts all string values from a JSON payload for text indexing.
    fn extract_text_from_payload(payload: &serde_json::Value) -> String {
        let mut texts = Vec::new();
        Self::collect_strings(payload, &mut texts);
        texts.join(" ")
    }

    /// Recursively collects all string values from a JSON value.
    fn collect_strings(value: &serde_json::Value, texts: &mut Vec<String>) {
        match value {
            serde_json::Value::String(s) => texts.push(s.clone()),
            serde_json::Value::Array(arr) => {
                for item in arr {
                    Self::collect_strings(item, texts);
                }
            }
            serde_json::Value::Object(obj) => {
                for v in obj.values() {
                    Self::collect_strings(v, texts);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn test_collection_create() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");

        let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();
        let config = collection.config();

        assert_eq!(config.dimension, 3);
        assert_eq!(config.metric, DistanceMetric::Cosine);
        assert_eq!(config.point_count, 0);
    }

    #[test]
    fn test_collection_upsert_and_search() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");

        let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

        let points = vec![
            Point::without_payload(1, vec![1.0, 0.0, 0.0]),
            Point::without_payload(2, vec![0.0, 1.0, 0.0]),
            Point::without_payload(3, vec![0.0, 0.0, 1.0]),
        ];

        collection.upsert(points).unwrap();
        assert_eq!(collection.len(), 3);

        let query = vec![1.0, 0.0, 0.0];
        let results = collection.search(&query, 2).unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].point.id, 1); // Most similar
    }

    #[test]
    fn test_dimension_mismatch() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");

        let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

        let points = vec![Point::without_payload(1, vec![1.0, 0.0])]; // Wrong dimension

        let result = collection.upsert(points);
        assert!(result.is_err());
    }

    #[test]
    fn test_collection_open_existing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");

        // Create and populate collection
        {
            let collection =
                Collection::create(path.clone(), 3, DistanceMetric::Euclidean).unwrap();
            let points = vec![
                Point::without_payload(1, vec![1.0, 2.0, 3.0]),
                Point::without_payload(2, vec![4.0, 5.0, 6.0]),
            ];
            collection.upsert(points).unwrap();
            collection.flush().unwrap();
        }

        // Reopen and verify
        let collection = Collection::open(path).unwrap();
        let config = collection.config();

        assert_eq!(config.dimension, 3);
        assert_eq!(config.metric, DistanceMetric::Euclidean);
        assert_eq!(collection.len(), 2);
    }

    #[test]
    fn test_collection_get_points() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");

        let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();
        let points = vec![
            Point::without_payload(1, vec![1.0, 0.0, 0.0]),
            Point::without_payload(2, vec![0.0, 1.0, 0.0]),
        ];
        collection.upsert(points).unwrap();

        // Get existing points
        let retrieved = collection.get(&[1, 2, 999]);

        assert!(retrieved[0].is_some());
        assert_eq!(retrieved[0].as_ref().unwrap().id, 1);
        assert!(retrieved[1].is_some());
        assert_eq!(retrieved[1].as_ref().unwrap().id, 2);
        assert!(retrieved[2].is_none()); // 999 doesn't exist
    }

    #[test]
    fn test_collection_delete_points() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");

        let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();
        let points = vec![
            Point::without_payload(1, vec![1.0, 0.0, 0.0]),
            Point::without_payload(2, vec![0.0, 1.0, 0.0]),
            Point::without_payload(3, vec![0.0, 0.0, 1.0]),
        ];
        collection.upsert(points).unwrap();
        assert_eq!(collection.len(), 3);

        // Delete one point
        collection.delete(&[2]).unwrap();
        assert_eq!(collection.len(), 2);

        // Verify it's gone
        let retrieved = collection.get(&[2]);
        assert!(retrieved[0].is_none());
    }

    #[test]
    fn test_collection_is_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");

        let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();
        assert!(collection.is_empty());

        collection
            .upsert(vec![Point::without_payload(1, vec![1.0, 0.0, 0.0])])
            .unwrap();
        assert!(!collection.is_empty());
    }

    #[test]
    fn test_collection_with_payload() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");

        let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

        let points = vec![Point::new(
            1,
            vec![1.0, 0.0, 0.0],
            Some(json!({"title": "Test Document", "category": "tech"})),
        )];
        collection.upsert(points).unwrap();

        let retrieved = collection.get(&[1]);
        assert!(retrieved[0].is_some());

        let point = retrieved[0].as_ref().unwrap();
        assert!(point.payload.is_some());
        assert_eq!(point.payload.as_ref().unwrap()["title"], "Test Document");
    }

    #[test]
    fn test_collection_search_dimension_mismatch() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");

        let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();
        collection
            .upsert(vec![Point::without_payload(1, vec![1.0, 0.0, 0.0])])
            .unwrap();

        // Search with wrong dimension
        let result = collection.search(&[1.0, 0.0], 5);
        assert!(result.is_err());
    }

    #[test]
    fn test_collection_upsert_replaces_payload() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");

        let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

        // Insert with payload
        collection
            .upsert(vec![Point::new(
                1,
                vec![1.0, 0.0, 0.0],
                Some(json!({"version": 1})),
            )])
            .unwrap();

        // Upsert without payload (should clear it)
        collection
            .upsert(vec![Point::without_payload(1, vec![1.0, 0.0, 0.0])])
            .unwrap();

        let retrieved = collection.get(&[1]);
        let point = retrieved[0].as_ref().unwrap();
        assert!(point.payload.is_none());
    }

    #[test]
    fn test_collection_flush() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");

        let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();
        collection
            .upsert(vec![Point::without_payload(1, vec![1.0, 0.0, 0.0])])
            .unwrap();

        // Explicit flush should succeed
        let result = collection.flush();
        assert!(result.is_ok());
    }

    #[test]
    fn test_collection_euclidean_metric() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");

        let collection = Collection::create(path, 3, DistanceMetric::Euclidean).unwrap();

        let points = vec![
            Point::without_payload(1, vec![0.0, 0.0, 0.0]),
            Point::without_payload(2, vec![1.0, 0.0, 0.0]),
            Point::without_payload(3, vec![10.0, 0.0, 0.0]),
        ];
        collection.upsert(points).unwrap();

        let query = vec![0.5, 0.0, 0.0];
        let results = collection.search(&query, 3).unwrap();

        // Point 1 (0,0,0) and Point 2 (1,0,0) should be closest to query (0.5,0,0)
        assert!(results[0].point.id == 1 || results[0].point.id == 2);
    }

    #[test]
    fn test_collection_text_search() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");

        let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

        let points = vec![
            Point::new(
                1,
                vec![1.0, 0.0, 0.0],
                Some(json!({"title": "Rust Programming", "content": "Learn Rust language"})),
            ),
            Point::new(
                2,
                vec![0.0, 1.0, 0.0],
                Some(json!({"title": "Python Tutorial", "content": "Python is great"})),
            ),
            Point::new(
                3,
                vec![0.0, 0.0, 1.0],
                Some(json!({"title": "Rust Performance", "content": "Rust is fast"})),
            ),
        ];
        collection.upsert(points).unwrap();

        // Search for "rust" - should match docs 1 and 3
        let results = collection.text_search("rust", 10);
        assert_eq!(results.len(), 2);

        let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
        assert!(ids.contains(&1));
        assert!(ids.contains(&3));
    }

    #[test]
    fn test_collection_hybrid_search() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");

        let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

        let points = vec![
            Point::new(
                1,
                vec![1.0, 0.0, 0.0],
                Some(json!({"title": "Rust Programming"})),
            ),
            Point::new(
                2,
                vec![0.9, 0.1, 0.0], // Similar vector to query
                Some(json!({"title": "Python Programming"})),
            ),
            Point::new(
                3,
                vec![0.0, 1.0, 0.0],
                Some(json!({"title": "Rust Performance"})),
            ),
        ];
        collection.upsert(points).unwrap();

        // Hybrid search: vector close to [1,0,0], text "rust"
        // Doc 1 matches both (vector + text)
        // Doc 2 matches vector only
        // Doc 3 matches text only
        let query = vec![1.0, 0.0, 0.0];
        let results = collection
            .hybrid_search(&query, "rust", 3, Some(0.5))
            .unwrap();

        assert!(!results.is_empty());
        // Doc 1 should rank high (matches both)
        assert_eq!(results[0].point.id, 1);
    }

    #[test]
    fn test_extract_text_from_payload() {
        // Test nested payload extraction
        let payload = json!({
            "title": "Hello",
            "meta": {
                "author": "World",
                "tags": ["rust", "fast"]
            }
        });

        let text = Collection::extract_text_from_payload(&payload);
        assert!(text.contains("Hello"));
        assert!(text.contains("World"));
        assert!(text.contains("rust"));
        assert!(text.contains("fast"));
    }

    #[test]
    fn test_text_search_empty_query() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");

        let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

        let points = vec![Point::new(
            1,
            vec![1.0, 0.0, 0.0],
            Some(json!({"content": "test document"})),
        )];
        collection.upsert(points).unwrap();

        // Empty query should return empty results
        let results = collection.text_search("", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_text_search_no_payload() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");

        let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

        // Points without payload
        let points = vec![
            Point::new(1, vec![1.0, 0.0, 0.0], None),
            Point::new(2, vec![0.0, 1.0, 0.0], None),
        ];
        collection.upsert(points).unwrap();

        // Text search should return empty (no text indexed)
        let results = collection.text_search("test", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_hybrid_search_text_weight_zero() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");

        let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

        let points = vec![
            Point::new(1, vec![1.0, 0.0, 0.0], Some(json!({"title": "Rust"}))),
            Point::new(2, vec![0.9, 0.1, 0.0], Some(json!({"title": "Python"}))),
        ];
        collection.upsert(points).unwrap();

        // vector_weight=1.0 means text_weight=0.0 (pure vector search)
        let query = vec![0.9, 0.1, 0.0];
        let results = collection
            .hybrid_search(&query, "rust", 2, Some(1.0))
            .unwrap();

        // Doc 2 should be first (closest vector) even though "rust" matches doc 1
        assert_eq!(results[0].point.id, 2);
    }

    #[test]
    fn test_hybrid_search_vector_weight_zero() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");

        let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

        let points = vec![
            Point::new(
                1,
                vec![1.0, 0.0, 0.0],
                Some(json!({"title": "Rust programming language"})),
            ),
            Point::new(
                2,
                vec![0.99, 0.01, 0.0], // Very close to query vector
                Some(json!({"title": "Python programming"})),
            ),
        ];
        collection.upsert(points).unwrap();

        // vector_weight=0.0 means text_weight=1.0 (pure text search)
        let query = vec![0.99, 0.01, 0.0];
        let results = collection
            .hybrid_search(&query, "rust", 2, Some(0.0))
            .unwrap();

        // Doc 1 should be first (matches "rust") even though doc 2 has closer vector
        assert_eq!(results[0].point.id, 1);
    }

    #[test]
    fn test_bm25_update_document() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");

        let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

        // Insert initial document
        let points = vec![Point::new(
            1,
            vec![1.0, 0.0, 0.0],
            Some(json!({"content": "rust programming"})),
        )];
        collection.upsert(points).unwrap();

        // Verify it's indexed
        let results = collection.text_search("rust", 10);
        assert_eq!(results.len(), 1);

        // Update document with different text
        let points = vec![Point::new(
            1,
            vec![1.0, 0.0, 0.0],
            Some(json!({"content": "python programming"})),
        )];
        collection.upsert(points).unwrap();

        // Should no longer match "rust"
        let results = collection.text_search("rust", 10);
        assert!(results.is_empty());

        // Should now match "python"
        let results = collection.text_search("python", 10);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_bm25_large_dataset() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");

        let collection = Collection::create(path, 4, DistanceMetric::Cosine).unwrap();

        // Insert 100 documents
        let points: Vec<Point> = (0..100)
            .map(|i| {
                let content = if i % 10 == 0 {
                    format!("rust document number {i}")
                } else {
                    format!("other document number {i}")
                };
                Point::new(
                    i,
                    vec![0.1, 0.2, 0.3, 0.4],
                    Some(json!({"content": content})),
                )
            })
            .collect();
        collection.upsert(points).unwrap();

        // Search for "rust" - should find 10 documents (0, 10, 20, ..., 90)
        let results = collection.text_search("rust", 100);
        assert_eq!(results.len(), 10);

        // All results should have IDs divisible by 10
        for result in &results {
            assert_eq!(result.point.id % 10, 0);
        }
    }

    #[test]
    fn test_bm25_persistence_on_reopen() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");

        // Create collection and add documents
        {
            let collection = Collection::create(path.clone(), 4, DistanceMetric::Cosine).unwrap();

            let points = vec![
                Point::new(
                    1,
                    vec![1.0, 0.0, 0.0, 0.0],
                    Some(json!({"content": "Rust programming language"})),
                ),
                Point::new(
                    2,
                    vec![0.0, 1.0, 0.0, 0.0],
                    Some(json!({"content": "Python tutorial"})),
                ),
                Point::new(
                    3,
                    vec![0.0, 0.0, 1.0, 0.0],
                    Some(json!({"content": "Rust is fast and safe"})),
                ),
            ];
            collection.upsert(points).unwrap();

            // Verify search works before closing
            let results = collection.text_search("rust", 10);
            assert_eq!(results.len(), 2);
        }

        // Reopen collection and verify BM25 index is rebuilt
        {
            let collection = Collection::open(path).unwrap();

            // BM25 should be rebuilt from persisted payloads
            let results = collection.text_search("rust", 10);
            assert_eq!(results.len(), 2);

            let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
            assert!(ids.contains(&1));
            assert!(ids.contains(&3));
        }
    }

    // =========================================================================
    // Tests for upsert_bulk (optimized bulk import)
    // =========================================================================

    #[test]
    fn test_upsert_bulk_basic() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");
        let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

        let points = vec![
            Point::new(1, vec![1.0, 0.0, 0.0], None),
            Point::new(2, vec![0.0, 1.0, 0.0], None),
            Point::new(3, vec![0.0, 0.0, 1.0], None),
        ];

        let inserted = collection.upsert_bulk(&points).unwrap();
        assert_eq!(inserted, 3);
        assert_eq!(collection.len(), 3);
    }

    #[test]
    fn test_upsert_bulk_with_payload() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");
        let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

        let points = vec![
            Point::new(1, vec![1.0, 0.0, 0.0], Some(json!({"title": "Doc 1"}))),
            Point::new(2, vec![0.0, 1.0, 0.0], Some(json!({"title": "Doc 2"}))),
        ];

        collection.upsert_bulk(&points).unwrap();
        let retrieved = collection.get(&[1, 2]);
        assert_eq!(retrieved.len(), 2);
        assert!(retrieved[0].as_ref().unwrap().payload.is_some());
    }

    #[test]
    fn test_upsert_bulk_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");
        let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

        let points: Vec<Point> = vec![];
        let inserted = collection.upsert_bulk(&points).unwrap();
        assert_eq!(inserted, 0);
    }

    #[test]
    fn test_upsert_bulk_dimension_mismatch() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");
        let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

        let points = vec![
            Point::new(1, vec![1.0, 0.0, 0.0], None),
            Point::new(2, vec![0.0, 1.0], None), // Wrong dimension
        ];

        let result = collection.upsert_bulk(&points);
        assert!(result.is_err());
    }

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn test_upsert_bulk_large_batch() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");
        let collection = Collection::create(path, 64, DistanceMetric::Cosine).unwrap();

        let points: Vec<Point> = (0_u64..500)
            .map(|i| {
                let vector: Vec<f32> = (0_u64..64)
                    .map(|j| ((i + j) % 100) as f32 / 100.0)
                    .collect();
                Point::new(i, vector, None)
            })
            .collect();

        let inserted = collection.upsert_bulk(&points).unwrap();
        assert_eq!(inserted, 500);
        assert_eq!(collection.len(), 500);
    }

    #[test]
    fn test_upsert_bulk_search_works() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");
        let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

        let points = vec![
            Point::new(1, vec![1.0, 0.0, 0.0], None),
            Point::new(2, vec![0.9, 0.1, 0.0], None),
            Point::new(3, vec![0.0, 1.0, 0.0], None),
        ];

        collection.upsert_bulk(&points).unwrap();

        let query = vec![1.0, 0.0, 0.0];
        let results = collection.search(&query, 3).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].point.id, 1);
    }

    #[test]
    fn test_upsert_bulk_bm25_indexing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_collection");
        let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

        let points = vec![
            Point::new(
                1,
                vec![1.0, 0.0, 0.0],
                Some(json!({"content": "Rust lang"})),
            ),
            Point::new(2, vec![0.0, 1.0, 0.0], Some(json!({"content": "Python"}))),
            Point::new(
                3,
                vec![0.0, 0.0, 1.0],
                Some(json!({"content": "Rust fast"})),
            ),
        ];

        collection.upsert_bulk(&points).unwrap();
        let results = collection.text_search("rust", 10);
        assert_eq!(results.len(), 2);
    }
}
