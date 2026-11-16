//! Compound queries — `UNION`, `UNION ALL`, `INTERSECT`, `EXCEPT` — as every
//! engine runs them: core's `Collection` and `Database` executors and the WASM
//! executor.
//!
//! - **Operands.** `a OP b OP c` applies the operators left to right. An
//!   operand other than the last keeps its own `ORDER BY` / `LIMIT` /
//!   `OFFSET`; without a `LIMIT` it is not capped (up to the engine's ceiling).
//! - **Window.** As in SQL, the `ORDER BY` / `LIMIT` / `OFFSET` written after
//!   the last operand window the combined rows; the last operand itself runs
//!   unwindowed. A compound has no implicit `LIMIT` (the exception
//!   [`super::DEFAULT_SELECT_LIMIT`] documents): without one, every combined
//!   row up to the engine's ceiling.
//! - **Columns.** Positional, named by the first operand: a later operand's
//!   listed column `i` lands under the first operand's column `i`, so the
//!   first operand's projection and the window's `ORDER BY` read it.
//! - **Identity.** Two rows are the same row when their projected values are:
//!   the listed columns, compared by position, or the id and payload for `*`
//!   ([`row_key`]), window outputs included. `UNION`, `INTERSECT` and
//!   `EXCEPT` return distinct rows; `UNION ALL` keeps every row. A row met
//!   twice keeps its first position and the better of its scores, with that
//!   score's breakdown (`vector_score`, ...).
//! - **Ceiling.** Every operand and the window are held to the engine's
//!   ceiling, a written `LIMIT` too.
//! - **Order.** The window's `ORDER BY` when there is one; otherwise the rows
//!   rank by score in the metric's direction (most similar first, closest
//!   first under a distance), ties keeping the order they were met in.

use std::collections::HashMap;

use crate::point::SearchResult;

use super::distinct_evaluator::{compute_distinct_key, key_columns};
use super::{Query, SelectColumns, SelectOrderBy, SelectStatement, SetOperator};

/// A row with its identity under its operand's projection.
type KeyedRow = (String, SearchResult);

/// The window the last operand's clauses put on the combined rows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompoundWindow<'q> {
    /// `ORDER BY` of the combined rows, if any.
    pub order_by: Option<&'q [SelectOrderBy]>,
    /// Rows skipped before the `LIMIT`.
    pub offset: u64,
    /// Rows returned at most; `None` when the query writes no `LIMIT`.
    pub limit: Option<u64>,
}

/// The window of `query`'s compound: the clauses after its last operand.
#[must_use]
pub fn compound_window(query: &Query) -> CompoundWindow<'_> {
    let last = last_operand(query);
    CompoundWindow {
        order_by: last.order_by.as_deref(),
        offset: last.offset.unwrap_or(0),
        limit: last.limit,
    }
}

/// The operands of `query`'s compound, each a standalone query an engine runs
/// as it runs any SELECT: the last without its window (it belongs to the
/// compound), and every operand without a `LIMIT` capped at `ceiling`.
#[must_use]
pub fn operand_queries(query: &Query, ceiling: u64) -> Vec<Query> {
    let mut first = query.clone();
    first.compound = None;
    let rest = query
        .compound
        .iter()
        .flat_map(|c| c.operations.iter())
        .map(|(_, select)| Query::new_select(select.clone()));
    let mut operands: Vec<Query> = std::iter::once(first).chain(rest).collect();
    if let Some(last) = operands.last_mut().filter(|_| query.compound.is_some()) {
        last.select.order_by = None;
        last.select.offset = None;
        last.select.limit = None;
    }
    for operand in &mut operands {
        operand.select.limit = Some(operand.select.limit.unwrap_or(ceiling).min(ceiling));
    }
    operands
}

