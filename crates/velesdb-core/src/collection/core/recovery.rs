//! Crash recovery: gap detection between vector storage and HNSW index.
//!
//! On [`Collection::open()`](super::super::Collection::open), vectors may
//! exist in storage but not in HNSW if a crash occurred between the storage
//! write and the HNSW batch insert (deferred indexer gap, delta buffer gap,
//! or normal insert gap).
//!
//! This module detects such gaps and re-indexes the missing vectors.
//!
//! ## Known limitation
//!
//! If a crash occurs between the HNSW delete and the storage delete being
//! persisted, a previously deleted vector may appear in storage but not in
//! HNSW — indistinguishable from an insert gap. Recovery will re-index the
//! deleted vector. This is an inherent trade-off without two-phase commit
//! and is acceptable because (a) the window is very small, and (b) a
//! resurrected vector is preferable to a silently lost one.

use crate::index::HnswIndex;
use crate::storage::{MmapStorage, PayloadStorage, VectorStorage};
use parking_lot::RwLock;
use std::sync::Arc;

/// Detects vectors in storage that are missing from the HNSW index and
/// re-indexes them.
///
/// Returns the number of recovered (re-indexed) vectors.
///
/// # Early exit
///
/// Returns `0` immediately if storage is empty or its count matches HNSW.
/// This heuristic may miss gaps in the theoretical case where a gap and an
/// HNSW orphan cancel out (e.g., one inserted + one deleted during the same
/// crash). This scenario requires two complementary failure modes and is
/// extremely unlikely in practice.
///
/// # Errors
///
/// Returns an error if vector retrieval from storage fails.
pub(crate) fn recover_hnsw_gap(
    vector_storage: &Arc<RwLock<MmapStorage>>,
    index: &Arc<HnswIndex>,
    dimension: usize,
) -> crate::error::Result<usize> {
    let storage = vector_storage.read();
    let storage_count = storage.len();
    let hnsw_count = index.len();

    if storage_count == 0 || storage_count == hnsw_count {
        return Ok(0);
    }

    let gap_ids = find_gap_ids(&storage, index);
    if gap_ids.is_empty() {
        return Ok(0);
    }

    let vectors = retrieve_valid_vectors(&storage, &gap_ids, dimension)?;
    let gap_total = gap_ids.len();
    drop(storage);

    let recovered = reindex_vectors(index, &vectors);
    tracing::warn!(
        recovered,
        gap_total,
        "Crash recovery: re-indexed gap vectors into HNSW"
    );
    Ok(recovered)
}

/// Returns storage IDs not present in the HNSW index.
fn find_gap_ids(storage: &MmapStorage, index: &HnswIndex) -> Vec<u64> {
    storage
        .ids()
        .into_iter()
        .filter(|id| !index.mappings.contains(*id))
        .collect()
}

/// Retrieves vectors for gap IDs, propagating IO errors.
///
/// Skips vectors with wrong dimension (corruption) or missing data
/// (concurrent deletion between `ids()` and `retrieve()`).
fn retrieve_valid_vectors(
    storage: &MmapStorage,
    gap_ids: &[u64],
    dimension: usize,
) -> crate::error::Result<Vec<(u64, Vec<f32>)>> {
    let mut vectors = Vec::with_capacity(gap_ids.len());
    for &id in gap_ids {
        match storage.retrieve(id) {
            Ok(Some(v)) if v.len() == dimension => vectors.push((id, v)),
            Ok(Some(v)) => tracing::warn!(
                id,
                expected = dimension,
                actual = v.len(),
                "Skipping gap vector with mismatched dimension"
            ),
            Ok(None) => {} // Deleted between ids() and retrieve()
            Err(e) => {
                return Err(crate::error::Error::Storage(format!(
                    "failed to retrieve gap vector {id}: {e}"
                )))
            }
        }
    }
    Ok(vectors)
}

/// Batch-inserts recovered vectors into the HNSW index.
fn reindex_vectors(index: &HnswIndex, vectors: &[(u64, Vec<f32>)]) -> usize {
    if vectors.is_empty() {
        return 0;
    }
    let refs: Vec<(u64, &[f32])> = vectors.iter().map(|(id, v)| (*id, v.as_slice())).collect();
    index.insert_batch_parallel(refs)
}

// ---------------------------------------------------------------------------
// Lifecycle helpers extracted from lifecycle.rs
// ---------------------------------------------------------------------------

use crate::collection::types::CollectionConfig;
use crate::error::{Error, Result};
use crate::storage::LogPayloadStorage;

/// Reads and deserializes the collection config from disk.
pub(super) fn load_config(path: &std::path::Path) -> Result<CollectionConfig> {
    let config_path = path.join("config.json");
    let config_data = std::fs::read_to_string(&config_path)?;
    serde_json::from_str(&config_data).map_err(|e| Error::Serialization(e.to_string()))
}

/// Reconciles `point_count` from the actual storage (config.json may be
/// stale if the previous process exited without calling `save_config`).
pub(super) fn reconcile_point_count(
    config: &CollectionConfig,
    vector_storage: &Arc<RwLock<MmapStorage>>,
    payload_storage: &Arc<RwLock<LogPayloadStorage>>,
) -> usize {
    if config.metadata_only {
        payload_storage.read().ids().len()
    } else {
        vector_storage.read().len()
    }
}

/// Runs crash recovery: detects vectors in storage but not in HNSW (gap
/// from crash during deferred merge, delta drain, or normal insert).
#[cfg(feature = "persistence")]
pub(super) fn run_crash_recovery(
    config: &CollectionConfig,
    vector_storage: &Arc<RwLock<MmapStorage>>,
    index: &Arc<HnswIndex>,
) -> Result<()> {
    if config.metadata_only || config.dimension == 0 {
        return Ok(());
    }
    let recovered = recover_hnsw_gap(vector_storage, index, config.dimension)?;
    if recovered > 0 {
        tracing::info!(
            collection = %config.name,
            recovered,
            "Collection gap recovery completed on open"
        );
    }
    Ok(())
}

/// No-op stub when persistence is disabled.
#[cfg(not(feature = "persistence"))]
pub(super) fn run_crash_recovery(
    _config: &CollectionConfig,
    _vector_storage: &Arc<RwLock<MmapStorage>>,
    _index: &Arc<HnswIndex>,
) -> Result<()> {
    Ok(())
}
