//! Crash recovery: 3-pass reconciliation between vector storage and HNSW.
//!
//! On [`Collection::open()`](super::super::Collection::open), the persisted
//! HNSW index may disagree with the WAL-replayed vector storage in three
//! ways, each handled by a dedicated pass in [`run_crash_recovery`]:
//!
//! 1. **Gap**: vectors in storage but not in HNSW (crash between the storage
//!    write and the HNSW batch insert — deferred indexer gap, delta buffer
//!    gap, or normal insert gap).
//! 2. **Orphans**: ids in HNSW but not in storage (delete reached the vector
//!    WAL but not the next index save).
//! 3. **Stale**: WAL-touched ids present on both sides whose indexed vector
//!    no longer matches storage (upsert after the last index save).
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
        tracing::debug!(
            storage_count,
            hnsw_count,
            "gap recovery skipped — counts match, no scan needed"
        );
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
    let config: CollectionConfig =
        serde_json::from_str(&config_data).map_err(|e| Error::Serialization(e.to_string()))?;
    validate_schema_version(&config)?;
    Ok(config)
}

/// Rejects collections written by a newer VelesDB with a higher schema
/// version. Treats `schema_version == 0` as v1 (silent migration of
/// pre-versioned collections).
fn validate_schema_version(config: &CollectionConfig) -> Result<()> {
    use crate::collection::types::CURRENT_SCHEMA_VERSION;

    let version = if config.schema_version == 0 {
        1
    } else {
        config.schema_version
    };
    if version > CURRENT_SCHEMA_VERSION {
        return Err(Error::IncompatibleSchemaVersion {
            found: version,
            supported: CURRENT_SCHEMA_VERSION,
        });
    }
    Ok(())
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

/// Runs the 3-pass crash reconciliation between vector storage and HNSW:
///
/// 1. **Gap** ([`recover_hnsw_gap`]): vectors in storage but not in HNSW
///    (crash during deferred merge, delta drain, or normal insert).
/// 2. **Orphans** ([`remove_orphan_ids`]): ids in HNSW but not in storage
///    (crash after a delete reached the vector WAL but before the next
///    index save).
/// 3. **Stale** ([`reindex_stale_wal_ids`]): WAL-touched ids present on both
///    sides whose indexed vector no longer matches storage (upsert after the
///    last index save).
///
/// Returns `Ok(true)` when any pass mutated the index — the caller must then
/// re-save it, because the vector WAL (the only other witness of the delta)
/// was truncated during replay.
#[cfg(feature = "persistence")]
pub(super) fn run_crash_recovery(
    config: &CollectionConfig,
    vector_storage: &Arc<RwLock<MmapStorage>>,
    index: &Arc<HnswIndex>,
    wal_touched_ids: &[u64],
) -> Result<bool> {
    if config.metadata_only || config.dimension == 0 {
        return Ok(false);
    }
    let recovered = recover_hnsw_gap(vector_storage, index, config.dimension)?;
    let orphans = remove_orphan_ids(vector_storage, index);
    let stale = reindex_stale_wal_ids(vector_storage, index, wal_touched_ids, config.dimension)?;
    if recovered + orphans + stale > 0 {
        tracing::info!(
            collection = %config.name,
            recovered,
            orphans,
            stale,
            "Collection index reconciliation completed on open"
        );
    }
    Ok(recovered + orphans + stale > 0)
}

/// No-op stub when persistence is disabled.
#[cfg(not(feature = "persistence"))]
pub(super) fn run_crash_recovery(
    _config: &CollectionConfig,
    _vector_storage: &Arc<RwLock<MmapStorage>>,
    _index: &Arc<HnswIndex>,
    _wal_touched_ids: &[u64],
) -> Result<bool> {
    Ok(false)
}

/// Pass 2: removes ids present in the HNSW mappings but absent from storage.
///
/// Such orphans arise when a crash persists a delete to the vector WAL but
/// not to the next HNSW save: on reopen the WAL replay removes the id from
/// storage while the loaded index still maps it. Without this pass the
/// tombstone would resurface in search results.
#[cfg(feature = "persistence")]
fn remove_orphan_ids(vector_storage: &Arc<RwLock<MmapStorage>>, index: &Arc<HnswIndex>) -> usize {
    if index.is_empty() {
        return 0;
    }
    let storage_ids: std::collections::HashSet<u64> =
        vector_storage.read().ids().into_iter().collect();
    // Collect before removing: mutating the sharded mappings while iterating
    // them would deadlock on the shard lock.
    let orphan_ids: Vec<u64> = index
        .mappings
        .iter()
        .map(|(id, _)| id)
        .filter(|id| !storage_ids.contains(id))
        .collect();
    for &id in &orphan_ids {
        index.remove(id);
    }
    if !orphan_ids.is_empty() {
        tracing::warn!(
            orphans = orphan_ids.len(),
            "Crash recovery: removed HNSW ids absent from vector storage"
        );
    }
    orphan_ids.len()
}

/// Pass 3: re-upserts WAL-touched ids whose indexed vector is stale.
///
/// The WAL replay applied these writes to storage and then truncated the
/// WAL, so storage is the source of truth. For every touched id present in
/// both storage and the index, the indexed sidecar vector is compared to
/// the storage bytes; on mismatch the storage value is re-upserted into the
/// index (tombstoning the stale graph node).
///
/// The caller guarantees the index has sidecar vector storage when any
/// touched id overlaps its mappings (see `rebuild_if_unverifiable`); an id
/// whose sidecar vector is missing anyway is treated as stale.
#[cfg(feature = "persistence")]
fn reindex_stale_wal_ids(
    vector_storage: &Arc<RwLock<MmapStorage>>,
    index: &Arc<HnswIndex>,
    wal_touched_ids: &[u64],
    dimension: usize,
) -> Result<usize> {
    let storage = vector_storage.read();
    let mut stale: Vec<(u64, Vec<f32>)> = Vec::new();
    for &id in wal_touched_ids {
        let Some(idx) = index.mappings.get_idx(id) else {
            continue; // Not indexed (deleted id — already handled by pass 2).
        };
        match storage.retrieve(id) {
            Ok(Some(v)) if v.len() == dimension => {
                let matches = index
                    .vectors
                    .with_vector(idx, |indexed| indexed == v.as_slice())
                    .unwrap_or(false);
                if !matches {
                    stale.push((id, v));
                }
            }
            Ok(_) => {} // Absent or corrupt-dimension: pass 1/2 territory.
            Err(e) => {
                return Err(Error::Storage(format!(
                    "failed to retrieve WAL-touched vector {id}: {e}"
                )))
            }
        }
    }
    drop(storage);

    let reindexed = reindex_vectors(index, &stale);
    if reindexed > 0 {
        tracing::warn!(
            reindexed,
            "Crash recovery: re-upserted stale WAL-touched vectors into HNSW"
        );
    }
    Ok(reindexed)
}
