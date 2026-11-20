//! Upsert operations for Collection.
//!
//! Read and delete operations are in `crud_read_delete.rs`.
//! Bulk-specific methods (`upsert_bulk`, `upsert_bulk_from_raw`) are in `crud_bulk.rs`.
//! Quantization caching helpers, secondary-index update helpers,
//! `DedupMap`, and the PQ cache guard are in `crud_helpers.rs`.

use super::crud_helpers::DedupMap;
use super::rows::{split_graph_rows, AbsentFields};
use super::size::{RowStores, StoreEffect};
use crate::collection::search::query::row_universe::RowUniverse;
use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::Point;
use crate::quantization::StorageMode;
use crate::storage::{LogPayloadStorage, PayloadStorage, VectorStorage};
use crate::velesql::QueryVectorTarget;

use std::collections::HashMap;

impl Collection {
    /// Inserts or updates points in the collection.
    ///
    /// Accepts any iterator of points (Vec, slice, array, etc.)
    ///
    /// A point with an empty vector stores none. In a graph that is a node
    /// without an embedding, written as [`Self::store_node_payloads`] writes
    /// one — the vector store is left alone, so an embedding the node already
    /// has is kept — while a point with a vector goes through the vector
    /// pipeline (`rows.rs`, which also resolves a repeated id as the same
    /// upserts made one by one would). A vector collection refuses it, every
    /// row of it storing a vector (`VELES-016`); a metadata-only collection
    /// stores only payloads.
    ///
    /// # Errors
    ///
    /// Returns an error if any point has a mismatched dimension, carries no
    /// vector on a vector collection
    /// ([`Error::VectorRequired`](crate::Error::VectorRequired)), or carries
    /// one into a metadata-only collection.
    pub fn upsert(&self, points: impl IntoIterator<Item = Point>) -> Result<()> {
        let points: Vec<Point> = points.into_iter().collect();
        // Parity item E + dimension validation at the cold boundary, before any
        // storage lock or WAL write — a violation rejects the whole batch.
        self.validate_points_for_upsert(&points)?;
        let (rows, storage_mode) = {
            let config = self.storage.config.read();
            (RowUniverse::of(&config), config.storage_mode)
        };
        match rows {
            // `upsert_metadata` is the storage entry for this path and applies
            // the runtime limits itself.
            RowUniverse::Payloads => self.upsert_metadata(points),
            RowUniverse::StoredNodes => self.upsert_graph_rows(points, storage_mode),
            RowUniverse::Vectors => {
                self.write_vector_rows(&points, storage_mode)?;
                self.report_upsert(points.len());
                Ok(())
            }
        }
    }

    /// The vector pipeline of [`Self::upsert`] for a checked batch whose
    /// points all store a vector; the caller reports the write.
    pub(super) fn write_vector_rows(
        &self,
        points: &[Point],
        storage_mode: StorageMode,
    ) -> Result<()> {
        self.upsert_storage_and_index(points, storage_mode)?;
        self.apply_sparse_batch_upsert(points)?;
        self.bump_generation_with_mirror_upserts(points);
        Ok(())
    }

    /// The checks [`Self::upsert`] runs on its points before any write, by
    /// the kind of collection ([`RowUniverse::of`]): a metadata-only
    /// collection refuses a vector
    /// ([`Error::VectorNotAllowed`](crate::Error::VectorNotAllowed),
    /// [`Self::validate_payload_rows`]); a vector collection runs the
    /// runtime ingest limits and the per-point dimension and finite checks.
    /// Streaming
    /// runs them too, when a point is queued — the only moment its caller can
    /// still hear of a refusal, since the drain writes it later — and refuses
    /// with [`BackpressureError::InvalidPoint`](crate::collection::streaming::BackpressureError::InvalidPoint);
    /// a surface calls this first to report the refusal with core's code.
    /// A graph checks each point by what it stores
    /// ([`Self::validate_graph_rows`]).
    ///
    /// # Errors
    ///
    /// The refusal of the first point that fails a check.
    pub(crate) fn validate_points_for_upsert(&self, points: &[Point]) -> Result<()> {
        match self.row_universe() {
            RowUniverse::Payloads => self.validate_payload_rows(points),
            RowUniverse::StoredNodes => self.validate_graph_rows(points),
            RowUniverse::Vectors => self.validate_vector_upsert_batch(points),
        }
    }

