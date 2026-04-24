//! JOIN execution strategies: lookup join, filtered join, and condition pushdown stripping.

use crate::{Result, SearchResult};

use super::Database;

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
    pub(super) fn execute_single_join(
        &self,
        results: &[SearchResult],
        join: &crate::velesql::JoinClause,
        pushed: &[crate::velesql::Condition],
    ) -> Result<Vec<SearchResult>> {
        let join_collection = self.resolve_collection(&join.table)?;

        if Self::is_lookup_join_eligible(join) && pushed.is_empty() {
            return Ok(Self::execute_lookup_join(results, join, &join_collection));
        }

        let column_store = if pushed.is_empty() {
            Self::build_join_column_store(&join_collection)?
        } else {
            Self::build_filtered_join_column_store(&join_collection, pushed)?
        };

        let joined =
            crate::collection::search::query::join::execute_join(results, join, &column_store)?;
        Ok(crate::collection::search::query::join::joined_to_search_results(joined))
    }
}
