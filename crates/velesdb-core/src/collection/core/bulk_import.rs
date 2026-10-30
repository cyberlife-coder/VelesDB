//! Raw bulk CRUD operations for Collection (`upsert_bulk_from_raw`).
//!
//! Extracted from `crud_bulk.rs` to keep each file under 500 NLOC.
//! These methods accept flat contiguous slices (zero-copy from numpy / FFI)
//! instead of `Point` structs, avoiding per-row `Vec<f32>` allocation.

use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::storage::VectorStorage;
use crate::validation::validate_dimension_match;

use std::collections::HashMap;

impl Collection {
    /// Bulk insert from contiguous flat slices (zero-copy from numpy / FFI).
    ///
    /// Accepts a flat `f32` slice of shape `(n, dimension)` in row-major order
    /// plus a matching `u64` ID slice of length `n`. This avoids per-row
    /// `Vec<f32>` allocation that `upsert_bulk` requires through `Point`.
    ///
    /// # Performance
    ///
    /// Eliminates `n * dimension * 4` bytes of intermediate copies compared
    /// to the `Point`-based `upsert_bulk` path. For 100K vectors at 768D
    /// this saves ~293 MB of heap allocations.
    ///
    /// # Errors
    ///
    /// - Returns [`crate::error::Error::InvalidVector`] if `vectors.len() != ids.len() * dimension`.
    /// - Returns [`crate::error::Error::DimensionMismatch`] if `dimension` does not match the collection.
    pub fn upsert_bulk_from_raw(
        &self,
        vectors: &[f32],
        ids: &[u64],
        dimension: usize,
        payloads: Option<&[Option<serde_json::Value>]>,
    ) -> Result<usize> {
        // LOCK ORDER: config(1, read) → store_vectors_and_payloads (see its
        //   doc: the storage locks, then the indexes derived from payloads) →
        //   HNSW index (internal) → vector_storage(2, read) → config(1, write).
        let n = ids.len();
        if n == 0 {
            return Ok(0);
        }

        // Validate inputs and enforce the runtime ingest limits (parity item E)
        // BEFORE any state mutation, so the caps are not bypassable on this
        // zero-copy path (the dominant Python/REST bulk upsert surface).
        self.validate_raw_inputs(vectors, ids, dimension, payloads)?;

        // Build (id, &[f32]) pairs by slicing the flat buffer -- zero copy.
        let vector_refs: Vec<(u64, &[f32])> = ids
            .iter()
            .enumerate()
            .map(|(i, &id)| (id, &vectors[i * dimension..(i + 1) * dimension]))
            .collect();

        // Payload entries for batch WAL write (only ids that have payloads). A
        // `None` entry stores nothing, so the id keeps its payload and every
        // index derived from it: the payload indexes below see only `entries`.
        let payload_entries: Vec<(u64, &serde_json::Value)> = payloads
            .into_iter()
            .flat_map(|ps| {
                ps.iter()
                    .enumerate()
                    .filter_map(|(i, opt)| opt.as_ref().map(|val| (ids[i], val)))
            })
            .collect();

        self.store_vectors_and_payloads(&vector_refs, &payload_entries)?;

        let inserted = self.bulk_index_or_defer(&vector_refs);
        // Two statements so the guards never overlap: the one-liner form
        // evaluates the RHS first, holding vector_storage (rank 2) while
        // acquiring config (rank 1) — a decreed-order inversion.
        let point_count = self.storage.vector_storage.read().len();
        self.storage.config.write().point_count = point_count;

        self.invalidate_caches_and_bump_generation();

        Ok(inserted)
    }

    /// Incremental histogram maintenance for a bulk upsert's payload entries.
    ///
    /// Decrements old values and increments new values in a single atomic
    /// read/modify/write cycle (Bug #49). Bug #47: only the last occurrence
    /// per ID is counted for new payloads to match last-writer-wins storage
    /// semantics. Shared by `upsert_bulk` and `upsert_bulk_from_raw`, whose
    /// entries leave out the points without a payload: those keep theirs.
    /// Bug #46: `old_payloads` holds the pre-batch payload of each id's first
    /// entry only, so an old value is decremented exactly once.
    ///
    /// A batch without payloads changes no histogram, and returns before
    /// reading the statistics file.
    pub(super) fn apply_histogram_replace_for_entries(
        &self,
        entries: &[(u64, &serde_json::Value)],
        old_payloads: &[Option<serde_json::Value>],
    ) {
        if entries.is_empty() {
            return;
        }
        let mut last: HashMap<u64, usize> = HashMap::with_capacity(entries.len());
        for (i, &(id, _)) in entries.iter().enumerate() {
            last.insert(id, i);
        }
        let new_payloads: Vec<Option<serde_json::Value>> = entries
            .iter()
            .enumerate()
            .map(|(i, &(id, payload))| (last.get(&id) == Some(&i)).then(|| payload.clone()))
            .collect();
        self.update_histograms_replace(old_payloads, &new_payloads);
    }

