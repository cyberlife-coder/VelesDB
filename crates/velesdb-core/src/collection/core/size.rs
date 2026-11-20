//! A collection's size: one count behind every size surface.
//!
//! [`Collection::len`], [`Collection::is_empty`] and `config().point_count`
//! — and through them every view and binding: `VectorCollection`,
//! `GraphCollection`, `MetadataCollection`, `AnyCollection`, the REST,
//! Python, Tauri and CLI counts — as well as ANALYZE's `row_count` and the
//! MATCH planner's `total_nodes` read one counter, in O(1). What it counts is
//! the collection's rows, by the rule every SELECT reads them by
//! ([`RowUniverse`]): a vector collection's vectors, a metadata collection's
//! payloads, a graph's stored nodes — with an embedding, a payload or both,
//! each counted once (`|V ∪ P|`).
//!
//! # How the count stays exact
//!
//! `Collection::open` counts it from storage ([`RowUniverse::count`]: for a
//! graph with embeddings, `min(|V|, |P|)` id lookups, once). From there,
//! every write that can change which ids a store holds runs through
//! [`Collection::write_vector_store`], [`Collection::write_payload_store`]
//! (or [`Collection::write_payloads_beside_vectors`], for a graph's write
//! that also reads the vector store; or [`RowStores`], which a row write
//! gets them through) or [`Collection::write_both_stores`],
//! which measure what the write changed and move the counter by it while the
//! write's storage guards are held:
//!
//! * a vector collection's size is `|V|`: a vector write reads the vector
//!   store's length before and after; a payload write cannot change it, and
//!   takes no vector guard. Its writes follow one rule of node locks: a row
//!   write — `upsert`, the bulk upserts, and the streaming drain and the DML
//!   writes that run through them — stores its payloads then its vectors,
//!   and makes its BM25 change, under the node locks of its ids
//!   ([`Collection::write_rows`]); a payload write to its rows alone —
//!   `upsert_metadata` ([`Collection::write_payloads_where_vectors`]) —
//!   checks, under those locks, that each id stores a vector; and every
//!   write that may remove one — a delete — takes them first. So a delete
//!   lands wholly before or after a write of the same id: a row keeps both
//!   its vector and its payload or loses both, and a payload stored belongs
//!   to a row.
//! * a metadata collection's is `|P|`, symmetrically.
//! * a graph's is `|V ∪ P|`. A write that only stores
//!   ([`StoreEffect::Stores`]) into ids its store already holds cannot change
//!   it — every id stays in that store, whatever the other does — so it runs
//!   under its store's write guard alone and moves nothing. Any other write
//!   first takes the node locks of its ids ([`NodeLocks`], a stripe per id),
//!   reads which of them the other store holds through that store's shared
//!   id index (`held_ids`: the index's own lock, no store guard), then counts
//!   the nodes among its ids before and after it writes, under its own
//!   store's write guard: every write that can change an id's membership of
//!   either store holds that id's lock while it does, so the membership read
//!   stays true until the size moved. No write takes the other store's guard
//!   at all: a graph's payload writers and vector writers wait on each other
//!   only where their ids share a stripe.
//!
//! Two writes that can change the same part of one size are ordered — they
//! share a guard one of them holds for writing (a vector or metadata
//! collection, a graph's write to one store) or a node lock (a graph's
//! writes to two stores) — so their measurements and updates never
//! interleave: the counter is exact whenever no write is in flight, and
//! never transiently off — a reader holding both stores' read guards reads
//! it equal to a recount, every size move made under the write guard of the
//! store whose change it counts (`size_tests.rs` races writers against one).
//! A write that fails part-way is measured all the same — what it left in
//! storage is what moved.
//!
//! A store mutation made outside these entry points would leave the
//! count off until the next open. Two things guard against one: they are the
//! only code in the crate that takes a storage write guard to change which
//! ids a store holds — the others take one to flush (the two flushes), to
//! compact (`compact_vector_storage`, which rewrites the vector file but keeps
//! every id) or, in `open`, to drain the replayed WAL ids — which a search for
//! `vector_storage.write()` / `payload_storage.write()` checks; and
//! `size_tests.rs` drives every write path of every kind of collection and
//! compares the count with a recount.

use std::sync::atomic::Ordering;

use rustc_hash::FxHashSet;

