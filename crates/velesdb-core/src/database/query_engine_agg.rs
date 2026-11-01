//! Database-level aggregation entry point.
//!
//! Extracted from `query_engine.rs` (file NLOC budget) so every surface — the
//! server `/query` handler, the CLI REPL, and future SDK consumers — can route
//! `GROUP BY` / scalar-aggregate queries through one method instead of each
//! re-implementing collection resolution or the read gate.

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
    /// Like [`Database::execute_query`], it passes the control-plane read gate
    /// before reading anything: a [`Deny`](crate::AccessDecision::Deny) aborts
    /// it with the observer's error, and an
    /// [`AllowWithScope`](crate::AccessDecision::AllowWithScope) filter is
    /// AND-composed into its `WHERE`, so `COUNT`, `SUM`, `AVG`, `MIN`, `MAX`,
    /// `GROUP BY` and `HAVING` see only the scoped rows.
    ///
    /// # Errors
    ///
    /// Returns an error if validation fails, the read gate refuses the read,
    /// the collection cannot be resolved, or aggregation execution fails.
    pub fn execute_aggregate(
        &self,
        query: &crate::velesql::Query,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value> {
        // Resolve scalar subqueries (EPIC-039) in WHERE/HAVING before validation
        // so the aggregate engine sees a subquery-free AST. Each subquery runs
        // through `execute_query` / `execute_aggregate`, so it is decided by
        // the read gate on its own collection.
        if let Some(rewritten) = self.resolve_subqueries(query, params)? {
            return self.execute_aggregate(&rewritten, params);
        }
        crate::velesql::QueryValidator::validate(query).map_err(|e| Error::Query(e.to_string()))?;
        let authorized = self.authorize_statement(query, params)?;
        let collection = self.resolve_read_target(&authorized, AGGREGATE_TARGET_MISSING)?;
        collection.execute_aggregate(authorized.query(), params)
    }
}

/// The refusal of an aggregation query that names no collection.
const AGGREGATE_TARGET_MISSING: &str = "aggregation query requires a target collection. Use \
     SELECT ... FROM <collection> ... GROUP BY, or pass {\"_collection\": \"name\"} in params.";
