//! Per-collection columnar mirror of payload scalars.
//!
//! Wires the `ColumnStore` typed/bitmap filter engine into the production
//! `SELECT ... WHERE` path. The mirror stores every top-level scalar payload
//! field in columnar form (numbers as `f64`, strings interned, bools) and
//! answers metadata filters with `RoaringBitmap` scans instead of per-row
//! JSON parsing.
//!
//! # Adaptive build
//!
//! The mirror is **not** built eagerly: building costs one full payload scan,
//! which would penalize collections that never run scan-heavy filters. Each
//! full sequential JSON scan records its row count as *scan debt*; once the
//! accumulated debt exceeds one full-scan-equivalent, the next metadata query
//! builds the mirror and subsequent filters run columnar. Limit-k queries
//! that early-exit after a few rows never accumulate enough debt to trigger
//! a build, so they keep their fast path.
//!
//! # Consistency model
//!
//! The invariant is *mirror present ⇒ in sync with payload storage*:
//!
//! - The main mutation paths (`upsert`, `upsert_metadata`, `upsert_bulk`,
//!   `delete`) maintain the mirror incrementally via
//!   `bump_generation_with_mirror_upserts` / `..._deletes`.
//! - Every other mutation path funnels through
//!   `invalidate_caches_and_bump_generation`, which drops the mirror, so a
//!   future write path can never serve stale columnar data — at worst it
//!   costs a lazy rebuild.
//!
//! False positives from the bitmap are removed by the JSON post-filter
//! (`scan_ids_with_filter`); the translation layer (`translate`) is designed
//! so false negatives are impossible (strict type-match eligibility).

use crate::column_store::{AutoVacuumConfig, ColumnStore, ColumnType, ColumnValue, VacuumConfig};
use crate::point::Point;
use crate::storage::{PayloadStorage, VectorStorage};
use parking_lot::RwLock;
use roaring::RoaringBitmap;
use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::atomic::{AtomicU64, Ordering};

mod translate;

#[cfg(test)]
mod mirror_tests;
#[cfg(test)]
mod translate_tests;

/// Collections below this size never build a mirror — sequential scans on
/// tiny collections are already microsecond-scale.
pub(crate) const MIRROR_MIN_ROWS: usize = 256;

/// Maximum number of payload fields mirrored as columns. Fields beyond the
/// cap are tracked in `uncolumnized` and fall back to the JSON filter.
const MAX_MIRROR_COLUMNS: usize = 64;

/// Tombstone ratio above which the mirror is dropped and rebuilt lazily
/// (compaction by reconstruction).
const BLOAT_MIN_ROWS: usize = 4096;

/// Columnar mirror handle owned by a `Collection`.
///
/// Lock order position: **1b** — the lazy build holds the state write lock
/// while acquiring `vector_storage` (2) and `payload_storage` (3) read locks,
/// in ascending order. Mutation hooks and queries acquire the state lock with
/// no other collection lock held.
#[derive(Default)]
pub(crate) struct PayloadMirror {
    state: RwLock<Option<MirrorState>>,
    scan_debt: AtomicU64,
}

/// Outcome of a mirror probe for a filter condition.
pub(crate) enum MirrorAnswer {
    /// Candidate point ids (superset of matches; caller must post-filter).
    Ids(Vec<u64>),
    /// The condition cannot be answered from columnar data — fall back.
    Unsupported,
    /// The mirror has not been built (or was invalidated).
    NotBuilt,
}

/// Built mirror: a `ColumnStore` plus point-id ↔ row-index mappings.
#[derive(Default)]
pub(super) struct MirrorState {
    pub(super) store: ColumnStore,
    /// row index → point id (append-only, parallel to column length).
    pub(super) row_ids: Vec<u64>,
    /// point id → live row index.
    pub(super) id_rows: FxHashMap<u64, u32>,
    /// Live (non-tombstoned) row indices; complement base for NOT / `!=`.
    pub(super) live: RoaringBitmap,
    /// Scalar fields seen but not mirrored (column cap reached) — conditions
    /// on these fields must fall back to the JSON filter.
    pub(super) uncolumnized: FxHashSet<String>,
}

/// A payload cell pre-converted outside the mirror lock. Strings stay
/// borrowed: interning requires the store's string table, which lives under
/// the state lock.
enum PreparedCell<'p> {
    Float(f64),
    Bool(bool),
    Str(&'p str),
}