use crate::collection::search::query::row_universe::RowUniverse;
use crate::collection::types::Collection;
use crate::lock_audit::{Mutex, MutexGuard};
use crate::storage::{LogPayloadStorage, MmapStorage, PayloadStorage, VectorStorage};

impl RowUniverse {
    /// The size counted from both stores: O(1), except for a graph whose two
    /// stores both hold ids, where the ids of the smaller store are looked
    /// up in the larger one's index. `Collection::open` counts it once.
    pub(crate) fn count(self, vectors: &MmapStorage, payloads: &LogPayloadStorage) -> usize {
        match self {
            Self::Vectors => vectors.len(),
            Self::Payloads => payloads.len(),
            Self::StoredNodes => node_count(vectors, payloads),
        }
    }
}

/// `|V ∪ P|` = `|V| + |P| - |V ∩ P|`, the intersection counted by probing
/// the larger store with each id of the smaller.
fn node_count(vectors: &MmapStorage, payloads: &LogPayloadStorage) -> usize {
    let in_both = if vectors.len() <= payloads.len() {
        vectors
            .ids()
            .into_iter()
            .filter(|&id| payloads.contains(id))
            .count()
    } else {
        payloads
            .ids()
            .into_iter()
            .filter(|&id| vectors.contains(id))
            .count()
    };
    vectors.len() + payloads.len() - in_both
}

/// How many distinct ids of `ids` either store holds: the part of a graph's
/// size a write over `ids` can change.
fn nodes_among(vectors: &MmapStorage, payloads: &LogPayloadStorage, ids: &[u64]) -> usize {
    ids.iter()
        .copied()
        .collect::<FxHashSet<u64>>()
        .into_iter()
        .filter(|&id| vectors.contains(id) || payloads.contains(id))
        .count()
}

/// What a write does to the ids it names in the store it writes: what lets a
/// graph's write skip the other store's guard ([`Collection::write_vector_store`],
/// [`Collection::write_payloads_beside_vectors`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StoreEffect {
    /// It only stores: every id it names holds a value in the store after it
    /// — a store that fails part-way removes nothing either.
    Stores,
    /// It may remove values too: a delete, or an upsert whose point without
    /// a payload deletes the stored one.
    StoresOrRemoves,
}

impl StoreEffect {
    /// Whether a write of this effect over `ids` leaves a graph's size as it
    /// is, judged under its store's write guard: it only stores, into ids the
    /// store (`holds`) already holds.
    fn keeps_nodes(self, ids: &[u64], holds: impl Fn(u64) -> bool) -> bool {
        self == Self::Stores && ids.iter().all(|&id| holds(id))
    }
}

impl Collection {
    /// The collection's size (its [`RowUniverse`]'s ids), in O(1).
    pub(crate) fn size(&self) -> usize {
        self.storage.size.load(Ordering::Relaxed)
    }

    /// How many vectors the collection stores: its size only for a vector
    /// collection. What sizes a vector search reads it.
    pub(crate) fn vector_count(&self) -> usize {
        self.storage.vector_storage.read().len()
    }

    /// Moves the size from `before` to `after`, as the caller measured them
    /// under the guards of its write, which it still holds.
    fn move_size(&self, before: usize, after: usize) {
        if after >= before {
            self.storage
                .size
                .fetch_add(after - before, Ordering::Relaxed);
        } else {
            self.storage
                .size
                .fetch_sub(before - after, Ordering::Relaxed);
        }
    }

    /// Runs `write` on the vector store under its write guard and moves the
    /// size by what it changed; `rows` are the collection's
    /// ([`Collection::row_universe`], read by the caller before it takes any
    /// lock), `ids` the ids `write` writes or removes, and `effect` what it
    /// does to them.
    ///
    /// LOCK ORDER: on a graph, see [`Self::write_graph_vectors`]; on a
    /// vector collection, node_locks(1c) when the write may remove a vector
    /// (module doc) → vector_storage(2); else vector_storage(2) alone.
    pub(crate) fn write_vector_store<T>(
        &self,
        rows: RowUniverse,
        ids: &[u64],
        effect: StoreEffect,
        write: impl FnOnce(&mut MmapStorage) -> T,
    ) -> T {
        if rows == RowUniverse::StoredNodes {
            return self.write_graph_vectors(ids, effect, write);
        }
        let _nodes = (rows == RowUniverse::Vectors && effect == StoreEffect::StoresOrRemoves)
            .then(|| self.storage.node_locks.lock(&distinct(ids)));
        let mut vectors = self.storage.vector_storage.write();
        let before = vectors.len();
        let written = write(&mut vectors);
        if rows == RowUniverse::Vectors {
            self.move_size(before, vectors.len());
        }
        drop(vectors);
        written
    }

