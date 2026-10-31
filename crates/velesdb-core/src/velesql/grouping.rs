//! Grouping for GROUP BY and aggregate queries, shared by every engine.
//!
//! [`Grouper`] takes the rows a query kept and returns one row per group.
//! Core's aggregate executor, core's vector GROUP BY and the WASM executor all
//! group through it; its documentation states the rules.

use std::borrow::Cow;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use rustc_hash::FxHasher;

use super::aggregator::{AggregateResult, Aggregator};
use super::ast::{
    AggregateArg, AggregateFunction, AggregateType, CompareOp, HavingClause, LogicalOp,
    SelectColumns, SelectStatement, Value,
};
use super::grouped_order::vector_group_score_is_average;

/// A row offered to a [`Grouper`].
#[derive(Debug, Clone, Copy)]
pub struct GroupInput<'a> {
    /// The point id.
    pub id: u64,
    /// The row's search score (any value outside a vector search).
    pub score: f32,
    /// The point payload.
    pub payload: Option<&'a serde_json::Value>,
}

/// One finished group.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupOutput {
    /// The id of the group's best row.
    pub id: u64,
    /// The group's score.
    pub score: f32,
    /// The group's output row: GROUP BY columns and SELECT aggregates.
    pub row: serde_json::Value,
}

/// Groups rows for one statement.
///
/// Takes the rows a query kept — each with its id, score and payload — and
/// returns one row per group: its GROUP BY values and its SELECT aggregates,
/// each under [`AggregateFunction::result_key`], filtered by HAVING. Core's
/// aggregate executor, core's vector GROUP BY and the WASM executor all group
/// through it, so the group set, a group's row, its id and score, and the
/// HAVING verdict are computed one way. One rule each:
///
/// - **Group key.** A GROUP BY column reads as a WHERE column does: `id` is
///   the point id, any other name a payload path (`meta.lang`). A row without
///   it belongs to the NULL group.
/// - **Aggregates.** Every running aggregate accumulates through
///   [`Aggregator`]: `COUNT(col)` counts non-null values, `SUM`, `AVG`, `MIN`
///   and `MAX` read numeric ones. `FIRST(col)` is the value in the group's
///   best row.
/// - **Best row.** The most similar row — the highest score, or the lowest
///   for a distance metric — the first seen on a tie or when no row has a
///   score. Outside a vector search every row scores alike, so it is the
///   first row of the group.
/// - **`score`.** In a vector GROUP BY the `score` column is each row's search
///   score; elsewhere it is a payload field of that name.
/// - **Group id and score.** A group is its best row's id; its score is that
///   row's, or the rows' mean when the SELECT asks for `AVG(score)` and not
///   `MAX(score)` ([`vector_group_score_is_average`]).
/// - **Order.** Groups come out in the order their first row arrived; a vector
///   GROUP BY's come out best score first, ties by id, as a search ranks.
pub struct Grouper<'s> {
    group_columns: &'s [String],
    selected: &'s [AggregateFunction],
    having: Option<&'s HavingClause>,
    /// Columns the running aggregates read, SELECT and HAVING alike. Rows
    /// are always counted, for `COUNT(*)`.
    columns: Vec<String>,
    /// Columns the SELECT's `FIRST` aggregates read, in SELECT order.
    first_columns: Vec<String>,
    vector: bool,
    higher_is_better: bool,
    average_score: bool,
    max_groups: usize,
    index: HashMap<GroupKey, usize>,
    groups: Vec<Group>,
}

struct Group {
    key: Vec<serde_json::Value>,
    aggregator: Aggregator,
    best_id: u64,
    best_score: f32,
    score_sum: f64,
    rows: u64,
    first_values: Vec<serde_json::Value>,
}

impl<'s> Grouper<'s> {
    /// A grouper for `stmt`, whose HAVING is `having` with its parameters
    /// already bound. `higher_is_better` is the collection metric's direction;
    /// more than `max_groups` groups is an error.
    #[must_use]
    pub fn new(
        stmt: &'s SelectStatement,
        having: Option<&'s HavingClause>,
        higher_is_better: bool,
        max_groups: usize,
    ) -> Self {
        let selected = select_aggregations(&stmt.columns);
        let accumulated = selected.iter().chain(
            having
                .iter()
                .flat_map(|h| h.conditions.iter().map(|c| &c.aggregate)),
        );
        let mut columns: Vec<String> = Vec::new();
        for column in accumulated.filter_map(aggregate_column) {
            if !columns.iter().any(|c| c == column) {
                columns.push(column.to_string());
            }
        }
        let first_columns = selected
            .iter()
            .filter(|agg| agg.function_type == AggregateType::First)
            .filter_map(aggregate_column)
            .map(str::to_string)
            .collect();
        Self {
            group_columns: stmt
                .group_by
                .as_ref()
                .map_or(&[][..], |group_by| group_by.columns.as_slice()),
            selected,
            having,
            columns,
            first_columns,
            vector: stmt.is_vector_group_by(),
            higher_is_better,
            average_score: vector_group_score_is_average(selected),
            max_groups,
            index: HashMap::new(),
            groups: Vec::new(),
        }
    }

