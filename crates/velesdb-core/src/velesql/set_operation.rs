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

use super::compound_projection::{
    exchange_member, preserve_operand_positions, CompoundProjection, VectorSortSlots,
};
use super::distinct_evaluator::{compute_distinct_key, key_columns, projected_values_key};
use super::json_path::ProjectedField;
use super::window_evaluator::WindowInputs;
use super::{Query, SelectColumns, SelectOrderBy, SelectStatement, SetOperator};

/// A surviving row keeps its own operand's positional schema until ordering.
type KeyedRow = (String, SearchResult, usize);

/// A physical SELECT column or one side of a window rank retained across a set.
#[derive(Clone, Copy)]
enum OperandField<'a> {
    Column(ProjectedField<'a>),
    Rank(&'a str),
    RankInput(&'a str),
    RankPath(&'a str, &'a str, ProjectedField<'a>),
}

impl<'a> OperandField<'a> {
    fn storage(self) -> ProjectedField<'a> {
        match self {
            Self::Column(field) | Self::RankPath(_, _, field) => field,
            Self::Rank(name) | Self::RankInput(name) => ProjectedField::Published(name),
        }
    }

    fn already_visible(self, target: ProjectedField<'_>, retained: &[String]) -> bool {
        matches!(self, Self::Rank(name) if target.name() == name && !retained.iter().any(|saved| saved == name))
    }

    fn unchanged(self, target: ProjectedField<'_>, row: &SearchResult) -> bool {
        let Self::Column(ProjectedField::Source(name)) = self else {
            return false;
        };
        target == ProjectedField::Published(name)
            && name != "id"
            && !name.contains('.')
            && row
                .point
                .payload
                .as_ref()
                .is_some_and(|p| p.get(name).is_some())
    }

    fn read(self, row: &SearchResult) -> std::borrow::Cow<'_, serde_json::Value> {
        let (name, index) = match self {
            Self::Column(field) => return field.read(row),
            Self::RankPath(root, tail, field) => {
                if let Some(serde_json::Value::Array(pair)) =
                    row.point.payload.as_ref().and_then(|p| p.get(root))
                {
                    let val = pair
                        .get(1)
                        .and_then(|v| super::json_path::payload_path(v, tail));
                    return std::borrow::Cow::Borrowed(val.unwrap_or(&serde_json::Value::Null));
                }
                return field.read(row);
            }
            Self::Rank(name) => (name, 0),
            Self::RankInput(name) => (name, 1),
        };
        let value = row.point.payload.as_ref().and_then(|p| p.get(name));
        let value = value.and_then(|v| match v {
            serde_json::Value::Array(pair) => pair.get(index),
            scalar => Some(scalar),
        });
        std::borrow::Cow::Borrowed(value.unwrap_or(&serde_json::Value::Null))
    }
}

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
    let lists_windows = matches!(columns, SelectColumns::Mixed { window_functions, qualified_wildcards, .. }
        if !window_functions.is_empty() && qualified_wildcards.is_empty());
    if !listed.is_empty() || lists_windows {
        return projected_values_key(
            result,
            &mut listed_fields(columns).map(|field| field.read(result)),
            include_score,
        );
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
/// Applications using `Database::execute_query` or `Collection::execute_query`
/// pass the whole compound. A custom executor runs the supplied operand. Its
/// column aliases may have been adjusted for positional alignment. When evaluating
/// its windows, call [`super::compound_rank_inputs`] with the original compound
/// and the supplied columns. The [window callback example](super::window_evaluator::evaluate_compound_windows)
/// shows how to keep a sort input before a window publishes its rank over it.
/// The `order_by` callback sorts by the expressions it receives, whose field
/// names may also have been adjusted for positional alignment.
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
    let vector_order = super::compound_projection::orders_named_vector(query);
    for operand in &mut operands {
        preserve_operand_positions(&mut operand.select.columns, vector_order);
    }
    let combined = combine_operands(query, &operands, higher_is_better, &mut run_operand)?;
    let window = compound_window(query);
    let PreparedCompound {
        mut rows,
        order,
        slots,
    } = prepare_compound_rows(query, &operands, combined);
    match window.order_by {
        Some(original) => order_by(&mut rows, order.as_deref().unwrap_or(original))?,
        None => rank_by_score(&mut rows, higher_is_better),
    }
    Ok(window_page(rows, slots.as_ref(), window, ceiling))
}

struct PreparedCompound {
    rows: Vec<SearchResult>,
    order: Option<Vec<SelectOrderBy>>,
    slots: Option<VectorSortSlots>,
}

#[inline(never)]
fn prepare_compound_rows(
    query: &Query,
    operands: &[Query],
    combined: Vec<KeyedRow>,
) -> PreparedCompound {
    let order = compound_window(query).order_by;
    let mut layout = CompoundProjection::for_query(query);
    let slots = layout.as_ref().and_then(|layout| {
        layout.vector_sort_slots(
            order?,
            listed_fields(&query.select.columns).map(OperandField::storage),
            combined.iter().map(|(_, row, _)| row),
            layout.wildcard_replacements(query),
        )
    });
    if let (Some(layout), Some(slots)) = (&mut layout, &slots) {
        layout.redirect_slots(slots);
    }
    let rows = aligned_rows(combined, operands, query, layout.as_ref(), slots.as_ref());
    let order = order.and_then(|order| layout.as_ref().map(|layout| layout.order(order)));
    PreparedCompound { rows, order, slots }
}

fn window_page(
    rows: Vec<SearchResult>,
    slots: Option<&VectorSortSlots>,
    window: CompoundWindow<'_>,
    ceiling: u64,
) -> Vec<SearchResult> {
    let offset = usize::try_from(window.offset).unwrap_or(usize::MAX);
    let limit = usize::try_from(window.limit.unwrap_or(ceiling).min(ceiling)).unwrap_or(usize::MAX);
    let mut page: Vec<_> = rows.into_iter().skip(offset).take(limit).collect();
    if let Some(slots) = slots {
        slots.publish(&mut page);
    }
    page
}

fn aligned_rows(
    combined: Vec<KeyedRow>,
    operands: &[Query],
    query: &Query,
    layout: Option<&CompoundProjection<'_>>,
    slots: Option<&VectorSortSlots>,
) -> Vec<SearchResult> {
    let renames: Vec<_> = operands
        .iter()
        .map(|operand| {
            (
                column_renames(&operand.select.columns, query, layout),
                super::compound_rank_inputs(query, &operand.select.columns),
                slots.map_or_else(Vec::new, |s| {
                    s.replaced_members(select_fields(&operand.select.columns).0)
                }),
            )
        })
        .collect();
    combined
        .into_iter()
        .map(|(_, mut row, operand)| {
            let (columns, ranks, originals) = &renames[operand];
            align_columns(&mut row, columns, ranks, originals);
            row
        })
        .collect()
}

fn combine_operands<E>(
    query: &Query,
    operands: &[Query],
    higher_is_better: bool,
    run: &mut dyn FnMut(&Query) -> Result<Vec<SearchResult>, E>,
) -> Result<Vec<KeyedRow>, E> {
    let mut combined = keyed(run(&operands[0])?, &operands[0].select, 0, query);
    let operators = query.compound.iter().flat_map(|c| c.operations.iter());
    for ((operator, _), (index, operand)) in operators.zip(operands.iter().enumerate().skip(1)) {
        let right = keyed(run(operand)?, &operand.select, index, query);
        combined = combine(combined, right, *operator, higher_is_better);
    }
    Ok(combined)
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
                .filter_map(|(key, mut row, operand)| {
                    adopt_if_better(&mut row, right.get(key.as_str())?, higher_is_better);
                    Some((key, row, operand))
                })
                .collect();
            distinct(kept, higher_is_better)
        }
        SetOperator::Except => {
            let right = best_rows(&right, higher_is_better);
            let kept = left
                .into_iter()
                .filter(|(key, _, _)| !right.contains_key(key.as_str()))
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
    for (key, row, operand) in rows {
        if let Some(&at) = position.get(&key) {
            adopt_if_better(&mut out[at].1, &row, higher_is_better);
        } else {
            position.insert(key.clone(), out.len());
            out.push((key, row, operand));
        }
    }
    out
}

/// Each key's best-scored row among `rows`.
fn best_rows(rows: &[KeyedRow], higher_is_better: bool) -> HashMap<&str, &SearchResult> {
    let mut best: HashMap<&str, &SearchResult> = HashMap::with_capacity(rows.len());
    for (key, row, _) in rows {
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
fn column_renames<'a>(
    columns: &'a SelectColumns,
    query: &'a Query,
    layout: Option<&CompoundProjection<'a>>,
) -> Vec<(OperandField<'a>, ProjectedField<'a>)> {
    let first = &query.select.columns;
    if super::compound_projection::qualified_wildcard(first) {
        let mut fields = wildcard_bindings(columns, query, layout);
        if let Some(layout) = layout {
            add_positional_vectors(&mut fields, layout, columns);
        }
        return fields;
    }
    // An identity assignment still restores a window output if an earlier
    // position writes the same name. Read its value before any assignment.
    let mut changed = rustc_hash::FxHashSet::default();
    let mut renames: Vec<_> = listed_fields(columns)
        .zip(listed_fields(first).enumerate().map(|(i, field)| {
            layout.map_or(Some(field.storage()), |l| l.destination(i, field.storage()))
        }))
        .filter_map(|(own, first)| first.map(|first| (own, first)))
        .filter(|(own, first)| {
            if *first == ProjectedField::Source("id") {
                return false;
            }
            if own.storage() == *first {
                return changed.contains(first);
            }
            changed.insert(*first);
            true
        })
        .collect();
    if let Some(layout) = layout {
        add_positional_vectors(&mut renames, layout, columns);
    }
    renames
}

fn add_positional_vectors<'a>(
    renames: &mut Vec<(OperandField<'a>, ProjectedField<'a>)>,
    layout: &CompoundProjection<'a>,
    columns: &'a SelectColumns,
) {
    let (cols, _) = select_fields(columns);
    for &(name, i) in layout.positional_vectors() {
        if cols.get(i).is_some_and(|col| col.name != name) {
            if let Some(source) = listed_fields(columns).nth(i) {
                renames.push((source, ProjectedField::Published(name)));
            }
        }
    }
}

fn align_columns(
    row: &mut SearchResult,
    renames: &[(OperandField<'_>, ProjectedField<'_>)],
    ranks: &[String],
    originals: &[(&str, &str)],
) {
    if renames.is_empty() && ranks.is_empty() {
        return;
    }
    let values: Vec<_> = renames
        .iter()
        .filter(|(own, first)| !own.unchanged(*first, row))
        .map(|(own, first)| (*first, own.read(row).into_owned()))
        .collect();
    let payload = row.point.payload.get_or_insert(serde_json::Value::Null);
    if !payload.is_object() {
        *payload = serde_json::Value::Object(serde_json::Map::new());
    }
    restore_rank_inputs(payload, ranks);
    for &(source, temporary) in originals {
        let input = payload.get_mut(source).map(serde_json::Value::take);
        exchange_member(
            payload,
            temporary,
            Some(serde_json::Value::Array(input.into_iter().collect())),
        );
    }
    for (first, value) in values {
        first.write(payload, value);
    }
}

fn restore_rank_inputs(payload: &mut serde_json::Value, ranks: &[String]) {
    for name in ranks {
        if let Some(serde_json::Value::Array(pair)) = payload.get_mut(name) {
            let original = pair.get_mut(1).map(std::mem::take);
            exchange_member(payload, name, original);
        }
    }
}

/// Named columns followed by the window outputs, in projection order.
pub(super) fn select_fields(
    columns: &SelectColumns,
) -> (&[super::Column], &[super::WindowFunction]) {
    match columns {
        SelectColumns::Columns(cols) => (cols.as_slice(), &[][..]),
        SelectColumns::Mixed {
            columns,
            window_functions,
            ..
        } => (columns.as_slice(), window_functions.as_slice()),
        _ => (&[][..], &[][..]),
    }
}

fn listed_fields(columns: &SelectColumns) -> impl Iterator<Item = OperandField<'_>> {
    let (cols, windows) = select_fields(columns);
    let wildcard = super::compound_projection::qualified_wildcard(columns);
    let named = WindowInputs::projection_fields(cols, windows).map(move |(col, field)| {
        let field = if wildcard {
            ProjectedField::Source(&col.name)
        } else {
            field
        };
        if let Some((root, tail)) = col.name.split_once('.') {
            if windows.iter().any(|wf| window_name(wf) == root) {
                return OperandField::RankPath(root, tail, field);
            }
        }
        if field.name() == col.name
            && col.name != "id"
            && !col.name.contains('.')
            && windows.iter().any(|wf| {
                wf.alias
                    .as_deref()
                    .unwrap_or(wf.function_type.default_alias())
                    == col.name
            })
        {
            OperandField::RankInput(&col.name)
        } else {
            OperandField::Column(field)
        }
    });
    let ranks = windows.iter().map(|wf| {
        OperandField::Rank(
            wf.alias
                .as_deref()
                .unwrap_or(wf.function_type.default_alias()),
        )
    });
    named.chain(ranks)
}

fn window_name(window: &super::WindowFunction) -> &str {
    window
        .alias
        .as_deref()
        .unwrap_or(window.function_type.default_alias())
}

#[path = "compound_wildcard.rs"]
mod wildcard;
use wildcard::{keyed, wildcard_bindings};

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
