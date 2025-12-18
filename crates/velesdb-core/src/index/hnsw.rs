//! HNSW (Hierarchical Navigable Small World) index implementation.
//!
//! This module provides a high-performance approximate nearest neighbor
//! search index based on the HNSW algorithm.

use crate::distance::DistanceMetric;
use crate::index::VectorIndex;
use hnsw_rs::prelude::*;
use parking_lot::RwLock;
use std::collections::HashMap;

/// HNSW index for efficient approximate nearest neighbor search.
///
/// # Example
///
/// ```rust,ignore
/// use velesdb_core::index::HnswIndex;
/// use velesdb_core::DistanceMetric;
///
/// let index = HnswIndex::new(768, DistanceMetric::Cosine);
/// index.insert(1, &vec![0.1; 768]);
/// let results = index.search(&vec![0.1; 768], 10);
/// ```
pub struct HnswIndex {
    /// Vector dimension
    dimension: usize,
    /// Distance metric
    metric: DistanceMetric,
    /// Internal HNSW index (type-erased for flexibility)
    inner: RwLock<HnswInner>,
    /// Mapping from external IDs to internal indices
    id_to_idx: RwLock<HashMap<u64, usize>>,
    /// Mapping from internal indices to external IDs
    idx_to_id: RwLock<HashMap<usize, u64>>,
    /// Next available internal index
    next_idx: RwLock<usize>,
}

