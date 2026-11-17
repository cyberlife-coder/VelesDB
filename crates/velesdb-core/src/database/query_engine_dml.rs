//! DML execution helpers for INSERT, UPSERT, and UPDATE statements.
//!
//! Extracted from `query_engine.rs` to keep that module focused on
//! query dispatch, plan caching, and SELECT execution.

use crate::{Error, Result, SearchResult};

use super::Database;

impl Database {
    /// Executes an INSERT or UPSERT statement (single or multi-row).
    ///
    /// `operation` is `"INSERT"` or `"UPSERT"`, reported to the observer's
    /// `on_dml_mutation_request` veto hook before any write.
    pub(super) fn execute_insert(
        &self,
        stmt: &crate::velesql::InsertStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
        operation: &str,
    ) -> Result<Vec<SearchResult>> {
        self.check_dml_mutation(operation, &stmt.table)?;
        let collection = self.resolve_writable_collection(&stmt.table)?;

        let mut points = Vec::with_capacity(stmt.rows.len());
        for row in &stmt.rows {
            let (id, vector, payload) = Self::resolve_insert_row(&stmt.columns, row, params)?;
            let point_id =
                id.ok_or_else(|| Error::Query("INSERT requires integer 'id' column".to_string()))?;
            points.push(Self::build_insert_point(
                &collection,
                point_id,
                vector,
                payload,
            )?);
        }

        let results: Vec<SearchResult> = points
            .iter()
            .map(|p| SearchResult::new(p.clone(), 0.0))
            .collect();
        // `on_upsert` fires inside `Collection::upsert`, once, as for every
        // write to a registered collection (`collection::write_telemetry`).
        collection.upsert(points)?;
        Ok(results)
    }

    /// Resolves column values from a single row into id, vector, and payload fields.
    #[allow(clippy::type_complexity)] // Reason: one-off tuple return for internal helper.
    fn resolve_insert_row(
        columns: &[String],
        row: &[crate::velesql::Value],
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<(
        Option<u64>,
        Option<Vec<f32>>,
        serde_json::Map<String, serde_json::Value>,
    )> {
        let mut id: Option<u64> = None;
        let mut payload = serde_json::Map::new();
        let mut vector: Option<Vec<f32>> = None;

        for (column, value_expr) in columns.iter().zip(row) {
            let resolved = Self::resolve_dml_value(value_expr, params)?;
            if column == "id" {
                id = Some(Self::json_to_u64_id(&resolved)?);
                continue;
            }
            if column == "vector" {
                vector = Some(Self::json_to_vector(&resolved)?);
                continue;
            }
            payload.insert(column.clone(), resolved);
        }

        Ok((id, vector, payload))
    }

    /// Builds a `Point` for an INSERT statement, validating vector presence.
    fn build_insert_point(
        collection: &crate::collection::Collection,
        point_id: u64,
        vector: Option<Vec<f32>>,
        payload: serde_json::Map<String, serde_json::Value>,
    ) -> Result<crate::Point> {
        if collection.is_metadata_only() {
            if vector.is_some() {
                return Err(Error::Query(
                    "INSERT on metadata-only collection cannot set 'vector'".to_string(),
                ));
            }
            Ok(crate::Point::metadata_only(
                point_id,
                serde_json::Value::Object(payload),
            ))
        } else {
            let vec_value = vector.ok_or_else(|| {
                Error::Query("INSERT on vector collection requires 'vector' column".to_string())
            })?;
            Ok(crate::Point::new(
                point_id,
                vec_value,
                Some(serde_json::Value::Object(payload)),
            ))
        }
    }

    /// Executes an UPDATE statement.
    pub(super) fn execute_update(
        &self,
        stmt: &crate::velesql::UpdateStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        self.check_dml_mutation("UPDATE", &stmt.table)?;
        let collection = self.resolve_writable_collection(&stmt.table)?;

        let assignments = Self::resolve_update_assignments(stmt, params)?;
        let filter = Self::build_dml_filter("UPDATE", stmt.where_clause.as_ref(), params)?;
        let rows = Self::matching_points(&collection, filter.as_ref());
        let updated_points = rows
            .iter()
            .map(|point| Self::apply_assignments_to_point(&collection, point, &assignments))
            .collect::<Result<Vec<_>>>()?;

        // An UPDATE writes via `Collection::upsert`, which reports the write to
        // the observer itself (`collection::write_telemetry`).
        Self::upsert_and_collect(&collection, updated_points)
    }

    /// Resolves and validates UPDATE assignment values.
    fn resolve_update_assignments(
        stmt: &crate::velesql::UpdateStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<(String, serde_json::Value)>> {
        let assignments = stmt
            .assignments
            .iter()
            .map(|a| Ok((a.column.clone(), Self::resolve_dml_value(&a.value, params)?)))
            .collect::<Result<Vec<_>>>()?;

        if assignments.iter().any(|(name, _)| name == "id") {
            return Err(Error::Query(
                "UPDATE cannot modify primary key column 'id'".to_string(),
            ));
        }
        Ok(assignments)
    }

    /// Upserts updated points and returns them as search results.
    fn upsert_and_collect(
        collection: &crate::collection::Collection,
        updated_points: Vec<crate::Point>,
    ) -> Result<Vec<SearchResult>> {
        if updated_points.is_empty() {
            return Ok(Vec::new());
        }
        let results = updated_points
            .iter()
            .map(|p| SearchResult::new(p.clone(), 0.0))
            .collect();
        collection.upsert(updated_points)?;
        Ok(results)
    }

    /// The points a DML WHERE `filter` selects — the rows UPDATE writes and a
    /// general DELETE removes. `WHERE id = n` / `id IN (...)` name their
    /// points: those are read, not the whole collection. `get` reads the scan
    /// universe — in a vector collection a point must carry its vector — so a
    /// DML statement touches exactly the rows a SELECT with the same WHERE
    /// sees.
    pub(super) fn matching_points(
        collection: &crate::collection::Collection,
        filter: Option<&crate::Filter>,
    ) -> Vec<crate::Point> {
        let ids = filter
            .and_then(|f| f.condition.point_ids())
            .unwrap_or_else(|| collection.all_point_ids());
        collection
            .get(&ids)
            .into_iter()
            .flatten()
            .filter(|point| Self::matches_dml_filter(point, filter))
            .collect()
    }

    /// Applies field assignments to a single point, producing the updated point.
    fn apply_assignments_to_point(
        collection: &crate::collection::Collection,
        point: &crate::Point,
        assignments: &[(String, serde_json::Value)],
    ) -> Result<crate::Point> {
        let mut payload_map = point
            .payload
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mut updated_vector = point.vector.clone();

        for (field, value) in assignments {
            if field == "vector" {
                if collection.is_metadata_only() {
                    return Err(Error::Query(
                        "UPDATE on metadata-only collection cannot set 'vector'".to_string(),
                    ));
                }
                updated_vector = Self::json_to_vector(value)?;
            } else {
                payload_map.insert(field.clone(), value.clone());
            }
        }

        Ok(if collection.is_metadata_only() {
            crate::Point::metadata_only(point.id, serde_json::Value::Object(payload_map))
        } else {
            crate::Point::new(
                point.id,
                updated_vector,
                Some(serde_json::Value::Object(payload_map)),
            )
        })
    }
}
