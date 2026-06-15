//! Index management methods for Collection (EPIC-009 propagation).

use crate::collection::types::Collection;
use crate::error::Result;
use crate::index::{JsonValue, SecondaryIndex};
use parking_lot::RwLock;
use std::collections::BTreeMap;

/// Index information response for API.
#[derive(Debug, Clone)]
pub struct IndexInfo {
    /// Node label.
    pub label: String,
    /// Property name.
    pub property: String,
    /// Index type (hash or range).
    pub index_type: String,
    /// Number of unique values indexed.
    pub cardinality: usize,
    /// Memory usage in bytes.
    pub memory_bytes: usize,
}

impl Collection {
    /// Creates a secondary metadata index for a payload field.
    ///
    /// When the index already exists, triggers a backfill to ensure all
    /// existing payloads are indexed (handles the case where bulk insert
    /// skipped per-point index updates).
    ///
    /// # Errors
    ///
    /// Returns Ok(()) on success. Index creation is idempotent.
    #[allow(clippy::unnecessary_wraps)] // Reason: Public API contract — callers expect Result
    pub fn create_index(&self, field_name: &str) -> Result<()> {
        let mut indexes = self.secondary_indexes.write();
        let is_new = !indexes.contains_key(field_name);
        indexes
            .entry(field_name.to_string())
            .or_insert_with(|| SecondaryIndex::BTree(RwLock::new(BTreeMap::new())));

        // Backfill: scan existing payloads to populate the index.
        // Runs for both new indexes AND existing indexes (to catch points
        // inserted via bulk paths that skipped per-point index updates).
        drop(indexes); // Release write lock before reading payloads
        self.backfill_secondary_index(field_name, is_new);

        Ok(())
    }

    /// Scans existing payloads and populates the secondary index for `field_name`.
    ///
    /// Runs for both new and existing indexes to catch points inserted via
    /// bulk paths that skipped per-point index updates.
    fn backfill_secondary_index(&self, field_name: &str, is_new: bool) {
        use crate::storage::PayloadStorage;

        let payload_storage = self.payload_storage.read();
        let ids = PayloadStorage::ids(&*payload_storage);
        let indexes = self.secondary_indexes.read();
        let Some(index) = indexes.get(field_name) else {
            return;
        };
        let SecondaryIndex::BTree(ref tree) = index;
        let mut tree_guard = tree.write();
        for id in ids {
            Self::backfill_single_payload(&*payload_storage, id, field_name, &mut tree_guard);
        }
        // Deduplicate each bucket in one O(k log k) pass rather than checking
        // contains() per-insertion (was O(k) per insert → O(k²) total for a
        // bucket of k IDs, e.g. low-cardinality fields like status/category).
        for ids_vec in tree_guard.values_mut() {
            ids_vec.sort_unstable();
            ids_vec.dedup();
        }
        if !is_new {
            tracing::debug!(
                field = field_name,
                "create_index: backfilled existing index (bulk insert recovery)"
            );
        }
    }

    /// Indexes a single payload entry for the given field, if present.
    ///
    /// Callers are responsible for deduplication (see `backfill_secondary_index`).
    fn backfill_single_payload(
        payload_storage: &dyn crate::storage::PayloadStorage,
        id: u64,
        field_name: &str,
        tree_guard: &mut std::collections::BTreeMap<JsonValue, Vec<u64>>,
    ) {
        if let Ok(Some(payload)) = payload_storage.retrieve(id) {
            if let Some(val) = payload.get(field_name) {
                if let Some(key) = JsonValue::from_json(val) {
                    tree_guard.entry(key).or_default().push(id);
                }
            }
        }
    }

    /// Drops a secondary metadata index for a payload field.
    ///
    /// Returns `true` if the index existed and was removed, `false` if no
    /// such index existed.
    #[must_use]
    pub fn drop_secondary_index(&self, field_name: &str) -> bool {
        self.secondary_indexes.write().remove(field_name).is_some()
    }

    /// Checks whether a secondary metadata index exists for a field.
    #[must_use]
    pub fn has_secondary_index(&self, field_name: &str) -> bool {
        self.secondary_indexes.read().contains_key(field_name)
    }

