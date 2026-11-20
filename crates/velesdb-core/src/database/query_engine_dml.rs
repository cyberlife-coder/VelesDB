//! DML execution helpers for INSERT, UPSERT, and UPDATE statements.
//!
//! Extracted from `query_engine.rs` to keep that module focused on
//! query dispatch, plan caching, and SELECT execution.

use crate::{Result, SearchResult};

use super::Database;

impl Database {
    /// Executes an INSERT or UPSERT statement (single or multi-row).
    ///
    /// `operation` is `"INSERT"` or `"UPSERT"`, reported to the observer's
    /// `on_dml_mutation_request` veto hook before any write. Each row is
    /// bound against the collection's `vector` column
    /// (`CollectionConfig::vector_column`): a graph with embeddings takes a row
    /// without one as a node without an embedding.
    pub(super) fn execute_insert(
        &self,
        stmt: &crate::velesql::InsertStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
        operation: &str,
    ) -> Result<Vec<SearchResult>> {
        self.check_dml_mutation(operation, &stmt.table)?;
        let collection = self.resolve_writable_collection(&stmt.table)?;

        let config = collection.config();
        let column = config.vector_column();
        let points = stmt
            .rows
            .iter()
            .map(|row| {
                crate::velesql::bind_insert_row(&stmt.columns, row, params)?.into_point(column)
            })
            .collect::<Result<Vec<_>>>()?;
        Self::upsert_and_collect(&collection, points)
    }

    /// Executes an UPDATE statement: rewrites the rows its WHERE selects
    /// (`Collection::rows_where`, the rows a SELECT with that WHERE reads).
    ///
    /// The WHERE's shape is checked first, then the `SET` list and the WHERE
    /// are bound — each once — before any row is read, so an unbound `$name`
    /// refuses the statement whatever its WHERE matches.
    pub(super) fn execute_update(
        &self,
        stmt: &crate::velesql::UpdateStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        self.check_dml_mutation("UPDATE", &stmt.table)?;
        let collection = self.resolve_writable_collection(&stmt.table)?;

        crate::velesql::check_dml_where("UPDATE", stmt.where_clause.as_ref())?;
        let assignments = crate::velesql::bind_update_assignments(&stmt.assignments, params)?;
        let filter = stmt
            .where_clause
            .as_ref()
            .map(|condition| crate::velesql::bind_condition(condition, params))
            .transpose()?
            .map(|bound| Self::build_dml_filter("UPDATE", &bound))
            .transpose()?;
        let config = collection.config();
        let column = config.vector_column();
        let updated_points = collection
            .rows_where(filter.as_ref())
            .iter()
            .map(|point| assignments.apply(point, column))
            .collect::<Result<Vec<_>>>()?;
        Self::upsert_and_collect(&collection, updated_points)
    }

    /// Writes rows through `Collection::upsert` — which writes each row by
    /// what it stores and reports the write to the observer itself
    /// (`collection::write_telemetry`) — then reads back, in statement order,
    /// what the id of each row written stores once the write returned, as a
    /// SELECT reads it (`Collection::get`). A row is not always what it
    /// leaves stored: one without a vector keeps the embedding a graph node
    /// has. The read is a second step, so what it returns is exactly what
    /// `get` answers then: a row already TTL-expired, or deleted by a
    /// concurrent write in between, is left out; an id the statement wrote
    /// twice is returned twice, both times as it is finally stored; and every
    /// row returned carries no sparse vectors (`get` reads none).
    fn upsert_and_collect(
        collection: &crate::collection::Collection,
        points: Vec<crate::Point>,
    ) -> Result<Vec<SearchResult>> {
        if points.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<u64> = points.iter().map(|point| point.id).collect();
        collection.upsert(points)?;
        Ok(collection
            .get(&ids)
            .into_iter()
            .flatten()
            .map(|point| SearchResult::new(point, 0.0))
            .collect())
    }
}
