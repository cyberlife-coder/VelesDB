//! A collection's rows: which ids are rows, how a row is read, and how a
//! graph's row is written back — the one rule every SELECT scan, JOIN side
//! and DML statement reads and writes rows by.
//!
//! Which ids are rows is [`RowUniverse`]'s rule (a vector collection's
//! vectors, a metadata collection's payloads, a graph's stored nodes). A row
//! is hydrated with what its id stores ([`Collection::get`]): a graph node
//! without an embedding reads back an empty `vector`, as a metadata row
//! does. A row without a vector cannot answer a `NEAR`, so a vector ranking
//! reads only the rows that store one ([`RowScope::Embedded`](crate::collection::search::query::row_universe::RowScope)).
//!
//! A graph's row is written by what it stores: one with a vector through the
//! vector upsert (its dimension checked), one without through the node
//! payload write, which never touches the vector store — a zero-length vector
//! must not reach storage or the HNSW index of a graph without embeddings.
//! Both writes run the same payload effects (BM25, histograms:
//! `Collection::apply_payload_effects`), and both store the sparse vectors a
//! row carries, which are independent of its dense one. Every write entry
//! point of a graph — `upsert`, `upsert_bulk`, `upsert_bulk_from_raw`,
//! `upsert_metadata`, streaming (which drains through `upsert`) and
//! `AnyCollection::upsert` — goes through these rules. A metadata
//! collection's rows are written by the payload half of the same rule
//! ([`Collection::write_payload_rows`]), by `upsert` and `upsert_bulk` alike.

use crate::collection::search::query::row_universe::RowUniverse;
use crate::collection::types::Collection;
use crate::error::Result;
use crate::filter::Filter;
use crate::point::Point;
use crate::quantization::StorageMode;

use rustc_hash::FxHashSet;
use std::collections::HashMap;

impl Collection {
    /// Which ids are this collection's rows ([`RowUniverse::of`]). Reads
    /// `config` (rank 1): call it before taking a storage guard.
    pub(crate) fn row_universe(&self) -> RowUniverse {
        RowUniverse::of(&self.storage.config.read())
    }

    /// Every row id ([`RowUniverse`]), in storage order.
    ///
    /// LOCK ORDER: config(1), released, then vector_storage(2) before
    /// payload_storage(3).
    pub(crate) fn row_ids(&self) -> Vec<u64> {
        let rows = self.row_universe();
        let vector_storage = self.storage.vector_storage.read();
        let payload_storage = self.storage.payload_storage.read();
        rows.ids(&vector_storage, &payload_storage)
    }

    /// The rows a metadata `filter` keeps — every row without one — in id
    /// order where the candidates come from storage: the rows an `UPDATE`
    /// writes, a non-id `DELETE` removes and a JOIN side reads, the same rows
    /// a SELECT with that filter sees.
    ///
    /// The candidates are the ids the filter names (`id = n`, `id IN (...)`),
    /// else the secondary-index bitmap it resolves to — a superset of its
    /// matches, `None` for any predicate it cannot resolve completely — else
    /// every row id; each is then hydrated as a row and checked against the
    /// whole filter.
    pub(crate) fn rows_where(&self, filter: Option<&Filter>) -> Vec<Point> {
        let ids = filter
            .and_then(|f| self.filter_candidate_ids(f))
            .unwrap_or_else(|| self.row_ids());
        self.get(&ids)
            .into_iter()
            .flatten()
            .filter(|point| {
                filter.is_none_or(|f| f.matches_point(point.id, point.payload.as_ref()))
            })
            .collect()
    }

    /// The ids `filter` narrows its matches to without a scan, when it can:
    /// the ids it names, else its secondary-index bitmap.
    fn filter_candidate_ids(&self, filter: &Filter) -> Option<Vec<u64>> {
        filter.condition.point_ids().or_else(|| {
            self.build_prefilter_bitmap(filter)
                .map(|bitmap| bitmap.iter().map(u64::from).collect())
        })
    }

    /// Writes a graph's rows by what each stores (see the module doc): the
    /// rows with a vector through the vector pipeline, the rest through
    /// [`Self::write_payload_rows`]. A repeated id is folded first
    /// ([`split_graph_rows`]), and the observer hears of the batch once.
    ///
    /// The caller has checked the batch
    /// ([`validate_points_for_upsert`](Self::validate_points_for_upsert)).
    pub(super) fn upsert_graph_rows(
        &self,
        points: Vec<Point>,
        storage_mode: StorageMode,
    ) -> Result<()> {
        let count = points.len();
        let (embedded, payload_only) = split_graph_rows(points, AbsentFields::Replace);
        if !embedded.is_empty() {
            self.write_vector_rows(&embedded, storage_mode)?;
        }
        self.write_payload_rows(&payload_only, AbsentFields::Replace)?;
        self.report_upsert(count);
        Ok(())
    }

    /// [`Self::upsert_bulk`] on a graph, taking its rows by value: the
    /// graph's checks, then its rows written by what each stores — the rows
    /// with a vector through the bulk vector write, the rest through
    /// [`Self::write_payload_rows`] — under the bulk contract that a row
    /// without a payload or without sparse vectors leaves the stored ones.
    /// Returns how many rows it wrote, the count the observer hears: a row
    /// that leaves a stored node as it was is not one.
    pub(super) fn upsert_bulk_graph_rows(&self, points: Vec<Point>) -> Result<usize> {
        self.validate_graph_rows(&points)?;
        let (embedded, payload_only) = split_graph_rows(points, AbsentFields::Keep);
        let inserted = if embedded.is_empty() {
            0
        } else {
            self.write_bulk_vector_rows(&embedded)?
        };
        let count = inserted + self.write_payload_rows(&payload_only, AbsentFields::Keep)?;
        self.report_upsert(count);
        Ok(count)
    }