    /// [`Self::write_vector_store`] on a graph. A write that keeps its nodes
    /// ([`StoreEffect::keeps_nodes`]) runs under `vector_storage`'s write
    /// guard alone and moves nothing. Any other takes the node locks of its
    /// ids, reads which of them the payload store holds (its shared id
    /// index) — which no other write can change while those locks are held
    /// — and counts the nodes among its ids before and after it writes,
    /// under the vector store's write guard (module doc).
    ///
    /// LOCK ORDER: vector_storage(2), released when the write may change
    /// its nodes; node_locks(1c) → vector_storage(2).
    fn write_graph_vectors<T>(
        &self,
        ids: &[u64],
        effect: StoreEffect,
        write: impl FnOnce(&mut MmapStorage) -> T,
    ) -> T {
        {
            let mut vectors = self.storage.vector_storage.write();
            if effect.keeps_nodes(ids, |id| vectors.contains(id)) {
                return write(&mut vectors);
            }
        }
        let ids = distinct(ids);
        let _nodes = self.storage.node_locks.lock(&ids);
        let in_payloads = held_by(&ids, |id| self.storage.held_payload_ids.contains(id));
        #[cfg(test)]
        super::write_hooks::after_node_membership();
        let mut vectors = self.storage.vector_storage.write();
        let before = nodes_given(&ids, |id| vectors.contains(id), &in_payloads);
        let written = write(&mut vectors);
        self.move_size(
            before,
            nodes_given(&ids, |id| vectors.contains(id), &in_payloads),
        );
        drop(vectors);
        written
    }

    /// Runs `write` on the payload store under its write guard and moves the
    /// size by what it changed; `rows` are the collection's
    /// ([`Collection::row_universe`], read by the caller before it takes any
    /// lock), `ids` the ids `write` writes or removes, and `effect` what it
    /// does to them.
    ///
    /// LOCK ORDER: as [`Self::write_payloads_beside_vectors`].
    pub(crate) fn write_payload_store<T>(
        &self,
        rows: RowUniverse,
        ids: &[u64],
        effect: StoreEffect,
        write: impl FnOnce(&mut LogPayloadStorage) -> T,
    ) -> T {
        self.write_payloads_beside_vectors(rows, ids, effect, |_, payloads| write(payloads))
    }

    /// [`Self::write_payload_store`] on a vector collection's rows: refuses,
    /// before writing anything, a batch naming an id that stores no vector —
    /// a payload there would be no row — holding the node locks of `ids`
    /// from that check through `write` ([`Self::write_payloads_where_vectors`]).
    ///
    /// # Errors
    ///
    /// [`Error::VectorRequired`](crate::Error::VectorRequired), naming the
    /// collection `name`, when an id stores no vector; else `write`'s error.
    ///
    /// LOCK ORDER: node_locks(1c) → payload_storage(3) → what `write` takes.
    pub(crate) fn write_payloads_of_vector_rows<T>(
        &self,
        name: &str,
        ids: &[u64],
        write: impl FnOnce(&mut LogPayloadStorage) -> crate::error::Result<T>,
    ) -> crate::error::Result<T> {
        self.write_payloads_where_vectors(ids, |stores_vector, payloads| {
            if !ids.iter().all(|&id| stores_vector(id)) {
                return Err(crate::Error::VectorRequired(name.to_string()));
            }
            write(payloads)
        })
    }

