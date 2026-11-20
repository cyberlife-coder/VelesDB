//! Payload-index, sparse-vector, and deferred-indexing helpers for CRUD operations.
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

    /// Applies a batch of payload writes to the indexes derived from the
    /// payloads: the secondary indexes, then the label and range indexes.
    ///
    /// `writes` is each write's `(id, payload stored)` in batch order, `None`
    /// for a write that deleted the payload; `old_payloads` is positional, the
    /// pre-batch payload of each id's first occurrence and `None` for a
    /// repeat (what `collect_old_payloads` returns). A repeated id resolves to
    /// its previous occurrence, so the batch indexes what storage ends with.
    ///
    /// Callers hold `payload_storage` (3) for writing across the write and
    /// this call, so no other writer can store a newer payload for these ids
    /// in between and leave the indexes describing one storage no longer
    /// has — and a reader that read the payload store's revision under its
    /// read guard finds every write that revision counts in the indexes: what
    /// a filtered ranked walk's prefilter bitmap relies on
    /// (`Collection::ranked_within_prefilter`).
    ///
    /// LOCK ORDER: under payload_storage(3): secondary_indexes(6), released;
    /// label_index(7), released; graph_range_indexes(7).
    pub(super) fn index_payload_writes<'a, I>(&self, writes: I, old_payloads: &'a [Option<Value>])
    where
        I: Iterator<Item = (u64, Option<&'a Value>)> + Clone,
    {
        let labelled =
            |payload: Option<&Value>| payload.is_some_and(|v| v.get("_labels").is_some());
        // An update that drops every label still has entries to remove, so the
        // old payloads decide this too. Unlabelled batches without a secondary
        // index (every plain vector upsert) allocate nothing and take no index
        // lock but the secondary map's read.
        let graph = writes.clone().any(|(_, new)| labelled(new))
            || old_payloads.iter().any(|old| labelled(old.as_ref()));
        let secondary = !self.query.secondary_indexes.read().is_empty();
        if !graph && !secondary {
            return;
        }
        let changes = Self::payload_changes(writes, old_payloads);
        if secondary {
            for &(id, old, new) in &changes {
                self.update_secondary_indexes_on_upsert(id, old, new);
            }
        }
        if graph {
            self.apply_payload_index_changes(changes.iter().copied());
        }
    }

    /// Brings the secondary, label and range indexes in line with what
    /// `storage` holds for `ids` after a batch write that returned an error.
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

    /// The payload-derived state a batch of payload writes changes beyond its
    /// storage guard: the persisted column histograms, then BM25 — the one
    /// routine every write path runs (the vector upsert, the metadata upsert,
    /// both bulk paths and the node-payload write), so a payload written by
    /// any of them is counted and searchable alike. The secondary, label and
    /// range indexes are not: each path maintains them under its payload
    /// write guard ([`Self::index_payload_writes`]).
    ///
    /// `writes` is each write's `(id, payload stored)` in batch order, `None`
    /// for a write that deleted the payload; `old_payloads` is positional, the
    /// pre-batch payload of each id's first occurrence and `None` for a repeat
    /// (what `collect_old_payloads` returns). The caller has stored and synced
    /// the payloads, so the BM25 WAL never leads payload durability.
    ///
    /// The histograms count only each id's last write (Bug #47) in one
    /// decrement-and-increment cycle (Bug #49). BM25 is WAL-before-apply at
    /// batch granularity (#389, #1797): one durability barrier for the
    /// batch, the in-memory index touched only once it succeeded — see
    /// [`Self::classify_text_mutations`] for which writes add, remove or do
    /// nothing. A batch that stores and replaces no payload reads no
    /// statistics file, and one that stores no payload into an empty index
    /// skips BM25 without classifying it.
    ///
    /// # Errors
    ///
    /// Propagates any BM25 WAL open / write / flush / fsync failure; the
    /// histograms, which describe the stored payloads, are updated first.
    pub(super) fn apply_payload_effects(
        &self,
        writes: &[(u64, Option<&Value>)],
        old_payloads: &[Option<Value>],
    ) -> Result<()> {
        self.apply_payload_histograms(writes, old_payloads);
        self.apply_payload_text(writes, old_payloads, |adds, removes| {
            self.change_text_index(adds, removes)
        })
    }

    /// The histogram half of [`Self::apply_payload_effects`]: each id's last
    /// write counted, in one decrement-and-increment cycle; a batch that
    /// stores and replaces no payload reads no statistics file.
    ///
    /// LOCK ORDER: stats_io_mutex(12), with no other collection lock held.
    pub(super) fn apply_payload_histograms(
        &self,
        writes: &[(u64, Option<&Value>)],
        old_payloads: &[Option<Value>],
    ) {
        debug_assert_eq!(
            writes.len(),
            old_payloads.len(),
            "old_payloads is positional: a short slice would silently drop removals"
        );
        let stores_none = writes.iter().all(|(_, new)| new.is_none());
        if !stores_none || old_payloads.iter().any(Option::is_some) {
            self.update_histograms_replace(old_payloads, &last_writes(writes));
        }
    }

    /// The BM25 half of [`Self::apply_payload_effects`]: the batch's adds and
    /// removes ([`Self::classify_text_mutations`]) handed to `change` — the
    /// change a write makes under the guard it holds — unless there are
    /// none; a batch that stores no payload into an empty index skips it
    /// without classifying it.
    ///
    /// # Errors
    ///
    /// What `change` returns.
    pub(super) fn apply_payload_text(
        &self,
        writes: &[(u64, Option<&Value>)],
        old_payloads: &[Option<Value>],
        change: impl FnOnce(&[(u64, String)], &[u64]) -> Result<()>,
    ) -> Result<()> {
        let stores_none = writes.iter().all(|(_, new)| new.is_none());
        if stores_none && self.storage.text_index.is_empty() {
            return Ok(());
        }
        let changes = Self::payload_changes(writes.iter().copied(), old_payloads);
        let (adds, removes) = Self::classify_text_mutations(&changes);
        if adds.is_empty() && removes.is_empty() {
            return Ok(());
        }
        change(&adds, &removes)
    }

    /// Appends `adds` and `removes` to the BM25 WAL under one barrier, then
    /// applies them in memory — WAL-before-apply — holding `text_wal` shared
    /// across both, so the BM25 snapshot `flush_full` takes cannot fall
    /// between them: a change is in the snapshot, or in the WAL that
    /// survives its truncate (`Collection::flush_bm25_index`).
    ///
    /// # Errors
    ///
    /// Propagates any WAL open / write / flush / fsync failure; memory is
    /// then left as it was.
    ///
    /// LOCK ORDER: text_wal(1a, shared), with no collection lock held;
    /// nothing is taken under it but the BM25 index's own locks.
    pub(super) fn change_text_index(&self, adds: &[(u64, String)], removes: &[u64]) -> Result<()> {
        let ordered = self.text_wal_shared();
        self.change_text_index_ordered(&ordered, adds, removes)
    }

    /// `text_wal` taken shared: what every BM25 change holds across its WAL
    /// append and its in-memory apply ([`Self::change_text_index`]), and what
    /// a delete takes before its node locks and storage guards, so that it
    /// never waits for a BM25 snapshot holding them
    /// ([`Self::change_text_index_ordered`]).
    ///
    /// LOCK ORDER: text_wal(1a), taken with no collection lock held — which
    /// a unit test checks here, on every BM25 change and delete it drives.
    pub(super) fn text_wal_shared(&self) -> TextWalShared<'_> {
        #[cfg(test)]
        assert_eq!(
            crate::lock_audit::guards_held(),
            0,
            "a BM25 change or a delete takes text_wal (1a) holding no collection lock, \
             config (1) included (collection/types.rs, LOCK ORDERING)"
        );
        #[cfg(test)]
        super::write_hooks::before_text_wal();
        self.storage.text_wal.read()
    }

    /// [`Self::change_text_index`] under the `text_wal` guard the caller
    /// already holds — a delete, which takes it first and keeps it across
    /// its storage writes. Taking it again here could deadlock: a snapshot
    /// queued for the exclusive guard parks every new shared taker.
    ///
    /// # Errors
    ///
    /// As [`Self::change_text_index`].
    pub(super) fn change_text_index_ordered(
        &self,
        _ordered: &TextWalShared<'_>,
        adds: &[(u64, String)],
        removes: &[u64],
    ) -> Result<()> {
        self.append_text_batch_to_wal(adds, removes)?;
        #[cfg(test)]
        super::write_hooks::after_text_wal();

        // Only now is the batch durable, so only now does memory change.
        for (id, text) in adds {
            self.storage.text_index.add_document(*id, text);
        }
        for id in removes {
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

    /// Replaces the sparse vectors of the upserted points, WAL-before-apply.
    ///
    /// Upsert replaces a point, and its sparse vectors with it: each point
    /// ends up with exactly the sparse vectors of its **last** occurrence in
    /// the batch, and leaves every sparse index that version does not name —
    /// a point upserted without sparse vectors leaves them all, as a point
    /// upserted without a payload loses its payload.
    // The comment below is the contract: WAL append and in-memory apply stay
    // indivisible with respect to compaction, which takes this lock through
    // its reset.
    #[expect(clippy::significant_drop_tightening)]
    pub(super) fn apply_sparse_batch_upsert(&self, points: &[Point]) -> Result<()> {
        let latest = latest_versions(points);
        // Keep WAL append and in-memory apply indivisible with respect to
        // compaction, which takes this lock exclusively through the reset.
        let mut indexes = self.query.sparse_indexes.write();
        let dropped = dropped_sparse_entries(&indexes, &latest);
        #[cfg(feature = "persistence")]
        {
            for (name, ids) in &dropped {
                let wal_path =
                    crate::index::sparse::persistence::wal_path_for_name(&self.storage.path, name);
                crate::index::sparse::persistence::wal_append_delete_batch(&wal_path, ids)?;
            }
            self.append_sparse_wal_entries(sparse_entries(&latest))?;
        }
        for (name, ids) in &dropped {
            if let Some(idx) = indexes.get(name) {
                for &id in ids {
                    idx.delete(id);
                }
            }
        }
        for (name, point_id, sv) in sparse_entries(&latest) {
            indexes
                .entry(name.to_owned())
                .or_default()
                .insert(point_id, sv);
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

    /// Indexes what storage holds for the ids the deferred indexer buffers
    /// ([`Collection::index_current`]), then retires the copied entries of
    /// the ids it settled: a point leaves the buffer only once indexed as
    /// storage holds it, or found deleted; an id whose insert failed, and an
    /// entry written again meanwhile, stay for the next merge. A no-op while
    /// another merge runs.
    #[cfg(feature = "persistence")]
    pub(super) fn merge_deferred_batch(&self, di: &crate::collection::streaming::DeferredIndexer) {
        let Some(merge) = di.begin_merge() else {
            return;
        };
        let ids = merge.ids();
        let settled = self.index_current(&ids);
        if settled.len() < ids.len() {
            tracing::warn!(
                "deferred merge: {} of {} buffered ids could not be indexed; they stay \
                 buffered, and searchable, for the next merge",
                ids.len() - settled.len(),
                ids.len()
            );
        }
        merge.finish(&settled);
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

/// Each write's payload when it is its id's last write in the batch, `None`
/// for an earlier one: what the batch leaves stored, positionally.
fn last_writes<'a>(writes: &[(u64, Option<&'a Value>)]) -> Vec<Option<&'a Value>> {
    let last: HashMap<u64, usize> = writes
        .iter()
        .enumerate()
        .map(|(i, &(id, _))| (id, i))
        .collect();
    writes
        .iter()
        .enumerate()
        .map(|(i, &(id, payload))| payload.filter(|_| last.get(&id) == Some(&i)))
        .collect()
}

/// The last occurrence of each point id in `points`, ordered by id: the
/// version an upsert batch leaves behind.
fn latest_versions(points: &[Point]) -> BTreeMap<u64, &Point> {
    points.iter().map(|point| (point.id, point)).collect()
}

/// `(index name, point id, vector)` for every sparse vector of `latest`.
fn sparse_entries<'a, 'b>(
    latest: &'b BTreeMap<u64, &'a Point>,
) -> impl Iterator<Item = (&'a str, u64, &'a crate::index::sparse::SparseVector)> + 'b
where
    'a: 'b,
{
    latest.values().flat_map(|point| {
        point
            .sparse_vectors
            .iter()
            .flatten()
            .map(move |(name, sv)| (name.as_str(), point.id, sv))
    })
}

/// Per existing sparse index, the points of `latest` it holds although
/// their new version carries no vector for it.
fn dropped_sparse_entries(
    indexes: &BTreeMap<String, crate::index::sparse::SparseInvertedIndex>,
    latest: &BTreeMap<u64, &Point>,
) -> Vec<(String, Vec<u64>)> {
    indexes
        .iter()
        .filter_map(|(name, idx)| {
            let ids: Vec<u64> = latest
                .values()
                .filter(|point| {
                    !point
                        .sparse_vectors
                        .as_ref()
                        .is_some_and(|map| map.contains_key(name))
                })
                .map(|point| point.id)
                .filter(|&id| idx.contains(id))
                .collect();
            (!ids.is_empty()).then(|| (name.clone(), ids))
        })
        .collect()
}

/// `text_wal` held shared ([`Collection::text_wal_shared`]): the proof a
/// BM25 change under a caller's guard asks for.
pub(super) type TextWalShared<'a> = crate::lock_audit::ReadGuard<'a, ()>;