    /// Recommends secondary indexes for scalar `ORDER BY <field>` queries
    /// (EPIC-081 phase 3a, recommendation-only).
    ///
    /// Returns one [`OrderByIndexSuggestion`] per field that drove at least
    /// `min_observations` eligible `ORDER BY <field>` queries down the
    /// exhaustive path (sorted by descending observation count, then field
    /// name). The state is derived from the **live** index, so a field whose
    /// index now fully covers the collection is *resolved* (the fast path fires)
    /// and is dropped from the advice. Remaining fields carry [`OrderByIndexState`]:
    /// `Missing` (no secondary index — `CREATE INDEX (<field>)` would enable the
    /// `O(log n + k)` fast path) or `BuiltButUncovered` (an index exists but does
    /// not fully cover the collection, so the gap is the data, not a missing
    /// index). Observation counts are cumulative since the collection was opened
    /// and do not decay.
    ///
    /// Observation-only: this never creates, drops, or mutates an index.
    ///
    /// [`OrderByIndexSuggestion`]: crate::collection::order_by_advisor::OrderByIndexSuggestion
    /// [`OrderByIndexState`]: crate::collection::order_by_advisor::OrderByIndexState
    #[must_use]
    pub(crate) fn order_by_index_advice(
        &self,
        min_observations: u64,
    ) -> Vec<crate::collection::order_by_advisor::OrderByIndexSuggestion> {
        use crate::collection::order_by_advisor::OrderByIndexSuggestion;
        // Snapshot under the advisor lock, release it before touching the
        // secondary-index lock so only one lock is ever held at a time.
        let observed = self.order_by_advisor.read().observed(min_observations);
        observed
            .into_iter()
            .filter_map(|(field, observed_count)| {
                self.order_by_index_state(&field)
                    .map(|state| OrderByIndexSuggestion {
                        field,
                        observed_count,
                        state,
                    })
            })
            .collect()
    }

    /// Live advice state for `field`, derived under one secondary-index read
    /// lock: `Missing` when no index exists, `BuiltButUncovered` when an index
    /// exists but does not fully cover the collection, or `None` when an index
    /// exists *and* fully covers it — in which case the ordered-index fast path
    /// already serves the field, so there is nothing to advise.
    fn order_by_index_state(
        &self,
        field: &str,
    ) -> Option<crate::collection::order_by_advisor::OrderByIndexState> {
        use crate::collection::order_by_advisor::OrderByIndexState;
        let point_count = self.len();
        let indexes = self.secondary_indexes.read();
        match indexes.get(field) {
            None => Some(OrderByIndexState::Missing),
            Some(index)
                if index
                    .ordered_ids_if_covered(false, 0, point_count)
                    .is_some() =>
            {
                None
            }
            Some(_) => Some(OrderByIndexState::BuiltButUncovered),
        }
    }

    /// Returns the set of payload field names covered by a secondary index
    /// (issue #607).
    ///
    /// Threaded into `QueryPlan::from_query_with_stats` via
    /// `Database::build_plan_with_stats` so `IndexLookup` plan nodes are
    /// generated when an `EXPLAIN` target references an indexed column.
    /// Returns an empty set for collections with no indexes registered.
    #[must_use]
    pub fn indexed_field_names(&self) -> std::collections::HashSet<String> {
        self.secondary_indexes.read().keys().cloned().collect()
    }

    /// Returns the top `limit` point IDs for `field_name` in index order
    /// (ascending, or descending when `descending`), **only when the index
    /// fully covers** the collection (every point carries the field). Returns
    /// `None` when no such index exists or coverage is incomplete.
    ///
    /// Backs the index-backed `ORDER BY <field> LIMIT k` fast path
    /// (EPIC-081 phase 2): the returned IDs are a snapshot, so the secondary
    /// index lock is released before the caller hydrates them via `get`.
    #[must_use]
    pub(crate) fn ordered_ids_if_covered(
        &self,
        field_name: &str,
        descending: bool,
        limit: usize,
    ) -> Option<Vec<u64>> {
        let point_count = self.len();
        let indexes = self.secondary_indexes.read();
        indexes
            .get(field_name)?
            .ordered_ids_if_covered(descending, limit, point_count)
    }

    /// Looks up matching point IDs for an indexed field value.
    #[must_use]
    pub fn secondary_index_lookup(&self, field_name: &str, value: &JsonValue) -> Option<Vec<u64>> {
        let indexes = self.secondary_indexes.read();
        let index = indexes.get(field_name)?;
        match index {
            SecondaryIndex::BTree(tree) => tree.read().get(value).cloned(),
        }
    }

