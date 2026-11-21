use crate::velesql::json_path::column_value;
use crate::velesql::{
    base_side, compare_json_values, row_field, side_alias, Column, OrderByExpr, SelectColumns,
    SelectStatement, WindowFunction,
};
use crate::SearchResult;
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::BTreeMap;

/// The source columns a query must still read after a window output takes
/// their qualifier's name. ORDER BY compares its input values; SELECT publishes its
/// explicitly named outputs after sorting, pagination and LET evaluation.
#[derive(Default)]
pub struct WindowInputs {
    ordering: BTreeMap<usize, Vec<Value>>,
    projection: BTreeMap<usize, Vec<Value>>,
    positions: Vec<usize>,
}

impl WindowInputs {
    /// Capture only qualified columns of a side a window will overwrite.
    #[must_use]
    pub fn capture(stmt: &SelectStatement, rows: &[SearchResult]) -> Self {
        let SelectColumns::Mixed {
            columns,
            window_functions,
            ..
        } = &stmt.columns
        else {
            return Self::default();
        };
        if window_functions.is_empty() {
            return Self::default();
        }
        let mut inputs = Self::default();
        for (index, name) in stmt
            .order_by
            .iter()
            .flatten()
            .enumerate()
            .filter_map(|(i, item)| {
                let OrderByExpr::Field(name) = &item.expr else {
                    return None;
                };
                shadowed_side(stmt, window_functions, name).then_some((i, name))
            })
        {
            inputs.ordering.insert(
                index,
                rows.iter().map(|row| scalar_key(row, name)).collect(),
            );
        }
        for (index, col) in columns
            .iter()
            .enumerate()
            .filter(|(_, col)| shadowed_path(window_functions, &col.name))
        {
            inputs.projection.insert(
                index,
                rows.iter()
                    .map(|row| column_value(row, &col.name))
                    .collect(),
            );
        }
        inputs
    }

    /// Whether sorting needs the original positions to compare keys or to
    /// carry selected source columns through the permutation.
    #[must_use]
    pub fn needs_positions(&self) -> bool {
        !self.ordering.is_empty() || !self.projection.is_empty()
    }

    /// The ascending comparison of an input ORDER BY column, when captured.
    #[must_use]
    pub fn compare(&self, column: usize, left: usize, right: usize) -> Option<Ordering> {
        let values = self.ordering.get(&column)?;
        Some(compare_json_values(values.get(left), values.get(right)))
    }

    /// The original positions of the ordered output rows, before OFFSET.
    pub fn ordered(&mut self, positions: &[usize]) {
        if !self.projection.is_empty() {
            self.positions = positions.to_vec();
        }
    }

    /// Publish selected column names on the surviving rows. Read all named
    /// expressions for a row before writing any output alias over an input.
    pub fn publish(mut self, rows: &mut [SearchResult], columns: &SelectColumns, offset: usize) {
        if self.projection.is_empty() {
            return;
        }
        let SelectColumns::Mixed {
            columns,
            window_functions,
            ..
        } = columns
        else {
            return;
        };
        let outputs = columns
            .iter()
            .enumerate()
            .filter(|(_, col)| {
                !names_window(window_functions, col.alias.as_deref().unwrap_or(&col.name))
            })
            .collect::<Vec<_>>();
        for (at, row) in rows.iter_mut().enumerate() {
            let at = offset.saturating_add(at);
            let index = self.positions.get(at).copied().unwrap_or(at);
            let values = outputs
                .iter()
                .map(|(index_col, col)| {
                    let key = col.alias.as_deref().unwrap_or(&col.name);
                    let value = self
                        .projection
                        .get_mut(index_col)
                        .and_then(|values| values.get_mut(index))
                        .map_or_else(|| column_value(row, &col.name), std::mem::take);
                    (key.to_owned(), value)
                })
                .collect::<Vec<_>>();
            if let Some(payload) = row.point.payload.as_mut().and_then(Value::as_object_mut) {
                payload.extend(values);
            }
        }
    }

    /// Resolve a complete SELECT projection. When a window shadows an input
    /// qualifier, `publish` has materialized all explicit output names.
    pub fn selected_columns<'a>(
        row: &'a SearchResult,
        columns: &'a [Column],
        windows: &'a [WindowFunction],
    ) -> impl Iterator<Item = (&'a Column, Value)> {
        let published =
            !windows.is_empty() && columns.iter().any(|col| shadowed_path(windows, &col.name));
        columns.iter().map(move |col| {
            let value = if published && col.name != "id" {
                let key = col.alias.as_deref().unwrap_or(&col.name);
                row.point
                    .payload
                    .as_ref()
                    .and_then(|p| p.get(key))
                    .cloned()
                    .unwrap_or(Value::Null)
            } else {
                column_value(row, &col.name)
            };
            (col, value)
        })
    }
}

fn shadowed_path(windows: &[WindowFunction], field: &str) -> bool {
    field
        .split_once('.')
        .is_some_and(|(side, _)| names_window(windows, side))
}

fn names_window(windows: &[WindowFunction], name: &str) -> bool {
    windows.iter().any(|wf| {
        wf.alias
            .as_deref()
            .unwrap_or(wf.function_type.default_alias())
            == name
    })
}

fn shadowed_side(stmt: &SelectStatement, windows: &[WindowFunction], name: &str) -> bool {
    let Some((side, _)) = name.split_once('.') else {
        return false;
    };
    !stmt.joins.is_empty()
        && names_window(windows, side)
        && (base_side(stmt) == side
            || stmt
                .joins
                .iter()
                .any(|join| side_alias(join.alias.as_deref(), &join.table) == side))
}

/// ORDER BY resolves a literal dotted key before traversing a nested path.
/// Arrays and objects compare solely by type, so their contents need no copy.
fn scalar_key(row: &SearchResult, field: &str) -> Value {
    match row.point.payload.as_ref().and_then(|p| row_field(p, field)) {
        Some(Value::Array(_)) => Value::Array(Vec::new()),
        Some(Value::Object(_)) => Value::Object(serde_json::Map::new()),
        value => value.cloned().unwrap_or(Value::Null),
    }
}
