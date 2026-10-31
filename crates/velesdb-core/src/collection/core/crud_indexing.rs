//! Label, sparse-vector, and deferred-indexing helpers for CRUD operations.
//!
//! Extracted from `crud.rs` to reduce NLOC.

use super::graph_property_index_wiring::NodePayloadChange;
use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::Point;
use crate::storage::{LogPayloadStorage, PayloadStorage};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

impl Collection {
    /// Resolves the effective "old payload" for a point, accounting for
    /// within-batch duplicate IDs.
    pub(super) fn resolve_effective_old<'a>(
        seen: &HashMap<u64, Option<&'a serde_json::Value>>,
        id: u64,
        pre_batch_old: Option<&'a serde_json::Value>,
    ) -> Option<&'a serde_json::Value> {
        if let Some(&inner) = seen.get(&id) {
            inner
        } else {
            pre_batch_old
        }
    }

    /// Caches the PQ code for a single point when the mode carries a guard.
    pub(super) fn maybe_quantize(
        collection: &Collection,
        point: &Point,
        pq_guard: &mut Option<super::crud_helpers::PqCacheGuard<'_>>,
    ) {
        if pq_guard.is_some() {
            collection.cache_pq_vector(point, pq_guard.as_deref_mut());
        }
    }

    /// Applies a batch of payload writes to the label and range indexes.
    ///
    /// `writes` is each write's `(id, payload stored)` in batch order, `None`
    /// for a write that deleted the payload; `old_payloads` is positional, the
    /// pre-batch payload of each id's first occurrence and `None` for a
    /// repeat (what `collect_old_payloads` returns). A repeated id resolves to
    /// its previous occurrence, so the batch indexes what storage ends with.
    ///
    /// Callers hold `payload_storage` (3) across the write and this call, so
    /// no other writer can store a newer payload for these ids in between and
    /// leave the indexes describing one storage no longer has.
    pub(super) fn index_payload_writes<'a, I>(&self, writes: I, old_payloads: &'a [Option<Value>])
    where
        I: Iterator<Item = (u64, Option<&'a Value>)> + Clone,
    {
        let labelled =
            |payload: Option<&Value>| payload.is_some_and(|v| v.get("_labels").is_some());
        // An update that drops every label still has entries to remove, so the
        // old payloads decide this too. Unlabelled batches (every plain vector
        // upsert) allocate nothing and take no index lock.
        if !writes.clone().any(|(_, new)| labelled(new))
            && !old_payloads.iter().any(|old| labelled(old.as_ref()))
        {
            return;
        }
        let changes = Self::payload_changes(writes, old_payloads);
        self.apply_payload_index_changes(changes.iter().copied());
    }

    /// Brings the label and range indexes in line with what `storage` holds
    /// for `ids` after a batch write that returned an error.
    ///
    /// A failed batch write is not all-or-nothing: `LogPayloadStorage` makes
    /// each record readable as it writes it, so a failure mid-batch leaves the
    /// records before it readable, and a failed sync or flush leaves all of
    /// them. Each id is read back and indexed as what storage now holds, against
    /// the payload it carried before the batch; an id the write never reached
    /// reads back that same payload and its entries are put back unchanged.
    /// Error paths only: the read-back costs one storage read per id.
    pub(super) fn index_payloads_after_failed_write(
        &self,
        storage: &LogPayloadStorage,
        ids: &[u64],
        old_payloads: &[Option<Value>],
    ) {
        let held = Self::read_back_payloads(storage, ids);
        let writes = ids.iter().copied().zip(held.iter().map(Option::as_ref));
        self.index_payload_writes(writes, old_payloads);
    }

    /// What `storage` holds for each of `ids`, in order: `None` where it holds
    /// no payload or the read fails.
    pub(super) fn read_back_payloads(
        storage: &LogPayloadStorage,
        ids: &[u64],
    ) -> Vec<Option<Value>> {
        ids.iter()
            .map(|&id| storage.retrieve(id).ok().flatten())
            .collect()
    }

    /// Pairs each write of a batch with the payload it replaces: the previous
    /// occurrence of a repeated id within the batch, else its pre-batch
    /// payload. See [`Self::index_payload_writes`] for the two inputs.
    pub(super) fn payload_changes<'a>(
        writes: impl Iterator<Item = (u64, Option<&'a Value>)>,
        old_payloads: &'a [Option<Value>],
    ) -> Vec<NodePayloadChange<'a>> {
        let mut seen: HashMap<u64, Option<&Value>> = HashMap::new();
        writes
            .enumerate()
            .map(|(i, (id, new))| {
                let pre_batch_old = old_payloads.get(i).and_then(Option::as_ref);
                let old = Self::resolve_effective_old(&seen, id, pre_batch_old);
                seen.insert(id, new);
                (id, old, new)
            })
            .collect()
    }

    /// Applies node payload changes to the two indexes derived from node
    /// payloads: the label index, then the graph range indexes, each in a
    /// single write lock scope. Each change removes the old payload's entries
    /// before indexing the new one's.
    ///
    /// LOCK ORDER: label_index(7), released, then graph_range_indexes(7) —
    /// the two share a rank with no order between them, so never both held.
    pub(super) fn apply_payload_index_changes<'a, I>(&self, changes: I)
    where
        I: IntoIterator<Item = NodePayloadChange<'a>>,
        I::IntoIter: Clone,
    {
        let changes = changes.into_iter();
        if changes.clone().next().is_none() {
            return;
        }
        {
            let mut label_idx = self.graph.label_index.write();
            for (id, old, new) in changes.clone() {
                if let Some(old_val) = old {
                    label_idx.remove_from_payload(id, old_val);
                }
                if let Some(new_val) = new {
                    label_idx.index_from_payload(id, new_val);
                }
            }
        }
        self.apply_range_index_changes(changes);
    }

    /// Collects sparse vectors from a point into the batch buffer.
    pub(super) fn collect_sparse_vectors(
        point: &Point,
        sparse_batch: &mut Vec<(u64, BTreeMap<String, crate::index::sparse::SparseVector>)>,
    ) {
        if let Some(sv_map) = &point.sparse_vectors {
            if !sv_map.is_empty() {
                sparse_batch.push((point.id, sv_map.clone()));
            }
        }
    }

    /// Updates the BM25 text index for a single point (WAL-then-apply).
    ///
    /// Applies the BM25 side of a whole batch under ONE durability barrier.
    ///
    /// The single upsert, metadata upsert, and bulk-import paths all route
    /// here: a per-point WAL append cost one `open` and one `fsync` PER
    /// DOCUMENT (#1797); every frame goes into a single `wal_append_batch`
    /// call instead.
    ///
    /// # The three cases a mixed batch contains
    ///
    /// Classified explicitly rather than folded together, because they are not
    /// the same operation and the single-point path distinguishes them:
    ///
    /// * payload carrying text → an `Add` frame;
    /// * payload with NO text  → nothing at all, no WAL entry (empty text is
    ///   not indexed — an empty extraction never was);
    /// * no payload at all     → a `Remove` frame, preserving delete semantics.
    ///
    /// # Ordering
    ///
    /// WAL-before-apply, at BATCH granularity: every frame is written and
    /// fsynced first, and the in-memory index is touched only once that
    /// succeeded. A WAL failure returns `Err` with the index untouched — never
    /// partially updated for a batch that was never acknowledged, which matters
    /// because a lost WAL entry is NOT rebuilt when a BM25 snapshot exists.
    ///
    /// # Errors
    ///
    /// Propagates any WAL open / write / flush / fsync failure.
    pub(super) fn bulk_update_text_index(
        &self,
        points: &[Point],
        old_payloads: &[Option<serde_json::Value>],
    ) -> Result<()> {
        debug_assert_eq!(
            points.len(),
            old_payloads.len(),
            "old_payloads is positional: a short slice would silently drop removals"
        );
        let writes = points.iter().map(|p| (p.id, p.payload.as_ref()));
        self.apply_text_changes(&Self::payload_changes(writes, old_payloads))
    }

    /// Applies a batch's payload changes to BM25 under one WAL barrier; see
    /// [`Self::bulk_update_text_index`] and [`Self::classify_text_mutations`].
    ///
    /// # Errors
    ///
    /// Propagates any WAL open / write / flush / fsync failure.
    pub(super) fn apply_text_changes(&self, changes: &[NodePayloadChange<'_>]) -> Result<()> {
        let (adds, removes) = Self::classify_text_mutations(changes);
        if adds.is_empty() && removes.is_empty() {
            return Ok(());
        }
        self.append_text_batch_to_wal(&adds, &removes)?;

        // Only now is the batch durable, so only now does memory change.
        for (id, text) in &adds {
            self.storage.text_index.add_document(*id, text);
        }
        for id in &removes {
            self.storage.text_index.remove_document(*id);
        }
        Ok(())
    }

    /// Splits a batch into the BM25 mutations it implies.
    ///
    /// The rule is that the index mirrors the payload: a point is searchable
    /// exactly when its current payload yields indexable text. Everything
    /// follows from that one predicate, applied to the old payload and the
    /// new one.
    ///
    /// | previously indexable | now indexable | mutation |
    /// |---|---|---|
    /// | no  | yes | add |
    /// | yes | yes | add (`add_document` replaces in place) |
    /// | yes | no  | **remove** |
    /// | no  | no  | none — no WAL record, no index lock |
    ///
    /// The third row is the one this used to get wrong. A comment here
    /// recorded "a payload with no indexable string writes nothing" as a
    /// deliberate decision, and it was — but only the *insert* case was in
    /// view. On an upsert it meant a point that had its text removed kept
    /// matching a term its payload no longer contains, with nothing to
    /// signal it. Changing the text was always fine, because `add_document`
    /// removes the previous version first; only clearing it went stale.
    ///
    /// The fourth row is why this needs the old payload rather than simply
    /// removing whenever there is no text: a bulk insert of points that never
    /// carry text would otherwise write one tombstone per point for documents
    /// that were never indexed. Deciding on the old payload keeps that path
    /// writing nothing, which is also what it did before.
    ///
    /// The changes come from [`Self::payload_changes`], which resolves a
    /// repeated id inside one batch to the batch's own earlier occurrence:
    /// `old_payloads` reports `None` for a duplicate, so the pre-batch value
    /// alone would miss that the batch had already indexed it.
    fn classify_text_mutations(
        changes: &[NodePayloadChange<'_>],
    ) -> (Vec<(u64, String)>, Vec<u64>) {
        let mut adds: Vec<(u64, String)> = Vec::new();
        let mut removes: Vec<u64> = Vec::new();
        for &(id, old, new) in changes {
            match Self::indexable_text(new) {
                Some(text) => adds.push((id, text)),
                None if Self::indexable_text(old).is_some() => removes.push(id),
                None => {}
            }
        }
        (adds, removes)
    }

    /// The text a payload contributes to BM25, or `None` when it contributes
    /// nothing — an absent payload and one with no indexable string are the
    /// same thing to the index.
    pub(super) fn indexable_text(payload: Option<&serde_json::Value>) -> Option<String> {
        let text = Self::extract_text_from_payload(payload?);
        (!text.is_empty()).then_some(text)
    }

    /// Writes the whole batch to the BM25 WAL under one durability barrier.
    ///
    /// Non-persistence builds have no on-disk WAL, so this is a no-op there and
    /// the caller proceeds straight to the in-memory update.
    #[allow(clippy::unused_self)] // Reason: needs `self.storage.path` under `persistence`.
    pub(super) fn append_text_batch_to_wal(
        &self,
        adds: &[(u64, String)],
        removes: &[u64],
    ) -> Result<()> {
        #[cfg(feature = "persistence")]
        {
            use crate::index::bm25_persistence_wal::{wal_append_batch, wal_path_for_bm25, WalOp};
            let ops: Vec<WalOp<'_>> = adds
                .iter()
                .map(|(id, text)| WalOp::Add {
                    id: *id,
                    text: text.as_str(),
                })
                .chain(removes.iter().map(|id| WalOp::Remove { id: *id }))
                .collect();
            wal_append_batch(&wal_path_for_bm25(&self.storage.path), &ops)?;
        }
        #[cfg(not(feature = "persistence"))]
        {
            let _ = (adds, removes);
        }
        Ok(())
    }

    /// Appends `remove_document` mutations for a whole batch to the BM25 WAL
    /// under ONE durability barrier.
    ///
    /// Calling a single-frame append in a loop pays one `open` + one fsync
    /// PER ID (the #1797 failure mode, resurfacing on the delete path —
    /// finding C3). The frames are identical
    /// to N sequential appends; only the syscall count differs. Callers keep
    /// WAL-before-apply at batch granularity: let this return `Ok` first,
    /// then remove the documents from the in-memory index.
    ///
    /// # Errors
    ///
    /// Propagates any WAL open / write / flush / fsync failure; nothing was
    /// acknowledged in that case and the in-memory index must not be touched.
    #[cfg(feature = "persistence")]
    pub(super) fn append_bm25_wal_remove_batch(&self, ids: &[u64]) -> Result<()> {
        use crate::index::bm25_persistence_wal::{wal_append_batch, wal_path_for_bm25, WalOp};
        let ops: Vec<WalOp<'_>> = ids.iter().map(|&id| WalOp::Remove { id }).collect();
        wal_append_batch(&wal_path_for_bm25(&self.storage.path), &ops)
    }

    /// Appends `(name, point_id, sparse_vector)` triples to the per-index
    /// sparse WAL under WAL-before-apply semantics.
    ///
    /// Centralises the `wal_path_for_name` + `wal_append_upsert` loop that was
    /// duplicated between `apply_sparse_batch_upsert` (single-point path) and
    /// `apply_sparse_batch_bulk` (bulk path). Callers keep ownership of their
    /// input shape (`Vec<(u64, BTreeMap)>` vs `BTreeMap<String, Vec<(u64,
    /// SparseVector)>>`) and build the iterator of triples themselves, so this
    /// helper never copies a caller's collection.
    ///
    /// It is not allocation-free, and this doc said it was until the run
    /// batching below arrived: `run` grows to the longest same-name run, and
    /// holds borrows rather than clones. The claim was true when the helper
    /// only forwarded one entry at a time; nothing re-checked it afterwards.
    ///
    /// Feature-gated on `persistence` — on targets without persistence the
    /// sparse WAL does not exist and the caller short-circuits.
    ///
    /// Issue #450 Phase 3.1.
    #[cfg(feature = "persistence")]
    pub(super) fn append_sparse_wal_entries<'a, I>(&self, entries: I) -> Result<()>
    where
        I: IntoIterator<Item = (&'a str, u64, &'a crate::index::sparse::SparseVector)>,
    {
        // Group consecutive entries sharing the same index name into one
        // `wal_append_upsert_batch` call: one `open` and one fsync per run
        // instead of per entry (#1797 shape). Callers yield entries grouped
        // by name (e.g. `apply_sparse_batch_bulk`), so a batch touching K
        // index names pays exactly K barriers. Mixed-name callers degrade
        // gracefully to one barrier per run boundary.
        let mut run_name: Option<&'a str> = None;
        let mut run: Vec<(u64, &'a crate::index::sparse::SparseVector)> = Vec::new();
        for (name, point_id, sv) in entries {
            if run_name != Some(name) {
                if let Some(prev) = run_name.take() {
                    self.flush_sparse_wal_run(prev, &run)?;
                    run.clear();
                }
                run_name = Some(name);
            }
            run.push((point_id, sv));
        }
        if let Some(prev) = run_name {
            self.flush_sparse_wal_run(prev, &run)?;
        }
        Ok(())
    }

    /// Writes one same-name run of sparse upserts under a single WAL barrier.
    #[cfg(feature = "persistence")]
    fn flush_sparse_wal_run(
        &self,
        name: &str,
        run: &[(u64, &crate::index::sparse::SparseVector)],
    ) -> Result<()> {
        let wal_path =
            crate::index::sparse::persistence::wal_path_for_name(&self.storage.path, name);
        crate::index::sparse::persistence::wal_append_upsert_batch(&wal_path, run)?;
        Ok(())
    }

    /// Applies buffered sparse vector upserts with WAL-before-apply semantics.
    // The comment below is the contract: WAL append and in-memory apply stay
    // indivisible with respect to compaction, which takes this lock through
    // its reset.
    #[expect(clippy::significant_drop_tightening)]
    pub(super) fn apply_sparse_batch_upsert(
        &self,
        sparse_batch: &[(u64, BTreeMap<String, crate::index::sparse::SparseVector>)],
    ) -> Result<()> {
        if sparse_batch.is_empty() {
            return Ok(());
        }
        // Keep WAL append and in-memory apply indivisible with respect to
        // compaction, which takes this lock exclusively through the reset.
        let mut indexes = self.query.sparse_indexes.write();
        #[cfg(feature = "persistence")]
        {
            self.append_sparse_wal_entries(sparse_batch.iter().flat_map(|(point_id, sv_map)| {
                sv_map
                    .iter()
                    .map(move |(name, sv)| (name.as_str(), *point_id, sv))
            }))?;
        }
        for (point_id, sv_map) in sparse_batch {
            for (name, sv) in sv_map {
                let idx = indexes.entry(name.clone()).or_default();
                idx.insert(*point_id, sv);
            }
        }
        Ok(())
    }

    /// Invalidates stats cache and bumps write generation.
    ///
    /// Also drops the payload mirror: any mutation path that does not
    /// explicitly maintain the mirror must invalidate it so stale columnar
    /// data can never serve queries (it is rebuilt lazily on demand).
    pub(super) fn invalidate_caches_and_bump_generation(&self) {
        *self.query.cached_stats.lock() = None;
        self.storage.payload_mirror.invalidate();
        self.generations
            .write_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Like [`Self::invalidate_caches_and_bump_generation`], but keeps the
    /// payload mirror warm by applying the upserted points incrementally.
    pub(super) fn bump_generation_with_mirror_upserts(&self, points: &[crate::point::Point]) {
        *self.query.cached_stats.lock() = None;
        self.storage.payload_mirror.apply_upserts(points);
        self.generations
            .write_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Like [`Self::invalidate_caches_and_bump_generation`], but keeps the
    /// payload mirror warm by tombstoning the deleted ids incrementally.
    pub(super) fn bump_generation_with_mirror_deletes(&self, ids: &[u64]) {
        *self.query.cached_stats.lock() = None;
        self.storage.payload_mirror.apply_deletes(ids);
        self.generations
            .write_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Indexes what storage holds for the ids the deferred indexer buffers,
    /// then retires the entries it copied: a point leaves the buffer only
    /// once indexed, as storage holds it (a deleted one is left out), and an
    /// entry written again meanwhile stays for the next merge. A no-op while
    /// another merge runs.
    #[cfg(feature = "persistence")]
    pub(super) fn merge_deferred_batch(&self, di: &crate::collection::streaming::DeferredIndexer) {
        let Some(entries) = di.begin_merge() else {
            return;
        };
        let ids: Vec<u64> = entries.iter().map(|&(id, _)| id).collect();
        self.index_current(&ids);
        let copied: Vec<(u64, &[f32])> =
            entries.iter().map(|(id, v)| (*id, v.as_slice())).collect();
        di.end_merge(&copied);
    }

    /// Batch-inserts into HNSW or defers into the deferred indexer.
    pub(super) fn bulk_index_or_defer(&self, vector_refs: &[(u64, &[f32])]) -> usize {
        let count = vector_refs.len();
        #[cfg(feature = "persistence")]
        if let Some(di) = self.streaming.deferred_indexer.load().as_ref() {
            // PERF3: the per-vector `to_vec()` copy is intrinsic to the
            // deferred contract — `vector_refs` borrows the caller's data and
            // does not outlive this call, while the deferred buffer must own
            // the vectors until the next merge. `DeltaBuffer` stores one
            // exact-sized `Vec<f32>` per entry (its per-entry ownership is
            // what makes upsert-replace/remove O(1) data-move); the copies
            // run OUTSIDE the buffer's write lock (see `DeltaBuffer::extend`)
            // and the buffer is bounded by `merge_threshold` entries.
            di.extend(vector_refs.iter().map(|(id, v)| (*id, v.to_vec())));
            if di.should_merge() {
                self.merge_deferred_batch(di);
            }
            #[allow(clippy::cast_possible_truncation)]
            self.generations
                .inserts_since_last_hnsw_save
                .fetch_add(count as u64, std::sync::atomic::Ordering::Relaxed);
            return count;
        }
        let inserted = self
            .storage
            .index
            .insert_batch_parallel(vector_refs.iter().copied());
        #[allow(clippy::cast_possible_truncation)]
        self.generations
            .inserts_since_last_hnsw_save
            .fetch_add(count as u64, std::sync::atomic::Ordering::Relaxed);
        inserted
    }
}