    /// Builds a pre-filter bitmap from a [`Filter`] using secondary indexes.
    ///
    /// Supports `Eq`, `Neq` (universe subtraction), `Gt`/`Gte`/`Lt`/`Lte`
    /// (range scan), `And` (intersection), and `Or` (union, only when all
    /// children resolve). Returns `None` when the condition cannot be resolved
    /// via indexes (e.g., `Not`, non-indexed fields), signalling the caller to
    /// fall back to post-filter.
    #[must_use]
    pub(crate) fn build_prefilter_bitmap(
        &self,
        filter: &crate::filter::Filter,
    ) -> Option<roaring::RoaringBitmap> {
        Self::bitmap_from_condition(&self.secondary_indexes, &filter.condition)
    }

    /// Recursively extracts bitmaps from conditions backed by secondary indexes.
    ///
    /// Supported conditions:
    /// - `Eq`: exact-match lookup
    /// - `Neq`: universe bitmap minus exact-match (all indexed IDs except matches)
    /// - `Gt`, `Gte`, `Lt`, `Lte`: range scan via `BTreeMap::range()`
    /// - `In`: union of per-value B-tree lookups
    /// - `Not { In }`: universe bitmap minus IN bitmap (set complement)
    /// - `And`: intersection of child bitmaps
    /// - `Or`: union of child bitmaps (all children must resolve)
    ///
    /// Returns `None` for `Not` wrapping non-`In` conditions and unsupported conditions.
    fn bitmap_from_condition(
        indexes: &std::sync::Arc<
            parking_lot::RwLock<std::collections::HashMap<String, SecondaryIndex>>,
        >,
        cond: &crate::filter::Condition,
    ) -> Option<roaring::RoaringBitmap> {
        match cond {
            crate::filter::Condition::Eq { field, value } => {
                Self::bitmap_for_eq_field(indexes, field, value)
            }
            crate::filter::Condition::Neq { .. } => {
                // NEQ cannot be safely pre-filtered from this index. A `universe
                // - eq` complement would be built from `all_ids_bitmap`, which
                // only contains points that HAVE a primitive value for the field
                // — but NEQ semantics also match points where the field is
                // ABSENT (see `Condition::Neq` in filter::matching, which is true
                // for a missing field). Those points are not in the universe, so
                // the complement is a strict subset and a bitmap-only caller
                // (e.g. the JOIN pre-filter) would drop them. Return `None` to
                // force a correct full-scan + post-filter.
                None
            }
            crate::filter::Condition::Gt { field, value }
            | crate::filter::Condition::Gte { field, value }
            | crate::filter::Condition::Lt { field, value }
            | crate::filter::Condition::Lte { field, value } => {
                Self::bitmap_for_range_field(indexes, field, value, cond)
            }
            crate::filter::Condition::In { field, values } => {
                Self::bitmap_for_in_field(indexes, field, values)
            }
            crate::filter::Condition::Not { condition } => {
                Self::bitmap_for_not_in(indexes, condition)
            }
            crate::filter::Condition::And { conditions } => {
                Self::bitmap_from_and(indexes, conditions)
            }
            crate::filter::Condition::Or { conditions } => {
                Self::bitmap_from_or(indexes, conditions)
            }
            _ => None,
        }
    }

    /// Looks up a single equality field in the secondary indexes.
    fn bitmap_for_eq_field(
        indexes: &std::sync::Arc<
            parking_lot::RwLock<std::collections::HashMap<String, SecondaryIndex>>,
        >,
        field: &str,
        value: &serde_json::Value,
    ) -> Option<roaring::RoaringBitmap> {
        let key = JsonValue::from_json(value)?;
        let guard = indexes.read();
        let index = guard.get(field)?;
        // `None` here means an indexed ID exceeded u32::MAX (incomplete bitmap);
        // propagate it so the caller falls back to a full scan. An empty bitmap
        // is a valid "no matches" pre-filter and is returned as-is.
        index.to_bitmap(&key)
    }

    /// Builds a bitmap for `IN(field, values)` by unioning per-value B-tree lookups.
    ///
    /// Acquires the secondary index read-lock once and iterates all values under
    /// the same guard. Values that don't convert to [`JsonValue`] or don't exist
    /// in the index are silently skipped (contribute empty bitmap).
    ///
    /// Time complexity: O(N × log K) where N = `values.len()`, K = index keys.
    /// Space: O(|result|) — single accumulator bitmap, no intermediate allocations.
    fn bitmap_for_in_field(
        indexes: &std::sync::Arc<
            parking_lot::RwLock<std::collections::HashMap<String, SecondaryIndex>>,
        >,
        field: &str,
        values: &[serde_json::Value],
    ) -> Option<roaring::RoaringBitmap> {
        if values.is_empty() {
            return Some(roaring::RoaringBitmap::new());
        }
        let guard = indexes.read();
        let index = guard.get(field)?;
        let mut acc = roaring::RoaringBitmap::new();
        for v in values {
            if let Some(key) = JsonValue::from_json(v) {
                // Propagate `None` (id > u32::MAX) so the whole IN falls back to scan.
                acc |= index.to_bitmap(&key)?;
            }
        }
        Some(acc)
    }

