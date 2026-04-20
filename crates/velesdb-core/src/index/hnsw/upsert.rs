//! Shared upsert-mapping logic for HNSW index variants.
//!
//! Both `HnswIndex` and `NativeHnswIndex` use identical mapping upsert
//! semantics. This module provides a single implementation to avoid
//! duplication.

use super::sharded_mappings::ShardedMappings;
use super::sharded_vectors::ShardedVectors;
use crate::validation::validate_dimension_match;

/// Result of an upsert mapping operation, carrying rollback information.
///
/// On success the caller uses `idx` as the internal HNSW index for the new
/// graph node. On graph-insert failure, the caller passes this struct to
/// [`rollback_upsert`] to restore the previous state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpsertResult {
    /// Newly allocated internal index for the vector.
    pub idx: usize,
    /// Previous internal index if this was an update (not a fresh insert).
    pub old_idx: Option<usize>,
}

/// Registers an ID with upsert semantics and cleans up stale vector data.
///
/// If the ID already existed, the old mapping is replaced and the stale
/// sidecar vector (if stored) is removed from `ShardedVectors`.
///
/// Returns an [`UpsertResult`] containing the new index and optional old
/// index for rollback purposes.
#[must_use]
pub(crate) fn upsert_mapping(
    mappings: &ShardedMappings,
    vectors: &ShardedVectors,
    enable_vector_storage: bool,
    id: u64,
) -> UpsertResult {
    let (idx, old_idx) = mappings.register_or_replace(id);
    if let Some(old) = old_idx {
        if enable_vector_storage {
            vectors.remove(old);
        }
    }
    UpsertResult { idx, old_idx }
}

/// Batch version of `upsert_mapping` with fast-path for new IDs.
///
/// Uses `register_or_replace_batch` which skips the expensive `entry()`
/// path for IDs that don't exist yet (common in pure-insert workloads).
///
/// # Phase Ordering
///
/// Callers must validate vector dimensions **before** calling this function.
/// Once mapping registration begins, the mutations cannot be cheaply undone
/// without explicit rollback. See `prepare_batch_insert()` in `batch.rs`
/// for the canonical call sequence.
#[must_use]
pub(crate) fn upsert_mapping_batch(
    mappings: &ShardedMappings,
    vectors: &ShardedVectors,
    enable_vector_storage: bool,
    ids: &[u64],
) -> Vec<UpsertResult> {
    let batch_results = mappings.register_or_replace_batch(ids);
    let mut results = Vec::with_capacity(batch_results.len());
    for (idx, old_idx) in batch_results {
        if let Some(old) = old_idx {
            if enable_vector_storage {
                vectors.remove(old);
            }
        }
        results.push(UpsertResult { idx, old_idx });
    }
    results
}

/// Validates dimensions for every vector in `items`, then registers the IDs
/// via [`upsert_mapping_batch`].
///
/// # Phase Ordering Invariant
///
/// Dimension validation runs to completion **before** any call to
/// `upsert_mapping_batch` — a mismatch discovered after partial upsert
/// would leave orphaned mappings. See `docs/SOUNDNESS.md` "HNSW Batch
/// Insertion Ordering".
///
/// Shared by `HnswIndex::prepare_batch_insert` and
/// `NativeHnswIndex::insert_batch` (#448 Group D). Generic over the vector
/// slice lifetime so callers can pass either `&Vec<f32>` or `&[f32]`.
///
/// # Errors
///
/// Returns [`crate::error::Error::DimensionMismatch`] on the first vector
/// whose dimension differs from `expected_dimension`.
pub(crate) fn validate_and_register_batch<V: AsRef<[f32]>>(
    mappings: &ShardedMappings,
    vectors: &ShardedVectors,
    enable_vector_storage: bool,
    expected_dimension: usize,
    items: &[(u64, V)],
) -> crate::error::Result<Vec<UpsertResult>> {
    for (_id, vec) in items {
        validate_dimension_match(expected_dimension, vec.as_ref().len())?;
    }

    let ids: Vec<u64> = items.iter().map(|(id, _)| *id).collect();
    Ok(upsert_mapping_batch(
        mappings,
        vectors,
        enable_vector_storage,
        &ids,
    ))
}

