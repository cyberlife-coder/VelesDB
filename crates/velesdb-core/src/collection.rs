//! Collection management for `VelesDB`.

use crate::distance::DistanceMetric;
use crate::error::{Error, Result};
use crate::point::{Point, SearchResult};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
#[derive(Debug, Clone)]
pub struct Collection {
    /// Path to the collection data.
    path: PathBuf,

    /// Collection configuration.
    config: Arc<RwLock<CollectionConfig>>,

    /// In-memory point storage (for MVP).
    points: Arc<RwLock<HashMap<u64, Point>>>,
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

        let collection = Self {
            path,
            config: Arc::new(RwLock::new(config)),
            points: Arc::new(RwLock::new(HashMap::new())),
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

        Ok(Self {
            path,
            config: Arc::new(RwLock::new(config)),
            points: Arc::new(RwLock::new(HashMap::new())),
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

        // Validate dimensions
        for point in &points {
            if point.dimension() != dimension {
                return Err(Error::DimensionMismatch {
                    expected: dimension,
                    actual: point.dimension(),
                });
            }
        }

        // Insert points
        let mut storage = self.points.write();
        for point in points {
            storage.insert(point.id, point);
        }

        // Update point count
        let mut config = self.config.write();
        config.point_count = storage.len();

        Ok(())
    }

    /// Retrieves points by their IDs.
    #[must_use]
    pub fn get(&self, ids: &[u64]) -> Vec<Option<Point>> {
        let storage = self.points.read();
        ids.iter().map(|id| storage.get(id).cloned()).collect()
    }

    /// Deletes points by their IDs.
    ///
    /// # Errors
    ///
    /// Currently infallible, but may return errors in future implementations.
    pub fn delete(&self, ids: &[u64]) -> Result<()> {
        let mut storage = self.points.write();
        for id in ids {
            storage.remove(id);
        }

        let mut config = self.config.write();
        config.point_count = storage.len();

        Ok(())
    }

    /// Searches for the k nearest neighbors of the query vector.
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

        let metric = config.metric;
        let higher_is_better = metric.higher_is_better();
        drop(config);

        let storage = self.points.read();

        // Calculate scores for all points (brute force for MVP)
        let mut scores: Vec<(u64, f32)> = storage
            .iter()
            .map(|(id, point)| (*id, metric.calculate(query, &point.vector)))
            .collect();

        // Sort by score
        if higher_is_better {
            scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        } else {
            scores.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        }

        // Take top k results
        let results: Vec<SearchResult> = scores
            .into_iter()
            .take(k)
            .filter_map(|(id, score)| {
                storage
                    .get(&id)
                    .map(|point| SearchResult::new(point.clone(), score))
            })
            .collect();

        Ok(results)
    }

    /// Returns the number of points in the collection.
    #[must_use]
    pub fn len(&self) -> usize {
        self.points.read().len()
    }

    /// Returns true if the collection is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.points.read().is_empty()
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
