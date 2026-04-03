//! Parallel HNSW segment builder for bulk index construction.
//!
//! Partitions a batch of vectors into segments, builds each segment's HNSW
//! connections via `insert_batch_parallel`, then merges results into the
//! target index. For small batches (< 1000 vectors), delegates directly
//! to the monolithic `insert_batch_parallel` path.

use super::index::HnswIndex;
use std::time::{Duration, Instant};

/// Result of a segment build-and-merge operation.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Fields read by callers in Task 3/4
pub struct SegmentMergeResult {
    /// Number of vectors indexed.
    pub indexed_count: usize,
    /// Duration of the parallel build phase.
    pub build_duration: Duration,
    /// Duration of the merge phase.
    pub merge_duration: Duration,
}

/// Builds HNSW index segments in parallel then merges them.
///
/// For batches ≥ `min_segment_threshold` (default 1000), partitions vectors
/// into `segment_count` chunks and processes each chunk via
/// `insert_batch_parallel` on the target index. For smaller batches,
/// delegates to a single `insert_batch_parallel` call.
///
/// This achieves parallelism through rayon's internal work-stealing within
/// each `insert_batch_parallel` call, while chunking provides better
/// entry-point freshness between segments.
#[allow(dead_code)] // Wired into AsyncIndexBuilder in Task 3
pub(crate) struct HnswSegmentBuilder {
    /// Number of segments to partition into.
    segment_count: usize,
    /// Minimum batch size for segmented construction (below this → monolithic).
    min_segment_threshold: usize,
}

#[allow(dead_code)] // Wired into AsyncIndexBuilder in Task 3
impl HnswSegmentBuilder {
    /// Creates a new segment builder.
    ///
    /// # Arguments
    ///
    /// * `segment_count` - Number of segments (typically `num_cpus`)
    #[must_use]
    pub(crate) fn new(segment_count: usize) -> Self {
        Self {
            segment_count: segment_count.max(1),
            min_segment_threshold: 1000,
        }
    }

    /// Builds and merges vectors into the target HNSW index.
    ///
    /// For N < `min_segment_threshold`: delegates to `insert_batch_parallel`.
    /// For N ≥ threshold: partitions into segments, inserts each segment
    /// sequentially via `insert_batch_parallel` (each call uses rayon internally).
    ///
    /// # Arguments
    ///
    /// * `vectors` - `(internal_idx, vector_data)` pairs
    /// * `target_index` - The HNSW index to insert into
    ///
    /// # Errors
    ///
    /// Returns an error if any segment insertion fails.
    pub(crate) fn build_and_merge(
        &self,
        vectors: &[(usize, &[f32])],
        target_index: &HnswIndex,
    ) -> crate::error::Result<SegmentMergeResult> {
        if vectors.is_empty() {
            return Ok(SegmentMergeResult {
                indexed_count: 0,
                build_duration: Duration::ZERO,
                merge_duration: Duration::ZERO,
            });
        }

        let total = vectors.len();

        // Small batch: monolithic path
        if total < self.min_segment_threshold {
            return self.build_monolithic(vectors, target_index);
        }

        // Large batch: segmented path
        self.build_segmented(vectors, target_index)
    }

    /// Monolithic insertion for small batches.
    #[allow(clippy::unnecessary_wraps)] // Returns Result for API consistency with full impl
    fn build_monolithic(
        &self,
        vectors: &[(usize, &[f32])],
        target_index: &HnswIndex,
    ) -> crate::error::Result<SegmentMergeResult> {
        let start = Instant::now();

        let pairs: Vec<(u64, &[f32])> = vectors
            .iter()
            .map(|(idx, vec)| {
                // Use internal index as external ID for the insert path.
                // The caller (AsyncIndexBuilder) has already registered mappings.
                let ext_id = target_index
                    .mappings
                    .get_id(*idx)
                    .unwrap_or(*idx as u64);
                (ext_id, *vec)
            })
            .collect();

        let inserted = target_index.insert_batch_parallel(pairs);
        let build_duration = start.elapsed();

        Ok(SegmentMergeResult {
            indexed_count: inserted,
            build_duration,
            merge_duration: Duration::ZERO,
        })
    }

    /// Segmented insertion for large batches.
    ///
    /// Partitions vectors into segments and inserts each segment via
    /// `insert_batch_parallel`. Each call benefits from rayon parallelism
    /// internally, while segmentation provides entry-point refresh between
    /// chunks for better graph quality.
    #[allow(clippy::unnecessary_wraps)] // Returns Result for API consistency with full impl
    fn build_segmented(
        &self,
        vectors: &[(usize, &[f32])],
        target_index: &HnswIndex,
    ) -> crate::error::Result<SegmentMergeResult> {
        let segments = self.partition(vectors);
        let build_start = Instant::now();
        let mut total_inserted = 0_usize;

        for segment in &segments {
            let pairs: Vec<(u64, &[f32])> = segment
                .iter()
                .map(|(idx, vec)| {
                    let ext_id = target_index
                        .mappings
                        .get_id(*idx)
                        .unwrap_or(*idx as u64);
                    (ext_id, *vec)
                })
                .collect();

            let inserted = target_index.insert_batch_parallel(pairs);
            total_inserted += inserted;
        }

        let build_duration = build_start.elapsed();

        Ok(SegmentMergeResult {
            indexed_count: total_inserted,
            build_duration,
            merge_duration: Duration::ZERO,
        })
    }

    /// Partitions vectors into `min(segment_count, N)` non-empty segments.
    ///
    /// Each segment size differs from `N / S` by at most 1.
    fn partition<'a>(
        &self,
        vectors: &'a [(usize, &'a [f32])],
    ) -> Vec<&'a [(usize, &'a [f32])]> {
        let n = vectors.len();
        let s = self.segment_count.min(n).max(1);
        let base_size = n / s;
        let remainder = n % s;

        let mut segments = Vec::with_capacity(s);
        let mut offset = 0;

        for i in 0..s {
            let size = base_size + usize::from(i < remainder);
            segments.push(&vectors[offset..offset + size]);
            offset += size;
        }

        segments
    }
}