    /// How a payload is written to a vector collection's rows apart from
    /// their vectors — `upsert_metadata` and a bulk upsert's second store; a
    /// plain `upsert` writes its payloads through
    /// [`Self::write_payload_store`], beside the vectors it stored in the
    /// same call, without this check. Under the node locks of `ids`, `write`
    /// gets whether each id stores a
    /// vector — read through the vector store's shared id index, no vector
    /// guard taken — and stores a payload only where one does, so the
    /// payload belongs to a row. Every write that may remove a vector of a
    /// vector collection takes the locks of its ids first
    /// ([`Self::write_vector_store`], [`Self::write_both_stores`]), so the
    /// answer stays true while `write` stores. A vector write or a search
    /// never waits for this write's store and sync. A vector collection's
    /// size counts its vectors, so nothing moves it.
    ///
    /// [`Self::write_payloads_of_vector_rows`] refuses a batch naming an id
    /// without a vector; a bulk upsert, whose vectors a delete may remove
    /// between its two stores, stores the payloads of the ids that still
    /// store one (`Collection::store_payload_entries`).
    ///
    /// LOCK ORDER: node_locks(1c) → payload_storage(3) → what `write` takes.
    pub(crate) fn write_payloads_where_vectors<T>(
        &self,
        ids: &[u64],
        write: impl FnOnce(&dyn Fn(u64) -> bool, &mut LogPayloadStorage) -> T,
    ) -> T {
        let _nodes = self.storage.node_locks.lock(&distinct(ids));
        #[cfg(test)]
        super::write_hooks::after_node_membership();
        let stores_vector = |id: u64| self.storage.held_vector_ids.contains(id);
        let mut payloads = self.storage.payload_storage.write();
        let written = write(&stores_vector, &mut payloads);
        drop(payloads);
        written
    }

    /// [`Self::write_payload_store`] for a write that asks whether an id of
    /// its batch holds a vector — a graph's node write, which creates a node
    /// only where none is stored: `write` gets that answer, read once its
    /// node locks are held, so it stays true while it writes. On a vector or
    /// metadata collection it answers `false`: their payload write takes no
    /// vector guard and creates no node.
    ///
    /// LOCK ORDER: on a graph, see [`Self::write_graph_payloads`]; else
    /// payload_storage(3) alone.
    pub(crate) fn write_payloads_beside_vectors<T>(
        &self,
        rows: RowUniverse,
        ids: &[u64],
        effect: StoreEffect,
        write: impl FnOnce(&dyn Fn(u64) -> bool, &mut LogPayloadStorage) -> T,
    ) -> T {
        if rows == RowUniverse::StoredNodes {
            return self.write_graph_payloads(ids, effect, write);
        }
        let mut payloads = self.storage.payload_storage.write();
        let before = payloads.len();
        let written = write(&|_| false, &mut payloads);
        if rows == RowUniverse::Payloads {
            self.move_size(before, payloads.len());
        }
        drop(payloads);
        written
    }

    /// [`Self::write_payloads_beside_vectors`] on a graph, as
    /// [`Self::write_graph_vectors`] writes its vectors: a write that keeps
    /// its nodes runs under `payload_storage`'s write guard alone — every id
    /// is a node stored in the payload store, so whether it holds a vector
    /// decides nothing — and any other under its node locks, the vector
    /// store's membership of its ids read first (its shared id index).
    ///
    /// LOCK ORDER: payload_storage(3), released when the write may change
    /// its nodes; node_locks(1c) → payload_storage(3).
    fn write_graph_payloads<T>(
        &self,
        ids: &[u64],
        effect: StoreEffect,
        write: impl FnOnce(&dyn Fn(u64) -> bool, &mut LogPayloadStorage) -> T,
    ) -> T {
        {
            let mut payloads = self.storage.payload_storage.write();
            if effect.keeps_nodes(ids, |id| payloads.contains(id)) {
                return write(&|_| true, &mut payloads);
            }
        }
        let ids = distinct(ids);
        let _nodes = self.storage.node_locks.lock(&ids);
        let in_vectors = held_by(&ids, |id| self.storage.held_vector_ids.contains(id));
        #[cfg(test)]
        super::write_hooks::after_node_membership();
        let mut payloads = self.storage.payload_storage.write();
        let before = nodes_given(&ids, |id| payloads.contains(id), &in_vectors);
        let written = write(&|id| in_vectors.contains(&id), &mut payloads);
        self.move_size(
            before,
            nodes_given(&ids, |id| payloads.contains(id), &in_vectors),
        );
        drop(payloads);
        written
    }