    /// `Not` conditions cannot be safely pre-filtered from this index.
    ///
    /// Like NEQ, a `universe - in` complement is built from `all_ids_bitmap`,
    /// which omits points whose field is absent — yet `NOT IN` matches those
    /// absent-field points. The complement would be a strict subset, so a
    /// bitmap-only caller (e.g. the JOIN pre-filter) would drop real matches.
    /// Always return `None` to force a correct full-scan + post-filter.
    #[allow(clippy::unnecessary_wraps)] // Reason: uniform Option return for bitmap_from_condition dispatch
    fn bitmap_for_not_in(
        _indexes: &std::sync::Arc<
            parking_lot::RwLock<std::collections::HashMap<String, SecondaryIndex>>,
        >,
        _inner: &crate::filter::Condition,
    ) -> Option<roaring::RoaringBitmap> {
        None
    }

    /// Builds a range bitmap for Gt/Gte/Lt/Lte using `SecondaryIndex::range_bitmap`.
    fn bitmap_for_range_field(
        indexes: &std::sync::Arc<
            parking_lot::RwLock<std::collections::HashMap<String, SecondaryIndex>>,
        >,
        field: &str,
        value: &serde_json::Value,
        cond: &crate::filter::Condition,
    ) -> Option<roaring::RoaringBitmap> {
        use std::ops::Bound;

        let key = JsonValue::from_json(value)?;
        let guard = indexes.read();
        let index = guard.get(field)?;
        let (from, to) = match cond {
            crate::filter::Condition::Gt { .. } => (Bound::Excluded(&key), Bound::Unbounded),
            crate::filter::Condition::Gte { .. } => (Bound::Included(&key), Bound::Unbounded),
            crate::filter::Condition::Lt { .. } => (Bound::Unbounded, Bound::Excluded(&key)),
            crate::filter::Condition::Lte { .. } => (Bound::Unbounded, Bound::Included(&key)),
            _ => return None,
        };
        index.range_bitmap(from, to)
    }

    /// Intersects bitmaps from AND-ed conditions.
    fn bitmap_from_and(
        indexes: &std::sync::Arc<
            parking_lot::RwLock<std::collections::HashMap<String, SecondaryIndex>>,
        >,
        conditions: &[crate::filter::Condition],
    ) -> Option<roaring::RoaringBitmap> {
        let mut result: Option<roaring::RoaringBitmap> = None;
        for cond in conditions {
            if let Some(bm) = Self::bitmap_from_condition(indexes, cond) {
                result = Some(match result {
                    Some(existing) => existing & &bm,
                    None => bm,
                });
            }
        }
        result
    }

    /// Unions bitmaps from OR-ed conditions.
    ///
    /// If ANY child returns `None` (cannot be pre-filtered), the entire OR
    /// must return `None` because the union would be incomplete -- the
    /// post-filter must evaluate the full OR instead.
    fn bitmap_from_or(
        indexes: &std::sync::Arc<
            parking_lot::RwLock<std::collections::HashMap<String, SecondaryIndex>>,
        >,
        conditions: &[crate::filter::Condition],
    ) -> Option<roaring::RoaringBitmap> {
        let mut result = roaring::RoaringBitmap::new();
        for cond in conditions {
            let bm = Self::bitmap_from_condition(indexes, cond)?;
            result |= bm;
        }
        Some(result)
    }

    /// Create a property index for O(1) equality lookups.
    ///
    /// # Arguments
    ///
    /// * `label` - Node label to index (e.g., "Person")
    /// * `property` - Property name to index (e.g., "email")
    ///
    /// # Errors
    ///
    /// Returns Ok(()) on success. Index creation is idempotent.
    #[allow(clippy::unnecessary_wraps)] // Reason: Public API contract — callers expect Result
    pub fn create_property_index(&self, label: &str, property: &str) -> Result<()> {
        let mut index = self.property_index.write();
        index.create_index(label, property);
        Ok(())
    }

