//! JOIN execution strategies: lookup join, filtered join, and condition pushdown stripping.

use crate::{Result, SearchResult};

use super::Database;

/// One cached JOIN-side `ColumnStore` with the counters it was built under.
///
/// See `Database::join_store_cache` (CACHE-03) for the validity contract.
pub(super) struct JoinStoreEntry {
    /// `(schema_version, write_generation)` read *before* the store was built,
    /// so a mutation racing the build can only make the entry look stale.
    stamp: (u64, u64),
    /// The shared store, cloned out cheaply per query.
    store: std::sync::Arc<crate::column_store::ColumnStore>,
}

impl Database {
    /// Returns `true` if the join condition references the primary key (`id`) on both sides.
    ///
    /// This enables the lookup join optimization path, which uses direct
    /// `collection.get(&[ids])` instead of building a full `ColumnStore`.
    pub(super) fn is_lookup_join_eligible(join: &crate::velesql::JoinClause) -> bool {
        let Some(ref condition) = join.condition else {
            return false;
        };
        let left_is_id = condition.left.column == "id";
        let right_is_id = condition.right.column == "id";
        left_is_id && right_is_id
    }

    /// Performs a lookup join by extracting keys from left-side results
    /// and retrieving matching points directly from the collection.
    ///
    /// This avoids building a full `ColumnStore` when the join key is the primary key.
    pub(super) fn execute_lookup_join(
        results: Vec<SearchResult>,
        join: &crate::velesql::JoinClause,
        collection: &crate::collection::Collection,
    ) -> Vec<SearchResult> {
        let unique_ids: Vec<u64> = results
            .iter()
            .map(|r| r.point.id)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        let fetched = collection.get(&unique_ids);
        let point_map: std::collections::HashMap<u64, crate::Point> =
            fetched.into_iter().flatten().map(|p| (p.id, p)).collect();

        let mut output = Vec::with_capacity(results.len());
        for left in results {
            if let Some(right_point) = point_map.get(&left.point.id) {
                let score = left.score;
                let merged = Self::merge_payloads_owned(left.point, right_point);
                output.push(SearchResult::new(merged, score));
            } else if matches!(join.join_type, crate::velesql::JoinType::Left) {
                output.push(left);
            }
        }
        output
    }

    /// Merges payloads from left and right points into a single point.
    ///
    /// Borrowing variant for callers that emit several rows from one left
    /// point (the indexed 1:N path); the by-value twin below moves the left
    /// point — vector included — instead of cloning it.
    pub(super) fn merge_payloads(left: &crate::Point, right: &crate::Point) -> crate::Point {
        Self::merge_payloads_owned(left.clone(), right)
    }

    /// By-value twin of [`Self::merge_payloads`]: consumes the left point so
    /// its vector is moved, not copied.
    pub(super) fn merge_payloads_owned(
        mut left: crate::Point,
        right: &crate::Point,
    ) -> crate::Point {
        let mut payload = left
            .payload
            .take()
            .and_then(|p| match p {
                serde_json::Value::Object(map) => Some(map),
                _ => None,
            })
            .unwrap_or_default();
        if let Some(right_obj) = right.payload.as_ref().and_then(|p| p.as_object()) {
            for (k, v) in right_obj {
                payload.insert(k.clone(), v.clone());
            }
        }
        left.payload = Some(serde_json::Value::Object(payload));
        left
    }

    /// Rebuilds a WHERE clause excluding conditions that were pushed down.
    ///
    /// Walks the condition tree and removes conditions present in the `pushed` set.
    /// For AND nodes, removes pushed children and collapses single-child ANDs.
    /// Returns `None` if all conditions were pushed.
    pub(super) fn strip_pushed_conditions(
        where_clause: Option<&crate::velesql::Condition>,
        pushed: &[crate::velesql::Condition],
    ) -> Option<crate::velesql::Condition> {
        let condition = where_clause?;
        if pushed.iter().any(|p| p == condition) {
            return None;
        }
        Self::strip_condition_recursive(condition, pushed)
    }

    /// Recursively strips pushed conditions from a condition tree.
    fn strip_condition_recursive(
        condition: &crate::velesql::Condition,
        pushed: &[crate::velesql::Condition],
    ) -> Option<crate::velesql::Condition> {
        use crate::velesql::Condition as C;
        match condition {
            C::And(left, right) => {
                let l = if pushed.iter().any(|p| p == left.as_ref()) {
                    None
                } else {
                    Self::strip_condition_recursive(left, pushed)
                };
                let r = if pushed.iter().any(|p| p == right.as_ref()) {
                    None
                } else {
                    Self::strip_condition_recursive(right, pushed)
                };
                match (l, r) {
                    (Some(l), Some(r)) => Some(C::And(Box::new(l), Box::new(r))),
                    (Some(c), None) | (None, Some(c)) => Some(c),
                    (None, None) => None,
                }
            }
            _ => Some(condition.clone()),
        }
    }

