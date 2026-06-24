//! Scalar subquery resolution (EPIC-039).
//!
//! A scalar subquery `(SELECT AVG(amount) FROM t)` in a `WHERE`/`HAVING`
//! predicate — or an `INSERT`/`UPDATE` value — is *parsed* as a
//! [`Value::Subquery`](crate::velesql::Value::Subquery) leaf. Before the outer
//! query is validated and executed, this module walks the AST, executes each
//! inner `SELECT`, reduces it to a single row / single column scalar, and
//! substitutes the resulting literal in place. The downstream filter,
//! aggregation, and DML paths then see only literals.
//!
//! Cardinality contract:
//! - **> 1 row** or **> 1 column** -> [`Error::Query`] with a cardinality message.
//! - **0 rows** -> `NULL` (a comparison against `NULL` is never true).
//!
//! Correlated subqueries (those that reference an outer column) are **not**
//! supported and are rejected with a clear message.

use std::collections::HashMap;

use crate::velesql::{
    Condition, DmlStatement, HavingClause, Query, SelectColumns, Subquery, Value,
};
use crate::{Error, Result};

use super::Database;

/// Maximum scalar-subquery nesting depth. Guards against runaway recursion on
/// deeply (or pathologically) nested subqueries.
const MAX_SUBQUERY_DEPTH: u32 = 8;

