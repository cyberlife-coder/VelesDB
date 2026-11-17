//! JOIN execution strategies: the lookup and index shortcuts, the generic
//! hash join, and condition pushdown stripping.

use crate::{Error, Result, SearchResult};

use super::Database;

/// One cached joined collection's points, with the counters they were read
/// under.
///
/// See `Database::join_store_cache` (CACHE-03) for the validity contract.
pub(super) struct JoinStoreEntry {
    /// `(schema_version, write_generation)` read *before* the points were
    /// read, so a mutation racing the read can only make the entry look stale.
    stamp: (u64, u64),
    /// The shared points, cloned out cheaply per query.
    points: std::sync::Arc<Vec<crate::Point>>,
}

impl Database {
    /// Whether the join is `ON base.id = side.id` with the base id read on
    /// the FROM side (the row's own point id), so the lookup path can fetch
    /// the side's points by the rows' ids directly.
    pub(super) fn is_lookup_join_eligible(
        join: &crate::velesql::JoinClause,
        prior_aliases: &[&str],
    ) -> bool {
        if join.condition.is_none() {
            return false;
        }
        let Some(keys) = crate::velesql::JoinKeys::of(join) else {
            return false;
        };
        let base_on_from = keys
            .base
            .table
            .as_deref()
            .is_none_or(|t| prior_aliases.first() == Some(&t));
        keys.side == "id" && keys.base.column == "id" && base_on_from
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

        let alias = crate::velesql::side_alias(join.alias.as_deref(), &join.table);
        let mut output = Vec::with_capacity(results.len());
        for left in results {
            if let Some(right_point) = point_map.get(&left.point.id) {
                let score = left.score;
                let merged = Self::merge_payloads_owned(left.point, right_point, alias);
                output.push(SearchResult::new(merged, score));
            } else if matches!(join.join_type, crate::velesql::JoinType::Left) {
                output.push(Self::unmatched_left(left, alias));
            }
        }
        output
    }

    /// Merges payloads from left and right points into a single point.
    ///
    /// Borrowing variant for callers that emit several rows from one left
    /// point (the indexed 1:N path); the by-value twin below moves the left
    /// point — vector included — instead of cloning it.
    pub(super) fn merge_payloads(
        left: &crate::Point,
        right: &crate::Point,
        alias: &str,
    ) -> crate::Point {
        Self::merge_payloads_owned(left.clone(), right, alias)
    }

    /// By-value twin of [`Self::merge_payloads`]: consumes the left point so
    /// its vector is moved, not copied.
    pub(super) fn merge_payloads_owned(
        mut left: crate::Point,
        right: &crate::Point,
        alias: &str,
    ) -> crate::Point {
        let empty = serde_json::Map::new();
        let fields = right
            .payload
            .as_ref()
            .and_then(|p| p.as_object())
            .unwrap_or(&empty);
        left.payload = Some(crate::velesql::merge_side(
            left.payload.take(),
            alias,
            fields,
            Some(right.id),
        ));
        left
    }

