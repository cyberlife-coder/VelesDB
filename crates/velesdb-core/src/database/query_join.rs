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
        results: &[SearchResult],
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
                let merged = Self::merge_payloads(&left.point, right_point);
                output.push(SearchResult::new(merged, left.score));
            } else if matches!(join.join_type, crate::velesql::JoinType::Left) {
                output.push(left.clone());
            }
        }
        output
    }

    /// Merges payloads from left and right points into a single point.
    pub(super) fn merge_payloads(left: &crate::Point, right: &crate::Point) -> crate::Point {
        let mut payload = left
            .payload
            .as_ref()
            .and_then(|p| p.as_object().cloned())
            .unwrap_or_default();
        if let Some(right_obj) = right.payload.as_ref().and_then(|p| p.as_object()) {
            for (k, v) in right_obj {
                payload.insert(k.clone(), v.clone());
            }
        }
        let mut merged = left.clone();
        merged.payload = Some(serde_json::Value::Object(payload));
        merged
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
        results: &[SearchResult],
        join: &crate::velesql::JoinClause,
        pushed: &[crate::velesql::Condition],
        row_budget: usize,
    ) -> Result<Vec<SearchResult>> {
        let join_collection = self.resolve_collection(&join.table)?;

        if Self::is_lookup_join_eligible(join) && pushed.is_empty() {
            return Ok(Self::execute_lookup_join(results, join, &join_collection));
        }
        if pushed.is_empty() {
            if let Some(output) =
                Self::try_indexed_join(results, join, &join_collection, row_budget)
            {
                return Ok(output);
            }
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
    /// Returns `None` when the shape is not eligible, falling through to the
    /// ColumnStore path: Right/Full joins need the unmatched join-side rows
    /// that only a full enumeration can produce, `id` is already served by
    /// the lookup fast path, and a column that is neither `id` nor indexed
    /// reaches that path's actionable error. Unlike the PK paths a non-PK
    /// key may match several join-side points; one merged row is emitted per
    /// (left row, match) pair, bounded by `row_budget`.
    fn try_indexed_join(
        results: &[SearchResult],
        join: &crate::velesql::JoinClause,
        collection: &crate::collection::Collection,
        row_budget: usize,
    ) -> Option<Vec<SearchResult>> {
        use crate::velesql::JoinType;
        if !matches!(join.join_type, JoinType::Inner | JoinType::Left) {
            return None;
        }
        let condition = crate::collection::search::query::join::resolve_join_condition(join)?;
        let index_column = condition.left.column.clone();
        if index_column == "id" || !collection.indexed_field_names().contains(&index_column) {
            return None;
        }

        let mut output = Vec::new();
        for left in results {
            if output.len() >= row_budget {
                break;
            }
            Self::append_indexed_matches(
                collection,
                left,
                &index_column,
                &condition.right.column,
                join.join_type,
                row_budget,
                &mut output,
            );
        }
        Some(output)
    }

    /// Appends the join output rows for one left-side result: every indexed
    /// match merged left-over-right, or the bare left row for an unmatched
    /// LEFT JOIN. The source key uses the same payload→key conversion as
    /// index maintenance (`JsonValue::from_json`), with the `id` fallback the
    /// PK key extractor also honours.
    fn append_indexed_matches(
        collection: &crate::collection::Collection,
        left: &SearchResult,
        index_column: &str,
        source_column: &str,
        join_type: crate::velesql::JoinType,
        row_budget: usize,
        output: &mut Vec<SearchResult>,
    ) {
        let source_value = left
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get(source_column).cloned())
            .or_else(|| (source_column == "id").then(|| serde_json::json!(left.point.id)));
        let matches = source_value
            .as_ref()
            .and_then(crate::index::JsonValue::from_json)
            .and_then(|key| collection.secondary_index_lookup(index_column, &key))
            .unwrap_or_default();

        if matches.is_empty() {
            if matches!(join_type, crate::velesql::JoinType::Left) {
                output.push(left.clone());
            }
            return;
        }
        for right_point in collection.get(&matches).into_iter().flatten() {
            if output.len() >= row_budget {
                return;
            }
            output.push(SearchResult::new(
                Self::merge_payloads(&left.point, &right_point),
                left.score,
            ));
        }
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
