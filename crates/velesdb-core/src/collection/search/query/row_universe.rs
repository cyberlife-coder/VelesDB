//! Which stored ids are rows — the one rule every SELECT scan, filter and
//! aggregate of a collection reads its rows by, and the one its size counts
//! (`collection/core/size.rs` counts [`RowUniverse`]'s ids).
//!
//! A point is stored as a vector, a payload, or both. Which of those make a
//! row depends on the kind of collection, read from its configuration alone
//! ([`RowUniverse::of`]) — never from what its stores happen to hold:
//!
//! - a **vector** collection's rows are its vectors: a payload-only id there
//!   is no row, however many vectors the collection holds — none included.
//!   Such a payload is an edge endpoint `store_node_payload` wrote, the
//!   residue of a torn delete, or one an earlier version let
//!   `upsert_metadata` store without a vector (it now refuses that id with
//!   `VELES-016`); it stays on disk, unreachable by every read — SELECT,
//!   `len`, `all_ids`, scroll, text and sparse search, MATCH — until a
//!   delete of its id removes it;
//! - a **metadata** collection stores no vector, so its rows are its payloads;
//! - a **graph**'s rows are its stored nodes — an embedding, a payload or
//!   both — the set `GraphCollection::len` counts and MATCH walks, so a
//!   payload-only node of a graph with embeddings is as much a row as an
//!   embedded one.
//!
//! A vector ranking reads fewer: a `NEAR` scores a row by its vector, so a
//! row without one cannot answer it ([`RowScope::Embedded`]) — the set the
//! HNSW index holds, which an exact scan ranking the same `NEAR` must agree
//! with. An aggregate whose WHERE holds a dense `NEAR` reads that set too
//! ([`RowScope::of_where`]), ranking none of it.
//!
//! The rows are hydrated as [`Collection::get`](crate::collection::types::Collection::get)
//! reads them: what the id stores, an empty `vector` where it stores none.

use crate::collection::CollectionConfig;
use crate::storage::{LogPayloadStorage, MmapStorage, PayloadStorage, VectorStorage};
use crate::velesql::Condition;

/// Whether `condition` holds a dense vector search (see
/// [`RowScope::of_where`]).
fn holds_dense_vector_search(condition: &Condition) -> bool {
    match condition {
        Condition::VectorSearch(_) | Condition::VectorFusedSearch(_) => true,
        Condition::And(left, right) | Condition::Or(left, right) => {
            holds_dense_vector_search(left) || holds_dense_vector_search(right)
        }
        Condition::Group(inner) | Condition::Not(inner) => holds_dense_vector_search(inner),
        _ => false,
    }
}

/// Which of a collection's rows a read asks for (see the module doc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowScope {
    /// Every row a SELECT reads.
    All,
    /// The rows a vector ranking can score: those storing a vector.
    Embedded,
}

impl RowScope {
    /// The rows a statement whose WHERE is `condition` reads: those storing
    /// a vector when it holds a dense vector search (`vector NEAR`,
    /// `NEAR_FUSED`), which only such rows can answer, else every row. An
    /// aggregate reads its rows by this rule: its `NEAR` ranks none of them
    /// and bounds none — it reads every row a `NEAR` could rank, whatever its
    /// `LIMIT` — so on a graph a node without an embedding is no row of it.
    /// A `SPARSE_NEAR` ranks rows by their sparse vectors, which a row
    /// without a dense one may carry, so it narrows nothing.
    pub(crate) fn of_where(condition: Option<&Condition>) -> Self {
        if condition.is_some_and(holds_dense_vector_search) {
            Self::Embedded
        } else {
            Self::All
        }
    }

    /// The ids this scope reads in a collection whose rows are `rows`
    /// ([`RowUniverse::of`]).
    pub(crate) const fn universe(self, rows: RowUniverse) -> RowUniverse {
        match self {
            Self::All => rows,
            Self::Embedded => RowUniverse::Vectors,
        }
    }
}

/// Which stored ids are a collection's rows (see the module doc): what a
/// SELECT reads, what its size counts, and which store a write of each kind
/// of collection reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowUniverse {
    /// Every id with a vector: a vector collection's rows.
    Vectors,
    /// Every id with a payload: a metadata collection's rows.
    Payloads,
    /// Every stored graph node: an id with a vector, a payload or both.
    StoredNodes,
}

impl RowUniverse {
    /// The rows of a collection configured as `config` — read before any
    /// storage guard is taken, config ranking first in the lock order.
    pub(crate) fn of(config: &CollectionConfig) -> Self {
        if config.graph_schema.is_some() {
            Self::StoredNodes
        } else if config.metadata_only {
            Self::Payloads
        } else {
            Self::Vectors
        }
    }

    /// Every row id, in storage order — ascending for a graph's nodes, whose
    /// two stores are merged.
    pub(crate) fn ids(
        self,
        vector_storage: &MmapStorage,
        payload_storage: &LogPayloadStorage,
    ) -> Vec<u64> {
        match self {
            Self::Vectors => vector_storage.ids(),
            Self::Payloads => payload_storage.ids(),
            Self::StoredNodes => {
                let mut ids = vector_storage.ids();
                ids.extend(payload_storage.ids());
                ids.sort_unstable();
                ids.dedup();
                ids
            }
        }
    }

    /// Whether `id` is a row, without listing the rows: index lookups, no
    /// stored vector or payload read.
    pub(crate) fn contains(
        self,
        vector_storage: &MmapStorage,
        payload_storage: &LogPayloadStorage,
        id: u64,
    ) -> bool {
        match self {
            Self::Vectors => vector_storage.contains(id),
            Self::Payloads => payload_storage.contains(id),
            Self::StoredNodes => vector_storage.contains(id) || payload_storage.contains(id),
        }
    }

    /// Whether an id storing a vector (`has_vector`) and a payload
    /// (`has_payload`) is a row — what a candidate list (`id IN (...)`
    /// literals, a bitmap superset) must pass before it becomes one.
    pub(crate) const fn admits(self, has_vector: bool, has_payload: bool) -> bool {
        match self {
            Self::Vectors => has_vector,
            Self::Payloads => has_payload,
            Self::StoredNodes => has_vector || has_payload,
        }
    }
}

#[cfg(test)]
#[path = "row_universe_tests.rs"]
mod tests;
