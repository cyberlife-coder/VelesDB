//! Database-level aggregation entry point.
//!
//! Extracted from `query_engine.rs` (file NLOC budget) so every surface — the
//! server `/query` handler, the CLI REPL, and future SDK consumers — can route
//! `GROUP BY` / scalar-aggregate queries through one method instead of each
//! re-implementing collection resolution.

use std::collections::HashMap;

use super::Database;
use crate::{Error, Result};

impl Database {
    /// Executes a `GROUP BY` / scalar-aggregate SELECT, returning the aggregate
    /// result as JSON (a single object for scalar aggregates, an array of group
    /// objects for `GROUP BY`).
    ///
    /// The target collection is resolved from the query's `FROM` clause, falling
    /// back to a `"_collection"` key in `params` (the convention the REPL/SDK use
    /// to inject the active collection). Callers should gate on
    /// [`crate::velesql::SelectStatement::is_aggregation_query`] — non-aggregate
    /// SELECTs belong on [`Database::execute_query`].
    ///
    /// # Errors
    ///
    /// Returns an error if validation fails, the collection cannot be resolved,
    /// or aggregation execution fails.
    pub fn execute_aggregate(
        &self,
        query: &crate::velesql::Query,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value> {
        crate::velesql::QueryValidator::validate(query).map_err(|e| Error::Query(e.to_string()))?;
        let name = aggregate_target_collection(query, params)?;
        let collection = self
            .get_any_collection(&name)
            .ok_or(Error::CollectionNotFound(name))?;
        collection.execute_aggregate(query, params)
    }
}

/// Resolves the target collection name for an aggregation query: the `FROM`
/// clause, else a `"_collection"` param, else a guidance error.
fn aggregate_target_collection(
    query: &crate::velesql::Query,
    params: &HashMap<String, serde_json::Value>,
) -> Result<String> {
    if !query.select.from.is_empty() {
        return Ok(query.select.from.clone());
    }
    if let Some(serde_json::Value::String(name)) = params.get("_collection") {
        return Ok(name.clone());
    }
    Err(Error::Query(
        "aggregation query requires a target collection. Use SELECT ... FROM \
         <collection> ... GROUP BY, or pass {\"_collection\": \"name\"} in params."
            .to_string(),
    ))
}