impl Database {
    /// Resolves every **scalar (non-correlated)** subquery in `query`, returning
    /// a rewritten clone when at least one such subquery was present, or `None`
    /// when there is nothing to resolve (the common path — no clone, no work).
    ///
    /// Correlated subqueries are intentionally left in place so the validator
    /// rejects them with the canonical V010 message; they are not resolvable.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Query`] if a subquery violates the one-row/one-column
    /// cardinality contract or its inner SELECT fails.
    pub(super) fn resolve_subqueries(
        &self,
        query: &Query,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<Option<Query>> {
        if !query_has_resolvable_subquery(query) {
            return Ok(None);
        }
        let mut rewritten = query.clone();
        self.rewrite_query_subqueries(&mut rewritten, params, 0)?;
        Ok(Some(rewritten))
    }

    /// Rewrites all subquery leaves of `query` in place at recursion `depth`.
    fn rewrite_query_subqueries(
        &self,
        query: &mut Query,
        params: &HashMap<String, serde_json::Value>,
        depth: u32,
    ) -> Result<()> {
        if let Some(cond) = query.select.where_clause.as_mut() {
            self.rewrite_condition(cond, params, depth)?;
        }
        if let Some(having) = query.select.having.as_mut() {
            self.rewrite_having(having, params, depth)?;
        }
        if let Some(dml) = query.dml.as_mut() {
            self.rewrite_dml(dml, params, depth)?;
        }
        Ok(())
    }

    /// Walks a WHERE condition tree, substituting subquery leaves with scalars.
    fn rewrite_condition(
        &self,
        cond: &mut Condition,
        params: &HashMap<String, serde_json::Value>,
        depth: u32,
    ) -> Result<()> {
        match cond {
            Condition::And(l, r) | Condition::Or(l, r) => {
                self.rewrite_condition(l, params, depth)?;
                self.rewrite_condition(r, params, depth)
            }
            Condition::Group(inner) | Condition::Not(inner) => {
                self.rewrite_condition(inner, params, depth)
            }
            other => self.rewrite_leaf_condition(other, params, depth),
        }
    }

    /// Substitutes subquery values carried directly by a leaf condition.
    fn rewrite_leaf_condition(
        &self,
        cond: &mut Condition,
        params: &HashMap<String, serde_json::Value>,
        depth: u32,
    ) -> Result<()> {
        match cond {
            Condition::Comparison(c) => self.rewrite_value(&mut c.value, params, depth),
            Condition::Between(c) => {
                self.rewrite_value(&mut c.low, params, depth)?;
                self.rewrite_value(&mut c.high, params, depth)
            }
            Condition::In(c) => self.rewrite_values(&mut c.values, params, depth),
            Condition::Contains(c) => self.rewrite_values(&mut c.values, params, depth),
            _ => Ok(()),
        }
    }

    /// Substitutes subquery thresholds in a HAVING clause.
    fn rewrite_having(
        &self,
        having: &mut HavingClause,
        params: &HashMap<String, serde_json::Value>,
        depth: u32,
    ) -> Result<()> {
        for cond in &mut having.conditions {
            self.rewrite_value(&mut cond.value, params, depth)?;
        }
        Ok(())
    }

    /// Substitutes subquery values in INSERT/UPSERT rows and UPDATE assignments.
    fn rewrite_dml(
        &self,
        dml: &mut DmlStatement,
        params: &HashMap<String, serde_json::Value>,
        depth: u32,
    ) -> Result<()> {
        match dml {
            DmlStatement::Insert(s) | DmlStatement::Upsert(s) => {
                for row in &mut s.rows {
                    self.rewrite_values(row, params, depth)?;
                }
                Ok(())
            }
            DmlStatement::Update(s) => {
                for assignment in &mut s.assignments {
                    self.rewrite_value(&mut assignment.value, params, depth)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Substitutes a single value if it is a scalar (non-correlated) subquery.
    ///
    /// Literals are left untouched; correlated subqueries are also left in place
    /// so the validator rejects them with the canonical V010 message.
    fn rewrite_value(
        &self,
        value: &mut Value,
        params: &HashMap<String, serde_json::Value>,
        depth: u32,
    ) -> Result<()> {
        if let Value::Subquery(subquery) = value {
            if subquery.correlations.is_empty() {
                *value = self.execute_scalar_subquery(subquery, params, depth)?;
            }
        }
        Ok(())
    }

    /// Substitutes every subquery in a slice of values.
    fn rewrite_values(
        &self,
        values: &mut [Value],
        params: &HashMap<String, serde_json::Value>,
        depth: u32,
    ) -> Result<()> {
        for value in values {
            self.rewrite_value(value, params, depth)?;
        }
        Ok(())
    }

    /// Executes one inner SELECT and reduces it to a scalar [`Value`].
    fn execute_scalar_subquery(
        &self,
        subquery: &Subquery,
        params: &HashMap<String, serde_json::Value>,
        depth: u32,
    ) -> Result<Value> {
        if depth >= MAX_SUBQUERY_DEPTH {
            return Err(Error::Query(format!(
                "scalar subquery nesting exceeds the maximum depth of {MAX_SUBQUERY_DEPTH}"
            )));
        }
        let mut inner = Query::from_select(subquery.select.clone());
        // Resolve nested subqueries first (depth-bounded recursion).
        self.rewrite_query_subqueries(&mut inner, params, depth + 1)?;
        self.run_scalar_subquery(&inner, params)
    }

    /// Runs a subquery-free inner SELECT and extracts its single scalar value.
    fn run_scalar_subquery(
        &self,
        inner: &Query,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<Value> {
        if inner.select.is_aggregation_query() {
            let json = self.execute_aggregate(inner, params)?;
            return scalar_from_aggregate(&json);
        }
        let column = projected_subquery_column(&inner.select)?;
        let results = self.execute_query(inner, params)?;
        scalar_from_rows(&results, &column)
    }
}

/// Returns `true` if any WHERE/HAVING/DML site of `query` carries a **scalar
/// (non-correlated)** subquery that this module can resolve.
///
/// Correlated subqueries are excluded: they are left in place for the validator
/// to reject (V010), and excluding them here keeps `resolve_subqueries` from
/// returning `Some` (and recursing) when nothing is actually resolvable.
fn query_has_resolvable_subquery(query: &Query) -> bool {
    let where_resolvable = query
        .select
        .where_clause
        .as_ref()
        .is_some_and(condition_has_resolvable_subquery);
    where_resolvable
        || having_has_resolvable_subquery(query)
        || dml_has_resolvable_subquery(query.dml.as_ref())
}

/// Returns `true` if `query` has a scalar (non-correlated) HAVING subquery.
fn having_has_resolvable_subquery(query: &Query) -> bool {
    query.has_having_subquery() && !query.has_correlated_having_subquery()
}

/// Returns `true` if a condition carries a scalar (non-correlated) subquery.
fn condition_has_resolvable_subquery(cond: &Condition) -> bool {
    cond.has_subquery() && !cond.has_correlated_subquery()
}

/// Returns `true` if an INSERT/UPSERT row or UPDATE assignment carries a scalar
/// (non-correlated) subquery. DML subqueries cannot be correlated (no outer row
/// scope), so any subquery here is resolvable.
fn dml_has_resolvable_subquery(dml: Option<&DmlStatement>) -> bool {
    match dml {
        Some(DmlStatement::Insert(s) | DmlStatement::Upsert(s)) => {
            s.rows.iter().any(|row| row.iter().any(Value::is_subquery))
        }
        Some(DmlStatement::Update(s)) => s.assignments.iter().any(|a| a.value.is_subquery()),
        _ => false,
    }
}

/// Identifies the single projected column of a non-aggregate scalar subquery.
///
/// A scalar subquery must project exactly one column; `SELECT *` and multi-
/// column projections violate the one-column cardinality contract.
fn projected_subquery_column(select: &crate::velesql::SelectStatement) -> Result<String> {
    match &select.columns {
        SelectColumns::Columns(cols) if cols.len() == 1 => Ok(cols[0].name.clone()),
        _ => Err(Error::Query(
            "scalar subquery must select exactly one column (e.g. \
             (SELECT amount FROM t WHERE ...) or an aggregate like (SELECT AVG(amount) FROM t))"
                .to_string(),
        )),
    }
}

/// Extracts the single scalar of an aggregate result object (e.g. `{"avg_amount": 30.0}`).
fn scalar_from_aggregate(json: &serde_json::Value) -> Result<Value> {
    let obj = json
        .as_object()
        .ok_or_else(|| Error::Query("scalar subquery aggregate did not return an object".into()))?;
    if obj.len() != 1 {
        return Err(Error::Query(
            "scalar subquery must return exactly one column".to_string(),
        ));
    }
    let only = obj
        .values()
        .next()
        .ok_or_else(|| Error::Query("scalar subquery aggregate returned no column".to_string()))?;
    json_to_value(only)
}

/// Extracts the single scalar of a row-projection subquery, enforcing the
/// one-row contract and returning `NULL` for an empty result.
fn scalar_from_rows(results: &[crate::SearchResult], column: &str) -> Result<Value> {
    match results {
        [] => Ok(Value::Null),
        [single] => {
            let cell = single
                .point
                .payload
                .as_ref()
                .and_then(|p| nested_payload_value(p, column))
                .unwrap_or(&serde_json::Value::Null);
            json_to_value(cell)
        }
        _ => Err(Error::Query(format!(
            "scalar subquery returned {} rows but must return at most one row",
            results.len()
        ))),
    }
}

/// Resolves a dot-separated payload path (e.g. `meta.amount`) to its JSON cell.
fn nested_payload_value<'a>(
    payload: &'a serde_json::Value,
    path: &str,
) -> Option<&'a serde_json::Value> {
    let mut current = payload;
    for part in path.split('.') {
        current = current.as_object()?.get(part)?;
    }
    Some(current)
}

/// Converts a resolved JSON scalar into a `VelesQL` [`Value`] literal.
fn json_to_value(json: &serde_json::Value) -> Result<Value> {
    match json {
        serde_json::Value::Null => Ok(Value::Null),
        serde_json::Value::Bool(b) => Ok(Value::Boolean(*b)),
        serde_json::Value::String(s) => Ok(Value::String(s.clone())),
        serde_json::Value::Number(n) => number_to_value(n),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => Err(Error::Query(
            "scalar subquery column resolved to a non-scalar (array/object) value".to_string(),
        )),
    }
}

/// Converts a JSON number to the narrowest `VelesQL` numeric [`Value`].
fn number_to_value(n: &serde_json::Number) -> Result<Value> {
    if let Some(i) = n.as_i64() {
        return Ok(Value::Integer(i));
    }
    if let Some(u) = n.as_u64() {
        return Ok(Value::UnsignedInteger(u));
    }
    if let Some(f) = n.as_f64() {
        return Ok(Value::Float(f));
    }
    Err(Error::Query(
        "scalar subquery returned an unrepresentable number".to_string(),
    ))
}