impl PreparedCell<'_> {
    /// The mirror column type this cell lands in.
    fn column_type(&self) -> ColumnType {
        match self {
            Self::Float(_) => ColumnType::Float,
            Self::Bool(_) => ColumnType::Bool,
            Self::Str(_) => ColumnType::String,
        }
    }
}

/// A payload row pre-converted outside the mirror lock: top-level scalar
/// fields only, keys borrowed from the payload.
type PreparedRow<'p> = Vec<(&'p str, PreparedCell<'p>)>;

/// Extracts mirrorable cells from a payload's top-level scalar fields.
///
/// Pure conversion — needs no shared state, so upsert batches run it outside
/// the mirror write lock. Non-scalar values (arrays, objects, nulls) and
/// dotted keys produce no cell — `push_row_unchecked` stores null for absent
/// columns, matching the JSON filter's "missing field never matches"
/// semantics. All numbers map to `Float` because the JSON filter compares
/// numbers as `f64` (`values_equal` / `compare_values`), making the `f64`
/// mirror exactly as faithful as the JSON path itself.
fn prepare_row(payload: Option<&serde_json::Value>) -> PreparedRow<'_> {
    let Some(serde_json::Value::Object(map)) = payload else {
        return Vec::new();
    };
    let mut cells = Vec::with_capacity(map.len());
    for (key, value) in map {
        // `get_field` splits on '.', so dotted keys are unreachable by
        // the JSON filter — never mirror them.
        if key.contains('.') {
            continue;
        }
        let cell = match value {
            serde_json::Value::Number(n) => match n.as_f64() {
                Some(f) => PreparedCell::Float(f),
                None => continue,
            },
            serde_json::Value::String(s) => PreparedCell::Str(s),
            serde_json::Value::Bool(b) => PreparedCell::Bool(*b),
            _ => continue,
        };
        cells.push((key.as_str(), cell));
    }
    cells
}

impl MirrorState {
    /// Tombstones the previous row for `id` (if any) and appends a new row.
    ///
    /// Returns `false` when the row index space (`u32`) is exhausted, which
    /// poisons the mirror (caller drops the state).
    pub(super) fn upsert_row(&mut self, id: u64, payload: Option<&serde_json::Value>) -> bool {
        self.upsert_prepared_row(id, &prepare_row(payload))
    }

    /// Lock-side half of an upsert: resolves prepared cells against the
    /// shared state (column creation, string interning) and appends the row.
    ///
    /// Returns `false` when the row index space (`u32`) is exhausted, which
    /// poisons the mirror (caller drops the state). Cells whose type
    /// conflicts with the existing column are nulled by `push_typed`.
    fn upsert_prepared_row(&mut self, id: u64, row: &PreparedRow<'_>) -> bool {
        self.tombstone(id);
        let Ok(row_idx) = u32::try_from(self.store.row_count()) else {
            return false;
        };
        // Keys are borrowed straight from the prepared row so the cells can
        // go to `push_row_unchecked` without `String` clones.
        let mut cells: Vec<(&str, ColumnValue)> = Vec::with_capacity(row.len());
        for (key, cell) in row {
            if !self.ensure_column(key, &cell.column_type()) {
                continue;
            }
            let value = match cell {
                PreparedCell::Float(f) => ColumnValue::Float(*f),
                PreparedCell::Bool(b) => ColumnValue::Bool(*b),
                PreparedCell::Str(s) => {
                    ColumnValue::String(self.store.string_table_mut().intern(s))
                }
            };
            cells.push((key, value));
        }
        self.store.push_row_unchecked(&cells);
        self.row_ids.push(id);
        self.id_rows.insert(id, row_idx);
        self.live.insert(row_idx);
        true
    }

    /// Tombstones the row for `id`, if present.
    pub(super) fn tombstone(&mut self, id: u64) {
        if let Some(row_idx) = self.id_rows.remove(&id) {
            self.store.tombstone_row(row_idx as usize);
            self.live.remove(row_idx);
        }
    }

