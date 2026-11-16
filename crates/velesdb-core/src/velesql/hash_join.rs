//! The one JOIN algorithm every engine runs: core's generic join stage and
//! the WASM executor.
//!
//! A JOIN matches the accumulated rows with the joined collection's points
//! on one equality (`ON a.x = b.y` or `USING (x)`) over any column, indexed or
//! not, of any JSON type. Two values match when a WHERE `=` would find them
//! equal (`1` and `1.0` are one number); `NULL` or an absent value matches
//! nothing. Each match is one row ([`merge_side`]); INNER keeps the matches,
//! LEFT adds each row without one — every column the joined collection has
//! (`id` included) `null`, flat and under its alias — RIGHT adds each point
//! without one, FULL adds both. Rows come out in base order, each row's matches in the joined
//! collection's order, then the unmatched points in that order.

use std::borrow::Cow;
use std::collections::HashMap;

use serde_json::{Map, Value};

use super::ast::{ColumnRef, JoinClause, JoinCondition, JoinType};
use super::group_key::normalise;
use super::join_row::{merge_side, side_alias};
use super::json_path::payload_path;
use crate::point::{Point, SearchResult};

/// The equality a JOIN matches on, oriented: `base` reads the accumulated
/// row, `side` the joined collection's point.
#[derive(Debug, Clone, PartialEq)]
pub struct JoinKeys {
    /// The base-side column: `id` (the row's point id), a column of the
    /// accumulated row, or `alias.column` of one of its sides.
    pub base: ColumnRef,
    /// The joined collection's column: `id` (the point id) or a payload path.
    pub side: String,
}

impl JoinKeys {
    /// The equality of `join`: `ON` oriented by which side names the joined
    /// collection — its alias first, then its table name, so a self-join
    /// (`FROM t JOIN t AS m ON t.boss = m.id`) reads `m` as the joined side
    /// — or a single-column `USING`. When neither side of `ON` names it,
    /// `ON base = joined` is read in the order written. `None` when the JOIN
    /// has neither, or a multi-column `USING`.
    #[must_use]
    pub fn of(join: &JoinClause) -> Option<Self> {
        if let Some(condition) = &join.condition {
            let (side, base) = orient(condition, join);
            return Some(Self {
                base: base.clone(),
                side: side.column.clone(),
            });
        }
        match join.using_columns.as_deref() {
            Some([column]) => Some(Self {
                base: ColumnRef {
                    table: None,
                    column: column.clone(),
                },
                side: column.clone(),
            }),
            _ => None,
        }
    }
}

/// `(joined side, base side)` of an `ON` equality.
fn orient<'c>(condition: &'c JoinCondition, join: &JoinClause) -> (&'c ColumnRef, &'c ColumnRef) {
    let named = |name: &str| {
        (
            condition.left.table.as_deref() == Some(name),
            condition.right.table.as_deref() == Some(name),
        )
    };
    let by_alias = join.alias.as_deref().map(named);
    for names in by_alias.into_iter().chain([named(&join.table)]) {
        match names {
            (true, false) => return (&condition.left, &condition.right),
            (false, true) => return (&condition.right, &condition.left),
            _ => {}
        }
    }
    (&condition.right, &condition.left)
}

/// `rows` joined with the joined collection's `side` points by `keys`, as
/// `join`'s type keeps them, at most `row_budget` rows. `prior_aliases`
/// names the sides `rows` already carry: a RIGHT or FULL row without a base
/// match carries each with a `null` id, so `WHERE o.id IS NULL` finds it.
#[must_use]
pub fn hash_join(
    rows: Vec<SearchResult>,
    side: &[Point],
    join: &JoinClause,
    keys: &JoinKeys,
    prior_aliases: &[&str],
    row_budget: usize,
) -> Vec<SearchResult> {
    let index = side_index(side, &keys.side);
    let mut join_out = JoinOutput::new(side, side_alias(join.alias.as_deref(), &join.table));
    let keeps_unmatched_rows = matches!(join.join_type, JoinType::Left | JoinType::Full);
    for row in rows {
        if join_out.rows.len() >= row_budget {
            return join_out.rows;
        }
        let hits = base_key(&row, &keys.base).and_then(|key| index.get(&key));
        match hits {
            Some(hits) => join_out.emit_matches(row, hits, row_budget),
            None if keeps_unmatched_rows => join_out.emit_unmatched_row(row),
            None => {}
        }
    }
    if matches!(join.join_type, JoinType::Right | JoinType::Full) {
        join_out.emit_unmatched_points(prior_aliases, row_budget);
    }
    join_out.rows
}

/// The rows a join emits, and which joined points matched.
struct JoinOutput<'a> {
    side: &'a [Point],
    alias: &'a str,
    matched: Vec<bool>,
    rows: Vec<SearchResult>,
    /// The joined collection's columns, each `null`: what a row without a
    /// match carries for that side, built on its first use.
    null_fields: Option<Map<String, Value>>,
}

impl<'a> JoinOutput<'a> {
    fn new(side: &'a [Point], alias: &'a str) -> Self {
        Self {
            side,
            alias,
            matched: vec![false; side.len()],
            rows: Vec::new(),
            null_fields: None,
        }
    }