/// The identity of `result` as a row of a SELECT projecting `columns`: the
/// projected values in order, window outputs included, or, for `*`, the id
/// and the full payload.
///
/// An operand's rows come out of the SELECT post-processing, so its window
/// functions are already evaluated into the row — unlike a SELECT's own
/// DISTINCT, which runs before them and so leaves them out of its key.
#[must_use]
pub fn row_key(result: &SearchResult, columns: &SelectColumns) -> String {
    let (mut names, include_score) = key_columns(columns);
    if !names.is_empty() {
        names.extend(window_outputs(columns));
    }
    let values = compute_distinct_key(result, &names, include_score);
    if names.is_empty() && !matches!(columns, SelectColumns::SimilarityScore(_)) {
        // `*` projects the id beside the payload: two points are one row
        // only when both agree.
        format!("{}\x1F{values}", result.point.id)
    } else {
        values
    }
}

/// Runs `query`'s compound: each operand through `run_operand`, combined and
/// windowed as the module docs set out. `order_by` sorts the combined rows by
/// the window's `ORDER BY`; `higher_is_better` is the first operand's metric
/// direction, which ranks them without one.
///
/// # Errors
///
/// The first error an operand or the ordering returns.
pub fn execute_compound<E>(
    query: &Query,
    higher_is_better: bool,
    ceiling: u64,
    mut run_operand: impl FnMut(&Query) -> Result<Vec<SearchResult>, E>,
    order_by: impl FnOnce(&mut [SearchResult], &[SelectOrderBy]) -> Result<(), E>,
) -> Result<Vec<SearchResult>, E> {
    let operands = operand_queries(query, ceiling);
    let operators = query.compound.iter().flat_map(|c| c.operations.iter());
    let mut combined = keyed(run_operand(&operands[0])?, &operands[0].select);
    for ((operator, _), operand) in operators.zip(&operands[1..]) {
        let mut right = keyed(run_operand(operand)?, &operand.select);
        align_columns(
            &mut right,
            &operand.select.columns,
            &operands[0].select.columns,
        );
        combined = combine(combined, right, *operator, higher_is_better);
    }
    let mut rows: Vec<SearchResult> = combined.into_iter().map(|(_, row)| row).collect();
    let window = compound_window(query);
    match window.order_by {
        Some(order) => order_by(&mut rows, order)?,
        None => rank_by_score(&mut rows, higher_is_better),
    }
    let offset = usize::try_from(window.offset).unwrap_or(usize::MAX);
    let limit = usize::try_from(window.limit.unwrap_or(ceiling).min(ceiling)).unwrap_or(usize::MAX);
    Ok(rows.into_iter().skip(offset).take(limit).collect())
}

/// Combines two keyed row sets under `operator`.
fn combine(
    left: Vec<KeyedRow>,
    right: Vec<KeyedRow>,
    operator: SetOperator,
    higher_is_better: bool,
) -> Vec<KeyedRow> {
    match operator {
        SetOperator::UnionAll => left.into_iter().chain(right).collect(),
        SetOperator::Intersect => {
            let right = best_rows(&right, higher_is_better);
            let kept = left
                .into_iter()
                .filter_map(|(key, mut row)| {
                    adopt_if_better(&mut row, right.get(key.as_str())?, higher_is_better);
                    Some((key, row))
                })
                .collect();
            distinct(kept, higher_is_better)
        }
        SetOperator::Except => {
            let right = best_rows(&right, higher_is_better);
            let kept = left
                .into_iter()
                .filter(|(key, _)| !right.contains_key(key.as_str()))
                .collect();
            distinct(kept, higher_is_better)
        }
        // `Union`, and any operator added later, take set semantics.
        _ => distinct(left.into_iter().chain(right).collect(), higher_is_better),
    }
}

/// Each key once, at its first position, with the best of its scores.
fn distinct(rows: Vec<KeyedRow>, higher_is_better: bool) -> Vec<KeyedRow> {
    let mut position: HashMap<String, usize> = HashMap::with_capacity(rows.len());
    let mut out: Vec<KeyedRow> = Vec::with_capacity(rows.len());
    for (key, row) in rows {
        if let Some(&at) = position.get(&key) {
            adopt_if_better(&mut out[at].1, &row, higher_is_better);
        } else {
            position.insert(key.clone(), out.len());
            out.push((key, row));
        }
    }
    out
}

