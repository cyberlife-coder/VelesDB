//! JOIN execution for the WASM VelesQL executor.
//!
//! Every JOIN runs core's algorithm ([`velesdb_core::velesql::hash_join`]):
//! one equality (`ON a.x = b.y` or `USING (x)`) over any column, matched as a
//! WHERE `=` matches, for INNER, LEFT, RIGHT and FULL joins. A joined row is
//! built as core builds it ([`velesdb_core::velesql::scope_base`] /
//! [`velesdb_core::velesql::merge_side`]): the base point's id and score,
//! each side's fields flat and under its alias. The joined rows then go
//! through the SELECT pipeline like any scan — WHERE here, then DISTINCT,
//! ORDER BY, projection and OFFSET / LIMIT in `velesql_select` — so
//! `o.amount`, `c.name` and a self-join's two `name`s read as in core.

use velesdb_core::point::{Point, SearchResult};
use velesdb_core::velesql::{
    base_side, hash_join, scope_base, side_alias, JoinKeys, SelectStatement,
};

use crate::database::DatabaseInner;
use crate::exec_error::ExecError;
use crate::velesql_scan::{scan_all, OwnedScanRow};
use crate::velesql_value::Params;
use crate::velesql_where;

/// The joined rows of a SELECT with JOIN clauses that its WHERE holds for, in
/// base order.
pub(crate) fn joined_rows(
    db: &DatabaseInner,
    stmt: &SelectStatement,
    params: &Params,
) -> Result<Vec<OwnedScanRow>, ExecError> {
    let from_alias = base_side(stmt);
    let mut rows: Vec<SearchResult> = scan_all(db, &stmt.from, None, params)?
        .into_iter()
        .map(|row| {
            SearchResult::new(
                Point::metadata_only(row.id, scope_base(row.payload, from_alias, row.id)),
                row.score,
            )
        })
        .collect();
    let mut prior_aliases = vec![from_alias];
    for join in &stmt.joins {
        let keys =
            JoinKeys::of(join).ok_or_else(|| velesdb_core::Error::join_without_key(&join.table))?;
        let side: Vec<Point> = scan_all(db, &join.table, None, params)?
            .into_iter()
            .map(|row| Point::metadata_only(row.id, row.payload.unwrap_or_default()))
            .collect();
        rows = hash_join(rows, &side, join, &keys, &prior_aliases, usize::MAX);
        prior_aliases.push(side_alias(join.alias.as_deref(), &join.table));
    }
    held_rows(rows, stmt, params)
}

/// The joined `rows` `stmt`'s WHERE holds for, in order.
fn held_rows(
    rows: Vec<SearchResult>,
    stmt: &SelectStatement,
    params: &Params,
) -> Result<Vec<OwnedScanRow>, ExecError> {
    let mut kept = Vec::with_capacity(rows.len());
    for row in rows {
        let row = OwnedScanRow::new(row.point.id, row.score, row.point.payload);
        let holds = match &stmt.where_clause {
            Some(cond) => velesql_where::matches(cond, row.id, row.payload.as_ref(), params)?,
            None => true,
        };
        if holds {
            kept.push(row);
        }
    }
    Ok(kept)
}

#[cfg(test)]
#[path = "velesql_join_tests.rs"]
mod tests;
