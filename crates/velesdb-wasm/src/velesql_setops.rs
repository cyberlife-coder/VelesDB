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

use std::collections::HashSet;

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

/// Equivalence key for set-operator deduplication.
///
/// Using `(id, canonical_json)` matches Postgres' "DISTINCT by tuple"
/// semantics: two rows with the same id but different payloads remain
/// distinct, while two rows with identical id + payload collapse to one.
type RowKey = (u64, String);

fn row_key(row: &QueryResultRow) -> RowKey {
    (row.id(), row.data_json())
}

/// Deduplicate rows while preserving first-seen order.
///
/// Finding F11: previous implementation used `Vec::contains` (O(n) per
/// lookup), yielding O(n^2) overall. `HashSet` insertion is O(1) amortized
/// and `HashSet::insert` returns `false` when the key was already present,
/// so we push to the output only on fresh insertions — order-preserving
/// and O(n) total.
fn dedup(rows: Vec<QueryResultRow>) -> Vec<QueryResultRow> {
    let mut seen: HashSet<RowKey> = HashSet::with_capacity(rows.len());
    let mut out: Vec<QueryResultRow> = Vec::with_capacity(rows.len());
    for row in rows {
        if seen.insert(row_key(&row)) {
            out.push(row);
        }
    }
    out
}

/// Intersection preserving left-side first-seen order.
///
/// Finding F11: replaced `Vec::contains` with `HashSet::contains`, and
/// `seen` Vec with a `HashSet`. Right keys are pre-collected once; the
/// outer walk over `left` is O(n) with O(1) membership checks.
fn intersect(left: Vec<QueryResultRow>, right: Vec<QueryResultRow>) -> Vec<QueryResultRow> {
    let right_keys: HashSet<RowKey> = right.iter().map(row_key).collect();
    let mut seen: HashSet<RowKey> = HashSet::new();
    let mut out = Vec::new();
    for row in left {
        let key = row_key(&row);
        if right_keys.contains(&key) && seen.insert(key) {
            out.push(row);
        }
    }
    out
}

/// Set-difference preserving left-side first-seen order.
///
/// Finding F11: same O(n^2) → O(n) rewrite as `intersect`; only the
/// polarity of the membership check changes.
fn except(left: Vec<QueryResultRow>, right: Vec<QueryResultRow>) -> Vec<QueryResultRow> {
    let right_keys: HashSet<RowKey> = right.iter().map(row_key).collect();
    let mut seen: HashSet<RowKey> = HashSet::new();
    let mut out = Vec::new();
    for row in left {
        let key = row_key(&row);
        if !right_keys.contains(&key) && seen.insert(key) {
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

    // --- Finding F11: O(n) dedup preserves first-seen order ---------------

    #[test]
    fn test_union_preserves_first_seen_order() {
        // First-seen order matters: UNION of [1,2,3] with [2,4] must
        // yield [1,2,3,4] — not [1,4,2,3] (HashMap iteration order).
        let left = vec![row(1), row(2), row(3)];
        let right = vec![row(2), row(4)];
        let out = combine(SetOperator::Union, left, right).expect("test: union order");
        let ids: Vec<u64> = out.iter().map(QueryResultRow::id).collect();
        assert_eq!(ids, vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_intersect_preserves_left_order() {
        let left = vec![row(3), row(1), row(2)];
        let right = vec![row(1), row(2), row(3)];
        let out = combine(SetOperator::Intersect, left, right).expect("test: intersect order");
        let ids: Vec<u64> = out.iter().map(QueryResultRow::id).collect();
        // Order must match the left-hand walk.
        assert_eq!(ids, vec![3, 1, 2]);
    }

    #[test]
    fn test_except_preserves_left_order() {
        let left = vec![row(4), row(2), row(3), row(1)];
        let right = vec![row(2)];
        let out = combine(SetOperator::Except, left, right).expect("test: except order");
        let ids: Vec<u64> = out.iter().map(QueryResultRow::id).collect();
        assert_eq!(ids, vec![4, 3, 1]);
    }

    #[test]
    fn test_dedup_dedups_repeated_rows() {
        let rows = vec![row(1), row(2), row(1), row(3), row(2)];
        let out = combine(
            SetOperator::Union,
            rows,
            // UNION with empty to trigger dedup branch.
            Vec::new(),
        )
        .expect("test: dedup");
        let ids: Vec<u64> = out.iter().map(QueryResultRow::id).collect();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn test_setops_handle_large_inputs() {
        // Regression: the previous O(n^2) implementation required ~2s for
        // 2000 rows. The hash-set path handles this in milliseconds. The
        // test here asserts correctness at N=1000; a regression would not
        // fail the assertion but would slow `cargo test` noticeably.
        let left: Vec<QueryResultRow> = (0..1000u64).map(row).collect();
        let right: Vec<QueryResultRow> = (500..1500u64).map(row).collect();
        let out = combine(SetOperator::Union, left, right).expect("test: large-union");
        assert_eq!(out.len(), 1500);
    }
}
