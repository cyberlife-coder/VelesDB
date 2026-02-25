#[cfg(feature = "persistence")]
use super::*;

impl Database {
    pub(super) fn resolve_dml_value(
        value: &crate::velesql::Value,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value> {
        match value {
            crate::velesql::Value::Integer(v) => Ok(serde_json::json!(v)),
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
        value
            .as_i64()
            .ok_or_else(|| Error::Query("id must be an integer".to_string()))
            .and_then(|v| u64::try_from(v).map_err(|_| Error::Query("id must be >= 0".to_string())))
    }

    pub(super) fn json_to_vector(value: &serde_json::Value) -> Result<Vec<f32>> {
        let arr = value
            .as_array()
            .ok_or_else(|| Error::Query("'vector' must be an array of numbers".to_string()))?;
        let mut out = Vec::with_capacity(arr.len());
        for v in arr {
            let f = v
                .as_f64()
                .ok_or_else(|| Error::Query("vector values must be numeric".to_string()))?;
            if !f.is_finite() || f < f64::from(f32::MIN) || f > f64::from(f32::MAX) {
                return Err(Error::Query(
                    "vector values must be finite f32-compatible numbers".to_string(),
                ));
            }
            #[allow(clippy::cast_possible_truncation)]
            let as_f32 = f as f32;
            out.push(as_f32);
        }
        Ok(out)
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

        let filter_condition =
            crate::Collection::extract_metadata_filter(condition).ok_or_else(|| {
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

    pub(super) fn build_join_column_store(collection: &Collection) -> Result<ColumnStore> {
        use crate::column_store::{ColumnType, ColumnValue};

        let ids = collection.all_ids();
        let points: Vec<_> = collection.get(&ids).into_iter().flatten().collect();

        let mut inferred: std::collections::BTreeMap<String, ColumnType> =
            std::collections::BTreeMap::new();
        inferred.insert("id".to_string(), ColumnType::Int);

        for point in &points {
            let Some(payload) = point.payload.as_ref() else {
                continue;
            };
            let Some(obj) = payload.as_object() else {
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

        let schema: Vec<(String, ColumnType)> = inferred.into_iter().collect();
        let schema_refs: Vec<(&str, ColumnType)> = schema
            .iter()
            .map(|(name, ty)| (name.as_str(), *ty))
            .collect();

        let mut store = ColumnStore::with_primary_key(&schema_refs, "id")
            .map_err(|e| Error::ColumnStoreError(e.to_string()))?;
        for point in &points {
            let Ok(pk) = i64::try_from(point.id) else {
                continue;
            };

            let mut values: Vec<(String, ColumnValue)> = Vec::with_capacity(schema.len());
            values.push(("id".to_string(), ColumnValue::Int(pk)));

            if let Some(obj) = point
                .payload
                .as_ref()
                .and_then(serde_json::Value::as_object)
            {
                for (key, value) in obj {
                    if key == "id" {
                        continue;
                    }
                    if !schema_refs.iter().any(|(name, _)| *name == key.as_str()) {
                        continue;
                    }
                    if let Some(column_value) = Self::json_to_column_value(value, &mut store) {
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
        }

        Ok(store)
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
