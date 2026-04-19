//! Raw bulk CRUD operations for Collection (`upsert_bulk_from_raw`).
//!
//! Extracted from `crud_bulk.rs` to keep each file under 500 NLOC.
//! These methods accept flat contiguous slices (zero-copy from numpy / FFI)
//! instead of `Point` structs, avoiding per-row `Vec<f32>` allocation.

use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::storage::{PayloadStorage, VectorStorage};
use crate::validation::validate_dimension_match;

use std::collections::{HashMap, HashSet};

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
        // LOCK ORDER: config(1, read) → payload_storage(3, read) →
        //   store_vectors_and_payload_entries (vector_storage(2) ‖ payload_storage(3)) →
        //   secondary_indexes(6, read) → label_index(7, write) → HNSW index (internal) →
        //   config(1, write).
        let n = ids.len();
        if n == 0 {
            return Ok(0);
        }

        // Validate inputs BEFORE any state mutation.
        self.validate_raw_inputs(vectors, ids, dimension, payloads)?;

        // Build (id, &[f32]) pairs by slicing the flat buffer -- zero copy.
        let vector_refs: Vec<(u64, &[f32])> = ids
            .iter()
            .enumerate()
            .map(|(i, &id)| (id, &vectors[i * dimension..(i + 1) * dimension]))
            .collect();

        // Collect pre-batch payloads BEFORE overwriting -- for histogram decrements.
        // Bug #46: deduplicate by ID -- only the first occurrence retrieves the
        // pre-batch value; duplicates get None so the old value is decremented
        // exactly once.
        let old_payloads: Vec<Option<serde_json::Value>> = if payloads.is_some() {
            let storage = self.payload_storage.read();
            let mut seen = HashSet::new();
            ids.iter()
                .map(|&id| {
                    if seen.insert(id) {
                        storage.retrieve(id).ok().flatten()
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            Vec::new()
        };

        // Payload entries for batch WAL write (only ids that have payloads).
        let payload_entries: Vec<(u64, &serde_json::Value)> = payloads
            .into_iter()
            .flat_map(|ps| {
                ps.iter()
                    .enumerate()
                    .filter_map(|(i, opt)| opt.as_ref().map(|val| (ids[i], val)))
            })
            .collect();

        self.store_vectors_and_payload_entries(&vector_refs, &payload_entries)?;

        self.update_text_index_from_raw(ids, payloads);
        self.update_label_index_from_raw(ids, payloads);
        self.update_secondary_indexes_from_raw(ids, payloads);

        let inserted = self.bulk_index_or_defer(vector_refs);
        self.config.write().point_count = self.vector_storage.read().len();

        // Incremental histogram maintenance: decrement old values, increment new.
        // Bug #47: only the last occurrence per ID is counted for new payloads
        // to match last-writer-wins storage semantics.
        if let Some(ps) = payloads {
            let mut dedup_map: HashMap<u64, usize> = HashMap::with_capacity(ids.len());
            for (i, &id) in ids.iter().enumerate() {
                dedup_map.insert(id, i);
            }
            let owned: Vec<Option<serde_json::Value>> = ps
                .iter()
                .enumerate()
                .map(|(i, opt)| {
                    if dedup_map.get(&ids[i]) == Some(&i) {
                        opt.clone()
                    } else {
                        None
                    }
                })
                .collect();
            self.update_histograms_replace(&old_payloads, &owned);
        }

        self.invalidate_caches_and_bump_generation();

        Ok(inserted)
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
        let collection_dim = self.config.read().dimension;
        validate_dimension_match(collection_dim, dimension)?;
        Ok(())
    }

    /// Stores pre-built payload entries via batch WAL write + flush.
    ///
    /// Extracted from `bulk_store_payloads` to accept `(u64, &Value)` pairs
    /// directly, avoiding the need to reconstruct `Point` structs.
    fn bulk_store_payload_entries(&self, entries: &[(u64, &serde_json::Value)]) -> Result<()> {
        self.bulk_store_payload_entries_inner(entries, true)
    }

    /// Stores payload entries with configurable fsync behavior.
    fn bulk_store_payload_entries_inner(
        &self,
        entries: &[(u64, &serde_json::Value)],
        fsync: bool,
    ) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        if fsync {
            self.payload_storage.write().store_batch(entries)?;
        } else {
            self.payload_storage.write().store_batch_deferred(entries)?;
        }
        Ok(())
    }

    /// Writes vectors and raw payload entries to storage (parallel when available).
    fn store_vectors_and_payload_entries(
        &self,
        vector_refs: &[(u64, &[f32])],
        payload_entries: &[(u64, &serde_json::Value)],
    ) -> Result<()> {
        // LOCK ORDER: vector_storage(2, write, parallel) ‖ payload_storage(3, write, parallel).
        //   Each rayon closure acquires only one lock — no ordering dependency between them.
        #[cfg(feature = "persistence")]
        {
            let (vec_result, pay_result) = rayon::join(
                || self.bulk_store_vectors(vector_refs),
                || self.bulk_store_payload_entries(payload_entries),
            );
            vec_result?;
            pay_result?;
        }

        #[cfg(not(feature = "persistence"))]
        {
            self.bulk_store_vectors(vector_refs)?;
            self.bulk_store_payload_entries(payload_entries)?;
        }

        Ok(())
    }

    /// Batch-updates secondary indexes from raw payload slices.
    ///
    /// For each point with a payload, updates all secondary indexes that
    /// have a matching field. Skips the update when no secondary indexes
    /// exist (fast path for bulk loading before `create_index`).
    fn update_secondary_indexes_from_raw(
        &self,
        ids: &[u64],
        payloads: Option<&[Option<serde_json::Value>]>,
    ) {
        let Some(ps) = payloads else { return };
        let indexes = self.secondary_indexes.read();
        if indexes.is_empty() {
            return;
        }
        for (i, opt) in ps.iter().enumerate() {
            let Some(payload) = opt else { continue };
            self.index_single_payload(&indexes, payload, ids[i]);
        }
    }

    /// Indexes a single payload against all secondary indexes.
    fn index_single_payload(
        &self,
        indexes: &std::collections::HashMap<String, crate::index::SecondaryIndex>,
        payload: &serde_json::Value,
        point_id: u64,
    ) {
        for (field, index) in indexes {
            if let Some(val) = payload.get(field) {
                if let Some(key) = crate::index::JsonValue::from_json(val) {
                    self.insert_into_secondary_index(index, key, point_id);
                }
            }
        }
    }

    /// Updates BM25 text index from raw payload slices.
    ///
    /// Points with `Some(payload)` get their text indexed.
    /// Points with `None` payload get their stale BM25 entry removed
    /// (consistent with `update_text_index` in `crud.rs`).
    fn update_text_index_from_raw(
        &self,
        ids: &[u64],
        payloads: Option<&[Option<serde_json::Value>]>,
    ) {
        let Some(ps) = payloads else { return };
        for (i, opt) in ps.iter().enumerate() {
            if let Some(payload) = opt {
                let text = Self::extract_text_from_payload(payload);
                if !text.is_empty() {
                    self.text_index.add_document(ids[i], &text);
                }
            } else {
                self.text_index.remove_document(ids[i]);
            }
        }
    }

    /// Batch-updates the label index from raw payload slices.
    ///
    /// Mirrors `update_text_index_from_raw` but for the label index.
    /// Only indexes payloads that contain `_labels` arrays.
    ///
    /// LOCK ORDER: label_index(7) -- after payload_storage(3).
    fn update_label_index_from_raw(
        &self,
        ids: &[u64],
        payloads: Option<&[Option<serde_json::Value>]>,
    ) {
        let Some(ps) = payloads else { return };
        let has_labels = ps
            .iter()
            .any(|opt| opt.as_ref().is_some_and(|v| v.get("_labels").is_some()));
        if !has_labels {
            return;
        }
        let mut label_idx = self.label_index.write();
        for (i, opt) in ps.iter().enumerate() {
            if let Some(payload) = opt {
                label_idx.index_from_payload(ids[i], payload);
            }
        }
    }
}