/// Each key's best-scored row among `rows`.
fn best_rows(rows: &[KeyedRow], higher_is_better: bool) -> HashMap<&str, &SearchResult> {
    let mut best: HashMap<&str, &SearchResult> = HashMap::with_capacity(rows.len());
    for (key, row) in rows {
        best.entry(key.as_str())
            .and_modify(|kept| {
                if is_better(row.score, kept.score, higher_is_better) {
                    *kept = row;
                }
            })
            .or_insert(row);
    }
    best
}

/// `kept` takes `other`'s score, and its breakdown, when it is better: the
/// row keeps its own position and projected values.
fn adopt_if_better(kept: &mut SearchResult, other: &SearchResult, higher_is_better: bool) {
    if is_better(other.score, kept.score, higher_is_better) {
        kept.score = other.score;
        kept.component_scores.clone_from(&other.component_scores);
    }
}

/// Whether score `a` beats `b` in the metric's direction.
fn is_better(a: f32, b: f32, higher_is_better: bool) -> bool {
    if higher_is_better {
        a > b
    } else {
        a < b
    }
}

/// The names a SELECT's window functions write their outputs under.
fn window_outputs(columns: &SelectColumns) -> impl Iterator<Item = String> + '_ {
    let functions = match columns {
        SelectColumns::Mixed {
            window_functions, ..
        } => window_functions.as_slice(),
        _ => &[],
    };
    functions.iter().map(|wf| {
        wf.alias
            .clone()
            .unwrap_or_else(|| wf.function_type.default_alias().to_string())
    })
}

/// Stable sort, best score first in the metric's direction.
fn rank_by_score(rows: &mut [SearchResult], higher_is_better: bool) {
    rows.sort_by(|a, b| {
        let ascending = a.score.total_cmp(&b.score);
        if higher_is_better {
            ascending.reverse()
        } else {
            ascending
        }
    });
}

/// Writes each row's listed column `i` under the first operand's column `i`
/// (a compound's columns are positional). Rows of a `*` operand, or under a
/// first operand's `id`, which reads the point id, are left as they are.
fn align_columns(rows: &mut [KeyedRow], columns: &SelectColumns, first: &SelectColumns) {
    let (Some(own), Some(first)) = (listed_columns(columns), listed_columns(first)) else {
        return;
    };
    let renames: Vec<(&str, &str)> = own
        .iter()
        .zip(first)
        .map(|(own, first)| (own.name.as_str(), first.name.as_str()))
        .filter(|(own, first)| own != first && *first != "id")
        .collect();
    if renames.is_empty() {
        return;
    }
    for (_, row) in rows {
        let values: Vec<(&str, serde_json::Value)> = renames
            .iter()
            .map(|(own, first)| (*first, column_value(row, own)))
            .collect();
        let mut payload = match row.point.payload.take() {
            Some(serde_json::Value::Object(map)) => map,
            _ => serde_json::Map::new(),
        };
        for (first, value) in values {
            payload.insert(first.to_string(), value);
        }
        row.point.payload = Some(serde_json::Value::Object(payload));
    }
}

/// The value a projection reads for `column`: the point id for `id`, else the
/// payload's field (by path), `null` when absent.
fn column_value(row: &SearchResult, column: &str) -> serde_json::Value {
    if column == "id" {
        return serde_json::Value::from(row.point.id);
    }
    row.point
        .payload
        .as_ref()
        .and_then(|p| super::grouping::payload_path(p, column))
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

/// The columns a SELECT lists by name, if it lists any.
fn listed_columns(columns: &SelectColumns) -> Option<&[super::Column]> {
    match columns {
        SelectColumns::Columns(cols) | SelectColumns::Mixed { columns: cols, .. }
            if !cols.is_empty() =>
        {
            Some(cols)
        }
        _ => None,
    }
}

/// Keys each row of an operand by its projection.
fn keyed(rows: Vec<SearchResult>, select: &SelectStatement) -> Vec<KeyedRow> {
    rows.into_iter()
        .map(|row| (row_key(&row, &select.columns), row))
        .collect()
}

/// The last operand of `query` (the query itself without a compound).
fn last_operand(query: &Query) -> &SelectStatement {
    query
        .compound
        .as_ref()
        .and_then(|c| c.operations.last())
        .map_or(&query.select, |(_, select)| select)
}

#[cfg(test)]
#[path = "set_operation_tests.rs"]
mod tests;