    /// Create a range index for O(log n) range queries.
    ///
    /// # Arguments
    ///
    /// * `label` - Node label to index (e.g., "Event")
    /// * `property` - Property name to index (e.g., "timestamp")
    ///
    /// # Errors
    ///
    /// Returns Ok(()) on success. Index creation is idempotent.
    #[allow(clippy::unnecessary_wraps)] // Reason: Public API contract — callers expect Result
    pub fn create_range_index(&self, label: &str, property: &str) -> Result<()> {
        let mut index = self.range_index.write();
        index.create_index(label, property);
        Ok(())
    }

    /// Check if a property index exists.
    #[must_use]
    pub fn has_property_index(&self, label: &str, property: &str) -> bool {
        self.property_index.read().has_index(label, property)
    }

    /// Check if a range index exists.
    #[must_use]
    pub fn has_range_index(&self, label: &str, property: &str) -> bool {
        self.range_index.read().has_index(label, property)
    }

    /// List all indexes on this collection.
    #[must_use]
    pub fn list_indexes(&self) -> Vec<IndexInfo> {
        let mut indexes = Vec::new();

        // Secondary indexes (metadata field indexes created via create_index)
        let sec_indexes = self.secondary_indexes.read();
        for (field, index) in sec_indexes.iter() {
            let cardinality = match index {
                crate::index::SecondaryIndex::BTree(tree) => tree.read().len(),
            };
            indexes.push(IndexInfo {
                label: "secondary".to_string(),
                property: field.clone(),
                index_type: "hash".to_string(),
                cardinality,
                memory_bytes: 0,
            });
        }
        drop(sec_indexes);

        // LOCK ORDER: property_index(7) read — then range_index(7) read.
        // Same level, reads-only; canonical order prevents deadlock.
        let prop_index = self.property_index.read();
        for (label, property) in prop_index.indexed_properties() {
            let cardinality = prop_index.cardinality(&label, &property).unwrap_or(0);
            indexes.push(IndexInfo {
                label,
                property,
                index_type: "hash".to_string(),
                cardinality,
                memory_bytes: 0,
            });
        }

        // List range indexes
        let range_idx = self.range_index.read();
        for (label, property) in range_idx.indexed_properties() {
            indexes.push(IndexInfo {
                label,
                property,
                index_type: "range".to_string(),
                cardinality: 0,
                memory_bytes: 0,
            });
        }

        indexes
    }

    /// Drop an index (either property or range).
    ///
    /// # Arguments
    ///
    /// * `label` - Node label
    /// * `property` - Property name
    ///
    /// # Returns
    ///
    /// Ok(true) if an index was dropped, Ok(false) if no index existed.
    ///
    /// # Errors
    ///
    /// Returns an error if underlying index stores fail while dropping.
    #[allow(clippy::unnecessary_wraps)] // Reason: Public API contract — callers expect Result
    pub fn drop_index(&self, label: &str, property: &str) -> Result<bool> {
        // Try property index first
        let dropped_prop = self.property_index.write().drop_index(label, property);
        if dropped_prop {
            return Ok(true);
        }

        // Try range index
        let dropped_range = self.range_index.write().drop_index(label, property);
        Ok(dropped_range)
    }

    /// Get total memory usage of all indexes.
    #[must_use]
    pub fn indexes_memory_usage(&self) -> usize {
        // LOCK ORDER: property_index(7) read — then range_index(7) read.
        // Same level, reads-only; canonical order prevents deadlock.
        let prop_mem = self.property_index.read().memory_usage();
        let range_mem = self.range_index.read().memory_usage();
        prop_mem + range_mem
    }

    /// Reorders HNSW graph nodes in BFS traversal order for improved cache locality.
    ///
    /// After bulk insertion, nodes are stored in insertion order. Calling this
    /// method reorders both the vector buffer and all adjacency lists so nodes
    /// that are close in the graph are also close in memory, reducing L2/L3
    /// cache misses during search traversal by 15–30% on indices ≥ 1 000 vectors.
    ///
    /// Also builds a PDX block-columnar layout for SIMD-parallel distance
    /// computation (see `reorder_for_locality` in `ARCHITECTURE.md`).
    ///
    /// # When to call
    ///
    /// Call once after bulk-loading vectors into a new collection, before the
    /// collection is opened for queries. Incremental inserts invalidate the
    /// locality benefit; re-call after the next compaction if latency regresses.
    /// No-op for collections with fewer than 1 000 vectors.
    ///
    /// # Errors
    ///
    /// Returns an error if vector storage reordering fails.
    pub fn reorder_for_locality(&self) -> Result<()> {
        self.index.reorder_for_locality()
    }
}
