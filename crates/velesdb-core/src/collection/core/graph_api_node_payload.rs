//! Node-payload writes for graph collections — one durability barrier per
//! call, whether the call carries one node or ten thousand (#2153).
//!
//! [`Collection::store_node_payload`] used to own the whole procedure, and
//! `AnyCollection::upsert`'s graph arm reached it in a loop. Under the default
//! [`DurabilityMode::Fsync`](crate::storage::DurabilityMode::Fsync) that cost
//! one `flush` + `sync_all` **per node**, plus an auto-snapshot check and two
//! `label_index` acquisitions each, while `Vector` and `Metadata` went through
//! `crud.rs` and paid one barrier for the whole batch. Same shape as the
//! `store_batch_async` defect fixed in #2151, one layer up.
//!
//! Rather than grow a second procedure beside the first, the single-node entry
//! point is now a batch of one. `PayloadStorage::store` and
//! `LogPayloadStorage::store_batch` both funnel into `store_batch_inner` —
//! same `write_store_record`, same `sync_wal_or_resync`, same
//! `maybe_auto_snapshot` — so a one-element batch writes byte-identical WAL and
//! pays exactly the barrier the single-node contract already promised. One
//! procedure means the label index, the property indexes and the mirror
//! invalidation cannot drift between the two paths, which is the failure mode a
//! parallel batch implementation would have introduced.

use rustc_hash::FxHashMap;
use serde_json::Value;

use crate::collection::types::Collection;
use crate::error::Result;
use crate::storage::LogPayloadStorage;

use super::graph_property_index_wiring::extract_labels;

impl Collection {
    /// Stores a JSON payload for a graph node.
    ///
    /// Also maintains the label index: if the payload contains a `_labels`
    /// array, each label is indexed for O(1) lookup in `find_start_nodes()`.
    /// On update (existing node), old labels are removed before new ones
    /// are inserted.
    ///
    /// # Errors
    ///
    /// Returns an error if storage fails.
    pub fn store_node_payload(&self, node_id: u64, payload: &Value) -> Result<()> {
        self.store_node_payloads(&[(node_id, payload)])
    }

    /// Stores JSON payloads for many graph nodes under **one** durability
    /// barrier, maintaining the same indexes [`Self::store_node_payload`] does.
    ///
    /// Duplicate ids resolve last-wins. That is load-bearing rather than a
    /// nicety: the un-index step below reads each node's *old* payload, and a
    /// repeated id would otherwise have the second occurrence read state the
    /// first had already replaced, leaving the first payload's label and
    /// property entries indexed against a node that no longer carries them.
    ///
    /// # Errors
    ///
    /// Returns an error if any payload exceeds the size limit, if any label is
    /// undeclared under a strict schema, or if the write or its barrier fails.
    /// Every payload is validated before anything is written, so a rejected
    /// batch commits nothing — the prefix before the offending entry is not
    /// left behind, which is what the per-node loop did.
    pub fn store_node_payloads(&self, entries: &[(u64, &Value)]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }

        let deduped = dedup_last_wins(entries);
        self.validate_node_payload_batch(&deduped)?;

        // LOCK ORDER: payload_storage(3) → label_index(7), released →
        // graph_range_indexes(7), released → composite_index_manager.
        //
        // `payload_storage` is held across the whole batch, which the single
        // write did not do between its store and its property indexing. The
        // window it closes matters more here: with the guard dropped, another
        // writer could store and index a newer payload for one of these nodes
        // before this call reindexes them, and this call would then index the
        // payload it just superseded. The old payloads are all read before
        // any index lock is taken: the reads hit the WAL, and holding
        // `label_index` across N of them would block its readers for the I/O.
        let ids: Vec<u64> = deduped.iter().map(|&(node_id, _)| node_id).collect();
        let mut storage = self.storage.payload_storage.write();
        let old_payloads = Self::collect_old_payloads_by_id(ids.iter().copied(), &storage);

        // The one barrier. `store_batch` applies the configured durability
        // mode once for the group, where N calls to `store` applied it N times.
        let stored = storage.store_batch(&deduped);
        if stored.is_ok() {
            let written = deduped
                .iter()
                .map(|&(node_id, payload)| (node_id, Some(payload)));
            self.reindex_node_payloads(written, &old_payloads);
        } else {
            self.reindex_after_failed_node_write(&storage, &ids, &old_payloads);
        }
        drop(storage);

