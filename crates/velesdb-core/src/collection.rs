//! Collection management for `VelesDB`.

use crate::distance::DistanceMetric;
use crate::error::{Error, Result};
use crate::index::{HnswIndex, VectorIndex};
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

        let collection = Self {
            path,
            config: Arc::new(RwLock::new(config)),
            vector_storage,
            payload_storage,
            index,
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

        Ok(Self {
            path,
            config: Arc::new(RwLock::new(config)),
            vector_storage,
            payload_storage,
            index,
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

            // 3. Update Index
            // Note: HnswIndex.insert() skips if ID already exists (no updates supported)
            // For true upsert semantics, we'd need to remove then re-insert
            self.index.insert(point.id, &point.vector);
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
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