    /// Runs `write` on both stores under their write guards and moves the
    /// size by what it changed; `rows` are the collection's, read by the
    /// caller before it takes any lock, and `ids` the ids `write` writes or
    /// removes. Unless the collection stores no vector it holds the node
    /// locks of its ids too: on a graph, so no write measuring against one
    /// store's membership of them runs meanwhile; on a vector collection, so
    /// no payload write that checked they store vectors
    /// ([`Self::write_payloads_of_vector_rows`]) runs meanwhile.
    ///
    /// LOCK ORDER: node_locks(1c, not on a metadata collection) →
    /// vector_storage(2) → payload_storage(3).
    pub(crate) fn write_both_stores<T>(
        &self,
        rows: RowUniverse,
        ids: &[u64],
        write: impl FnOnce(&mut MmapStorage, &mut LogPayloadStorage) -> T,
    ) -> T {
        let distinct_ids = distinct(ids);
        let _nodes =
            (rows != RowUniverse::Payloads).then(|| self.storage.node_locks.lock(&distinct_ids));
        let mut vectors = self.storage.vector_storage.write();
        let mut payloads = self.storage.payload_storage.write();
        let measure = |vectors: &MmapStorage, payloads: &LogPayloadStorage| match rows {
            RowUniverse::StoredNodes => nodes_among(vectors, payloads, &distinct_ids),
            _ => rows.count(vectors, payloads),
        };
        let before = measure(&vectors, &payloads);
        let written = write(&mut vectors, &mut payloads);
        self.move_size(before, measure(&vectors, &payloads));
        // Released only once the size moved (module doc).
        drop(payloads);
        drop(vectors);
        written
    }
}

/// The two stores of a row write — an upsert, bulk or not, storing a vector
/// and its payload per id — as [`Collection::write_rows`] hands them over.
/// On a vector collection it holds `text_wal` (1a) shared and then the node
/// locks (1c) of the write's ids for as long as it lives: across the payload
/// store, the vector store and the BM25 change, which therefore goes through
/// [`RowStores::change_text_index`], never `text_wal` taken again.
pub(crate) struct RowStores<'c> {
    collection: &'c Collection,
    rows: RowUniverse,
    text_wal: Option<super::crud_indexing::TextWalShared<'c>>,
    _nodes: Option<Vec<MutexGuard<'c, ()>>>,
}

impl RowStores<'_> {
    /// Runs `write` on the payload store ([`Collection::write_payload_store`]),
    /// `ids` the ids it writes or removes. Inside a vector collection's row
    /// write that takes no node lock — its payload write never does — so the
    /// locks this holds are not taken twice.
    pub(crate) fn payloads<T>(
        &self,
        ids: &[u64],
        effect: StoreEffect,
        write: impl FnOnce(&mut LogPayloadStorage) -> T,
    ) -> T {
        self.collection
            .write_payload_store(self.rows, ids, effect, write)
    }

    /// Runs `write`, which only stores, on the vector store
    /// ([`Collection::write_vector_store`] with [`StoreEffect::Stores`]),
    /// `ids` the ids it writes. A vector collection's write that only stores
    /// takes no node lock, so the locks this holds are not taken twice.
    pub(crate) fn vectors<T>(&self, ids: &[u64], write: impl FnOnce(&mut MmapStorage) -> T) -> T {
        self.collection
            .write_vector_store(self.rows, ids, StoreEffect::Stores, write)
    }

    /// Applies the write's BM25 change: under the `text_wal` guard this holds
    /// on a vector collection ([`Collection::change_text_index_ordered`]), so
    /// a delete of one of its ids sees the document added or not at all;
    /// elsewhere taking it ([`Collection::change_text_index`]).
    ///
    /// # Errors
    ///
    /// Any BM25 WAL failure.
    pub(crate) fn change_text_index(
        &self,
        adds: &[(u64, String)],
        removes: &[u64],
    ) -> crate::error::Result<()> {
        match &self.text_wal {
            Some(text_wal) => self
                .collection
                .change_text_index_ordered(text_wal, adds, removes),
            None => self.collection.change_text_index(adds, removes),
        }
    }
}

