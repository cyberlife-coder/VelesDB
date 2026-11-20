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

use super::size::StoreEffect;
use crate::collection::types::Collection;
use crate::error::Result;

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
    /// On a vector collection, whose rows are its vectors, a payload stored
    /// here for an id without a vector is an edge endpoint (`add_edge`
    /// requires a payload at both ends) and no row: no `SELECT`, count or
    /// enumeration reads it.
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
        let written = self.write_deduped_node_payloads(&deduped, &[]);
        // Node payload writes bypass the upsert mirror hooks — drop the
        // payload mirror so it can never serve stale columnar data — and
        // bump the write generation so any cached plan for this collection
        // is invalidated on the next query (CACHE-01). Once for the batch,
        // not once per node, and after a failed write too: the records it
        // left readable are new.
        self.invalidate_caches_and_bump_generation();
        written?;
        self.report_upsert(entries.len());
        Ok(())
    }

    /// The write half of [`Self::store_node_payloads`], for a batch its
    /// caller has validated ([`Self::validate_node_payload_batch`]) and
    /// reports to the observer itself: `Collection::upsert` writes a graph's
    /// payload-only rows through here beside its embedded ones.
    ///
    /// Each id of `create_if_absent` — a row carrying no payload, which
    /// keeps the one stored — is written with an empty payload where no node
    /// is stored, as an upsert of it would create the node; it is decided
    /// under the storage guards, so a node a concurrent write creates is
    /// never overwritten. Returns the ids it created. The caller brings the
    /// payload mirror and the write generation in line, after a failed write
    /// too.
    pub(super) fn write_node_payloads(
        &self,
        entries: &[(u64, &Value)],
        create_if_absent: &[u64],
    ) -> Result<Vec<u64>> {
        if entries.is_empty() && create_if_absent.is_empty() {
            return Ok(Vec::new());
        }
        self.write_deduped_node_payloads(&dedup_last_wins(entries), create_if_absent)
    }

    /// Writes a batch holding each id once under one barrier — with an empty
    /// payload for each id of `create_if_absent` that stores no node
    /// ([`Self::write_node_payloads`]) — and brings every payload-derived
    /// index in line with what storage then holds: the secondary, label and
    /// range indexes under the storage guard, as every write maintains them
    /// ([`Self::index_payload_writes`]). The size moves by the nodes it created
    /// ([`Self::write_payloads_beside_vectors`], which hands over the vector
    /// store a graph asks "is this id a node" of). Returns the ids of
    /// `create_if_absent` it created.
    ///
    /// The effects every write path shares — BM25 and the histograms
    /// ([`Self::apply_payload_effects`]) — follow a batch the store wrote:
    /// their WAL never leads payload durability, so a failed write runs none
    /// of them. The in-memory label, range and secondary indexes follow what
    /// storage then holds, read back per node
    /// ([`Self::reindex_after_failed_node_write`]).
    fn write_deduped_node_payloads(
        &self,
        deduped: &[(u64, &Value)],
        create_if_absent: &[u64],
    ) -> Result<Vec<u64>> {
        let empty = Value::Object(serde_json::Map::new());
        let touched: Vec<u64> = deduped
            .iter()
            .map(|&(node_id, _)| node_id)
            .chain(create_if_absent.iter().copied())
            .collect();
        // LOCK ORDER: as `write_payloads_beside_vectors` (on a graph that
        // may change its nodes, node_locks(1c) first), then
        // payload_storage(3) → as `index_payload_writes`.
        let written = self.write_payloads_beside_vectors(
            self.row_universe(),
            &touched,
            StoreEffect::Stores,
            |has_vector, storage| {
                let batch = node_batch(deduped, create_if_absent, &empty, has_vector, storage);
                self.store_node_batch(storage, batch)
            },
        );
        let NodeBatchWrite {
            ids,
            batch,
            old_payloads,
            stored,
            read_back,
        } = written;
        if ids.is_empty() {
            // Every row kept a stored node: nothing was written.
            return Ok(Vec::new());
        }
        let held: Vec<Option<&Value>> = match &read_back {
            None => batch.iter().map(|&(_, payload)| Some(payload)).collect(),
            Some(read_back) => read_back.iter().map(Option::as_ref).collect(),
        };

        let effects = match stored {
            Ok(()) => {
                let writes: Vec<(u64, Option<&Value>)> = ids.iter().copied().zip(held).collect();
                self.apply_payload_effects(&writes, &old_payloads)
            }
            Err(error) => Err(error.into()),
        };
        let created = ids[deduped.len()..].to_vec();
        effects.map(|()| created)
    }

    /// Stores `batch` — each id once — under the one barrier, and brings the
    /// secondary, label and range indexes in line with what storage then
    /// holds, under the guard the caller holds ([`Self::index_payload_writes`]).
    ///
    /// `payload_storage` is held across the whole batch, which the single
    /// write did not do between its store and its property indexing. The
    /// window it closes matters more here: with the guard dropped, another
    /// writer could store and index a newer payload for one of these nodes
    /// before this call reindexes them, and this call would then index the
    /// payload it just superseded. The old payloads are all read before any
    /// index lock is taken: the reads hit the WAL, and holding `label_index`
    /// across N of them would block its readers for the I/O.
    fn store_node_batch<'v>(
        &self,
        storage: &mut crate::storage::LogPayloadStorage,
        batch: Vec<(u64, &'v Value)>,
    ) -> NodeBatchWrite<'v> {
        let ids: Vec<u64> = batch.iter().map(|&(node_id, _)| node_id).collect();
        let old_payloads = Self::collect_old_payloads_by_id(ids.iter().copied(), storage);
        // The one barrier. `store_batch` applies the configured durability
        // mode once for the group, where N calls to `store` applied it N times.
        let stored = storage.store_batch(&batch);
        let read_back = if stored.is_ok() {
            let writes = batch
                .iter()
                .map(|&(node_id, payload)| (node_id, Some(payload)));
            self.index_payload_writes(writes, &old_payloads);
            None
        } else {
            Some(self.reindex_after_failed_node_write(storage, &ids, &old_payloads))
        };
        NodeBatchWrite {
            ids,
            batch,
            old_payloads,
            stored,
            read_back,
        }
    }

    /// Rejects the batch before any mutation, so a bad entry commits nothing.
    ///
    /// Called before `payload_storage` is acquired, and that is required rather
    /// than tidy: `validate_node_labels_against_schema` reads `config` (lock
    /// order 1) and `payload_storage` is order 3, so validating after the
    /// storage guard would invert the documented order to 3 -> 1.
    pub(super) fn validate_node_payload_batch(&self, deduped: &[(u64, &Value)]) -> Result<()> {
        let max_payload_size = self.runtime_limits().max_payload_size;
        for &(node_id, payload) in deduped {
            // Parity item E: graph node writes take a raw `&Value` rather than
            // a `Point`, so they bypass `enforce_upsert_limits` and need the
            // shared payload-size gate here. `max_vectors_per_collection` is
            // deliberately not checked: it caps the vectors a collection
            // stores (`enforce_vector_count`), and a node-payload write
            // stores none.
            Self::enforce_payload_limits(node_id, payload, max_payload_size)?;
            self.validate_node_labels_against_schema(payload)?;
        }
        Ok(())
    }

    /// Reindexes the nodes of a batch whose write returned an error, and
    /// returns what storage now holds for each of `ids`.
    ///
    /// A failed batch is not all-or-nothing: `LogPayloadStorage` makes each
    /// record readable as it writes it, so the records before the error are
    /// readable, and all of them when only the sync failed. The indexes follow
    /// what storage now holds, read back per node; a node the write never
    /// reached reads back its old payload and keeps its entries.
    pub(super) fn reindex_after_failed_node_write(
        &self,
        storage: &crate::storage::LogPayloadStorage,
        ids: &[u64],
        old_payloads: &[Option<Value>],
    ) -> Vec<Option<Value>> {
        let held = Self::read_back_payloads(storage, ids);
        let writes = ids.iter().copied().zip(held.iter().map(Option::as_ref));
        self.index_payload_writes(writes, old_payloads);
        held
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

/// A node batch as [`Collection::store_node_batch`] wrote it.
struct NodeBatchWrite<'v> {
    /// The ids of `batch`, in order.
    ids: Vec<u64>,
    /// What the batch wrote, each id once.
    batch: Vec<(u64, &'v Value)>,
    /// Each id's payload before the batch.
    old_payloads: Vec<Option<Value>>,
    /// How the write ended.
    stored: std::io::Result<()>,
    /// What storage holds for each id after a failed write; `None` when the
    /// write succeeded and holds `batch`.
    read_back: Option<Vec<Option<Value>>>,
}

/// The batch a node-payload write stores: `deduped`, then an `empty` payload
/// for each id of `create_if_absent` that stores no node — neither a payload
/// nor, as `has_vector` answers on a graph, a vector.
fn node_batch<'v>(
    deduped: &[(u64, &'v Value)],
    create_if_absent: &[u64],
    empty: &'v Value,
    has_vector: &dyn Fn(u64) -> bool,
    payloads: &crate::storage::LogPayloadStorage,
) -> Vec<(u64, &'v Value)> {
    let absent = |id: u64| !has_vector(id) && !payloads.contains(id);
    let mut batch = deduped.to_vec();
    batch.extend(
        create_if_absent
            .iter()
            .filter(|&&id| absent(id))
            .map(|&id| (id, empty)),
    );
    batch
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