    /// Executes a single JOIN using the optimal strategy: lookup, filtered, or full.
    ///
    /// `row_budget` bounds how many joined rows are materialized (the query's
    /// effective `LIMIT + OFFSET`), preventing OOM on RIGHT/FULL joins over large
    /// stores. It never drops rows within the requested window.
    pub(super) fn execute_single_join(
        &self,
        results: Vec<SearchResult>,
        join: &crate::velesql::JoinClause,
        pushed: &[crate::velesql::Condition],
        row_budget: usize,
    ) -> Result<Vec<SearchResult>> {
        let join_collection = self.resolve_collection(&join.table)?;

        if Self::is_lookup_join_eligible(join) && pushed.is_empty() {
            return Ok(Self::execute_lookup_join(results, join, &join_collection));
        }
        if pushed.is_empty() && Self::indexed_join_eligible(join, &join_collection) {
            return Ok(Self::execute_indexed_join(
                results,
                join,
                &join_collection,
                row_budget,
            ));
        }

        let column_store = if pushed.is_empty() {
            self.cached_join_column_store(&join.table, &join_collection)?
        } else {
            std::sync::Arc::new(Self::build_filtered_join_column_store(
                &join_collection,
                pushed,
            )?)
        };

        let joined = crate::collection::search::query::join::execute_join(
            results,
            join,
            &column_store,
            row_budget,
        )?;
        Ok(crate::collection::search::query::join::joined_to_search_results(joined))
    }

    /// Answers an Inner/Left JOIN from a secondary index on the join-side
    /// column, avoiding the per-query `ColumnStore` materialization.
    ///
    /// Eligibility mirrors the pre-grouped shape: Right/Full joins need the
    /// unmatched join-side rows only a full enumeration can produce, `id` is
    /// already served by the lookup fast path, and a column that is neither
    /// `id` nor indexed falls through to the `ColumnStore` path.
    fn indexed_join_eligible(
        join: &crate::velesql::JoinClause,
        collection: &crate::collection::Collection,
    ) -> bool {
        use crate::velesql::JoinType;
        if !matches!(join.join_type, JoinType::Inner | JoinType::Left) {
            return false;
        }
        let Some(condition) = crate::collection::search::query::join::resolve_join_condition(join)
        else {
            return false;
        };
        let index_column = &condition.left.column;
        index_column != "id" && collection.indexed_field_names().contains(index_column)
    }

    /// Build side of the indexed join: one `secondary_index_lookup` per
    /// DISTINCT key, then one batched hydration over the deduplicated match
    /// ids — the hash-join build the per-left-row shape never had.
    #[allow(clippy::type_complexity)] // Reason: internal pair of build-side maps, named at the single call site
    fn indexed_join_build_side(
        collection: &crate::collection::Collection,
        keys: &[Option<crate::index::JsonValue>],
        index_column: &str,
    ) -> (
        std::collections::BTreeMap<crate::index::JsonValue, Vec<u64>>,
        std::collections::HashMap<u64, crate::Point>,
    ) {
        let mut key_matches: std::collections::BTreeMap<crate::index::JsonValue, Vec<u64>> =
            std::collections::BTreeMap::new();
        for key in keys.iter().flatten() {
            if !key_matches.contains_key(key) {
                let ids = collection
                    .secondary_index_lookup(index_column, key)
                    .unwrap_or_default();
                key_matches.insert(key.clone(), ids);
            }
        }
        let all_ids: Vec<u64> = {
            let mut seen = std::collections::HashSet::new();
            key_matches
                .values()
                .flatten()
                .copied()
                .filter(|id| seen.insert(*id))
                .collect()
        };
        let point_map: std::collections::HashMap<u64, crate::Point> = collection
            .get(&all_ids)
            .into_iter()
            .flatten()
            .map(|p| (p.id, p))
            .collect();
        (key_matches, point_map)
    }

