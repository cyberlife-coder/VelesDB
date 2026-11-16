#[cfg(feature = "persistence")]
use super::{Database, Error, Result};

impl Database {
    pub(super) fn resolve_dml_value(
        value: &crate::velesql::Value,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value> {
        match value {
            crate::velesql::Value::Integer(v) => Ok(serde_json::json!(v)),
            crate::velesql::Value::UnsignedInteger(v) => Ok(serde_json::json!(v)),
            crate::velesql::Value::Float(v) => Ok(serde_json::json!(v)),
            crate::velesql::Value::String(v) => Ok(serde_json::json!(v)),
            crate::velesql::Value::Boolean(v) => Ok(serde_json::json!(v)),
            crate::velesql::Value::Null => Ok(serde_json::Value::Null),
            crate::velesql::Value::Parameter(name) => params
                .get(name)
                .cloned()
                .ok_or_else(|| Error::Config(format!("Missing query parameter: ${name}"))),
            crate::velesql::Value::Temporal(expr) => Ok(serde_json::json!(expr.to_epoch_seconds())),
            crate::velesql::Value::Subquery(_) => Err(Error::Query(
                "Subquery values are not supported in INSERT/UPDATE".to_string(),
            )),
        }
    }

    pub(super) fn json_to_u64_id(value: &serde_json::Value) -> Result<u64> {
        // Try u64 first (covers full range), fall back to i64 for negative error.
        if let Some(u) = value.as_u64() {
            return Ok(u);
        }
        value
            .as_i64()
            .ok_or_else(|| Error::Query("id must be an integer".to_string()))
            .and_then(|v| u64::try_from(v).map_err(|_| Error::Query("id must be >= 0".to_string())))
    }

    pub(super) fn json_to_vector(value: &serde_json::Value) -> Result<Vec<f32>> {
        let arr = value
            .as_array()
            .ok_or_else(|| Error::Query("'vector' must be an array of numbers".to_string()))?;
        arr.iter().map(Self::json_element_to_f32).collect()
    }

    /// Converts a single JSON number to an f32, validating range and finiteness.
    fn json_element_to_f32(v: &serde_json::Value) -> Result<f32> {
        let f = v
            .as_f64()
            .ok_or_else(|| Error::Query("vector values must be numeric".to_string()))?;
        if !f.is_finite() || f < f64::from(f32::MIN) || f > f64::from(f32::MAX) {
            return Err(Error::Query(
                "vector values must be finite f32-compatible numbers".to_string(),
            ));
        }
        #[allow(clippy::cast_possible_truncation)]
        // Reason: range validated above against f32::MIN..=f32::MAX.
        let as_f32 = f as f32;
        Ok(as_f32)
    }

    pub(super) fn build_dml_filter(
        statement: &str,
        where_clause: Option<&crate::velesql::Condition>,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Option<crate::Filter>> {
        let Some(condition) = where_clause else {
            return Ok(None);
        };

        if Self::contains_non_metadata_condition(condition) {
            return Err(Error::Query(format!(
                "{statement} WHERE supports metadata predicates only (no similarity/NEAR/MATCH)"
            )));
        }

        // Resolve parameter placeholders before the filter conversion, which
        // would otherwise silently turn them into NULL (matching no rows).
        let resolved = crate::velesql::bind_condition(condition, params)?;
        let filter_condition = crate::collection::Collection::extract_metadata_filter(&resolved)
            .ok_or_else(|| {
                Error::Query("UPDATE WHERE produced empty metadata filter".to_string())
            })?;
        Ok(Some(crate::Filter::new(crate::Condition::from(
            filter_condition,
        ))))
    }

    fn contains_non_metadata_condition(condition: &crate::velesql::Condition) -> bool {
        match condition {
            crate::velesql::Condition::Similarity(_)
            | crate::velesql::Condition::VectorSearch(_)
            | crate::velesql::Condition::VectorFusedSearch(_)
            | crate::velesql::Condition::SparseVectorSearch(_)
            | crate::velesql::Condition::GraphMatch(_) => true,
            crate::velesql::Condition::And(left, right)
            | crate::velesql::Condition::Or(left, right) => {
                Self::contains_non_metadata_condition(left)
                    || Self::contains_non_metadata_condition(right)
            }
            crate::velesql::Condition::Group(inner) | crate::velesql::Condition::Not(inner) => {
                Self::contains_non_metadata_condition(inner)
            }
            _ => false,
        }
    }

    pub(super) fn matches_dml_filter(point: &crate::Point, filter: Option<&crate::Filter>) -> bool {
        match filter {
            Some(filter) => Self::point_matches_filter(point, filter),
            None => true,
        }
    }

    /// The joined collection's points the pushed-down `filters` keep. Each
    /// condition's field names are stripped of table prefixes (e.g.,
    /// `inventory.price` → `price`) before evaluation against point payloads.
    pub(super) fn filtered_join_points(
        collection: &crate::collection::Collection,
        filters: &[crate::velesql::Condition],
    ) -> Vec<crate::Point> {
        let combined = Self::combine_filter_conditions(filters);
        let stripped = Self::strip_table_prefix_from_condition(combined);
        let filter = crate::Filter::new(crate::Condition::from(stripped));
        let ids = Self::join_candidate_ids(collection, &filter);
        collection
            .get(&ids)
            .into_iter()
            .flatten()
            .filter(|p| Self::point_matches_filter(p, &filter))
            .collect()
    }

    /// Enumerates the join-side candidate IDs to feed into the post-filter.
    ///
    /// When the pushed-down `filter` references secondary-indexed fields,
    /// `build_prefilter_bitmap` resolves it to a Roaring bitmap of candidate IDs,
    /// avoiding the O(N) `all_point_ids()` scan. The bitmap is a guaranteed SUPERSET of
    /// the matches — `point_matches_filter` re-checks every returned point, so
    /// results are identical to the scan path. The superset guarantee holds
    /// because `build_prefilter_bitmap` returns `None` (forcing the
    /// `all_point_ids()` fallback) for any predicate it cannot resolve completely: `!=` / `NOT IN`
    /// (whose matches include field-absent points the index does not store) and
    /// any field carrying an ID above `u32::MAX` (unrepresentable in the bitmap).
    /// Falls back to `all_point_ids()` — every point, with or without a
    /// payload — whenever no index can resolve the predicate.
    fn join_candidate_ids(
        collection: &crate::collection::Collection,
        filter: &crate::Filter,
    ) -> Vec<u64> {
        match collection.build_prefilter_bitmap(filter) {
            Some(bitmap) => bitmap.iter().map(u64::from).collect(),
            None => collection.all_point_ids(),
        }
    }

    /// Evaluates a filter against a point: a `VelesQL` `id` column reads its
    /// id, as in every clause.
    fn point_matches_filter(point: &crate::Point, filter: &crate::Filter) -> bool {
        filter.matches_point(point.id, point.payload.as_ref())
    }

    /// Combines multiple `velesql::Condition`s into a single AND tree.
    fn combine_filter_conditions(
        filters: &[crate::velesql::Condition],
    ) -> crate::velesql::Condition {
        let mut iter = filters.iter().cloned();
        let Some(first) = iter.next() else {
            unreachable!("caller must pass at least one filter");
        };
        iter.fold(first, |acc, c| {
            crate::velesql::Condition::And(Box::new(acc), Box::new(c))
        })
    }

    /// Strips table prefixes from all column references in a `velesql::Condition`.
    ///
    /// Converts qualified names like `inventory.price` to `price` so that
    /// `Filter::matches_point` can evaluate them against unqualified payload keys.
    pub(super) fn strip_table_prefix_from_condition(
        condition: crate::velesql::Condition,
    ) -> crate::velesql::Condition {
        use crate::velesql::Condition as C;
        match condition {
            C::And(l, r) => C::And(
                Box::new(Self::strip_table_prefix_from_condition(*l)),
                Box::new(Self::strip_table_prefix_from_condition(*r)),
            ),
            C::Or(l, r) => C::Or(
                Box::new(Self::strip_table_prefix_from_condition(*l)),
                Box::new(Self::strip_table_prefix_from_condition(*r)),
            ),
            C::Not(inner) => C::Not(Box::new(Self::strip_table_prefix_from_condition(*inner))),
            C::Group(inner) => C::Group(Box::new(Self::strip_table_prefix_from_condition(*inner))),
            leaf => Self::strip_table_prefix_on_leaf(leaf),
        }
    }

    /// Applies [`strip_prefix`] to the `column` field of leaf (non-composite)
    /// conditions. Engine-handled variants (`VectorSearch`, `GraphMatch`, …)
    /// pass through unchanged.
    ///
    /// [`strip_prefix`]: Self::strip_prefix
    fn strip_table_prefix_on_leaf(
        condition: crate::velesql::Condition,
    ) -> crate::velesql::Condition {
        use crate::velesql::Condition as C;
        match condition {
            C::Comparison(mut cmp) => {
                cmp.column = Self::strip_prefix(&cmp.column);
                C::Comparison(cmp)
            }
            C::In(mut inc) => {
                inc.column = Self::strip_prefix(&inc.column);
                C::In(inc)
            }
            C::Between(mut btw) => {
                btw.column = Self::strip_prefix(&btw.column);
                C::Between(btw)
            }
            C::Like(mut lk) => {
                lk.column = Self::strip_prefix(&lk.column);
                C::Like(lk)
            }
            C::IsNull(mut isn) => {
                isn.column = Self::strip_prefix(&isn.column);
                C::IsNull(isn)
            }
            C::Match(mut m) => {
                m.column = Self::strip_prefix(&m.column);
                C::Match(m)
            }
            C::Contains(mut cc) => {
                cc.column = Self::strip_prefix(&cc.column);
                C::Contains(cc)
            }
            C::ContainsText(mut ct) => {
                ct.column = Self::strip_prefix(&ct.column);
                C::ContainsText(ct)
            }
            C::GeoDistance(mut gd) => {
                gd.column = Self::strip_prefix(&gd.column);
                C::GeoDistance(gd)
            }
            C::GeoBbox(mut gb) => {
                gb.column = Self::strip_prefix(&gb.column);
                C::GeoBbox(gb)
            }
            // Engine-handled conditions pass through unchanged.
            other => other,
        }
    }

    /// Strips the `table.` prefix from a column name, if present.
    fn strip_prefix(column: &str) -> String {
        column
            .split_once('.')
            .map_or_else(|| column.to_string(), |(_, col)| col.to_string())
    }

    /// Fetches every live point of `collection` for JOIN-side materialization.
    ///
    /// Goes through `Collection::get`, so lazily-expired TTL points are
    /// already excluded from the result.
    pub(super) fn fetch_join_points(
        collection: &crate::collection::Collection,
    ) -> Vec<crate::Point> {
        let ids = collection.all_point_ids();
        collection.get(&ids).into_iter().flatten().collect()
    }
}
