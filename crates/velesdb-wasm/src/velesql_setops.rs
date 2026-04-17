//! Set operations (UNION, UNION ALL, INTERSECT, EXCEPT) for the WASM
//! VelesQL executor (S4-13).
//!
//! Compound queries apply set operators left-to-right. De-duplication uses
//! the row's `(id, canonical_json)` pair as the equivalence key so two rows
//! with the same id but different payloads remain distinct (a correctness
//! property that matches Postgres' "distinct by tuple" semantics).
//!
//! UNION ALL skips the de-dup pass entirely (its whole point is to keep
//! duplicates). INTERSECT / EXCEPT both go through the de-dup path because
//! SQL set semantics require them.

use velesdb_core::velesql::{CompoundQuery, Query, SelectStatement, SetOperator};

use crate::database::DatabaseInner;
use crate::velesql_result::QueryResultRow;
use crate::velesql_value::Params;

/// Executes a compound query: left SELECT combined with every right-hand
/// operand via the declared operator.
pub(crate) fn execute(
    db: &mut DatabaseInner,
    base_query: &Query,
    compound: &CompoundQuery,
    params: &Params,
) -> Result<Vec<QueryResultRow>, String> {
    let mut accumulator = run_select(db, &base_query.select, params)?;
    for (op, right_select) in &compound.operations {
        let rhs = run_select(db, right_select, params)?;
        accumulator = combine(*op, accumulator, rhs)?;
    }
    Ok(accumulator)
}

fn run_select(
    db: &mut DatabaseInner,
    select: &SelectStatement,
    params: &Params,
) -> Result<Vec<QueryResultRow>, String> {
    let mut wrapper = Query::new_select(select.clone());
    wrapper.compound = None;
    crate::velesql_select::execute(db, &wrapper, params)
}

fn combine(
    op: SetOperator,
    left: Vec<QueryResultRow>,
    right: Vec<QueryResultRow>,
) -> Result<Vec<QueryResultRow>, String> {
    match op {
        SetOperator::UnionAll => Ok(concat(left, right)),
        SetOperator::Union => Ok(dedup(concat(left, right))),
        SetOperator::Intersect => Ok(intersect(left, right)),
        SetOperator::Except => Ok(except(left, right)),
        _ => Err(format!("Unsupported set operator in WASM: {op:?}")),
    }
}

fn concat(mut left: Vec<QueryResultRow>, right: Vec<QueryResultRow>) -> Vec<QueryResultRow> {
    left.extend(right);
    left
}

fn dedup(rows: Vec<QueryResultRow>) -> Vec<QueryResultRow> {
    let mut seen: Vec<(u64, String)> = Vec::with_capacity(rows.len());
    let mut out: Vec<QueryResultRow> = Vec::with_capacity(rows.len());
    for row in rows {
        let key = (row.id(), row.data_json());
        if !seen.contains(&key) {
            seen.push(key);
            out.push(row);
        }
    }
    out
}

fn intersect(left: Vec<QueryResultRow>, right: Vec<QueryResultRow>) -> Vec<QueryResultRow> {
    let right_keys: Vec<(u64, String)> = right.iter().map(|r| (r.id(), r.data_json())).collect();
    let mut out = Vec::new();
    let mut seen: Vec<(u64, String)> = Vec::new();
    for row in left {
        let key = (row.id(), row.data_json());
        if right_keys.contains(&key) && !seen.contains(&key) {
            seen.push(key);
            out.push(row);
        }
    }
    out
}

fn except(left: Vec<QueryResultRow>, right: Vec<QueryResultRow>) -> Vec<QueryResultRow> {
    let right_keys: Vec<(u64, String)> = right.iter().map(|r| (r.id(), r.data_json())).collect();
    let mut seen: Vec<(u64, String)> = Vec::new();
    let mut out = Vec::new();
    for row in left {
        let key = (row.id(), row.data_json());
        if !right_keys.contains(&key) && !seen.contains(&key) {
            seen.push(key);
            out.push(row);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: u64) -> QueryResultRow {
        QueryResultRow::build(id, 0.0, None).expect("test: row")
    }

    #[test]
    fn test_union_dedups() {
        let left = vec![row(1), row(2)];
        let right = vec![row(2), row(3)];
        let out = combine(SetOperator::Union, left, right).expect("test: union");
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn test_union_all_keeps_duplicates() {
        let left = vec![row(1), row(2)];
        let right = vec![row(2), row(3)];
        let out = combine(SetOperator::UnionAll, left, right).expect("test: union all");
        assert_eq!(out.len(), 4);
    }

    #[test]
    fn test_intersect_returns_common() {
        let left = vec![row(1), row(2), row(3)];
        let right = vec![row(2), row(3), row(4)];
        let out = combine(SetOperator::Intersect, left, right).expect("test: intersect");
        let ids: Vec<u64> = out.iter().map(QueryResultRow::id).collect();
        assert!(ids.contains(&2));
        assert!(ids.contains(&3));
        assert!(!ids.contains(&1));
    }

    #[test]
    fn test_except_subtracts_right() {
        let left = vec![row(1), row(2), row(3)];
        let right = vec![row(2)];
        let out = combine(SetOperator::Except, left, right).expect("test: except");
        let ids: Vec<u64> = out.iter().map(QueryResultRow::id).collect();
        assert!(ids.contains(&1));
        assert!(ids.contains(&3));
        assert!(!ids.contains(&2));
    }

    #[test]
    fn test_intersect_empty_when_disjoint() {
        let left = vec![row(1)];
        let right = vec![row(2)];
        let out = combine(SetOperator::Intersect, left, right).expect("test: empty intersect");
        assert!(out.is_empty());
    }
}