    /// A LEFT JOIN row whose join side found no match: the alias carries a
    /// `null` id (`velesql::merge_side`).
    pub(super) fn unmatched_left(mut left: SearchResult, alias: &str) -> SearchResult {
        left.point.payload = Some(crate::velesql::merge_side(
            left.point.payload.take(),
            alias,
            &serde_json::Map::new(),
            None,
        ));
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
            // Pushdown analysis classifies a group's inside, so a group is
            // stripped through, not kept whole.
            C::Group(inner) if !pushed.iter().any(|p| p == inner.as_ref()) => {
                Self::strip_condition_recursive(inner, pushed).map(|kept| C::Group(Box::new(kept)))
            }
            C::Group(_) => None,
            _ => Some(condition.clone()),
        }
    }

    /// Executes a single JOIN using the optimal strategy: lookup, filtered, or full.
    ///
    /// `row_budget` bounds how many joined rows are materialized (the query's
    /// effective `LIMIT + OFFSET`), preventing OOM on RIGHT/FULL joins over large
    /// stores. It never drops rows within the requested window.
    /// `results` joined with `join`'s collection, on any column and for
    /// every join type, through [`crate::velesql::hash_join`]. An INNER or
    /// LEFT join on `id` or on an indexed column, with nothing pushed to its
    /// side, takes the lookup or index shortcut instead, which answers it
    /// without reading the whole side. RIGHT and FULL need every point of the
    /// side, so they never do; nor does a join after one, whose rows for
    /// unmatched points carry no base id, nor an index join whose base keys
    /// include an array or object, which the index does not key.
    pub(super) fn execute_single_join(
        &self,
        results: Vec<SearchResult>,
        join: &crate::velesql::JoinClause,
        step: &super::query_engine_join::JoinStep<'_>,
    ) -> Result<Vec<SearchResult>> {
        use crate::velesql::JoinType;
        let join_collection = self.resolve_collection(&join.table)?;
        let shortcut = step.pushed.is_empty()
            && !step.after_outer_join
            && matches!(join.join_type, JoinType::Inner | JoinType::Left);
        if shortcut && Self::is_lookup_join_eligible(join, step.prior_aliases) {
            return Ok(Self::execute_lookup_join(results, join, &join_collection));
        }
        if shortcut
            && Self::indexed_join_eligible(join, &join_collection)
            && Self::base_keys_are_scalar(&results, join)
        {
            return Ok(Self::execute_indexed_join(
                results,
                join,
                &join_collection,
                step.row_budget,
            ));
        }
        let keys = crate::velesql::JoinKeys::of(join).ok_or_else(|| {
            Error::Query(format!(
                "JOIN on table '{}' must use ON condition or USING(single_column).",
                join.table
            ))
        })?;
        let side = self.join_side_points(&join.table, &join_collection, step.pushed);
        Ok(crate::velesql::hash_join(
            results,
            &side,
            join,
            &keys,
            step.prior_aliases,
            step.row_budget,
        ))
    }

    /// Whether every row's base value for the indexed join is a scalar (or
    /// absent): the secondary index keys strings, numbers and booleans only.
    fn base_keys_are_scalar(results: &[SearchResult], join: &crate::velesql::JoinClause) -> bool {
        let Some(condition) = crate::collection::search::query::join::resolve_join_condition(join)
        else {
            return false;
        };
        results.iter().all(|row| {
            Self::left_join_value(row, &condition.right.column)
                .is_none_or(|value| !value.is_array() && !value.is_object())
        })
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
        let alias = crate::velesql::side_alias(join.alias.as_deref(), &join.table);

        // Phase 1: one key per left row (same payload->key conversion as
        // index maintenance); `id` is the point id, as in every clause.
        let values: Vec<Option<serde_json::Value>> = results
            .iter()
            .map(|left| Self::left_join_value(left, source_column))
            .collect();
        let keys: Vec<Option<crate::index::JsonValue>> = values
            .iter()
            .map(|value| value.as_ref().and_then(crate::index::JsonValue::from_json))
            .collect();

        // Phase 2: build side — one lookup per distinct key, one batched get.
        let (key_matches, point_map) =
            Self::indexed_join_build_side(collection, &keys, index_column);

        // Phase 3: emit — one merged row per (left row, match), budget-bounded.
        let mut output = Vec::new();
        for ((left, key), value) in results.into_iter().zip(keys).zip(&values) {
            if output.len() >= row_budget {
                break;
            }
            let candidates = key.as_ref().and_then(|k| key_matches.get(k));
            let match_ids =
                Self::exact_matches(candidates, &point_map, index_column, value.as_ref());
            if match_ids.is_empty() {
                if matches!(join.join_type, crate::velesql::JoinType::Left) {
                    output.push(Self::unmatched_left(left, alias));
                }
                continue;
            }
            Self::emit_indexed_rows(left, &match_ids, &point_map, alias, row_budget, &mut output);
        }
        output
    }

    /// A left row's join value: its point id for `id`, as in every clause,
    /// else its payload field.
    fn left_join_value(left: &SearchResult, column: &str) -> Option<serde_json::Value> {
        if column == "id" {
            return Some(serde_json::Value::from(left.point.id));
        }
        left.point.payload.as_ref()?.get(column).cloned()
    }

    /// The index's candidates for a left `value`, kept only where the join
    /// side holds exactly that value. The index keys numbers by `f64`, so for
    /// an integer at or past 2^53 its bucket also holds a neighbour's rows,
    /// which the join must not pair; any other value's bucket is exact.
    fn exact_matches<'a>(
        candidates: Option<&'a Vec<u64>>,
        point_map: &std::collections::HashMap<u64, crate::Point>,
        index_column: &str,
        value: Option<&serde_json::Value>,
    ) -> std::borrow::Cow<'a, [u64]> {
        let (Some(candidates), Some(value)) = (candidates, value) else {
            return std::borrow::Cow::Borrowed(&[]);
        };
        if crate::filter::exact_as_f64(value) {
            return std::borrow::Cow::Borrowed(candidates);
        }
        candidates
            .iter()
            .copied()
            .filter(|id| {
                point_map
                    .get(id)
                    .and_then(|right| right.payload.as_ref()?.get(index_column))
                    .is_some_and(|held| same_number(held, value))
            })
            .collect::<Vec<u64>>()
            .into()
    }

    /// Emits the merged rows of one left row's match list, budget-bounded.
    ///
    /// The final row of a key takes the left point by move — the 1:1 common
    /// case therefore never clones the vector. A match list whose ids all
    /// failed to hydrate emits nothing, exactly as the per-row `get` loop
    /// behaved.
    fn emit_indexed_rows(
        left: SearchResult,
        match_ids: &[u64],
        point_map: &std::collections::HashMap<u64, crate::Point>,
        alias: &str,
        row_budget: usize,
        output: &mut Vec<SearchResult>,
    ) {
        let rights: Vec<&crate::Point> = match_ids
            .iter()
            .filter_map(|id| point_map.get(id))
            .collect();
        let Some((last_right, head_rights)) = rights.split_last() else {
            return;
        };
        let score = left.score;
        for right_point in head_rights {
            if output.len() >= row_budget {
                return;
            }
            output.push(SearchResult::new(
                Self::merge_payloads(&left.point, right_point, alias),
                score,
            ));
        }
        if output.len() < row_budget {
            output.push(SearchResult::new(
                Self::merge_payloads_owned(left.point, last_right, alias),
                score,
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
    pub(super) fn cached_join_points(
        &self,
        name: &str,
        collection: &crate::collection::Collection,
    ) -> std::sync::Arc<Vec<crate::Point>> {
        let stamp = (
            self.schema_version
                .load(std::sync::atomic::Ordering::Acquire),
            collection.write_generation(),
        );
        if let Some(entry) = self.join_store_cache.read().get(name) {
            if entry.stamp == stamp {
                return std::sync::Arc::clone(&entry.points);
            }
        }

        let points = std::sync::Arc::new(Self::fetch_join_points(collection));
        if !Self::points_carry_ttl(&points) {
            self.join_store_cache.write().insert(
                name.to_string(),
                JoinStoreEntry {
                    stamp,
                    points: std::sync::Arc::clone(&points),
                },
            );
        }
        points
    }

    /// The joined collection's points a join reads: every live point,
    /// cached while the collection is unchanged, or those the predicates
    /// pushed to that side keep.
    fn join_side_points(
        &self,
        name: &str,
        collection: &crate::collection::Collection,
        pushed: &[crate::velesql::Condition],
    ) -> std::sync::Arc<Vec<crate::Point>> {
        if pushed.is_empty() {
            self.cached_join_points(name, collection)
        } else {
            std::sync::Arc::new(Self::filtered_join_points(collection, pushed))
        }
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

/// Whether two JSON values are the same, numbers by exact value.
fn same_number(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    match (a, b) {
        (serde_json::Value::Number(x), serde_json::Value::Number(y)) => {
            crate::velesql::compare_numbers(x, y).is_eq()
        }
        _ => a == b,
    }
}
