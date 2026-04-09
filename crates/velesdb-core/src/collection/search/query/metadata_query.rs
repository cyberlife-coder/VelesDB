use super::{Collection, SearchResult};

impl Collection {
    /// Attempts to resolve a metadata-only query using secondary indexes.
    ///
    /// Tries an Eq lookup on the first matching indexed field (fastest path).
    /// For AND conditions, flattens the tree and probes each leaf for an indexed Eq.
    /// Returns `None` when no index covers the condition, signaling the caller
    /// to fall back to a sequential scan.
    pub(super) fn execute_indexed_metadata_query(
        &self,
        cond: &crate::velesql::Condition,
        execution_limit: usize,
    ) -> Option<Vec<SearchResult>> {
        // Try simple Eq lookup first (fastest path).
        if let Some((field_name, key)) = Self::extract_index_lookup_condition(cond) {
            let ids = self.secondary_index_lookup(&field_name, &key)?;
            tracing::debug!(
                field = %field_name,
                ids_count = ids.len(),
                limit = execution_limit,
                "indexed metadata query: Eq lookup"
            );
            // Skip index path when too many hits — sequential scan with early
            // exit is faster than hydrating thousands of index results.
            if ids.len() > execution_limit.saturating_mul(50).max(1000) {
                tracing::debug!("indexed metadata query: too many hits, falling through to scan");
                return None; // Fall through to scan
            }
            let filter = crate::filter::Filter::new(crate::filter::Condition::from(cond.clone()));
            return Some(self.scan_ids_with_filter(&ids, &filter, execution_limit));
        }

        // For AND conditions, find the first Eq sub-condition that has an index,
        // use it to narrow the candidate set, then post-filter the rest.
        if let crate::velesql::Condition::And(ref _left, ref _right) = cond {
            // Flatten the AND tree into a list of leaf conditions.
            let mut leaves = Vec::new();
            Self::flatten_and_conditions(cond, &mut leaves);
            for sub in &leaves {
                if let Some((field_name, key)) = Self::extract_index_lookup_condition(sub) {
                    if let Some(ids) = self.secondary_index_lookup(&field_name, &key) {
                        let filter = crate::filter::Filter::new(crate::filter::Condition::from(
                            cond.clone(),
                        ));
                        return Some(self.scan_ids_with_filter(&ids, &filter, execution_limit));
                    }
                }
            }
        }

        None
    }

    /// Flattens a binary AND tree into a list of leaf conditions.
    pub(super) fn flatten_and_conditions<'a>(
        cond: &'a crate::velesql::Condition,
        out: &mut Vec<&'a crate::velesql::Condition>,
    ) {
        match cond {
            crate::velesql::Condition::And(left, right) => {
                Self::flatten_and_conditions(left, out);
                Self::flatten_and_conditions(right, out);
            }
            crate::velesql::Condition::Group(inner) => {
                Self::flatten_and_conditions(inner, out);
            }
            other => out.push(other),
        }
    }

    /// Scans a set of candidate IDs and applies a filter, returning matching results.
    ///
    /// Uses score `1.0` for metadata-only matches (no vector similarity involved).
    /// Also used by `dispatch_metadata_only` for bitmap-derived candidate sets.
    pub(super) fn scan_ids_with_filter(
        &self,
        ids: &[u64],
        filter: &crate::filter::Filter,
        execution_limit: usize,
    ) -> Vec<SearchResult> {
        let mut results = Vec::new();
        for point in self.get(ids).into_iter().flatten() {
            let payload = point.payload.clone().unwrap_or(serde_json::Value::Null);
            if filter.matches(&payload) {
                results.push(SearchResult::new(point, 1.0));
                if results.len() >= execution_limit {
                    break;
                }
            }
        }
        results
    }

    /// Extracts an `(field_name, value)` pair from an `Eq` comparison condition.
    ///
    /// Returns `None` for non-Eq operators or when the value cannot be converted
    /// to a `JsonValue` suitable for index lookup.
    pub(super) fn extract_index_lookup_condition(
        cond: &crate::velesql::Condition,
    ) -> Option<(String, crate::index::JsonValue)> {
        if let crate::velesql::Condition::Comparison(cmp) = cond {
            if cmp.operator == crate::velesql::CompareOp::Eq {
                return crate::index::JsonValue::from_ast_value(&cmp.value)
                    .map(|v| (cmp.column.clone(), v));
            }
        }
        None
    }
}
