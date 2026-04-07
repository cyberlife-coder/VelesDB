//! Scroll cursor for paginated iteration over collection points.
//!
//! Provides `ScrollBatch` and `Collection::scroll_batch` for deterministic,
//! ascending-ID iteration with optional payload filtering.

use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::point::Point;
use crate::storage::{PayloadStorage, VectorStorage};

/// Result of a single scroll batch operation.
///
/// Contains the points in this batch (ascending ID order) and the cursor
/// position for resuming iteration.
#[derive(Debug, Clone)]
pub struct ScrollBatch {
    /// Points in this batch, ordered by ascending ID.
    pub points: Vec<Point>,
    /// Cursor for the next batch (`None` if no more points).
    /// This is the ID of the last point in this batch.
    pub next_cursor: Option<u64>,
}

impl Collection {
    /// Returns the next batch of points starting after `cursor`.
    ///
    /// - `cursor`: `None` to start from the beginning, `Some(id)` to resume
    ///   after the given point ID (exclusive).
    /// - `batch_size`: Maximum number of points to return. Must be > 0.
    /// - `filter`: Optional payload filter. Points not matching are skipped.
    ///
    /// Points are returned in ascending ID order for deterministic iteration.
    ///
    /// # Errors
    ///
    /// Returns `Error::Config` if `batch_size` is 0.
    pub fn scroll_batch(
        &self,
        cursor: Option<u64>,
        batch_size: usize,
        filter: Option<&Filter>,
    ) -> Result<ScrollBatch> {
        if batch_size == 0 {
            return Err(Error::Config(
                "batch_size must be greater than 0".to_string(),
            ));
        }

        // all_point_ids() returns IDs pre-sorted via BTreeSet (see crud_read_delete.rs).
        // Binary search via partition_point is O(log N) per batch.
        let ids = self.all_point_ids();

        let start = match cursor {
            Some(c) => ids.partition_point(|&id| id <= c),
            None => 0,
        };

        let candidates = &ids[start..];
        let points = self.collect_filtered_batch(candidates, batch_size, filter);

        let next_cursor = points.last().map(|p| p.id);
        Ok(ScrollBatch {
            points,
            next_cursor,
        })
    }

    /// Collects up to `batch_size` points from `candidate_ids`, applying an optional filter.
    fn collect_filtered_batch(
        &self,
        candidate_ids: &[u64],
        batch_size: usize,
        filter: Option<&Filter>,
    ) -> Vec<Point> {
        let config = self.config.read();
        let is_metadata_only = config.metadata_only;
        drop(config);

        let payload_storage = self.payload_storage.read();
        let vector_storage = self.vector_storage.read();

        let mut points = Vec::with_capacity(batch_size);
        for &id in candidate_ids {
            if points.len() >= batch_size {
                break;
            }
            if let Some(point) =
                Self::build_point(id, is_metadata_only, &*payload_storage, &*vector_storage)
            {
                if Self::passes_filter(&point, filter) {
                    points.push(point);
                }
            }
        }
        points
    }

    /// Builds a `Point` from storage. Always returns `Some`; points without a
    /// stored vector get an empty vector slice.
    fn build_point(
        id: u64,
        is_metadata_only: bool,
        payload_storage: &dyn PayloadStorage,
        vector_storage: &dyn VectorStorage,
    ) -> Option<Point> {
        let payload = payload_storage.retrieve(id).ok().flatten();
        // Graph nodes inserted via upsert_node_payload() have no vector in storage.
        // Use unwrap_or_default() so payload-only nodes are included, not silently skipped.
        let vector = if is_metadata_only {
            Vec::new()
        } else {
            vector_storage
                .retrieve(id)
                .ok()
                .flatten()
                .unwrap_or_default()
        };
        Some(Point {
            id,
            vector,
            payload,
            sparse_vectors: None,
        })
    }

    /// Returns `true` if the point passes the optional filter.
    fn passes_filter(point: &Point, filter: Option<&Filter>) -> bool {
        match (filter, &point.payload) {
            (Some(f), Some(payload)) => f.matches(payload),
            (Some(_), None) => false,
            (None, _) => true,
        }
    }
}
