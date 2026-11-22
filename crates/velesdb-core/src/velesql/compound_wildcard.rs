//! Explicit outputs over a wildcard retain the rest of the source document.
use super::{
    listed_fields, row_key, select_fields, window_name, CompoundProjection, KeyedRow, OperandField,
    ProjectedField, Query, SearchResult, SelectColumns, SelectStatement,
};
use serde_json::Value;

pub(super) fn wildcard_bindings<'a>(
    columns: &'a SelectColumns,
    query: &'a Query,
    layout: Option<&CompoundProjection<'a>>,
) -> Vec<(OperandField<'a>, ProjectedField<'a>)> {
    let (cols, windows) = select_fields(&query.select.columns);
    let outputs = cols
        .iter()
        .map(|c| c.alias.as_deref().unwrap_or(&c.name))
        .chain(windows.iter().map(window_name));
    let order = super::compound_window(query).order_by.unwrap_or_default();
    let retained = super::super::compound_rank_inputs(query, columns);
    let mut bindings: Vec<_> = listed_fields(columns)
        .zip(outputs)
        .enumerate()
        .filter_map(|(i, (source, output))| {
            let field = ProjectedField::Published(output);
            let dest = layout.map_or(Some(field), |l| l.destination(i, field))?;
            if dest == ProjectedField::Source("id") {
                return None;
            }
            let source = match source {
                OperandField::RankInput(name)
                    if !order.iter().any(|item| {
                        super::super::compound_projection::reads_vector_input(&item.expr, name)
                    }) =>
                {
                    OperandField::Rank(name)
                }
                other => other,
            };
            Some((source, dest))
        })
        .collect();
    let mut seen = rustc_hash::FxHashSet::default();
    bindings.reverse();
    bindings.retain(|(source, target)| {
        seen.insert(target.name()) && !source.already_visible(*target, &retained)
    });
    bindings.reverse();
    bindings
}

pub(super) fn keyed(
    rows: Vec<SearchResult>,
    select: &SelectStatement,
    operand: usize,
    query: &Query,
) -> Vec<KeyedRow> {
    if !super::super::compound_projection::qualified_wildcard(&query.select.columns) {
        return rows
            .into_iter()
            .map(|row| (row_key(&row, &select.columns), row, operand))
            .collect();
    }
    let key = WildcardKey::new(&select.columns, query);
    rows.into_iter()
        .map(|mut row| (key.encode(&mut row), row, operand))
        .collect()
}

/// The wildcard publishes the point ID, payload members, named expressions,
/// scores and windows in that order. Only the final value of each name counts.
struct WildcardKey<'a> {
    fields: Vec<(OperandField<'a>, ProjectedField<'a>)>,
    restore: Vec<String>,
    scores: Vec<&'a str>,
    implicit_id: bool,
}

impl<'a> WildcardKey<'a> {
    fn new(columns: &'a SelectColumns, query: &'a Query) -> Self {
        let first = &query.select.columns;
        let (_, windows) = select_fields(first);
        let is_window = |name: &str| windows.iter().any(|wf| window_name(wf) == name);
        let mut restore = super::super::compound_rank_inputs(query, columns);
        restore.retain(|name| !is_window(name));
        let scores: Vec<_> = match first {
            SelectColumns::Mixed {
                similarity_scores, ..
            } => similarity_scores
                .iter()
                .map(|s| s.alias.as_deref().unwrap_or("similarity"))
                .filter(|name| !is_window(name))
                .collect(),
            _ => Vec::new(),
        };
        let mut fields = wildcard_bindings(columns, query, None);
        fields.retain(|(_, target)| !scores.contains(&target.name()));
        Self {
            fields,
            restore,
            scores,
            implicit_id: !is_window("id"),
        }
    }

    fn encode(&self, row: &mut SearchResult) -> String {
        let payload = row.point.payload.as_ref().and_then(Value::as_object);
        let mut changes = Vec::new();
        let mut omitted = Vec::new();
        for name in &self.restore {
            let original = payload
                .and_then(|p| p.get(name))
                .and_then(Value::as_array)
                .and_then(|pair| pair.get(1));
            if let Some(value) = original {
                changes.push((name.as_str(), Some(value.clone())));
            } else {
                omitted.push(name.as_str());
            }
        }
        omitted.extend(
            self.implicit_id
                .then_some("id")
                .into_iter()
                .chain(self.scores.iter().copied())
                .filter(|name| payload.is_some_and(|p| p.contains_key(*name))),
        );
        for (source, dest) in &self.fields {
            if !source.unchanged(*dest, row) {
                omitted.retain(|name| *name != dest.name());
                changes.push((dest.name(), Some(source.read(row).into_owned())));
            }
        }
        Self::exchange(row, &mut changes);
        let values = filtered_payload_key(row, &omitted, !self.scores.is_empty());
        changes.reverse();
        Self::exchange(row, &mut changes);
        format!("{}\x1f{values}", row.point.id)
    }

    fn exchange(row: &mut SearchResult, changes: &mut [(&str, Option<Value>)]) {
        if let Some(payload) = row.point.payload.as_mut() {
            for (name, value) in changes {
                *value = super::exchange_member(payload, name, value.take());
            }
        }
    }
}

/// Excluding a real JSON member for comparison must not move it or its neighbours
/// in the input object when serde_json preserves insertion order.
fn filtered_payload_key(row: &SearchResult, omitted: &[&str], include_score: bool) -> String {
    use std::fmt::Write;
    let Some(map) = row
        .point
        .payload
        .as_ref()
        .and_then(Value::as_object)
        .filter(|map| omitted.iter().any(|name| map.contains_key(*name)))
    else {
        return super::compute_distinct_key(row, &[], include_score);
    };
    let mut names: Vec<_> = map
        .keys()
        .map(String::as_str)
        .filter(|name| !omitted.contains(name))
        .collect();
    names.sort_unstable();
    let mut key = String::from("{");
    for (i, name) in names.into_iter().enumerate() {
        if i != 0 {
            key.push(',');
        }
        let _ = write!(
            key,
            "{}:{}",
            Value::from(name),
            super::super::distinct_evaluator::canonical_payload(&map[name])
        );
    }
    key.push('}');
    if include_score {
        let _ = write!(key, "\x1f{}", row.score);
    }
    key
}