    /// Writes rows without a vector, each id once: their payloads through
    /// the node-payload write, which leaves the vector store — and an
    /// embedding the node already has — alone, and their sparse vectors as
    /// `absent` says: replaced, as an upsert replaces a point's, or added,
    /// as a bulk write adds them. A graph's payload-only rows and every row
    /// of a metadata collection are written by this rule, so the two kinds
    /// store a row the same way, sparse vectors included.
    ///
    /// A row carrying no payload keeps the stored one (`Keep`; under
    /// `Replace` every row carries one, [`AbsentFields::normalize`]); where
    /// its id stores no row it creates one with an empty payload, as an
    /// upsert of it would — so a row carrying only sparse vectors is a row.
    /// Returns how many rows wrote something: a payload, sparse vectors, or
    /// the row they created.
    pub(super) fn write_payload_rows(&self, rows: &[Point], absent: AbsentFields) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let created = self.write_row_payloads(rows)?;
        match absent {
            AbsentFields::Replace => self.apply_sparse_batch_upsert(rows)?,
            AbsentFields::Keep => {
                self.apply_sparse_batch_bulk(&Self::collect_sparse_batch(rows))?;
            }
        }
        Ok(rows
            .iter()
            .filter(|row| row.payload.is_some() || carries_sparse(row) || created.contains(&row.id))
            .count())
    }

    /// The payload half of [`Self::write_payload_rows`]: stores the rows'
    /// payloads — an empty one for each row without one whose id stores no
    /// row — and returns the ids it created. The payload mirror follows
    /// incrementally when every row carried its payload, and is dropped
    /// otherwise (a row that kept its stored payload is no empty row), after
    /// a failed write too: the records it left readable are new.
    fn write_row_payloads(&self, rows: &[Point]) -> Result<FxHashSet<u64>> {
        let entries: Vec<(u64, &serde_json::Value)> = rows
            .iter()
            .filter_map(|row| row.payload.as_ref().map(|payload| (row.id, payload)))
            .collect();
        let bare: Vec<u64> = rows
            .iter()
            .filter(|row| row.payload.is_none())
            .map(|row| row.id)
            .collect();
        let written = self.write_node_payloads(&entries, &bare);
        if written.is_ok() && bare.is_empty() {
            self.bump_generation_with_mirror_upserts(rows);
        } else {
            self.invalidate_caches_and_bump_generation();
        }
        Ok(written?.into_iter().collect())
    }
}

/// Whether `row` carries sparse vectors to store.
pub(super) fn carries_sparse(row: &Point) -> bool {
    row.sparse_vectors
        .as_ref()
        .is_some_and(|sparse| !sparse.is_empty())
}

/// What a graph row's absent payload and absent sparse vectors do to the
/// stored ones. An absent vector always keeps the stored one: an empty
/// vector is "no vector", and a node may carry none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AbsentFields {
    /// An upsert replaces the row: no payload deletes the stored one — a
    /// row without a vector then stores an empty one, the node it writes —
    /// and no sparse vectors drop the stored ones.
    Replace,
    /// A bulk write leaves what the row does not carry as it is.
    Keep,
}

impl AbsentFields {
    /// `point` as the node-payload write stores it under this rule: a row
    /// without a vector or a payload is an empty payload under `Replace`.
    fn normalize(self, mut point: Point) -> Point {
        if self == Self::Replace && point.is_metadata_only() && point.payload.is_none() {
            point.payload = Some(serde_json::Value::Object(serde_json::Map::new()));
        }
        point
    }

    /// `later`, a write of an id `earlier` already wrote in the same batch,
    /// completed with what it leaves of `earlier`: its vector when `later`
    /// carries none, and under `Keep` its payload and its sparse vectors
    /// (by name) where `later` carries none.
    fn carry(self, later: &mut Point, earlier: Point) {
        if later.is_metadata_only() {
            later.vector = earlier.vector;
        }
        if self == Self::Keep {
            if later.payload.is_none() {
                later.payload = earlier.payload;
            }
            later.sparse_vectors = match (earlier.sparse_vectors, later.sparse_vectors.take()) {
                (Some(mut kept), Some(written)) => {
                    kept.extend(written);
                    Some(kept)
                }
                (kept, written) => written.or(kept),
            };
        }
    }
}

/// A graph batch folded to one row per id, then split by what each row
/// stores, `(with a vector, without)`.
///
/// The fold leaves each id the row its writes, made one after the other,
/// leave: a later write overrides what it carries and keeps, through
/// [`AbsentFields::carry`], what it does not — a vector always, its payload
/// and sparse vectors under `Keep`. So a batch writes what the same writes
/// made one by one do, whichever store each occurrence of an id reaches,
/// and the two writes cannot disagree on which one wins.
pub(super) fn split_graph_rows(
    points: Vec<Point>,
    absent: AbsentFields,
) -> (Vec<Point>, Vec<Point>) {
    let mut rows: Vec<Point> = Vec::with_capacity(points.len());
    let mut at: HashMap<u64, usize> = HashMap::with_capacity(points.len());
    for point in points {
        let point = absent.normalize(point);
        if let Some(&i) = at.get(&point.id) {
            let earlier = std::mem::replace(&mut rows[i], point);
            absent.carry(&mut rows[i], earlier);
        } else {
            at.insert(point.id, rows.len());
            rows.push(point);
        }
    }
    rows.into_iter().partition(|row| !row.is_metadata_only())
}