/// Soft-deletes a single ID: removes it from mappings and, when vector
/// storage is enabled, removes the corresponding sidecar slot.
///
/// Returns `true` if the ID existed and was removed, `false` if it was
/// already absent. The HNSW graph node itself is left in place — it becomes
/// a tombstone that is filtered out during search via the reverse mapping.
///
/// Shared by `HnswIndex::remove` and `NativeHnswIndex::remove` (identical
/// bodies, #448 Group F consolidation).
pub(crate) fn soft_delete(
    mappings: &ShardedMappings,
    vectors: &ShardedVectors,
    enable_vector_storage: bool,
    id: u64,
) -> bool {
    if let Some(old_idx) = mappings.remove(id) {
        if enable_vector_storage {
            vectors.remove(old_idx);
        }
        true
    } else {
        false
    }
}

/// Reconciles pre-registered mapping indices with graph-assigned node IDs.
///
/// `upsert_mapping_batch` allocates internal indices optimistically (one per
/// item) but `parallel_insert` may assign different node IDs when the HNSW
/// graph recycles slots or the rayon ordering diverges. This helper brings
/// the mappings back in sync with whatever the graph decided:
///
/// * If the graph-assigned ID equals the pre-registered index, nothing to do.
/// * Otherwise, remove the stale reverse mapping (`result.idx -> ext_id`) and
///   restore the authoritative one (`ext_id <-> assigned_id`).
///
/// Returns the list of authoritative storage indices, in input order, so the
/// caller can store sidecar vectors at the correct slot.
///
/// Both `HnswIndex::insert_batch_parallel` and `NativeHnswIndex::insert_batch`
/// used to duplicate this logic — consolidated here for #448 Group D.
#[must_use]
pub(crate) fn reconcile_batch_mappings(
    mappings: &ShardedMappings,
    rollback_info: &[(u64, UpsertResult)],
    assigned_ids: &[usize],
) -> Vec<usize> {
    let mut storage_ids = Vec::with_capacity(assigned_ids.len());
    for (assigned_id, (ext_id, result)) in assigned_ids.iter().zip(rollback_info) {
        if *assigned_id == result.idx {
            storage_ids.push(result.idx);
        } else {
            // Graph assigned a different node ID than upsert_mapping expected.
            // Remove the stale reverse mapping (result.idx -> ext_id) and
            // establish the correct mapping (ext_id <-> assigned_id).
            mappings.remove_reverse(result.idx);
            mappings.restore(*ext_id, *assigned_id);
            storage_ids.push(*assigned_id);
        }
    }
    storage_ids
}

/// Rolls back every upsert in `rollback_info`, in reverse order, after a
/// batched graph insertion fails.
///
/// Reverse order is mandatory for duplicate-ID chains (a later upsert inside
/// the same batch overwrites an earlier one; restoring forward would undo the
/// later state before the earlier rollback depends on it).
///
/// Both `HnswIndex::insert_batch_parallel` and `NativeHnswIndex::insert_batch`
/// used to duplicate this loop — consolidated here for #448 Group D.
pub(crate) fn rollback_batch(mappings: &ShardedMappings, rollback_info: &[(u64, UpsertResult)]) {
    for (id, result) in rollback_info.iter().rev() {
        rollback_upsert(mappings, *id, result);
    }
}

/// Rolls back mapping state after a failed graph insertion.
///
/// Removes the newly-allocated mapping and, if this was an update,
/// restores the previous mapping so the point remains searchable
/// through its old graph node.
///
/// **Transient gap**: Between `remove` and `restore`, the ID has no
/// mapping for a brief window (nanoseconds). A concurrent search during
/// this window will not find the point. This only occurs on graph-insert
/// failure, which is rare (allocation error).
///
/// **Sidecar loss**: The old sidecar vector (in `ShardedVectors`) was
/// already removed by [`upsert_mapping`] and cannot be cheaply restored.
/// The HNSW graph still holds the vector data in `ContiguousVectors` for
/// traversal, so the point remains searchable -- only sidecar reranking
/// precision is lost for the affected point until the next successful
/// upsert.
pub(crate) fn rollback_upsert(mappings: &ShardedMappings, id: u64, result: &UpsertResult) {
    // Only remove if the current mapping still points to our index.
    // A within-batch duplicate may have already overwritten the mapping
    // with a newer index — removing it would corrupt that later entry.
    let current_idx = mappings.get_idx(id);
    if current_idx == Some(result.idx) {
        mappings.remove(id);
        if let Some(old_idx) = result.old_idx {
            mappings.restore(id, old_idx);
        }
    }
}