    /// The checks a metadata collection's rows pass, on every write path: a
    /// point with an empty vector is a payload row, and one carrying a vector
    /// is refused ([`QueryVectorTarget::check_write`]: `VELES-014`).
    pub(super) fn validate_payload_rows(&self, points: &[Point]) -> Result<()> {
        self.with_write_target(|target| {
            points
                .iter()
                .filter(|point| !point.is_metadata_only())
                .try_for_each(|point| target.check_write(point.dimension()))
        })
    }

    /// Runs `check` against what a vector written into this collection must
    /// match ([`QueryVectorTarget::check_write`], the rule every write
    /// surface applies, DML included): its name and the dimension of the
    /// vectors it stores, `None` when it stores none — taken from an owned
    /// snapshot of the config, so the checks run without its lock.
    fn with_write_target(
        &self,
        check: impl FnOnce(&QueryVectorTarget<'_>) -> Result<()>,
    ) -> Result<()> {
        let (name, dimension) = {
            let config = self.storage.config.read();
            (config.name.clone(), config.query_vector_target().dimension)
        };
        check(&QueryVectorTarget {
            collection: &name,
            dimension,
        })
    }

    /// The checks a graph batch passes. Every row's payload passes those of a
    /// node-payload write (payload limits, labels against a strict schema),
    /// whether the row carries a vector or not, and every row's sparse
    /// vectors their name and finite checks; a row with a vector also passes
    /// the vector checks of a vector upsert — the vector cap, counted over
    /// these rows only, the write rule (a graph without embeddings refuses
    /// it with `VELES-014`, one with embeddings checks its dimension) and
    /// finite components.
    pub(super) fn validate_graph_rows(&self, points: &[Point]) -> Result<()> {
        let empty = serde_json::Value::Object(serde_json::Map::new());
        let payloads: Vec<(u64, &serde_json::Value)> = points
            .iter()
            .map(|point| (point.id, point.payload.as_ref().unwrap_or(&empty)))
            .collect();
        self.validate_node_payload_batch(&payloads)?;
        let embedded = points.iter().filter(|point| !point.is_metadata_only());
        self.enforce_vector_count(
            embedded.clone().count(),
            self.runtime_limits().max_vectors_per_collection,
        )?;
        self.with_write_target(|target| {
            points.iter().try_for_each(|point| {
                if point.is_metadata_only() {
                    Self::validate_sparse_vectors(point)
                } else {
                    Self::validate_vector_point(point, target)
                }
            })
        })
    }

    /// Validates a vector-collection upsert batch at the cold boundary:
    /// runtime ingest limits (parity item E) followed by the per-point
    /// dimension check. Shared by [`Self::upsert`] and [`Self::upsert_bulk`]
    /// so both ingest paths apply the identical pre-storage validation.
    ///
    /// Sparse index names are checked here too: they are request-controlled
    /// (`Point::sparse_vectors` map keys) and later reach the filesystem as
    /// `sparse-{name}.*` path components, so a hostile name must be refused
    /// before it can enter the `sparse_indexes` map or a WAL path (issue
    /// #2593 — the guard `sparse_file_prefix` documents as its own).
    ///
    /// # Errors
    ///
    /// Returns [`Error::GuardRail`](crate::Error::GuardRail) on a limit
    /// violation, or [`QueryVectorTarget::check_required_write`]'s refusal —
    /// [`Error::VectorRequired`](crate::Error::VectorRequired) for a point
    /// without a vector, every row of a vector collection storing one, or
    /// the dimension mismatch error — or
    /// [`Error::SchemaValidation`](crate::Error::SchemaValidation) on an
    /// invalid sparse index name, or
    /// [`Error::InvalidVector`](crate::Error::InvalidVector) on a non-finite
    /// dense component or sparse weight.
    pub(super) fn validate_vector_upsert_batch(&self, points: &[Point]) -> Result<()> {
        self.enforce_upsert_limits(points)?;
        self.with_write_target(|target| {
            points.iter().try_for_each(|point| {
                target.check_required_write(point.dimension())?;
                Self::validate_point_contents(point)
            })
        })
    }

    /// The per-point checks of a graph row carrying a vector: the write rule
    /// ([`QueryVectorTarget::check_write`]: a collection that stores vectors
    /// checks the dimension), then [`Self::validate_point_contents`].
    fn validate_vector_point(point: &Point, target: &QueryVectorTarget<'_>) -> Result<()> {
        target.check_write(point.dimension())?;
        Self::validate_point_contents(point)
    }

    /// What a vector upsert checks once the vector's length passed: finite
    /// dense components and [`Self::validate_sparse_vectors`].
    fn validate_point_contents(point: &Point) -> Result<()> {
        crate::validation::validate_vector_is_finite(&point.vector)?;
        Self::validate_sparse_vectors(point)
    }

    /// Valid sparse index names (they reach the filesystem, see
    /// [`Self::validate_vector_upsert_batch`]) and finite sparse weights.
    pub(super) fn validate_sparse_vectors(point: &Point) -> Result<()> {
        if let Some(sv_map) = &point.sparse_vectors {
            for (name, vector) in sv_map {
                crate::validation::validate_sparse_index_name(name)?;
                crate::validation::validate_sparse_vector_is_finite(vector)?;
            }
        }
        Ok(())
    }

    /// Stores vectors, payloads, and indexes for a batch of points.
    ///
    /// Three-phase pipeline to minimize lock contention and I/O:
    /// 1. Batch storage, by the rule every row write follows
    ///    ([`Self::write_rows`]): the payloads, then the vectors (one fsync
    ///    each), then the BM25 change — on a vector collection all three
    ///    under the node locks of the batch's ids, so a delete of one of them
    ///    lands wholly before or after
    /// 2. The histograms ([`Self::apply_payload_histograms`]), then per-point
    ///    updates: secondary indexes, quantization
    /// 3. Batch HNSW insert via `bulk_index_or_defer()`
    ///
    /// # Crash Recovery
    ///
    /// A crash between Phase 1 and Phase 3 leaves vectors durably stored but
    /// absent from the HNSW index. On the next `Collection::open()`, gap
    /// detection compares storage IDs against HNSW mappings and re-indexes
    /// any missing vectors. The recovery window is bounded by one batch.
    ///
    /// Sparse vectors are not phase work: `apply_sparse_batch_upsert`
    /// replaces them afterwards.
    fn upsert_storage_and_index(&self, points: &[Point], storage_mode: StorageMode) -> Result<()> {
        let ids: Vec<u64> = points.iter().map(|p| p.id).collect();
        let writes = payload_writes(points);
        // Phase 1: the stores and the BM25 change, under the row write's
        // locks. The payloads are stored and flushed before the BM25 change,
        // so the BM25 WAL never leads their durability.
        let (old_payloads, text_changed) =
            self.write_rows(self.row_universe(), &ids, |stores| -> Result<_> {
                let old_payloads = self.batch_store_all(stores, points)?;
                let text_changed = self.apply_payload_text(&writes, &old_payloads, |a, r| {
                    stores.change_text_index(a, r)
                });
                Ok((old_payloads, text_changed))
            })?;

        // Phase 2 (no lock held): the histograms describe the stored
        // payloads, whatever BM25 did.
        self.apply_payload_histograms(&writes, &old_payloads);
        text_changed?;
        self.per_point_updates(points, &old_payloads, storage_mode);

        // Phase 3: Batch HNSW insert
        let vector_refs: Vec<(u64, &[f32])> =
            points.iter().map(|p| (p.id, p.vector.as_slice())).collect();
        self.bulk_index_or_defer(&vector_refs);
        Ok(())
    }

    /// Phase 1: Batch-stores payloads, then vectors, through `stores`.
    ///
    /// Pre-collects old payloads (needed for secondary index updates),
    /// then writes all payloads and vectors in single batch calls (1 fsync each).
    ///
    /// Deduplicates intra-batch duplicate IDs using last-writer-wins semantics:
    /// only the final occurrence per ID is written to the WAL, avoiding wasteful
    /// intermediate entries that would bloat the log and slow replay.
    ///
    /// After this method returns, vectors and payloads are durable on disk.
    /// A crash before Phase 3 (HNSW insertion) is recovered by gap detection
    /// on the next `Collection::open()`.
    ///
    /// # Sequential I/O (was Parallel I/O via `rayon::join`, Issue #424 — reverted)
    ///
    /// Payload and vector writes run **sequentially**. They previously ran
    /// concurrently via `rayon::join` (Issue #424); that was reverted because
    /// it deadlocked the whole process under concurrent multi-caller load —
    /// see `tests/tokio_blocking_lock_contention_repro.rs` for the isolated
    /// reproduction and mechanism.
    ///
    /// `rayon::join`, called here from a foreign thread (i.e. a
    /// `tokio::task::spawn_blocking` worker, not a rayon pool thread —
    /// exactly how every `velesdb-memory` MCP tool call reaches this code:
    /// `crates/velesdb-memory/src/mcp.rs:172,199`), dispatches the second
    /// closure onto rayon's small **global** thread pool (sized to
    /// `num_cpus`), a pool shared process-wide with unrelated parallel work
    /// (HNSW bulk indexing, quantization). `parking_lot::RwLock::write()` is
    /// a genuine OS-level blocking wait — not cooperative with rayon's
    /// scheduler — so once enough concurrent callers (e.g. 20+ concurrent
    /// `remember`s hitting the same collection) each occupy a rayon worker
    /// blocked on this method's `RwLock`, the global pool can be fully
    /// exhausted by jobs that are themselves waiting on each other's
    /// progress, wedging not just this collection but every consumer of the
    /// shared rayon pool in the process — severely enough in the
    /// investigation's reproduction to also stall `tokio`'s own timer wheel,
    /// defeating an external `tokio::time::timeout` guard.
    ///
    /// The two writes are independent (payload and vector storage use
    /// separate `RwLock`s — positions 3 and 2 in the lock order) and each is
    /// a single small write + fsync, so the parallelism this bought was
    /// marginal against the deadlock risk it introduced. Sequential
    /// execution removes the shared-pool interaction entirely and cannot
    /// deadlock this way.
    ///
    /// - Neither store takes both write guards. On a vector collection the
    ///   row write holds the node locks of the batch's ids across both
    ///   ([`Self::write_rows`]); on a graph a store that may change its nodes
    ///   takes their node locks itself, so it can count the nodes it changes
    ///   (`size.rs`).
    /// - The payload store comes first: a crash between the two leaves a
    ///   payload without a vector, no row, which gap detection and the next
    ///   write of the id resolve.
    /// - The payload write holds the payload lock from reading the old
    ///   payloads to updating the label and range indexes, and releases it
    ///   before the vector write.
    ///
    /// Returns the old payloads for Phase 2.
    fn batch_store_all(
        &self,
        stores: &RowStores<'_>,
        points: &[Point],
    ) -> Result<Vec<Option<serde_json::Value>>> {
        // Issue #425: Build the dedup map once and share it across both
        // write paths, avoiding redundant HashMap construction.
        let dedup_map = Self::build_dedup_map(points);

        // Sequential: see the "Sequential I/O" doc section above for why this
        // is no longer `rayon::join` (deadlock under concurrent load).
        let old_payloads = self.write_and_flush_payloads(stores, points, &dedup_map)?;
        #[cfg(test)]
        super::write_hooks::between_row_stores();
        Self::write_deduped_vectors(stores, points, &dedup_map)?;

        Ok(old_payloads)
    }

    /// Writes deduped payloads, flushes the storage, and brings the label and
    /// range indexes in line with what it stored; returns the old payloads.
    ///
    /// Called sequentially from [`Self::batch_store_all`] (previously via
    /// `rayon::join`, reverted — see that method's doc comment). One
    /// `payload_storage` write guard spans the old-payload read, the write and
    /// the index update, so a concurrent writer cannot store a payload for
    /// these ids in between: the indexes end up describing what storage holds.
    /// A payload-less point deletes the stored payload here, and the indexes
    /// follow — on an error too: a failed write or flush can leave part or all
    /// of the batch readable, and the indexes are then rebuilt for these ids
    /// from what storage holds before the error is returned.
    ///
    /// Issue #425: Accepts a pre-computed `dedup_map` to avoid rebuilding
    /// the last-writer-wins map redundantly.
    ///
    /// The write moves the collection's size by what it changed
    /// ([`RowStores::payloads`]).
    ///
    /// LOCK ORDER: as [`Self::write_payload_store`]: payload_storage(3)
    /// → label_index(7), released → graph_range_indexes(7).
    fn write_and_flush_payloads(
        &self,
        stores: &RowStores<'_>,
        points: &[Point],
        dedup_map: &DedupMap,
    ) -> Result<Vec<Option<serde_json::Value>>> {
        let ids: Vec<u64> = points.iter().map(|p| p.id).collect();
        // A point without a payload deletes the stored one.
        let effect = if points.iter().all(|p| p.payload.is_some()) {
            StoreEffect::Stores
        } else {
            StoreEffect::StoresOrRemoves
        };
        stores.payloads(&ids, effect, |payload_storage| {
            let old_payloads = Self::collect_old_payloads(points, payload_storage);
            let written = Self::write_deduped_payloads(points, payload_storage, dedup_map)
                .and_then(|()| payload_storage.flush().map_err(Into::into));
            if let Err(error) = written {
                self.index_payloads_after_failed_write(payload_storage, &ids, &old_payloads);
                return Err(error);
            }
            self.index_payload_writes(
                points.iter().map(|p| (p.id, p.payload.as_ref())),
                &old_payloads,
            );
            Ok(old_payloads)
        })
    }

    /// Retrieves pre-batch payloads, querying storage only once per unique ID.
    ///
    /// For intra-batch duplicates, only the first occurrence needs the pre-batch
    /// value; subsequent occurrences are handled by `seen_payloads` in Phase 2.
    pub(crate) fn collect_old_payloads(
        points: &[Point],
        storage: &LogPayloadStorage,
    ) -> Vec<Option<serde_json::Value>> {
        Self::collect_old_payloads_by_id(points.iter().map(|p| p.id), storage)
    }

    /// [`Self::collect_old_payloads`] over the ids of a batch, in order.
    pub(super) fn collect_old_payloads_by_id(
        ids: impl ExactSizeIterator<Item = u64>,
        storage: &LogPayloadStorage,
    ) -> Vec<Option<serde_json::Value>> {
        let mut seen = std::collections::HashSet::with_capacity(ids.len());
        ids.map(|id| {
            if seen.insert(id) {
                // First occurrence — retrieve pre-batch payload from storage
                storage.retrieve(id).ok().flatten()
            } else {
                None // Duplicate — Phase 2 uses seen_payloads instead
            }
        })
        .collect()
    }

    /// Builds a last-writer-wins dedup map: `point_id -> index_of_last_occurrence`.
    ///
    /// Issue #425: Computed once in `batch_store_all` and shared by both
    /// `write_deduped_payloads` and `write_deduped_vectors` to avoid
    /// redundant `HashMap` construction.
    pub(super) fn build_dedup_map(points: &[Point]) -> DedupMap {
        let mut map = HashMap::with_capacity(points.len());
        for (i, p) in points.iter().enumerate() {
            map.insert(p.id, i);
        }
        map
    }

    /// Writes only the last payload per ID to the WAL, then deletes IDs whose
    /// final occurrence has `payload=None`.
    ///
    /// Issue #425: Accepts a pre-computed `dedup_map` instead of building
    /// its own, consolidating the two redundant maps into one.
    fn write_deduped_payloads(
        points: &[Point],
        storage: &mut LogPayloadStorage,
        dedup_map: &DedupMap,
    ) -> Result<()> {
        // Only write the final payload per ID (skip intermediate duplicates)
        let deduped: Vec<(u64, &serde_json::Value)> = points
            .iter()
            .enumerate()
            .filter(|&(i, p)| dedup_map.get(&p.id) == Some(&i) && p.payload.is_some())
            .filter_map(|(_, p)| p.payload.as_ref().map(|pl| (p.id, pl)))
            .collect();
        // Deferred: the caller (`write_and_flush_payloads`) ends with
        // `flush()`, which applies the configured durability barrier once for
        // stores and tombstones together — `store_batch` + per-id `delete`
        // here would fsync 1 + N_DELETED times for the same guarantee.
        storage.store_batch_deferred(&deduped)?;

        // Delete IDs whose final occurrence has payload=None — one batched
        // tombstone append instead of one barrier per id.
        let final_deletes: Vec<u64> = points
            .iter()
            .enumerate()
            .filter(|&(i, p)| dedup_map.get(&p.id) == Some(&i) && p.payload.is_none())
            .map(|(_, p)| p.id)
            .collect();
        storage.delete_batch(&final_deletes)?;
        Ok(())
    }

    /// Writes only the last vector per ID to vector storage, through
    /// `stores` ([`RowStores::vectors`]).
    ///
    /// Issue #425: Accepts a pre-computed `dedup_map` instead of building
    /// its own, consolidating the two redundant maps into one.
    fn write_deduped_vectors(
        stores: &RowStores<'_>,
        points: &[Point],
        dedup_map: &DedupMap,
    ) -> Result<()> {
        let deduped: Vec<(u64, &[f32])> = points
            .iter()
            .enumerate()
            .filter(|&(i, p)| dedup_map.get(&p.id) == Some(&i))
            .map(|(_, p)| (p.id, p.vector.as_slice()))
            .collect();

        let ids: Vec<u64> = deduped.iter().map(|&(id, _)| id).collect();
        stores.vectors(&ids, |vector_storage| {
            vector_storage.store_batch(&deduped)?;
            vector_storage.flush()?;
            Ok(())
        })
    }

    /// Returns `true` when the per-point loop can be skipped entirely.
    ///
    /// Issue #425: For the common case (`StorageMode::Full`, no secondary
    /// indexes), the loop does zero useful work. Skipping avoids PQ cache
    /// guard acquisition, `seen_payloads` HashMap allocation, and the
    /// per-point loop.
    fn can_skip_phase2(&self, storage_mode: StorageMode) -> bool {
        // PQ is the only mode with per-upsert quantization caching; every
        // other mode (Full, RaBitQ, SQ8, Binary) does none here. Secondary
        // indexes require per-point old/new payload diffing.
        !matches!(storage_mode, StorageMode::ProductQuantization)
            && self.query.secondary_indexes.read().is_empty()
    }

    /// Phase 2: Per-point updates that don't need storage write locks.
    ///
    /// Tracks the effective "old payload" per ID to handle within-batch
    /// duplicates correctly: when id=5 appears twice, the second occurrence
    /// sees the first occurrence's payload as its "old" (not the pre-batch
    /// original), ensuring secondary indexes stay consistent.
    ///
    /// Issue #425: Fast-path skips the entire loop when no secondary
    /// processing is needed (see `can_skip_phase2`).
    ///
    /// Issue #486: For SQ8/Binary modes with rayon, quantization runs in
    /// parallel before the main loop, avoiding the per-point lock overhead.
    fn per_point_updates(
        &self,
        points: &[Point],
        old_payloads: &[Option<serde_json::Value>],
        storage_mode: StorageMode,
    ) {
        // Issue #425: Fast-path — skip the loop entirely when no secondary
        // processing is needed. Avoids lock acquisition, HashMap allocation,
        // and the per-point loop for the common StorageMode::Full case.
        if self.can_skip_phase2(storage_mode) {
            return;
        }

        // PQ is the only mode with per-upsert quantization work; its quantizer
        // carries shared lazy-training state, so points run sequentially under
        // one cache guard.
        let mut pq_guard = super::crud_helpers::pq_cache_guard(self, storage_mode);

        self.per_point_sequential_updates(points, old_payloads, &mut pq_guard);
    }

    /// Runs the sequential per-point loop for secondary indexes and (when
    /// not pre-computed) quantization. The label and range indexes are not
    /// phase-2 work: `write_and_flush_payloads` updates them under the
    /// payload lock; nor are BM25 and the histograms, which
    /// [`Self::apply_payload_effects`] updates for the batch.
    ///
    /// Extracted from `per_point_updates` to keep each function under 50 NLOC.
    fn per_point_sequential_updates(
        &self,
        points: &[Point],
        old_payloads: &[Option<serde_json::Value>],
        pq_guard: &mut Option<super::crud_helpers::PqCacheGuard<'_>>,
    ) {
        let mut seen_payloads: HashMap<u64, Option<&serde_json::Value>> =
            HashMap::with_capacity(points.len());
        for (point, pre_batch_old) in points.iter().zip(old_payloads) {
            let effective_old =
                Self::resolve_effective_old(&seen_payloads, point.id, pre_batch_old.as_ref());
            Self::maybe_quantize(self, point, pq_guard);
            self.update_secondary_indexes_on_upsert(
                point.id,
                effective_old,
                point.payload.as_ref(),
            );
            seen_payloads.insert(point.id, point.payload.as_ref());
        }
    }

    /// Inserts or updates rows by their payloads, by the kind of collection
    /// ([`RowUniverse::of`]):
    ///
    /// - a **metadata** collection writes its rows as [`Self::upsert`] does
    ///   — by the graph rule for rows without a vector (`rows.rs`): a vector
    ///   is refused (`VELES-014`), a row without a payload stores an empty
    ///   one, and its sparse vectors replace the stored ones;
    /// - a **graph** writes them by its rule of [`Self::upsert`]: its checks,
    ///   and a node's embedding kept by a payload write;
    /// - a **vector** collection's rows are its vectors, so this replaces
    ///   the payloads of points it already stores — a payload-less point
    ///   deletes the stored payload, as `upsert` does — and refuses the whole
    ///   batch, before writing anything, when one id stores no vector: a
    ///   payload there would be no row of it. It writes payloads only, so a
    ///   point carrying a dense vector is refused too, the whole batch before
    ///   anything is written — `upsert` writes a vector. Sparse vectors are
    ///   not written here.
    ///
    /// # Errors
    ///
    /// [`Error::VectorRequired`](crate::Error::VectorRequired) on a vector
    /// collection when an id stores no vector, and
    /// [`Error::VectorNotAllowed`](crate::Error::VectorNotAllowed) when a
    /// point carries a dense vector; the runtime limits'
    /// refusals; a vector or an invalid sparse vector on a metadata
    /// collection; a storage failure; or the refusal of [`Self::upsert`] on
    /// a graph.
    pub fn upsert_metadata(&self, points: impl IntoIterator<Item = Point>) -> Result<()> {
        let points: Vec<Point> = points.into_iter().collect();
        match self.row_universe() {
            RowUniverse::StoredNodes => self.upsert(points),
            RowUniverse::Payloads => self.upsert_payload_rows(points),
            RowUniverse::Vectors => self.upsert_vector_row_payloads(&points),
        }
    }

    /// [`Self::upsert_metadata`] on a metadata collection — the storage entry
    /// of its `upsert`: the checks its rows run on every write path (a
    /// vector refused, [`Self::validate_payload_rows`]; their sparse vectors
    /// and the runtime limits), then the rows written by the graph rule
    /// under `upsert`'s contract ([`AbsentFields::Replace`]), the observer
    /// told once.
    fn upsert_payload_rows(&self, points: Vec<Point>) -> Result<()> {
        self.validate_payload_rows(&points)?;
        self.validate_payload_row_contents(&points)?;
        let count = points.len();
        let (embedded, rows) = split_graph_rows(points, AbsentFields::Replace);
        debug_assert!(embedded.is_empty(), "a metadata row stores no vector");
        self.write_payload_rows(&rows, AbsentFields::Replace)?;
        self.report_upsert(count);
        Ok(())
    }

    /// What a metadata collection's rows pass beyond refusing a vector, on
    /// both its write paths: their sparse vectors' checks — the names reach
    /// the filesystem ([`Self::validate_vector_upsert_batch`]) — and the
    /// runtime limits.
    pub(super) fn validate_payload_row_contents(&self, points: &[Point]) -> Result<()> {
        points.iter().try_for_each(Self::validate_sparse_vectors)?;
        self.enforce_upsert_limits(points)
    }

    /// [`Self::upsert_metadata`] on a vector collection: replaces the
    /// payloads of points that store a vector, after refusing a batch that
    /// carries one (it writes none) and the runtime limits.
    /// The payload effects (histograms, BM25) follow the stored payloads
    /// once the guards are released, as on the vector upsert path.
    fn upsert_vector_row_payloads(&self, points: &[Point]) -> Result<()> {
        let name = self.storage.config.read().name.clone();
        if points.iter().any(|point| !point.vector.is_empty()) {
            return Err(crate::Error::VectorNotAllowed(name));
        }
        self.enforce_upsert_limits(points)?;
        let old_payloads = self.store_metadata_rows(&name, points)?;
        self.apply_payload_effects(&payload_writes(points), &old_payloads)?;
        self.bump_generation_with_mirror_upserts(points);
        self.report_upsert(points.len());
        Ok(())
    }

    /// Stores the payloads of a batch of the rows of the vector collection
    /// `name` and brings the label and range indexes in line with them;
    /// returns each point's pre-batch payload (`collect_old_payloads`), for
    /// the histograms. A point without a payload deletes the stored one. The
    /// whole batch is refused, before anything is written, when one id
    /// stores no vector ([`Self::write_payloads_of_vector_rows`]).
    ///
    /// LOCK ORDER: node_locks(1c) → payload_storage(3) → label_index(7),
    /// released → graph_range_indexes(7); the payload flush runs under
    /// `payload_storage`.
    fn store_metadata_rows(
        &self,
        name: &str,
        points: &[Point],
    ) -> Result<Vec<Option<serde_json::Value>>> {
        let ids: Vec<u64> = points.iter().map(|p| p.id).collect();
        self.write_payloads_of_vector_rows(name, &ids, |payload_storage| {
            let mut label_idx = self.graph.label_index.write();
            // Collect old payloads for histogram decrements before they are
            // overwritten. Bug #46: use collect_old_payloads to deduplicate by
            // ID — only the first occurrence retrieves the pre-batch value;
            // duplicates get None so the old value is decremented exactly once.
            let old_payloads_for_hist = Self::collect_old_payloads(points, payload_storage);
            let effective_olds =
                self.apply_metadata_point_writes(points, payload_storage, &mut label_idx)?;
            // label_index(7) released before graph_range_indexes(7), its rank-mate.
            drop(label_idx);
            self.apply_range_index_changes(
                points
                    .iter()
                    .zip(&effective_olds)
                    .map(|(point, old)| (point.id, old.as_ref(), point.payload.as_ref())),
            );
            payload_storage.flush()?;
            Ok(old_payloads_for_hist)
        })
    }

    /// Applies the per-point payload store/delete, secondary-index and
    /// label-index updates for [`Self::store_metadata_rows`].
    ///
    /// Extracted from [`Self::upsert_metadata`] so each function stays within
    /// the complexity budget. Both write guards are passed in by the caller,
    /// which holds them across the whole batch in lock order (3 → 7).
    ///
    /// Returns the effective old payload of each point, in order, for the
    /// caller's graph range index update once `label_idx` is released.
    fn apply_metadata_point_writes(
        &self,
        points: &[Point],
        payload_storage: &mut LogPayloadStorage,
        label_idx: &mut crate::collection::graph::LabelIndex,
    ) -> Result<Vec<Option<serde_json::Value>>> {
        // Pass 1: resolve the old payload each occurrence would have seen in
        // the sequential loop. Intra-batch duplicates observe the previous
        // occurrence's payload (via `seen`), not the pre-batch value, exactly
        // as the per-point store-then-retrieve interleaving behaved.
        let mut seen: HashMap<u64, Option<serde_json::Value>> =
            HashMap::with_capacity(points.len());
        let mut effective_olds: Vec<Option<serde_json::Value>> = Vec::with_capacity(points.len());
        for point in points {
            let old_payload = match seen.get(&point.id) {
                Some(prev) => prev.clone(),
                None => payload_storage.retrieve(point.id).ok().flatten(),
            };
            effective_olds.push(old_payload);
            seen.insert(point.id, point.payload.clone());
        }

        // Pass 2: payload writes, last-writer-wins deduped, buffered under
        // the caller's single `flush()` barrier — the per-point `store` +
        // `delete` calls each carried their own fsync (#1797 shape) for the
        // same end state and the same durability point (return of
        // `upsert_metadata`).
        let dedup_map = Self::build_dedup_map(points);
        Self::write_deduped_payloads(points, payload_storage, &dedup_map)?;

        // Pass 3: in-memory index maintenance, per occurrence, in order.
        for (point, old_payload) in points.iter().zip(&effective_olds) {
            self.update_secondary_indexes_on_upsert(
                point.id,
                old_payload.as_ref(),
                point.payload.as_ref(),
            );

            // Maintain label index for _labels-bearing payloads.
            if let Some(ref old) = old_payload {
                label_idx.remove_from_payload(point.id, old);
            }
            if let Some(ref payload) = point.payload {
                label_idx.index_from_payload(point.id, payload);
            }
        }
        Ok(effective_olds)
    }
}

/// Each point's `(id, payload)` write, in batch order: `None` where the
/// point carries no payload, which deletes the stored one.
pub(super) fn payload_writes(points: &[Point]) -> Vec<(u64, Option<&serde_json::Value>)> {
    points.iter().map(|p| (p.id, p.payload.as_ref())).collect()
}