/// Internal HNSW index wrapper to handle different distance metrics.
enum HnswInner {
    Cosine(Hnsw<'static, f32, DistCosine>),
    Euclidean(Hnsw<'static, f32, DistL2>),
    DotProduct(Hnsw<'static, f32, DistDot>),
}

impl HnswIndex {
    /// Creates a new HNSW index with the specified dimension and metric.
    ///
    /// # Arguments
    ///
    /// * `dimension` - The dimension of vectors to index
    /// * `metric` - The distance metric to use for similarity calculations
    ///
    /// # HNSW Parameters
    ///
    /// - `max_nb_connection` (M): 16 - Number of connections per layer
    /// - `ef_construction`: 200 - Size of dynamic candidate list during construction
    /// - `max_elements`: `100_000` - Maximum number of elements (can grow)
    #[must_use]
    pub fn new(dimension: usize, metric: DistanceMetric) -> Self {
        // HNSW parameters optimized for recall/speed tradeoff
        let max_nb_connection = 16;
        let ef_construction = 200;
        let max_elements = 100_000;

        let inner = match metric {
            DistanceMetric::Cosine => HnswInner::Cosine(Hnsw::new(
                max_nb_connection,
                max_elements,
                16,
                ef_construction,
                DistCosine,
            )),
            DistanceMetric::Euclidean => HnswInner::Euclidean(Hnsw::new(
                max_nb_connection,
                max_elements,
                16,
                ef_construction,
                DistL2,
            )),
            DistanceMetric::DotProduct => HnswInner::DotProduct(Hnsw::new(
                max_nb_connection,
                max_elements,
                16,
                ef_construction,
                DistDot,
            )),
        };

        Self {
            dimension,
            metric,
            inner: RwLock::new(inner),
            id_to_idx: RwLock::new(HashMap::new()),
            idx_to_id: RwLock::new(HashMap::new()),
            next_idx: RwLock::new(0),
        }
    }
}

impl VectorIndex for HnswIndex {
    fn insert(&self, id: u64, vector: &[f32]) {
        assert_eq!(
            vector.len(),
            self.dimension,
            "Vector dimension mismatch: expected {}, got {}",
            self.dimension,
            vector.len()
        );

        // Get or create internal index for this ID
        let mut id_to_idx = self.id_to_idx.write();
        let mut idx_to_id = self.idx_to_id.write();
        let mut next_idx = self.next_idx.write();

        let idx = if let Some(&existing_idx) = id_to_idx.get(&id) {
            existing_idx
        } else {
            let idx = *next_idx;
            *next_idx += 1;
            id_to_idx.insert(id, idx);
            idx_to_id.insert(idx, id);
            idx
        };

        drop(id_to_idx);
        drop(idx_to_id);
        drop(next_idx);

        // Insert into HNSW index
        let inner = self.inner.write();
        match &*inner {
            HnswInner::Cosine(hnsw) => {
                hnsw.insert((vector, idx));
            }
            HnswInner::Euclidean(hnsw) => {
                hnsw.insert((vector, idx));
            }
            HnswInner::DotProduct(hnsw) => {
                hnsw.insert((vector, idx));
            }
        }
    }

    fn search(&self, query: &[f32], k: usize) -> Vec<(u64, f32)> {
        assert_eq!(
            query.len(),
            self.dimension,
            "Query dimension mismatch: expected {}, got {}",
            self.dimension,
            query.len()
        );

        let ef_search = 50.max(k * 2); // ef should be >= k
        let inner = self.inner.read();
        let idx_to_id = self.idx_to_id.read();

        let results: Vec<(u64, f32)> = match &*inner {
            HnswInner::Cosine(hnsw) => {
                let neighbours = hnsw.search(query, k, ef_search);
                neighbours
                    .iter()
                    .filter_map(|n| {
                        idx_to_id.get(&n.d_id).map(|&id| {
                            // Cosine: hnsw_rs returns distance, we want similarity
                            (id, 1.0 - n.distance)
                        })
                    })
                    .collect()
            }
            HnswInner::Euclidean(hnsw) => {
                let neighbours = hnsw.search(query, k, ef_search);
                neighbours
                    .iter()
                    .filter_map(|n| idx_to_id.get(&n.d_id).map(|&id| (id, n.distance)))
                    .collect()
            }
            HnswInner::DotProduct(hnsw) => {
                let neighbours = hnsw.search(query, k, ef_search);
                neighbours
                    .iter()
                    .filter_map(|n| {
                        idx_to_id.get(&n.d_id).map(|&id| {
                            // DotProduct: higher is better
                            (id, -n.distance)
                        })
                    })
                    .collect()
            }
        };

        results
    }

    fn remove(&self, id: u64) -> bool {
        let mut id_to_idx = self.id_to_idx.write();
        let mut idx_to_id = self.idx_to_id.write();

        if let Some(idx) = id_to_idx.remove(&id) {
            idx_to_id.remove(&idx);
            // Note: hnsw_rs doesn't support direct removal
            // We mark it as removed in our mappings
            true
        } else {
            false
        }
    }

    fn len(&self) -> usize {
        self.id_to_idx.read().len()
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn metric(&self) -> DistanceMetric {
        self.metric
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // TDD Tests - Written BEFORE implementation (RED phase)
    // =========================================================================

    #[test]
    fn test_hnsw_new_creates_empty_index() {
        // Arrange & Act
        let index = HnswIndex::new(768, DistanceMetric::Cosine);

        // Assert
        assert!(index.is_empty());
        assert_eq!(index.len(), 0);
        assert_eq!(index.dimension(), 768);
        assert_eq!(index.metric(), DistanceMetric::Cosine);
    }

    #[test]
    fn test_hnsw_insert_single_vector() {
        // Arrange
        let index = HnswIndex::new(3, DistanceMetric::Cosine);
        let vector = vec![1.0, 0.0, 0.0];

        // Act
        index.insert(1, &vector);

        // Assert
        assert_eq!(index.len(), 1);
        assert!(!index.is_empty());
    }

    #[test]
    fn test_hnsw_insert_multiple_vectors() {
        // Arrange
        let index = HnswIndex::new(3, DistanceMetric::Cosine);

        // Act
        index.insert(1, &[1.0, 0.0, 0.0]);
        index.insert(2, &[0.0, 1.0, 0.0]);
        index.insert(3, &[0.0, 0.0, 1.0]);

        // Assert
        assert_eq!(index.len(), 3);
    }

    #[test]
    fn test_hnsw_search_returns_k_nearest() {
        // Arrange
        let index = HnswIndex::new(3, DistanceMetric::Cosine);
        index.insert(1, &[1.0, 0.0, 0.0]);
        index.insert(2, &[0.9, 0.1, 0.0]); // Similar to 1
        index.insert(3, &[0.0, 1.0, 0.0]); // Different

        // Act
        let results = index.search(&[1.0, 0.0, 0.0], 2);

        // Assert
        assert_eq!(results.len(), 2);
        // First result should be exact match (id=1)
        assert_eq!(results[0].0, 1);
    }

    #[test]
    fn test_hnsw_search_empty_index() {
        // Arrange
        let index = HnswIndex::new(3, DistanceMetric::Cosine);

        // Act
        let results = index.search(&[1.0, 0.0, 0.0], 10);

        // Assert
        assert!(results.is_empty());
    }

    #[test]
    fn test_hnsw_remove_existing_vector() {
        // Arrange
        let index = HnswIndex::new(3, DistanceMetric::Cosine);
        index.insert(1, &[1.0, 0.0, 0.0]);
        index.insert(2, &[0.0, 1.0, 0.0]);

        // Act
        let removed = index.remove(1);

        // Assert
        assert!(removed);
        assert_eq!(index.len(), 1);
    }

    #[test]
    fn test_hnsw_remove_nonexistent_vector() {
        // Arrange
        let index = HnswIndex::new(3, DistanceMetric::Cosine);
        index.insert(1, &[1.0, 0.0, 0.0]);

        // Act
        let removed = index.remove(999);

        // Assert
        assert!(!removed);
        assert_eq!(index.len(), 1);
    }

    #[test]
    fn test_hnsw_euclidean_metric() {
        // Arrange
        let index = HnswIndex::new(3, DistanceMetric::Euclidean);
        index.insert(1, &[0.0, 0.0, 0.0]);
        index.insert(2, &[1.0, 0.0, 0.0]); // Distance 1
        index.insert(3, &[3.0, 4.0, 0.0]); // Distance 5

        // Act
        let results = index.search(&[0.0, 0.0, 0.0], 3);

        // Assert
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].0, 1); // Closest (exact match)
    }

    #[test]
    fn test_hnsw_dot_product_metric() {
        // Arrange - Use normalized positive vectors for dot product
        // DistDot in hnsw_rs requires non-negative dot products
        let index = HnswIndex::new(3, DistanceMetric::DotProduct);

        // Insert vectors with distinct dot products when queried with [1,0,0]
        index.insert(1, &[1.0, 0.0, 0.0]); // dot=1.0 with query
        index.insert(2, &[0.5, 0.5, 0.5]); // dot=0.5 with query
        index.insert(3, &[0.1, 0.1, 0.1]); // dot=0.1 with query

        // Act - Query with unit vector x
        let query = [1.0, 0.0, 0.0];
        let results = index.search(&query, 3);

        // Assert
        assert_eq!(results.len(), 3);
        // All three IDs should be present in results
        let ids: Vec<u64> = results.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&1));
        assert!(ids.contains(&2));
        assert!(ids.contains(&3));
    }

    #[test]
    #[should_panic(expected = "Vector dimension mismatch")]
    fn test_hnsw_insert_wrong_dimension_panics() {
        // Arrange
        let index = HnswIndex::new(3, DistanceMetric::Cosine);

        // Act - should panic
        index.insert(1, &[1.0, 0.0]); // Wrong dimension
    }

    #[test]
    #[should_panic(expected = "Query dimension mismatch")]
    fn test_hnsw_search_wrong_dimension_panics() {
        // Arrange
        let index = HnswIndex::new(3, DistanceMetric::Cosine);
        index.insert(1, &[1.0, 0.0, 0.0]);

        // Act - should panic
        let _ = index.search(&[1.0, 0.0], 10); // Wrong dimension
    }

    #[test]
    fn test_hnsw_update_existing_vector() {
        // Arrange
        let index = HnswIndex::new(3, DistanceMetric::Cosine);
        index.insert(1, &[1.0, 0.0, 0.0]);

        // Act - Insert with same ID should update
        index.insert(1, &[0.0, 1.0, 0.0]);

        // Assert
        assert_eq!(index.len(), 1); // Still only one entry
    }

    #[test]
    fn test_hnsw_thread_safety() {
        use std::sync::Arc;
        use std::thread;

        // Arrange
        let index = Arc::new(HnswIndex::new(3, DistanceMetric::Cosine));
        let mut handles = vec![];

        // Act - Insert from multiple threads
        for i in 0..10 {
            let index_clone = Arc::clone(&index);
            handles.push(thread::spawn(move || {
                #[allow(clippy::cast_precision_loss)]
                index_clone.insert(i, &[i as f32, 0.0, 0.0]);
            }));
        }

        for handle in handles {
            handle.join().expect("Thread panicked");
        }

        // Assert
        assert_eq!(index.len(), 10);
    }
}