    /// Enforces the runtime ingest limits (parity item E) for the raw bulk
    /// path, reusing the shared `Collection` gates so the slice-based and
    /// `Point`-based ingest paths apply identical caps.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::Error::GuardRail`] when the batch would push the
    /// collection past `max_vectors_per_collection`, or when any payload
    /// exceeds `max_payload_size`.
    fn enforce_raw_upsert_limits(
        &self,
        ids: &[u64],
        payloads: Option<&[Option<serde_json::Value>]>,
    ) -> Result<()> {
        let limits = self.runtime_limits();
        self.enforce_vector_count(ids.len(), limits.max_vectors_per_collection)?;
        if let Some(ps) = payloads {
            for (i, opt) in ps.iter().enumerate() {
                if let Some(payload) = opt {
                    Self::enforce_payload_limits(ids[i], payload, limits.max_payload_size)?;
                }
            }
        }
        Ok(())
    }

    /// Validates raw bulk-insert inputs before any state mutation.
    fn validate_raw_inputs(
        &self,
        vectors: &[f32],
        ids: &[u64],
        dimension: usize,
        payloads: Option<&[Option<serde_json::Value>]>,
    ) -> Result<()> {
        let n = ids.len();
        let expected_len = n.checked_mul(dimension).ok_or_else(|| {
            Error::InvalidVector(format!(
                "overflow computing {n} * {dimension} for flat vector length"
            ))
        })?;
        if vectors.len() != expected_len {
            return Err(Error::InvalidVector(format!(
                "flat vectors length {} != ids.len() ({n}) * dimension ({dimension}) = {expected_len}",
                vectors.len()
            )));
        }
        if let Some(ps) = payloads {
            if ps.len() != n {
                return Err(Error::InvalidVector(format!(
                    "payloads length ({}) must match ids length ({n})",
                    ps.len()
                )));
            }
        }
        let collection_dim = self.storage.config.read().dimension;
        validate_dimension_match(collection_dim, dimension)?;
        self.enforce_raw_upsert_limits(ids, payloads)?;
        Ok(())
    }

    /// Batch-updates secondary indexes from bulk payload entries.
    ///
    /// For each entry, **removes the prior indexed value and inserts the new
    /// one** (via [`update_secondary_indexes_on_upsert`]) so an overwrite of an
    /// already-indexed point does not leave a stale entry in the old key's
    /// bucket. Without the removal the same id would sit in both its old and
    /// new key buckets, inflating `Σ buckets` above `point_count` so the
    /// ordered-index coverage check declines the fast path — and a restart that
    /// rebuilds from the final stored payload (backfill) would then diverge from
    /// the live tree (EPIC-081 phase 3d restore equivalence). A repeated id
    /// replaces its previous entry's value, not the pre-batch one, through
    /// [`Self::payload_changes`].
    ///
    /// Skips entirely when no secondary indexes exist (fast path for bulk
    /// loading before `create_index`).
    ///
    /// [`update_secondary_indexes_on_upsert`]: Self::update_secondary_indexes_on_upsert
    pub(super) fn update_secondary_indexes_for_entries(
        &self,
        entries: &[(u64, &serde_json::Value)],
        old_payloads: &[Option<serde_json::Value>],
    ) {
        if self.query.secondary_indexes.read().is_empty() {
            return;
        }
        let writes = entries.iter().map(|&(id, payload)| (id, Some(payload)));
        for (id, old, new) in Self::payload_changes(writes, old_payloads) {
            self.update_secondary_indexes_on_upsert(id, old, new);
        }
    }

    /// Updates the BM25 text index from bulk payload entries (WAL-then-apply),
    /// through the same classification as every other write path
    /// (`classify_text_mutations` in `crud_indexing.rs`, whose doc holds the
    /// truth table). A bulk point without a payload is not an entry, keeps its
    /// payload, and keeps its document.
    ///
    /// Issue #389: the mutations are appended to the BM25 WAL before they are
    /// applied in memory, so a crash between the two replays them on the next
    /// open; a WAL error propagates with the in-memory index untouched.
    pub(super) fn update_text_index_for_entries(
        &self,
        entries: &[(u64, &serde_json::Value)],
        old_payloads: &[Option<serde_json::Value>],
    ) -> Result<()> {
        let writes = entries.iter().map(|&(id, payload)| (id, Some(payload)));
        self.apply_text_changes(&Self::payload_changes(writes, old_payloads))
    }
}
