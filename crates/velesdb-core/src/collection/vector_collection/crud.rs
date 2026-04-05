//! CRUD and index-mutation operations for `VectorCollection`.

use crate::error::Result;
use crate::point::Point;

use super::VectorCollection;

impl VectorCollection {
    /// Bulk insert optimized for high-throughput import.
    ///
    /// # Errors
    ///
    /// Returns an error if any point has a mismatched dimension.
    pub fn upsert_bulk(&self, points: &[Point]) -> Result<usize> {
        self.inner.upsert_bulk(points)
    }

    /// Bulk insert from contiguous flat slices (zero-copy from numpy / FFI).
    ///
    /// Accepts a flat `f32` slice of shape `(n, dimension)` in row-major order
    /// plus a matching `u64` ID slice of length `n`. Avoids per-row `Vec<f32>`
    /// allocation, saving ~293 MB for 100K vectors at 768D.
    ///
    /// # Errors
    ///
    /// - Returns [`Error::InvalidVector`] if `vectors.len() != ids.len() * dimension`.
    /// - Returns [`Error::DimensionMismatch`] if `dimension` mismatches the collection.
    pub fn upsert_bulk_from_raw(
        &self,
        vectors: &[f32],
        ids: &[u64],
        dimension: usize,
        payloads: Option<&[Option<serde_json::Value>]>,
    ) -> Result<usize> {
        self.inner
            .upsert_bulk_from_raw(vectors, ids, dimension, payloads)
    }

    /// Inserts or updates points in the collection.
    ///
    /// # Errors
    ///
    /// - Returns an error if any point's dimension does not match the collection.
    /// - Returns an error if storage operations fail.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use velesdb_core::{VectorCollection, DistanceMetric, Point, StorageMode};
    /// # use serde_json::json;
    /// # let coll = VectorCollection::create("./data/v".into(), "v", 128, DistanceMetric::Cosine, StorageMode::Full)?;
    /// coll.upsert(vec![
    ///     Point::new(1, vec![0.1; 128], Some(json!({"title": "Hello"}))),
    ///     Point::new(2, vec![0.2; 128], None),
    /// ])?;
    /// # Ok::<(), velesdb_core::Error>(())
    /// ```
    pub fn upsert(&self, points: impl IntoIterator<Item = Point>) -> Result<()> {
        self.inner.upsert(points)
    }

    /// Retrieves points by IDs, returning `None` for missing entries.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use velesdb_core::{VectorCollection, DistanceMetric, StorageMode};
    /// # let coll = VectorCollection::create("./data/v".into(), "v", 128, DistanceMetric::Cosine, StorageMode::Full)?;
    /// let points = coll.get(&[1, 2, 3]);
    /// for (id, maybe_point) in [1, 2, 3].iter().zip(&points) {
    ///     if let Some(p) = maybe_point {
    ///         println!("Found point {id} with payload {:?}", p.payload);
    ///     }
    /// }
    /// # Ok::<(), velesdb_core::Error>(())
    /// ```
    #[must_use]
    pub fn get(&self, ids: &[u64]) -> Vec<Option<Point>> {
        self.inner.get(ids)
    }

    /// Deletes points by IDs.
    ///
    /// Missing IDs are silently ignored.
    ///
    /// # Errors
    ///
    /// - Returns an error if storage operations fail.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use velesdb_core::{VectorCollection, DistanceMetric, StorageMode};
    /// # let coll = VectorCollection::create("./data/v".into(), "v", 128, DistanceMetric::Cosine, StorageMode::Full)?;
    /// coll.delete(&[1, 2, 3])?;
    /// # Ok::<(), velesdb_core::Error>(())
    /// ```
    pub fn delete(&self, ids: &[u64]) -> Result<()> {
        self.inner.delete(ids)
    }

    /// Inserts or updates metadata-only points (no vectors).
    ///
    /// # Errors
    ///
    /// - Returns an error if storage operations fail.
    pub fn upsert_metadata(
        &self,
        points: impl IntoIterator<Item = crate::point::Point>,
    ) -> Result<()> {
        self.inner.upsert_metadata(points)
    }

    /// Creates a secondary metadata index on a payload field.
    ///
    /// # Errors
    ///
    /// - Returns an error if the index already exists or storage fails.
    pub fn create_index(&self, field: &str) -> Result<()> {
        self.inner.create_index(field)
    }

    /// Creates a property index for O(1) equality lookups.
    ///
    /// # Errors
    ///
    /// - Returns an error if the index already exists or storage fails.
    pub fn create_property_index(&self, label: &str, property: &str) -> Result<()> {
        self.inner.create_property_index(label, property)
    }

    /// Creates a range index for O(log n) range queries.
    ///
    /// # Errors
    ///
    /// - Returns an error if the index already exists or storage fails.
    pub fn create_range_index(&self, label: &str, property: &str) -> Result<()> {
        self.inner.create_range_index(label, property)
    }

    /// Drops an index, returning `true` if an index was removed.
    ///
    /// # Errors
    ///
    /// - Returns an error if the drop operation fails.
    pub fn drop_index(&self, label: &str, property: &str) -> Result<bool> {
        self.inner.drop_index(label, property)
    }

    /// Adds a graph edge to the collection.
    ///
    /// # Errors
    ///
    /// Returns an error if the edge cannot be stored.
    pub fn add_edge(&self, edge: crate::collection::GraphEdge) -> Result<()> {
        self.inner.add_edge(edge)
    }
}