        // Node payload writes bypass the upsert mirror hooks — drop the
        // payload mirror so it can never serve stale columnar data. Once for
        // the batch, not once per node, and after a failed write too: the
        // records it left readable are new.
        self.storage.payload_mirror.invalidate();

        // Bump write generation so any cached plan for this collection is
        // invalidated on the next query (CACHE-01).
        self.generations
            .write_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        stored.map_err(Into::into)
    }

    /// Rejects the batch before any mutation, so a bad entry commits nothing.
    ///
    /// Called before `payload_storage` is acquired, and that is required rather
    /// than tidy: `validate_node_labels_against_schema` reads `config` (lock
    /// order 1) and `payload_storage` is order 3, so validating after the
    /// storage guard would invert the documented order to 3 -> 1.
    fn validate_node_payload_batch(&self, deduped: &[(u64, &Value)]) -> Result<()> {
        let max_payload_size = self.runtime_limits().max_payload_size;
        for &(node_id, payload) in deduped {
            // Parity item E: graph node writes take a raw `&Value` rather than
            // a `Point`, so they bypass `enforce_upsert_limits` and need the
            // shared payload-size gate here. `max_vectors_per_collection` is
            // deliberately not checked: vector-less node writes never touch
            // `config.point_count`, so a projected count would be meaningless.
            Self::enforce_payload_limits(node_id, payload, max_payload_size)?;
            self.validate_node_labels_against_schema(payload)?;
        }
        Ok(())
    }

    /// Moves these nodes' label, range and composite index entries from the
    /// payloads they carried before the batch to `held`, what storage now
    /// holds for each (`None`: nothing).
    fn reindex_node_payloads<'a, I>(&self, held: I, old_payloads: &'a [Option<Value>])
    where
        I: Iterator<Item = (u64, Option<&'a Value>)> + Clone,
    {
        self.index_payload_writes(held.clone(), old_payloads);
        for ((node_id, new), old) in held.zip(old_payloads) {
            if let Some(old) = old {
                self.deindex_node_composites(node_id, old);
            }
            if let Some(new) = new {
                self.index_node_composites(node_id, new);
            }
        }
    }

    /// Reindexes the nodes of a batch whose write returned an error.
    ///
    /// A failed batch is not all-or-nothing: `LogPayloadStorage` makes each
    /// record readable as it writes it, so the records before the error are
    /// readable, and all of them when only the sync failed. The indexes follow
    /// what storage now holds, read back per node; a node the write never
    /// reached reads back its old payload and keeps its entries.
    fn reindex_after_failed_node_write(
        &self,
        storage: &LogPayloadStorage,
        ids: &[u64],
        old_payloads: &[Option<Value>],
    ) {
        let held = Self::read_back_payloads(storage, ids);
        let held = ids.iter().copied().zip(held.iter().map(Option::as_ref));
        self.reindex_node_payloads(held, old_payloads);
    }

    /// Enforces strict-schema node-type validation for a node payload write.
    ///
    /// In schemaless mode (the default) or when the payload carries no
    /// `_labels`, this is a no-op. In strict mode every declared label is
    /// checked against the schema before any mutation takes place, so an
    /// undeclared node type is rejected atomically with no partial write.
    ///
    /// # Errors
    ///
    /// Returns `Error::SchemaValidation` if any label in `_labels` is not
    /// declared in the strict schema.
    pub(super) fn validate_node_labels_against_schema(&self, payload: &Value) -> Result<()> {
        let Some(schema) = self.non_schemaless_graph_schema() else {
            return Ok(());
        };
        for label in extract_labels(payload) {
            schema.validate_node_type(&label)?;
        }
        Ok(())
    }
}

/// Keeps the last entry for each id, in the order those last entries appear.
///
/// Order is preserved rather than sorted so the WAL records land in the
/// caller's sequence, which keeps a batch's replay indistinguishable from the
/// equivalent run of single writes.
fn dedup_last_wins<'a>(entries: &[(u64, &'a Value)]) -> Vec<(u64, &'a Value)> {
    let mut last_index: FxHashMap<u64, usize> = FxHashMap::default();
    for (index, &(node_id, _)) in entries.iter().enumerate() {
        last_index.insert(node_id, index);
    }
    entries
        .iter()
        .enumerate()
        .filter(|(index, &(node_id, _))| last_index[&node_id] == *index)
        .map(|(_, &entry)| entry)
        .collect()
}

#[cfg(test)]
#[path = "graph_api_node_payload_tests.rs"]
mod tests;