    /// One row per match of `row`, the last match taking `row` itself.
    fn emit_matches(&mut self, row: SearchResult, hits: &[usize], row_budget: usize) {
        let room = row_budget.saturating_sub(self.rows.len());
        let hits = &hits[..hits.len().min(room)];
        let Some((&last, rest)) = hits.split_last() else {
            return;
        };
        for &at in rest {
            self.emit_match(row.clone(), at);
        }
        self.emit_match(row, last);
    }

    fn emit_match(&mut self, mut row: SearchResult, at: usize) {
        let point = &self.side[at];
        self.matched[at] = true;
        row.point.payload = Some(merge_side(
            row.point.payload.take(),
            self.alias,
            fields(point),
            Some(point.id),
        ));
        self.rows.push(row);
    }

    /// A LEFT or FULL row without a match: every column of the joined
    /// collection, its `id` included, is `null`.
    fn emit_unmatched_row(&mut self, mut row: SearchResult) {
        let side = self.side;
        let nulls = self.null_fields.get_or_insert_with(|| null_fields(side));
        row.point.payload = Some(merge_side(
            row.point.payload.take(),
            self.alias,
            nulls,
            None,
        ));
        self.rows.push(row);
    }

    /// RIGHT or FULL: each point no row matched, as a row of its own with
    /// score `0` whose prior sides carry a `null` id.
    fn emit_unmatched_points(&mut self, prior_aliases: &[&str], row_budget: usize) {
        let absent: Map<String, Value> = prior_aliases
            .iter()
            .map(|alias| ((*alias).to_string(), serde_json::json!({ "id": null })))
            .collect();
        for (point, _) in self.side.iter().zip(&self.matched).filter(|(_, m)| !**m) {
            if self.rows.len() >= row_budget {
                return;
            }
            let payload = merge_side(
                Some(Value::Object(absent.clone())),
                self.alias,
                fields(point),
                Some(point.id),
            );
            self.rows.push(SearchResult::new(
                Point::metadata_only(point.id, payload),
                0.0,
            ));
        }
    }
}

/// The joined points by join key, each key's points in collection order.
fn side_index(side: &[Point], column: &str) -> HashMap<String, Vec<usize>> {
    let mut index: HashMap<String, Vec<usize>> = HashMap::new();
    for (at, point) in side.iter().enumerate() {
        let value = if column == "id" {
            Some(Cow::Owned(Value::from(point.id)))
        } else {
            point
                .payload
                .as_ref()
                .and_then(|payload| payload_path(payload, column))
                .map(Cow::Borrowed)
        };
        if let Some(key) = value.and_then(|v| join_key(&v)) {
            index.entry(key).or_default().push(at);
        }
    }
    index
}

/// A row's base-side join key: `alias.column` read under that side when the
/// row carries the side (absent there, no key: another side's field of that
/// name never stands in), else the row's own column (`id` is its point id).
fn base_key(row: &SearchResult, column: &ColumnRef) -> Option<String> {
    let payload = row.point.payload.as_ref();
    let side = column
        .table
        .as_deref()
        .and_then(|table| payload.and_then(|p| p.get(table)))
        .filter(|side| side.is_object());
    if let Some(side) = side {
        return payload_path(side, &column.column).and_then(join_key);
    }
    if column.column == "id" {
        return join_key(&Value::from(row.point.id));
    }
    payload
        .and_then(|p| payload_path(p, &column.column))
        .and_then(join_key)
}

/// The hash key of a join value: one key for values a WHERE `=` finds
/// equal; none for `NULL`, which matches nothing.
fn join_key(value: &Value) -> Option<String> {
    (!value.is_null()).then(|| canonical(normalise(Cow::Borrowed(value))).to_string())
}

/// `value` spelled one way for every value WHERE `=` finds equal to it. The
/// top level is already [`normalise`]d (one number for `1` and `1.0`);
/// nested inside an array or object, as WHERE compares JSON there, `-0.0`
/// is `0.0`, object keys are in name order, and `1` and `1.0` stay apart.
fn canonical(value: Value) -> Value {
    match value {
        Value::Number(n) if n.as_f64() == Some(0.0) && n.is_f64() => Value::from(0.0),
        Value::Array(items) => Value::Array(items.into_iter().map(canonical).collect()),
        Value::Object(map) => {
            let sorted: std::collections::BTreeMap<String, Value> =
                map.into_iter().map(|(k, v)| (k, canonical(v))).collect();
            Value::Object(sorted.into_iter().collect())
        }
        other => other,
    }
}

/// Every column a joined point has, and `id`, each `null`, in name order.
fn null_fields(side: &[Point]) -> Map<String, Value> {
    let mut columns: std::collections::BTreeSet<&str> = std::collections::BTreeSet::from(["id"]);
    for point in side {
        columns.extend(fields(point).keys().map(String::as_str));
    }
    columns
        .into_iter()
        .map(|column| (column.to_string(), Value::Null))
        .collect()
}

/// A joined point's fields (none for a payload that is not an object).
fn fields(point: &Point) -> &Map<String, Value> {
    static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
    point
        .payload
        .as_ref()
        .and_then(Value::as_object)
        .unwrap_or_else(|| EMPTY.get_or_init(Map::new))
}

#[cfg(test)]
#[path = "hash_join_tests.rs"]
mod tests;
