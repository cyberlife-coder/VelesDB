//! Shared test fixtures for `velesdb-core` tests.
//!
//! Centralizes collection creation, point generation, and setup patterns
//! to avoid duplication across test modules. All items are `#[cfg(test)]`
//! gated at the module level.
//!
//! # Usage
//!
//! ```rust,ignore
//! use crate::test_fixtures::fixtures::{setup_collection, make_point};
//!
//! let (_dir, col) = setup_collection(4);
//! let p = make_point(1, vec![1.0, 0.0, 0.0, 0.0]);
//! ```

#[cfg(test)]
#[allow(deprecated)] // Collection is deprecated but used in legacy tests.
pub(crate) mod fixtures {
    use crate::collection::Collection;
    use crate::distance::DistanceMetric;
    use crate::point::Point;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Creates a test collection with the given dimension and cosine metric.
    ///
    /// Returns the `TempDir` guard (must be kept alive for the collection's
    /// lifetime) and the newly created `Collection`.
    pub fn setup_collection(dim: usize) -> (TempDir, Collection) {
        let dir = tempfile::tempdir().expect("test: tempdir creation");
        let col = Collection::create(PathBuf::from(dir.path()), dim, DistanceMetric::Cosine)
            .expect("test: collection creation");
        (dir, col)
    }

    /// Creates a test collection pre-populated with the given points.
    ///
    /// Combines [`setup_collection`] with an immediate `upsert` call,
    /// eliminating the two-step boilerplate common in test setup functions.
    #[allow(dead_code)] // Available for future test adoption.
    pub fn setup_collection_with_points(dim: usize, points: Vec<Point>) -> (TempDir, Collection) {
        let (dir, col) = setup_collection(dim);
        col.upsert(points).expect("test: upsert");
        (dir, col)
    }

    /// Creates a simple test point with no payload or sparse vectors.
    #[allow(dead_code)] // Available for future test adoption.
    pub fn make_point(id: u64, vector: Vec<f32>) -> Point {
        Point {
            id,
            vector,
            payload: None,
            sparse_vectors: None,
        }
    }

    /// Creates a test point with a JSON payload.
    pub fn make_point_with_payload(id: u64, vector: Vec<f32>, payload: serde_json::Value) -> Point {
        Point {
            id,
            vector,
            payload: Some(payload),
            sparse_vectors: None,
        }
    }
}