    /// Executes the indexed non-PK join in three grouped phases.
    ///
    /// The per-left-row shape issued one `secondary_index_lookup` and one
    /// storage `get` PER LEFT ROW, re-hydrating the same join-side points
    /// once per left row sharing a key (10k left rows on one hot key = 10k
    /// identical fetches). Phases: (1) resolve each left row's join key
    /// once; (2) one index lookup per DISTINCT key and one batched
    /// hydration over the deduplicated match ids — a proper hash-join build
    /// side; (3) emit, moving each left row into its final match and
    /// cloning only for the extra rows of a 1:N key.
    fn execute_indexed_join(
        results: Vec<SearchResult>,
        join: &crate::velesql::JoinClause,
        collection: &crate::collection::Collection,
        row_budget: usize,
    ) -> Vec<SearchResult> {
        // Eligibility was checked by `indexed_join_eligible`.
        let Some(condition) = crate::collection::search::query::join::resolve_join_condition(join)
        else {
            return Vec::new();
        };
        let index_column = &condition.left.column;
        let source_column = &condition.right.column;

        // Phase 1: one key per left row (same payload->key conversion as
        // index maintenance, with the `id` fallback the PK extractor honours).
        let keys: Vec<Option<crate::index::JsonValue>> = results
            .iter()
            .map(|left| {
                left.point
                    .payload
                    .as_ref()
                    .and_then(|p| p.get(source_column).cloned())
                    .or_else(|| (source_column == "id").then(|| serde_json::json!(left.point.id)))
                    .as_ref()
                    .and_then(crate::index::JsonValue::from_json)
            })
            .collect();

        // Phase 2: build side — one lookup per distinct key, one batched get.
        let (key_matches, point_map) =
            Self::indexed_join_build_side(collection, &keys, index_column);

        // Phase 3: emit — one merged row per (left row, match), budget-bounded.
        let mut output = Vec::new();
        'rows: for (left, key) in results.into_iter().zip(keys) {
            if output.len() >= row_budget {
                break;
            }
            let matches = key
                .as_ref()
                .and_then(|k| key_matches.get(k))
                .filter(|ids| !ids.is_empty());
            let Some(match_ids) = matches else {
                if matches!(join.join_type, crate::velesql::JoinType::Left) {
                    output.push(left);
                }
                continue;
            };
            let rights: Vec<&crate::Point> = match_ids
                .iter()
                .filter_map(|id| point_map.get(id))
                .collect();
            let Some((last_right, head_rights)) = rights.split_last() else {
                // Matches existed but none hydrated — the row vanishes,
                // exactly as the per-row `get` loop emitted nothing here.
                continue;
            };
            let score = left.score;
            for right_point in head_rights {
                if output.len() >= row_budget {
                    continue 'rows;
                }
                output.push(SearchResult::new(
                    Self::merge_payloads(&left.point, right_point),
                    score,
                ));
            }
            if output.len() >= row_budget {
                continue;
            }
            // The final row of a key takes the left point by move — the
            // 1:1 common case therefore never clones the vector.
            output.push(SearchResult::new(
                Self::merge_payloads_owned(left.point, last_right),
                score,
            ));
        }
        output
    }

    /// Returns the JOIN-side `ColumnStore` for `collection`, rebuilding only
    /// when the collection changed since the cached copy was built.
    ///
    /// The `(schema_version, write_generation)` stamp is read *before* the
    /// build: a mutation racing the build can only make the cached entry look
    /// older than it is, forcing a rebuild on the next query — never a stale
    /// hit. Collections carrying TTL points are never cached, because expiry
    /// is evaluated lazily at read time and does not bump `write_generation`.
    pub(super) fn cached_join_column_store(
        &self,
        name: &str,
        collection: &crate::collection::Collection,
    ) -> Result<std::sync::Arc<crate::column_store::ColumnStore>> {
        let stamp = (
            self.schema_version
                .load(std::sync::atomic::Ordering::Acquire),
            collection.write_generation(),
        );
        if let Some(entry) = self.join_store_cache.read().get(name) {
            if entry.stamp == stamp {
                return Ok(std::sync::Arc::clone(&entry.store));
            }
        }

        let points = Self::fetch_join_points(collection);
        let carries_ttl = Self::points_carry_ttl(&points);
        let refs: Vec<&crate::Point> = points.iter().collect();
        let store = std::sync::Arc::new(Self::build_column_store_from_points(&refs)?);
        if !carries_ttl {
            self.join_store_cache.write().insert(
                name.to_string(),
                JoinStoreEntry {
                    stamp,
                    store: std::sync::Arc::clone(&store),
                },
            );
        }
        Ok(store)
    }

    /// Returns `true` if any point carries the reserved durable-TTL payload key.
    fn points_carry_ttl(points: &[crate::Point]) -> bool {
        points.iter().any(|p| {
            p.payload
                .as_ref()
                .and_then(serde_json::Value::as_object)
                .is_some_and(|map| map.contains_key(crate::collection::expiry::EXPIRES_AT_KEY))
        })
    }
}
