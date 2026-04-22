#[cfg(feature = "persistence")]
use super::{ColumnStore, Database, Error, Result};

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

    pub(super) fn build_update_filter(
        where_clause: Option<&crate::velesql::Condition>,
    ) -> Result<Option<crate::Filter>> {
        let Some(condition) = where_clause else {
            return Ok(None);
        };

        if Self::contains_non_metadata_condition(condition) {
            return Err(Error::Query(
                "UPDATE WHERE supports metadata predicates only (no similarity/NEAR/MATCH)"
                    .to_string(),
            ));
        }

        let filter_condition = crate::collection::Collection::extract_metadata_filter(condition)
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

    pub(super) fn matches_update_filter(
        point: &crate::Point,
        filter: Option<&crate::Filter>,
    ) -> bool {
        let Some(filter) = filter else {
            return true;
        };

        let mut obj = point
            .payload
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();
        obj.insert("id".to_string(), serde_json::json!(point.id));
        filter.matches(&serde_json::Value::Object(obj))
    }

    /// Builds a `ColumnStore` from a collection, filtering points by pushed-down conditions.
    ///
    /// When `filters` is empty, behaves identically to `build_join_column_store`.
    /// Each condition's field names are stripped of table prefixes (e.g., `inventory.price` → `price`)
    /// before evaluation against point payloads.
    pub(super) fn build_filtered_join_column_store(
        collection: &crate::collection::Collection,
        filters: &[crate::velesql::Condition],
    ) -> Result<ColumnStore> {
        if filters.is_empty() {
            return Self::build_join_column_store(collection);
        }

        let combined = Self::combine_filter_conditions(filters);
        let stripped = Self::strip_table_prefix_from_condition(combined);
        let filter = crate::Filter::new(crate::Condition::from(stripped));

        let ids = collection.all_ids();
        let points: Vec<_> = collection.get(&ids).into_iter().flatten().collect();
        let matching: Vec<_> = points
            .iter()
            .filter(|p| Self::point_matches_filter(p, &filter))
            .collect();

        Self::build_column_store_from_points(&matching)
    }

    /// Evaluates a filter against a point's payload with `id` injected.
    fn point_matches_filter(point: &crate::Point, filter: &crate::Filter) -> bool {
        let mut obj = point
            .payload
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();
        obj.insert("id".to_string(), serde_json::json!(point.id));
        filter.matches(&serde_json::Value::Object(obj))
    }

    /// Combines multiple `velesql::Condition`s into a single AND tree.
    fn combine_filter_conditions(
        filters: &[crate::velesql::Condition],
    ) -> crate::velesql::Condition {
        let mut iter = filters.iter().cloned();
        let first = iter.next().expect("filters is non-empty");
        iter.fold(first, |acc, c| {
            crate::velesql::Condition::And(Box::new(acc), Box::new(c))
        })
    }

    /// Strips table prefixes from all column references in a `velesql::Condition`.
    ///
    /// Converts qualified names like `inventory.price` to `price` so that
    /// `Filter::matches` can evaluate them against unqualified payload keys.
    fn strip_table_prefix_from_condition(
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

    /// Builds a `ColumnStore` from a slice of point references.
    fn build_column_store_from_points(points: &[&crate::Point]) -> Result<ColumnStore> {
        let owned: Vec<crate::Point> = points.iter().copied().cloned().collect();
        let schema = Self::infer_column_schema(&owned);
        let schema_refs: Vec<(&str, crate::column_store::ColumnType)> = schema
            .iter()
            .map(|(name, ty)| (name.as_str(), ty.clone()))
            .collect();

        let mut store = ColumnStore::with_primary_key(&schema_refs, "id")
            .map_err(|e| Error::ColumnStoreError(e.to_string()))?;

        for point in &owned {
            Self::insert_point_row(point, &schema_refs, &mut store)?;
        }

        Ok(store)
    }

    pub(super) fn build_join_column_store(
        collection: &crate::collection::Collection,
    ) -> Result<ColumnStore> {
        let ids = collection.all_ids();
        let points: Vec<_> = collection.get(&ids).into_iter().flatten().collect();

        let schema = Self::infer_column_schema(&points);
        let schema_refs: Vec<(&str, crate::column_store::ColumnType)> = schema
            .iter()
            .map(|(name, ty)| (name.as_str(), ty.clone()))
            .collect();

        let mut store = ColumnStore::with_primary_key(&schema_refs, "id")
            .map_err(|e| Error::ColumnStoreError(e.to_string()))?;

        for point in &points {
            Self::insert_point_row(point, &schema_refs, &mut store)?;
        }

        Ok(store)
    }

    /// Infers a consistent column schema from point payloads.
    ///
    /// Removes columns whose types are inconsistent across points.
    fn infer_column_schema(
        points: &[crate::Point],
    ) -> Vec<(String, crate::column_store::ColumnType)> {
        use crate::column_store::ColumnType;

        let mut inferred: std::collections::BTreeMap<String, ColumnType> =
            std::collections::BTreeMap::new();
        inferred.insert("id".to_string(), ColumnType::Int);

        for point in points {
            let Some(obj) = point.payload.as_ref().and_then(|p| p.as_object()) else {
                continue;
            };
            for (key, value) in obj {
                if key == "id" {
                    continue;
                }
                let Some(col_type) = Self::json_to_column_type(value) else {
                    continue;
                };
                if let Some(existing) = inferred.get(key) {
                    if *existing != col_type {
                        inferred.remove(key);
                    }
                } else {
                    inferred.insert(key.clone(), col_type);
                }
            }
        }

        inferred.into_iter().collect()
    }

    /// Inserts a single point as a row into the column store.
    fn insert_point_row(
        point: &crate::Point,
        schema_refs: &[(&str, crate::column_store::ColumnType)],
        store: &mut ColumnStore,
    ) -> Result<()> {
        use crate::column_store::ColumnValue;

        let Ok(pk) = i64::try_from(point.id) else {
            return Ok(());
        };

        let mut values: Vec<(String, ColumnValue)> = Vec::with_capacity(schema_refs.len());
        values.push(("id".to_string(), ColumnValue::Int(pk)));

        if let Some(obj) = point
            .payload
            .as_ref()
            .and_then(serde_json::Value::as_object)
        {
            for (key, value) in obj {
                if key == "id" || !schema_refs.iter().any(|(name, _)| *name == key.as_str()) {
                    continue;
                }
                if let Some(column_value) = Self::json_to_column_value(value, store) {
                    values.push((key.clone(), column_value));
                }
            }
        }

        let row: Vec<(&str, ColumnValue)> = values
            .iter()
            .map(|(name, value)| (name.as_str(), value.clone()))
            .collect();
        store
            .insert_row(&row)
            .map_err(|e| Error::ColumnStoreError(e.to_string()))?;
        Ok(())
    }

    fn json_to_column_type(value: &serde_json::Value) -> Option<crate::column_store::ColumnType> {
        use crate::column_store::ColumnType;
        match value {
            serde_json::Value::Number(n) if n.is_i64() => Some(ColumnType::Int),
            serde_json::Value::Number(_) => Some(ColumnType::Float),
            serde_json::Value::String(_) => Some(ColumnType::String),
            serde_json::Value::Bool(_) => Some(ColumnType::Bool),
            _ => None,
        }
    }

    fn json_to_column_value(
        value: &serde_json::Value,
        store: &mut ColumnStore,
    ) -> Option<crate::column_store::ColumnValue> {
        use crate::column_store::ColumnValue;
        match value {
            serde_json::Value::Number(n) => {
                if let Some(v) = n.as_i64() {
                    Some(ColumnValue::Int(v))
                } else {
                    n.as_f64().map(ColumnValue::Float)
                }
            }
            serde_json::Value::String(s) => {
                let sid = store.string_table_mut().intern(s);
                Some(ColumnValue::String(sid))
            }
            serde_json::Value::Bool(b) => Some(ColumnValue::Bool(*b)),
            serde_json::Value::Null => Some(ColumnValue::Null),
            _ => None,
        }
    }
}