    /// Adds a row to its group.
    ///
    /// # Errors
    ///
    /// Returns a message when the row would open a group past `max_groups`.
    pub fn push(&mut self, row: GroupInput<'_>) -> Result<(), String> {
        let key = GroupKey::new(
            self.group_columns
                .iter()
                .map(|column| {
                    self.read(&row, column)
                        .map_or(serde_json::Value::Null, Cow::into_owned)
                })
                .collect(),
        );
        let slot = if let Some(&slot) = self.index.get(&key) {
            slot
        } else {
            if self.groups.len() >= self.max_groups {
                return Err(format!("Too many groups (limit: {})", self.max_groups));
            }
            let first_values = self.first_values(&row);
            self.groups
                .push(Group::new(key.values.clone(), &row, first_values));
            self.index.insert(key, self.groups.len() - 1);
            self.groups.len() - 1
        };
        self.accumulate(slot, &row);
        Ok(())
    }

    /// The groups, HAVING applied, in output order. An aggregate query
    /// without GROUP BY yields its one group even over no rows.
    #[must_use]
    pub fn finish(mut self) -> Vec<GroupOutput> {
        if self.groups.is_empty() && self.group_columns.is_empty() {
            self.groups.push(Group::empty(self.first_columns.len()));
        }
        let mut out: Vec<GroupOutput> = Vec::with_capacity(self.groups.len());
        for group in std::mem::take(&mut self.groups) {
            if let Some(output) = self.output(group) {
                out.push(output);
            }
        }
        if self.vector {
            let higher_is_better = self.higher_is_better;
            out.sort_by(|a, b| {
                rank_scores(a.score, b.score, higher_is_better).then(a.id.cmp(&b.id))
            });
        }
        out
    }

    fn output(&self, group: Group) -> Option<GroupOutput> {
        let result = group.aggregator.finalize();
        if self.having.is_some_and(|having| !having.accepts(&result)) {
            return None;
        }
        let mut row = serde_json::Map::new();
        for (column, value) in self.group_columns.iter().zip(group.key) {
            row.insert(column.clone(), value);
        }
        let mut firsts = group.first_values.into_iter();
        for agg in self.selected {
            let value = if agg.function_type == AggregateType::First {
                firsts.next().unwrap_or(serde_json::Value::Null)
            } else {
                result.value_of(agg)
            };
            row.insert(agg.result_key(), value);
        }
        let score = if self.average_score && group.rows > 0 {
            mean(group.score_sum, group.rows)
        } else {
            group.best_score
        };
        Some(GroupOutput {
            id: group.best_id,
            score,
            row: serde_json::Value::Object(row),
        })
    }

    fn accumulate(&mut self, slot: usize, row: &GroupInput<'_>) {
        let better = {
            let group = &self.groups[slot];
            group.rows > 0 && is_better(row.score, group.best_score, self.higher_is_better)
        };
        let first_values = better.then(|| self.first_values(row));
        let values: Vec<_> = self.columns.iter().map(|c| self.read(row, c)).collect();
        let group = &mut self.groups[slot];
        group.aggregator.process_count();
        for (column, value) in self.columns.iter().zip(values) {
            if let Some(value) = value {
                group.aggregator.process_value(column, &value);
            }
        }
        group.rows += 1;
        group.score_sum += f64::from(row.score);
        if let Some(first_values) = first_values {
            group.best_id = row.id;
            group.best_score = row.score;
            group.first_values = first_values;
        }
    }

    fn first_values(&self, row: &GroupInput<'_>) -> Vec<serde_json::Value> {
        self.first_columns
            .iter()
            .map(|column| {
                self.read(row, column)
                    .map_or(serde_json::Value::Null, Cow::into_owned)
            })
            .collect()
    }

    /// The value of `column` in `row`: the point id, the search score in a
    /// vector GROUP BY, else the payload path.
    fn read<'r>(&self, row: &GroupInput<'r>, column: &str) -> Option<Cow<'r, serde_json::Value>> {
        match column {
            "id" => Some(Cow::Owned(serde_json::json!(row.id))),
            "score" if self.vector => Some(Cow::Owned(serde_json::json!(row.score))),
            _ => row
                .payload
                .and_then(|p| payload_path(p, column))
                .map(Cow::Borrowed),
        }
    }
}

impl Group {
    fn new(
        key: Vec<serde_json::Value>,
        row: &GroupInput<'_>,
        first_values: Vec<serde_json::Value>,
    ) -> Self {
        Self {
            key,
            aggregator: Aggregator::new(),
            best_id: row.id,
            best_score: row.score,
            score_sum: 0.0,
            rows: 0,
            first_values,
        }
    }

    fn empty(firsts: usize) -> Self {
        Self {
            key: Vec::new(),
            aggregator: Aggregator::new(),
            best_id: 0,
            best_score: 0.0,
            score_sum: 0.0,
            rows: 0,
            first_values: vec![serde_json::Value::Null; firsts],
        }
    }
}

