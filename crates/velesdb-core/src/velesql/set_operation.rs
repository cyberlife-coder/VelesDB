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

use std::cmp::Ordering;
use std::collections::HashMap;

use crate::distance::rank_scores;
use crate::point::SearchResult;

use super::compound_projection::{preserve_operand_positions, CompoundProjection};
use super::distinct_evaluator::{compute_distinct_key, key_columns, projected_key};
use super::json_path::ProjectedField;
use super::window_evaluator::WindowInputs;
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
    let (listed, include_score) = key_columns(columns);
    if !listed.is_empty() {
        return projected_key(result, listed_fields(columns), include_score);
    }
    let values = compute_distinct_key(result, &[], include_score);
    if matches!(columns, SelectColumns::SimilarityScore(_)) {
        values
    } else {
        // `*` projects the id beside the payload: two points are one row
        // only when both agree.
        format!("{}\x1F{values}", result.point.id)
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
    let mut operands = operand_queries(query, ceiling);
    let operators = query.compound.iter().flat_map(|c| c.operations.iter());
    let first = &query.select.columns;
    let layout = CompoundProjection::for_query(query);
    let mut combined = keyed_operand(
        &mut operands[0],
        first,
        layout.as_ref(),
        false,
        &mut run_operand,
    )?;
    for ((operator, _), operand) in operators.zip(&mut operands[1..]) {
        let right = keyed_operand(operand, first, layout.as_ref(), true, &mut run_operand)?;
        combined = combine(combined, right, *operator, higher_is_better);
    }
    let mut rows: Vec<SearchResult> = combined.into_iter().map(|(_, row)| row).collect();
    let window = compound_window(query);
    match window.order_by {
        Some(order) => {
            let mapped = layout.as_ref().map(|layout| layout.order(order));
            order_by(&mut rows, mapped.as_deref().unwrap_or(order))?;
        }
        None => rank_by_score(&mut rows, higher_is_better),
    }
    let offset = usize::try_from(window.offset).unwrap_or(usize::MAX);
    let limit = usize::try_from(window.limit.unwrap_or(ceiling).min(ceiling)).unwrap_or(usize::MAX);
    Ok(rows.into_iter().skip(offset).take(limit).collect())
}

fn keyed_operand<'a, E>(
    operand: &mut Query,
    first: &'a SelectColumns,
    layout: Option<&CompoundProjection<'a>>,
    later_operand: bool,
    run: &mut impl FnMut(&Query) -> Result<Vec<SearchResult>, E>,
) -> Result<Vec<KeyedRow>, E> {
    let preserved = preserve_operand_positions(&mut operand.select.columns);
    let mut rows = keyed(run(operand)?, &operand.select);
    if later_operand || preserved || layout.is_some() {
        align_columns(&mut rows, &operand.select.columns, first, layout);
    }
    Ok(rows)
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
    rank_scores(a, b, higher_is_better) == Ordering::Less
}

/// Stable sort, best score first in the metric's direction, NaN last.
fn rank_by_score(rows: &mut [SearchResult], higher_is_better: bool) {
    rows.sort_by(|a, b| rank_scores(a.score, b.score, higher_is_better));
}

/// Writes each row's listed column `i` under the first operand's column `i`
/// (a compound's columns are positional). Rows of a `*` operand, or under a
/// first operand's `id`, which reads the point id, are left as they are.
fn align_columns<'a>(
    rows: &mut [KeyedRow],
    columns: &SelectColumns,
    first: &'a SelectColumns,
    layout: Option<&CompoundProjection<'a>>,
) {
    // An identity assignment still restores a window output if an earlier
    // position writes the same name. Read its value before any assignment.
    let mut changed = rustc_hash::FxHashSet::default();
    let renames: Vec<_> = listed_fields(columns)
        .zip(
            listed_fields(first)
                .enumerate()
                .map(|(i, field)| layout.map_or(Some(field), |l| l.destination(i, field))),
        )
        .filter_map(|(own, first)| first.map(|first| (own, first)))
        .filter(|(own, first)| {
            if *first == ProjectedField::Source("id") {
                return false;
            }
            if own == first {
                return changed.contains(first);
            }
            changed.insert(*first);
            true
        })
        .collect();
    if renames.is_empty() {
        return;
    }
    for (_, row) in rows {
        let values: Vec<_> = renames
            .iter()
            .map(|(own, first)| (*first, own.read(row).into_owned()))
            .collect();
        let payload = row.point.payload.get_or_insert(serde_json::Value::Null);
        if !payload.is_object() {
            *payload = serde_json::Value::Object(serde_json::Map::new());
        }
        for (first, value) in values {
            first.write(payload, value);
        }
    }
}

/// Named columns followed by the window outputs, in projection order.
fn listed_fields(columns: &SelectColumns) -> impl Iterator<Item = ProjectedField<'_>> {
    let (cols, windows) = match columns {
        SelectColumns::Columns(cols) => (cols.as_slice(), &[][..]),
        SelectColumns::Mixed {
            columns,
            window_functions,
            ..
        } if !columns.is_empty() => (columns.as_slice(), window_functions.as_slice()),
        _ => (&[][..], &[][..]),
    };
    let named = WindowInputs::projection_fields(cols, windows).map(|(_, field)| field);
    let ranks = windows.iter().map(|wf| {
        ProjectedField::Published(
            wf.alias
                .as_deref()
                .unwrap_or(wf.function_type.default_alias()),
        )
    });
    named.chain(ranks)
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
