//! Graduated ef\_construction schedule for 3-phase batch insertion.
//!
//! Extracted from `backend_adapter.rs` to reduce NLOC. Contains:
//! - `BatchEfSchedule`: Phase-based ef schedule for VAMANA/DiskANN pattern
//! - `compute_batch_ef_schedule`: Computes the schedule from base parameters

/// Graduated ef\_construction schedule for 3-phase batch insertion.
///
/// Based on the VAMANA/DiskANN pattern: the first 10% of a batch uses full
/// `ef_construction` to build a quality scaffold while the graph is sparse,
/// the middle 80% uses a reduced ef (0.5x) since the graph is dense enough
/// for fast convergence, and the final 10% uses moderate ef (0.75x) to
/// finalize connections with reasonable quality.
///
/// For small batches (< 1000), all phases use full `ef_construction`.
#[derive(Debug, Clone)]
pub(in crate::index::hnsw::native) struct BatchEfSchedule {
    /// Phase 1 ef: full `ef_construction` for the scaffold.
    pub scaffold_ef: usize,
    /// Phase 2 ef: reduced (0.5x) for the dense bulk.
    pub bulk_ef: usize,
    /// Phase 3 ef: moderate (0.75x) for finalization.
    pub finalize_ef: usize,
    /// Number of nodes in the scaffold phase (first 10%).
    pub scaffold_count: usize,
    /// Index at which the finalize phase begins (90% of batch).
    pub finalize_start: usize,
}

impl BatchEfSchedule {
    /// Returns the appropriate ef for a node at the given position in the batch.
    #[inline]
    #[must_use]
    pub fn ef_for_position(&self, position: usize) -> usize {
        if position < self.scaffold_count {
            self.scaffold_ef
        } else if position >= self.finalize_start {
            self.finalize_ef
        } else {
            self.bulk_ef
        }
    }
}

/// Computes a graduated ef schedule for batch insertion.
///
/// For batches < 1000, returns uniform full ef (no reduction).
/// For larger batches, applies the 3-phase graduated schedule with
/// a floor of `2 * m` to guarantee minimum graph connectivity.
#[must_use]
pub(in crate::index::hnsw::native) fn compute_batch_ef_schedule(
    base_ef: usize,
    batch_size: usize,
    m: usize,
) -> BatchEfSchedule {
    let floor = 2 * m;

    if batch_size < 1000 {
        return BatchEfSchedule {
            scaffold_ef: base_ef,
            bulk_ef: base_ef,
            finalize_ef: base_ef,
            scaffold_count: batch_size,
            finalize_start: batch_size,
        };
    }

    let scaffold_count = batch_size / 10;
    let finalize_start = batch_size - (batch_size / 10);

    BatchEfSchedule {
        scaffold_ef: base_ef,
        bulk_ef: (base_ef / 2).max(floor),
        finalize_ef: (base_ef * 3 / 4).max(floor),
        scaffold_count,
        finalize_start,
    }
}
