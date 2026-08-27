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
use crate::storage::PayloadStorage;

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

        // Validate the whole batch before touching anything. Beyond the
        // all-or-nothing contract above, this ordering is required:
        // `validate_node_labels_against_schema` reads `config` (lock order 1)
        // and `payload_storage` below is order 3, so resolving the schema after
        // the storage guard would invert the documented order to 3 -> 1.
        let max_payload_size = self.runtime_limits().max_payload_size;
        for &(node_id, payload) in &deduped {
            // Parity item E: graph node writes take a raw `&Value` rather than
            // a `Point`, so they bypass `enforce_upsert_limits` and need the
            // shared payload-size gate here. `max_vectors_per_collection` is
            // deliberately not checked: vector-less node writes never touch
            // `config.point_count`, so a projected count would be meaningless.
            Self::enforce_payload_value_size(node_id, payload, max_payload_size)?;
            self.validate_node_labels_against_schema(payload)?;
        }

        // LOCK ORDER: payload_storage(3) → label_index(7) → graph_range_indexes(7).
        //
        // `payload_storage` is held across the whole batch, which the single
        // write did not do between its store and its property indexing. The
        // window it closes matters more here: with the guard dropped, another
        // writer could store and index a newer payload for one of these nodes
        // before this call's `index_node_properties` runs, and this call would
        // then index the payload it just superseded.
        let mut storage = self.storage.payload_storage.write();

        // Un-index whatever these ids carried before. Must complete before the
        // write below, which replaces the payloads it reads here.
        //
        // Read first, then un-index, rather than interleaving the two: the
        // retrieves hit the WAL, and holding `label_index` across N of them
        // would block every reader of the label index for the whole I/O pass.
        // The guard is taken once for the removals, not once per node — taking
        // it per node is part of the cost this batch exists to remove.
        let mut superseded: Vec<(u64, Value)> = Vec::new();
        for &(node_id, _) in &deduped {
            if let Ok(Some(old_payload)) = storage.retrieve(node_id) {
                superseded.push((node_id, old_payload));
            }
        }
        {
            let mut label_idx = self.graph.label_index.write();
            for (node_id, old_payload) in &superseded {
                label_idx.remove_from_payload(*node_id, old_payload);
            }
        }
        // `label_index` released before the graph property indexes: both sit at
        // lock order 7, with no ordering between them.
        for (node_id, old_payload) in &superseded {
            self.deindex_node_properties(*node_id, old_payload);
        }

        // The one barrier. `store_batch` applies the configured durability
        // mode once for the group, where N calls to `store` applied it N times.
        storage.store_batch(&deduped)?;

        {
            let mut label_idx = self.graph.label_index.write();
            for &(node_id, payload) in &deduped {
                label_idx.index_from_payload(node_id, payload);
            }
        }
        for &(node_id, payload) in &deduped {
            self.index_node_properties(node_id, payload);
        }
        drop(storage);

        // Node payload writes bypass the upsert mirror hooks — drop the
        // payload mirror so it can never serve stale columnar data. Once for
        // the batch, not once per node.
        self.storage.payload_mirror.invalidate();

        // Bump write generation so any cached plan for this collection is
        // invalidated on the next query (CACHE-01).
        self.generations
            .write_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
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