impl HavingClause {
    /// Whether a group whose aggregates are `result` passes this HAVING, its
    /// conditions combined left to right by their AND / OR operators.
    #[must_use]
    pub fn accepts(&self, result: &AggregateResult) -> bool {
        let mut verdicts = self
            .conditions
            .iter()
            .map(|c| compare(result.value_of(&c.aggregate).as_f64(), c.operator, &c.value));
        let Some(first) = verdicts.next() else {
            return true;
        };
        verdicts.enumerate().fold(first, |acc, (i, verdict)| {
            match self.operators.get(i).copied().unwrap_or(LogicalOp::And) {
                LogicalOp::Or => acc || verdict,
                LogicalOp::And => acc && verdict,
            }
        })
    }
}

/// The mean of `rows` scores summing to `sum`.
// Reason: a mean of f32 scores; the row count is bounded by the result set.
#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn mean(sum: f64, rows: u64) -> f32 {
    (sum / rows as f64) as f32
}

/// Compares an aggregate against a HAVING threshold. A missing aggregate or a
/// non-numeric threshold fails; equality allows for float rounding in sums.
fn compare(aggregate: Option<f64>, op: CompareOp, threshold: &Value) -> bool {
    let Some(value) = aggregate else {
        return false;
    };
    // Reason: thresholds compare as f64, like the aggregates they test.
    #[allow(clippy::cast_precision_loss)]
    let threshold = match threshold {
        Value::Integer(i) => *i as f64,
        Value::UnsignedInteger(u) => *u as f64,
        Value::Float(f) => *f,
        _ => return false,
    };
    let epsilon = f64::EPSILON * value.abs().max(threshold.abs()).max(1.0);
    match op {
        CompareOp::Eq => (value - threshold).abs() < epsilon,
        CompareOp::NotEq => (value - threshold).abs() >= epsilon,
        CompareOp::Gt => value > threshold,
        CompareOp::Gte => value >= threshold,
        CompareOp::Lt => value < threshold,
        CompareOp::Lte => value <= threshold,
    }
}

/// A payload value by dotted path (`meta.lang`), as WHERE reads it.
pub(crate) fn payload_path<'a>(
    payload: &'a serde_json::Value,
    path: &str,
) -> Option<&'a serde_json::Value> {
    path.split('.')
        .try_fold(payload, |current, part| match current {
            serde_json::Value::Object(map) => map.get(part),
            _ => None,
        })
}

/// A field of an output row: the key itself first — a group row stores a
/// dotted name such as `first_meta.title` flat — then the dotted path.
#[must_use]
pub fn row_field<'a>(row: &'a serde_json::Value, name: &str) -> Option<&'a serde_json::Value> {
    row.get(name).or_else(|| payload_path(row, name))
}

/// The column an aggregate reads; `None` for `COUNT(*)`.
fn aggregate_column(agg: &AggregateFunction) -> Option<&str> {
    match &agg.argument {
        AggregateArg::Column(column) => Some(column),
        AggregateArg::Score => Some("score"),
        AggregateArg::Wildcard => None,
    }
}

fn select_aggregations(columns: &SelectColumns) -> &[AggregateFunction] {
    match columns {
        SelectColumns::Aggregations(aggregations) | SelectColumns::Mixed { aggregations, .. } => {
            aggregations
        }
        _ => &[],
    }
}

/// Whether `candidate` is a more similar score than `best`. A NaN is never
/// better, and anything is better than a NaN.
fn is_better(candidate: f32, best: f32, higher_is_better: bool) -> bool {
    !candidate.is_nan()
        && (best.is_nan()
            || if higher_is_better {
                candidate > best
            } else {
                candidate < best
            })
}

/// Orders scores most similar first, NaN last.
fn rank_scores(a: f32, b: f32, higher_is_better: bool) -> std::cmp::Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => std::cmp::Ordering::Equal,
        (true, false) => std::cmp::Ordering::Greater,
        (false, true) => std::cmp::Ordering::Less,
        (false, false) if higher_is_better => b.total_cmp(&a),
        (false, false) => a.total_cmp(&b),
    }
}

/// A group key with its hash computed once.
#[derive(Clone)]
struct GroupKey {
    values: Vec<serde_json::Value>,
    hash: u64,
}

impl GroupKey {
    fn new(values: Vec<serde_json::Value>) -> Self {
        let mut hasher = FxHasher::default();
        for value in &values {
            hash_value(value, &mut hasher);
        }
        let hash = hasher.finish();
        Self { values, hash }
    }
}

fn hash_value(value: &serde_json::Value, hasher: &mut FxHasher) {
    match value {
        serde_json::Value::Null => 0u8.hash(hasher),
        serde_json::Value::Bool(b) => {
            1u8.hash(hasher);
            b.hash(hasher);
        }
        serde_json::Value::Number(n) => {
            2u8.hash(hasher);
            if let Some(f) = n.as_f64() {
                f.to_bits().hash(hasher);
            }
        }
        serde_json::Value::String(s) => {
            3u8.hash(hasher);
            s.hash(hasher);
        }
        _ => {
            4u8.hash(hasher);
            value.to_string().hash(hasher);
        }
    }
}

impl Hash for GroupKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
    }
}

impl PartialEq for GroupKey {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash && self.values == other.values
    }
}

impl Eq for GroupKey {}
