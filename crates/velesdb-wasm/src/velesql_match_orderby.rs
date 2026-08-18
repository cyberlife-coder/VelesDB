//! MATCH `RETURN ... ORDER BY` sorting for the WASM executor.
//!
//! Mirrors core's `order_match_results` semantics
//! (`match_exec/order_by.rs` + `similarity.rs`): a deterministic baseline keyed
//! by the row's bound node ids, then each ORDER BY key applied as a STABLE sort
//! least-significant-first (`.rev()`), so multi-key ordering is correct and ties
//! resolve to a total, deterministic order. Sorting happens BEFORE the LIMIT
//! truncation (the caller collects the full candidate set first).
//!
//! Supported keys (the forms the WASM in-memory graph path can evaluate
//! exactly): `depth` (constant within a single pattern, so a defined no-op) and
//! an `alias.property` path. `similarity()` / `similarity(field, $v)`,
//! arithmetic, and aggregate expressions need vector / score-context evaluation
//! the WASM MATCH path does not materialize (it performs no vector scoring), so
//! they are rejected with a clear error rather than silently mis-ordered.

use std::cmp::Ordering;

use velesdb_core::velesql::{OrderByExpr, OrderByItem};

use crate::velesql_result::QueryResultRow;

/// A MATCH result row paired with the data needed to order it.
pub(crate) struct MatchCandidate {
    /// The row's bound node ids in pattern order (`[a]`, `[a, b]`, `[a, b, c]`).
    /// Each matched row has a distinct tuple, so this is a per-row-unique
    /// deterministic tie-break baseline (a TOTAL order) — mirroring core keying
    /// its baseline on the full match identity rather than the anchor alone
    /// (anchor `a` repeats across the many `b`/`c` of a star pattern).
    pub baseline: Vec<u64>,
    /// Alias-keyed row JSON (e.g. `{"a": {...}, "b": {...}}`), used to resolve
    /// `alias.property` ORDER BY keys via `get_nested_field`.
    pub value: serde_json::Value,
    /// The serialized row returned to the caller after sorting.
    pub row: QueryResultRow,
}

/// Sorts MATCH candidates in place per the RETURN `ORDER BY`, then truncates to
/// `limit`. With no ORDER BY, only the limit is applied (traversal order
/// preserved) — matching the prior behavior for that case.
///
/// # Errors
///
/// Returns an error for ORDER BY forms the WASM MATCH path cannot evaluate
/// (`similarity()`, `similarity(field, $v)`, arithmetic, aggregate).
pub(crate) fn order_and_limit(
    candidates: &mut Vec<MatchCandidate>,
    order_by: Option<&[OrderByItem]>,
    limit: Option<u64>,
) -> Result<(), String> {
    if let Some(items) = order_by {
        // Deterministic baseline: the bound-node-id tuple is unique per row, so
        // it gives the stable per-key sorts below a total tie-break order.
        candidates.sort_by(|a, b| a.baseline.cmp(&b.baseline));
        // Apply least-significant key first so the most-significant wins.
        for item in items.iter().rev() {
            sort_by_key(candidates, &item.expr, item.descending)?;
        }
    }
    if let Some(limit) = limit {
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        candidates.truncate(limit);
    }
    Ok(())
}

/// Applies one STABLE ORDER BY key.
fn sort_by_key(
    candidates: &mut [MatchCandidate],
    expr: &OrderByExpr,
    descending: bool,
) -> Result<(), String> {
    match expr {
        OrderByExpr::Field(f) if f == "depth" => {
            // WASM patterns have a fixed depth per query, so this is a defined
            // no-op within a single pattern — kept for parity, not silently
            // dropped.
            Ok(())
        }
        OrderByExpr::Field(f) => sort_by_property(candidates, f, descending),
        OrderByExpr::SimilarityBare => {
            reject_unsupported("similarity() (the browser MATCH path performs no vector scoring)")
        }
        OrderByExpr::Similarity(_) => reject_unsupported("similarity(field, $v)"),
        OrderByExpr::Arithmetic(_) => reject_unsupported("arithmetic expressions"),
        _ => reject_unsupported("aggregate expressions"),
    }
}

/// Uniform rejection for ORDER BY forms the WASM MATCH path cannot evaluate
/// (it materializes no vector scores; only `depth` and `alias.property` work).
fn reject_unsupported(form: &str) -> Result<(), String> {
    Err(format!(
        "MATCH ORDER BY {form} is not supported in WASM (use depth or alias.property)"
    ))
}

/// Sorts by an `alias.property` JSON path (dot-nested), nulls last in ASC.
fn sort_by_property(
    candidates: &mut [MatchCandidate],
    path: &str,
    descending: bool,
) -> Result<(), String> {
    if !path.contains('.') {
        return reject_unsupported(&format!("expression '{path}'"));
    }
    sort_stable(candidates, descending, |a, b| {
        let va = crate::filter::get_nested_field(&a.value, path);
        let vb = crate::filter::get_nested_field(&b.value, path);
        crate::velesql_orderby::compare_json_with_nulls(va, vb)
    });
    Ok(())
}

/// STABLE sort applying the comparison and the ASC/DESC direction.
fn sort_stable<F>(candidates: &mut [MatchCandidate], descending: bool, cmp: F)
where
    F: Fn(&MatchCandidate, &MatchCandidate) -> Ordering,
{
    candidates.sort_by(|a, b| {
        let ord = cmp(a, b);
        if descending {
            ord.reverse()
        } else {
            ord
        }
    });
}

#[cfg(test)]
#[path = "velesql_match_orderby_tests.rs"]
mod tests;