    /// Ensures a column exists for `key`, honoring the column cap.
    ///
    /// Returns `true` when the field is mirrored (column exists or was
    /// created); `false` when capped out (field recorded as uncolumnized).
    fn ensure_column(&mut self, key: &str, col_type: &ColumnType) -> bool {
        if self.store.get_column(key).is_some() {
            return true;
        }
        if self.uncolumnized.contains(key) {
            return false;
        }
        if self.store.column_names().count() >= MAX_MIRROR_COLUMNS {
            self.uncolumnized.insert(key.to_string());
            return false;
        }
        self.store.add_column_backfilled(key, col_type);
        true
    }

    /// Whether tombstones dominate the store (time to compact via rebuild).
    fn is_bloated(&self) -> bool {
        let total = self.store.row_count();
        total > BLOAT_MIN_ROWS && self.live.len().saturating_mul(2) < total as u64
    }

    /// PostgreSQL-inspired auto-vacuum: compacts the store in place when the
    /// tombstone ratio crosses the [`AutoVacuumConfig`] threshold.
    ///
    /// Runs under the mirror state write lock (position 1b) with no other
    /// collection lock held; `ColumnStore::vacuum` is a pure in-memory pass,
    /// so there is no lock-ordering or reentrance hazard.
    fn auto_vacuum_if_due(&mut self, config: &AutoVacuumConfig) {
        if config.should_trigger(self.store.row_count(), self.store.deleted_row_count()) {
            self.vacuum_compact();
        }
    }

    /// Vacuums the store and rebuilds `row_ids` / `id_rows` / `live` against
    /// the compacted row indices. The vacuum keeps surviving rows in
    /// ascending old-index order, which matches ascending iteration over the
    /// pre-vacuum `live` bitmap — so enumeration yields the new dense index.
    fn vacuum_compact(&mut self) {
        self.store.vacuum(VacuumConfig::default());
        let old_row_ids = std::mem::take(&mut self.row_ids);
        let old_live = std::mem::take(&mut self.live);
        self.id_rows.clear();
        for (new_idx, old_idx) in old_live.iter().enumerate() {
            let (Some(&id), Ok(idx32)) =
                (old_row_ids.get(old_idx as usize), u32::try_from(new_idx))
            else {
                // Unreachable: `live` ⊆ `0..row_count == old_row_ids.len()`
                // and the compacted index space only shrinks.
                break;
            };
            self.row_ids.push(id);
            self.id_rows.insert(id, idx32);
            self.live.insert(idx32);
        }
    }
}

impl PayloadMirror {
    /// Drops the mirror; it will be rebuilt lazily when scan debt warrants it.
    pub(crate) fn invalidate(&self) {
        *self.state.write() = None;
    }

    /// Records rows visited by a full sequential JSON scan.
    pub(crate) fn add_scan_debt(&self, rows: u64) {
        self.scan_debt.fetch_add(rows, Ordering::Relaxed);
    }

    pub(crate) fn scan_debt(&self) -> u64 {
        self.scan_debt.load(Ordering::Relaxed)
    }

    /// Whether the mirror is currently built — the cheap not-built fast path
    /// for the mutation hooks.
    ///
    /// Racing against a concurrent lazy build is safe: the build holds the
    /// state write lock across its whole storage snapshot, and mutation paths
    /// write storage *before* calling the hooks — so this check either blocks
    /// until the build finishes (then the hook applies its batch, idempotent),
    /// or returns `false` before the build starts, in which case the build's
    /// snapshot already contains the batch.
    fn is_built(&self) -> bool {
        self.state.read().is_some()
    }