impl Collection {
    /// How every row write stores — `upsert` and the bulk upserts, and the
    /// streaming drain and the DML writes that run through them — handing
    /// `write` the two stores ([`RowStores`]); `rows` are the collection's,
    /// read before any lock, and `ids` every id the write stores.
    ///
    /// On a vector collection — the one rule of its row writes — `text_wal`
    /// is taken shared, then the node locks of `ids`, both held until
    /// `write` returns: its payload store, its vector store and its BM25
    /// change run under them. A delete takes the same two first
    /// (`Collection::delete_counted`), so it lands wholly before or wholly
    /// after the write: an id ends with both its vector and its payload, or
    /// neither, and its BM25 document follows whichever came last. The
    /// payload store comes first: a crash between the two leaves a payload
    /// without a vector, which is no row — every read skips it, and a delete
    /// or the next write of the id clears it — never a vector without the
    /// payload written with it. The histograms are not changed under the
    /// locks: their I/O lock (12) is taken with no other lock held.
    ///
    /// A graph's row write takes no lock here: each store takes the node
    /// locks it needs to keep the graph's size exact, as
    /// [`Self::write_vector_store`] and [`Self::write_payload_store`] do.
    ///
    /// LOCK ORDER: on a vector collection, text_wal(1a, shared) →
    /// node_locks(1c) → payload_storage(3), released → vector_storage(2),
    /// released → the BM25 index's own locks; elsewhere what each store
    /// takes.
    pub(crate) fn write_rows<T>(
        &self,
        rows: RowUniverse,
        ids: &[u64],
        write: impl FnOnce(&RowStores<'_>) -> T,
    ) -> T {
        let locked = rows == RowUniverse::Vectors;
        let text_wal = locked.then(|| self.text_wal_shared());
        let nodes = locked.then(|| self.storage.node_locks.lock(&distinct(ids)));
        write(&RowStores {
            collection: self,
            rows,
            text_wal,
            _nodes: nodes,
        })
    }
}

/// `ids` without repeats, in no particular order.
fn distinct(ids: &[u64]) -> Vec<u64> {
    ids.iter()
        .copied()
        .collect::<FxHashSet<u64>>()
        .into_iter()
        .collect()
}

/// The ids of `ids` that `holds` holds.
fn held_by(ids: &[u64], holds: impl Fn(u64) -> bool) -> FxHashSet<u64> {
    ids.iter().copied().filter(|&id| holds(id)).collect()
}

/// How many of the distinct `ids` are nodes: held by the store a write
/// writes (`own`) or, as read under the write's node locks, by the other.
fn nodes_given(ids: &[u64], own: impl Fn(u64) -> bool, other: &FxHashSet<u64>) -> usize {
    ids.iter()
        .filter(|&&id| own(id) || other.contains(&id))
        .count()
}

/// How many node locks a graph has: a write takes those of its ids' stripes.
const NODE_LOCKS: u64 = 64;

/// A collection's node locks (lock order position **1c**). On a graph, a
/// write that may change which ids are nodes holds the locks of its ids
/// from reading the other store's membership of them until it has moved the
/// size, so the membership it read holds throughout — every write that
/// changes an id's membership of either store takes its lock first. A write
/// that only stores into ids its store already holds changes no membership
/// and takes none. On a vector collection, a row write holds them across
/// its two stores and its BM25 change ([`Collection::write_rows`]), a payload
/// write to its rows from checking that its ids store vectors through its
/// write, and every write that may remove a vector takes them first. A
/// metadata collection takes none. Stripes, not one lock per id: ids sharing a
/// stripe serialize, others do not; readers never take them.
pub(crate) struct NodeLocks {
    stripes: Box<[Mutex<()>]>,
}

impl NodeLocks {
    pub(crate) fn new() -> Self {
        Self {
            stripes: (0..NODE_LOCKS).map(|_| Mutex::new(())).collect(),
        }
    }

    /// Locks the stripes of `ids`, in ascending stripe order, so two writes
    /// never wait on each other's stripes in a cycle.
    fn lock(&self, ids: &[u64]) -> Vec<MutexGuard<'_, ()>> {
        let mut stripes: Vec<usize> = ids.iter().map(|&id| stripe_of(id)).collect();
        stripes.sort_unstable();
        stripes.dedup();
        stripes
            .into_iter()
            .map(|i| self.stripes[i].lock())
            .collect()
    }
}

/// The stripe of `id`: below `NODE_LOCKS`, so it fits a `usize`.
fn stripe_of(id: u64) -> usize {
    usize::try_from((id ^ (id >> 32)) % NODE_LOCKS).unwrap_or_default()
}