    /// Applies an upsert batch incrementally (no-op when not built).
    ///
    /// Intra-batch duplicate ids resolve to last-writer-wins because each
    /// `upsert_prepared_row` tombstones the previous row for the same id.
    ///
    /// INVARIANT: the whole batch is applied under one state write-lock hold,
    /// so concurrent filtered queries (`candidate_ids`) never observe a
    /// half-applied batch — do not split this into per-point locking. Only
    /// row *preparation* (payload → scalar cells) runs outside the lock;
    /// string interning and store insertion need the shared state and stay
    /// under it.
    pub(crate) fn apply_upserts(&self, points: &[Point]) {
        if !self.is_built() {
            return;
        }
        let prepared: Vec<(u64, PreparedRow<'_>)> = points
            .iter()
            .map(|point| (point.id, prepare_row(point.payload.as_ref())))
            .collect();
        let mut guard = self.state.write();
        let Some(state) = guard.as_mut() else {
            return; // invalidated between the fast-path check and the lock
        };
        let mut healthy = true;
        for (id, row) in &prepared {
            if !state.upsert_prepared_row(*id, row) {
                healthy = false;
                break;
            }
        }
        if !healthy || state.is_bloated() {
            *guard = None;
        }
    }

    /// Applies a delete batch incrementally (no-op when not built), then
    /// evaluates the auto-vacuum trigger so tombstone bloat is compacted on
    /// the delete path (PostgreSQL-inspired: 20% dead-row ratio, ≥ 50 dead).
    ///
    /// INVARIANT: same batch atomicity as [`Self::apply_upserts`] — one write
    /// lock for the whole batch, never per-id locking.
    pub(crate) fn apply_deletes(&self, ids: &[u64]) {
        if !self.is_built() {
            return;
        }
        let mut guard = self.state.write();
        if let Some(state) = guard.as_mut() {
            for &id in ids {
                state.tombstone(id);
            }
            state.auto_vacuum_if_due(&AutoVacuumConfig::default());
        }
    }

    /// Translates a filter condition to candidate point ids via columnar scans.
    pub(crate) fn candidate_ids(&self, condition: &crate::filter::Condition) -> MirrorAnswer {
        let guard = self.state.read();
        let Some(state) = guard.as_ref() else {
            return MirrorAnswer::NotBuilt;
        };
        match translate::condition_bitmap(state, condition) {
            Some(eval) => MirrorAnswer::Ids(
                eval.bits
                    .iter()
                    .filter_map(|row_idx| state.row_ids.get(row_idx as usize).copied())
                    .collect(),
            ),
            None => MirrorAnswer::Unsupported,
        }
    }
}

impl crate::collection::types::Collection {
    /// Probes the payload mirror for candidate ids, building it first when
    /// enough full-scan debt has accumulated.
    ///
    /// Returns `None` when the condition is unsupported or the mirror is not
    /// (yet) worth building — the caller falls back to the JSON scan path.
    pub(crate) fn mirror_candidate_ids(
        &self,
        condition: &crate::filter::Condition,
    ) -> Option<Vec<u64>> {
        match self.payload_mirror.candidate_ids(condition) {
            MirrorAnswer::Ids(ids) => return Some(ids),
            MirrorAnswer::Unsupported => return None,
            MirrorAnswer::NotBuilt => {}
        }
        if !self.mirror_build_due() {
            return None;
        }
        self.build_payload_mirror();
        match self.payload_mirror.candidate_ids(condition) {
            MirrorAnswer::Ids(ids) => Some(ids),
            MirrorAnswer::Unsupported | MirrorAnswer::NotBuilt => None,
        }
    }

    /// Whether accumulated scan debt justifies the one-off build cost.
    fn mirror_build_due(&self) -> bool {
        let rows = self.config.read().point_count;
        rows >= MIRROR_MIN_ROWS && self.payload_mirror.scan_debt() >= rows as u64
    }

    /// Builds the mirror from storage under the mirror write lock.
    ///
    /// LOCK ORDER: `payload_mirror` (1b) → `vector_storage` (2) →
    /// `payload_storage` (3), ascending. Mutation hooks acquire the mirror
    /// lock with no other collection lock held, so concurrent writers block
    /// here during the build and re-apply their batch afterwards (idempotent),
    /// keeping the mirror complete.
    pub(crate) fn build_payload_mirror(&self) {
        let mut guard = self.payload_mirror.state.write();
        if guard.is_some() {
            return; // another query won the build race
        }
        let vector_ids = {
            let vectors = self.vector_storage.read();
            vectors.ids()
        };
        let payload_storage = self.payload_storage.read();
        let mut state = MirrorState::default();
        let mut seen: FxHashSet<u64> = FxHashSet::default();
        for id in vector_ids.into_iter().chain(payload_storage.ids()) {
            if !seen.insert(id) {
                continue;
            }
            let payload = payload_storage.retrieve(id).ok().flatten();
            if !state.upsert_row(id, payload.as_ref()) {
                return; // u32 row space exhausted — leave mirror unbuilt
            }
        }
        drop(payload_storage);
        self.payload_mirror.scan_debt.store(0, Ordering::Relaxed);
        *guard = Some(state);
    }
}
