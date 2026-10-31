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
    AggregateFunction, AggregateType, CompareOp, HavingClause, LogicalOp, SelectColumns,
    SelectStatement, Value, WithClause, WithValue,
};
use super::grouped_order::vector_group_score_is_average;
use super::json_order::compare_json_values;
use super::validation::{ValidationError, ValidationErrorKind};

/// A row offered to a [`Grouper`].
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct GroupInput<'a> {
    /// The point id.
    pub id: u64,
    /// The row's search score; read only in a vector GROUP BY.
    pub score: f32,
    /// The point payload.
    pub payload: Option<&'a serde_json::Value>,
}

impl<'a> GroupInput<'a> {
    /// A row with this point id, search score and payload.
    #[must_use]
    pub const fn new(id: u64, score: f32, payload: Option<&'a serde_json::Value>) -> Self {
        Self { id, score, payload }
    }
}

/// One finished group.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct GroupOutput {
    /// The id of the group's best row.
    pub id: u64,
    /// The group's score.
    pub score: f32,
    /// The group's output row: GROUP BY columns and SELECT aggregates.
    pub row: serde_json::Value,
}

/// Default maximum number of groups when the query names none (memory
/// protection).
pub(crate) const DEFAULT_MAX_GROUPS: usize = 10_000;

/// Server-side hard ceiling on the number of GROUP BY groups (#903): a query's
/// `WITH (max_groups = N)` can lower its budget, never raise it past this.
pub(crate) const SERVER_MAX_GROUPS_CEILING: usize = 1_000_000;

/// The group budget of a SQL GROUP BY: `WITH (max_groups = N)` (or
/// `group_limit`), at least 1 and clamped to [`SERVER_MAX_GROUPS_CEILING`],
/// else [`DEFAULT_MAX_GROUPS`].
pub(crate) fn max_groups(with: Option<&WithClause>) -> usize {
    with.into_iter()
        .flat_map(|with| &with.options)
        .find(|opt| opt.key == "max_groups" || opt.key == "group_limit")
        .and_then(|opt| match opt.value {
            WithValue::Integer(n) => Some(usize::try_from(n.max(1)).unwrap_or(usize::MAX)),
            _ => None,
        })
        .map_or(DEFAULT_MAX_GROUPS, |limit| {
            limit.min(SERVER_MAX_GROUPS_CEILING)
        })
}

/// The columns a statement's aggregates read, and how a row is read for
/// them: one definition for the [`Grouper`] and for core's ungrouped
/// aggregation, which accumulates a single group in parallel chunks.
#[derive(Debug, Clone)]
pub(crate) struct AggregateColumns {
    /// Columns the running aggregates read, SELECT and HAVING alike. Rows are
    /// always counted, for `COUNT(*)`.
    columns: Vec<String>,
    vector: bool,
}

impl AggregateColumns {
    pub(crate) fn new(stmt: &SelectStatement, having: Option<&HavingClause>) -> Self {
        let accumulated = select_aggregations(&stmt.columns).iter().chain(
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
        Self {
            columns,
            vector: stmt.is_vector_group_by(),
        }
    }

    /// Feeds `row` to `aggregator`.
    pub(crate) fn accumulate(&self, aggregator: &mut Aggregator, row: &GroupInput<'_>) {
        aggregator.process_count();
        for column in &self.columns {
            if let Some(value) = read(row, column, self.vector) {
                aggregator.process_value(column, &value);
            }
        }
    }
}

/// Groups rows for one statement.
///
/// Takes the rows a query kept — each with its id, score and payload — and
/// returns one row per group: its GROUP BY values and its SELECT aggregates,
/// each under its alias or default name (`count`, `sum_price`), filtered by
/// HAVING. Core's
/// aggregate executor, core's vector GROUP BY and the WASM executor all group
/// through it, so the group set, a group's row, its id and score, and the
/// HAVING verdict are computed one way, whatever order the rows arrive in.
/// One rule each:
///
/// - **Group key.** A GROUP BY column reads as a WHERE column does: `id` is
///   the point id, any other name a payload path (`meta.lang`). A row without
///   it belongs to the NULL group. Numbers are one key by value: `1` and
///   `1.0`, `0.0` and `-0.0`, group together.
/// - **Aggregates.** Every running aggregate accumulates through
///   [`Aggregator`]: `COUNT(col)` counts non-null values, `SUM`, `AVG`, `MIN`
///   and `MAX` read numeric ones. An aggregate reads `id` as the point id too.
///   `FIRST(col)` is the value in the group's best row.
/// - **Best row.** In a vector GROUP BY, the most similar row — the highest
///   score, or the lowest for a distance metric — and, among equally similar
///   rows, the smallest point id. Elsewhere rows carry no score, and the best
///   row is the one with the smallest point id.
/// - **`score`.** In a vector GROUP BY the `score` column is each row's search
///   score; elsewhere it is a payload field of that name.
/// - **Group id and score.** A group is its best row's id; its score is that
///   row's, or the rows' mean when the SELECT asks for `AVG(score)` and not
///   `MAX(score)` (vector GROUP BY only).
/// - **Order.** A vector GROUP BY's groups come best score first, ties by id,
///   as a search ranks; other groups by GROUP BY key, ascending, in ORDER BY's
///   value order ([`compare_json_values`]).
/// - **Budget.** A SQL GROUP BY holds at most `WITH (max_groups = N)` groups,
///   10 000 by default and 1 000 000 at most; a vector GROUP BY groups search
///   results, already bounded by its LIMIT.
#[derive(Debug)]
pub struct Grouper<'s> {
    group_columns: &'s [String],
    selected: &'s [AggregateFunction],
    having: Option<&'s HavingClause>,
    columns: AggregateColumns,
    /// Columns the SELECT's `FIRST` aggregates read, in SELECT order.
    first_columns: Vec<String>,
    higher_is_better: bool,
    average_score: bool,
    max_groups: usize,
    /// Groups by key hash; a hash names more than one group only on a
    /// collision.
    index: HashMap<u64, Vec<usize>>,
    groups: Vec<Group>,
    /// The key of the row being pushed, reused from row to row.
    key: Vec<serde_json::Value>,
}

#[derive(Debug)]
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
    /// bound. `higher_is_better` is the collection metric's direction.
    ///
    /// # Errors
    ///
    /// `V018` for a HAVING threshold that is not a number, which a bound
    /// parameter can be although validation passed.
    pub fn new(
        stmt: &'s SelectStatement,
        having: Option<&'s HavingClause>,
        higher_is_better: bool,
    ) -> crate::error::Result<Self> {
        if let Some(having) = having {
            having
                .check_thresholds(true)
                .map_err(|e| crate::error::Error::Query(e.to_string()))?;
        }
        let selected = select_aggregations(&stmt.columns);
        let first_columns = selected
            .iter()
            .filter(|agg| agg.function_type == AggregateType::First)
            .filter_map(aggregate_column)
            .map(str::to_string)
            .collect();
        let vector = stmt.is_vector_group_by();
        Ok(Self {
            group_columns: stmt
                .group_by
                .as_ref()
                .map_or(&[][..], |group_by| group_by.columns.as_slice()),
            selected,
            having,
            columns: AggregateColumns::new(stmt, having),
            first_columns,
            higher_is_better,
            average_score: vector && vector_group_score_is_average(selected),
            max_groups: if vector {
                usize::MAX
            } else {
                max_groups(stmt.with_clause.as_ref())
            },
            index: HashMap::new(),
            groups: Vec::new(),
            key: Vec::new(),
        })
    }

    /// Adds a row to its group.
    ///
    /// # Errors
    ///
    /// [`Error::Config`](crate::error::Error::Config) when the row would open
    /// a group past the statement's group budget (`WITH (max_groups = N)`).
    pub fn push(&mut self, row: GroupInput<'_>) -> crate::error::Result<()> {
        let vector = self.columns.vector;
        self.key.clear();
        self.key.extend(self.group_columns.iter().map(|column| {
            read(&row, column, vector).map_or(serde_json::Value::Null, |value| normalise(&value))
        }));
        let hash = hash_key(&self.key);
        let groups = &self.groups;
        let key = &self.key;
        let found = self
            .index
            .get(&hash)
            .and_then(|slots| slots.iter().copied().find(|&slot| groups[slot].key == *key));
        let slot = if let Some(slot) = found {
            slot
        } else {
            if self.groups.len() >= self.max_groups {
                return Err(crate::error::Error::Config(format!(
                    "Too many groups (limit: {})",
                    self.max_groups
                )));
            }
            let first_values = self.first_values(&row);
            let slot = self.groups.len();
            self.groups
                .push(Group::new(self.key.clone(), &row, first_values));
            self.index.entry(hash).or_default().push(slot);
            slot
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
        let mut groups = std::mem::take(&mut self.groups);
        if self.columns.vector {
            let higher_is_better = self.higher_is_better;
            groups.sort_by(|a, b| {
                rank_scores(
                    a.score(self.average_score),
                    b.score(self.average_score),
                    higher_is_better,
                )
                .then(a.best_id.cmp(&b.best_id))
            });
        } else {
            groups.sort_by(|a, b| compare_keys(&a.key, &b.key));
        }
        groups
            .into_iter()
            .filter_map(|group| self.output(group))
            .collect()
    }

    fn output(&self, group: Group) -> Option<GroupOutput> {
        let score = group.score(self.average_score);
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
        Some(GroupOutput {
            id: group.best_id,
            score,
            row: serde_json::Value::Object(row),
        })
    }

    fn accumulate(&mut self, slot: usize, row: &GroupInput<'_>) {
        let better = {
            let group = &self.groups[slot];
            group.rows > 0 && self.beats(row, group)
        };
        let first_values = better.then(|| self.first_values(row));
        let group = &mut self.groups[slot];
        self.columns.accumulate(&mut group.aggregator, row);
        group.rows += 1;
        group.score_sum += f64::from(row.score);
        if let Some(first_values) = first_values {
            group.best_id = row.id;
            group.best_score = row.score;
            group.first_values = first_values;
        }
    }

    /// Whether `row` is a better row for `group` than its best so far: in a
    /// vector GROUP BY a more similar score (a NaN never is) or, equally
    /// similar, a smaller id; elsewhere a smaller id. The best row of a group
    /// thus never depends on the order rows arrive in.
    fn beats(&self, row: &GroupInput<'_>, group: &Group) -> bool {
        let by_score = if self.columns.vector {
            rank_scores(row.score, group.best_score, self.higher_is_better)
        } else {
            std::cmp::Ordering::Equal
        };
        by_score.then(row.id.cmp(&group.best_id)).is_lt()
    }

    fn first_values(&self, row: &GroupInput<'_>) -> Vec<serde_json::Value> {
        self.first_columns
            .iter()
            .map(|column| {
                read(row, column, self.columns.vector)
                    .map_or(serde_json::Value::Null, Cow::into_owned)
            })
            .collect()
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

    /// The group's score: its best row's, or the rows' mean when `average`.
    fn score(&self, average: bool) -> f32 {
        if average && self.rows > 0 {
            mean(self.score_sum, self.rows)
        } else {
            self.best_score
        }
    }
}

/// The value of `column` in `row`: the point id, the search score in a vector
/// GROUP BY, else the payload path.
fn read<'r>(
    row: &GroupInput<'r>,
    column: &str,
    vector: bool,
) -> Option<Cow<'r, serde_json::Value>> {
    match column {
        "id" => Some(Cow::Owned(serde_json::json!(row.id))),
        "score" if vector => Some(Cow::Owned(serde_json::json!(row.score))),
        _ => row
            .payload
            .and_then(|p| payload_path(p, column))
            .map(Cow::Borrowed),
    }
}

/// A group key value with one representation per number: an integral value
/// that fits `i64` as an integer (`1.0` -> `1`, `-0.0` -> `0`), any other
/// float as itself. Every other value is kept.
fn normalise(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Number(n) if !n.is_i64() && !n.is_u64() => {
            let f = n.as_f64().unwrap_or(0.0);
            let integral = matches!(f.fract().classify(), std::num::FpCategory::Zero);
            if integral && f.abs() < EXACT_F64_INTEGER_LIMIT {
                // Reason: `f` is integral and below 2^53, so the cast is exact.
                #[allow(clippy::cast_possible_truncation)]
                let as_int = f as i64;
                serde_json::json!(as_int)
            } else {
                value.clone()
            }
        }
        _ => value.clone(),
    }
}

/// 2^53: every integral `f64` below it is exactly an `i64`.
const EXACT_F64_INTEGER_LIMIT: f64 = 9_007_199_254_740_992.0;

/// Orders group keys column by column in ORDER BY's value order.
fn compare_keys(a: &[serde_json::Value], b: &[serde_json::Value]) -> std::cmp::Ordering {
    a.iter()
        .zip(b)
        .map(|(x, y)| compare_json_values(Some(x), Some(y)))
        .find(|ordering| ordering.is_ne())
        .unwrap_or(std::cmp::Ordering::Equal)
}

impl HavingClause {
    /// Whether a group whose aggregates are `result` passes this HAVING, with
    /// SQL's precedence: `AND` binds tighter than `OR`, so `a OR b AND c` is
    /// `a OR (b AND c)`.
    pub(crate) fn accepts(&self, result: &AggregateResult) -> bool {
        let mut verdicts = self
            .conditions
            .iter()
            .map(|c| compare(result.value_of(&c.aggregate).as_f64(), c.operator, &c.value));
        let Some(first) = verdicts.next() else {
            return true;
        };
        let (mut any, mut all) = (false, first);
        for (i, verdict) in verdicts.enumerate() {
            match self.operators.get(i).copied().unwrap_or(LogicalOp::And) {
                LogicalOp::And => all = all && verdict,
                LogicalOp::Or => {
                    any = any || all;
                    all = verdict;
                }
            }
        }
        any || all
    }

    /// `V018` unless every threshold is a number. Before execution
    /// (`bound == false`) a `$parameter` or a scalar subquery passes too:
    /// both are replaced by their value before the query runs, and checked
    /// again then, by [`Grouper::new`].
    pub(crate) fn check_thresholds(&self, bound: bool) -> Result<(), ValidationError> {
        let accepted = |value: &Value| match value {
            Value::Integer(_) | Value::UnsignedInteger(_) | Value::Float(_) => true,
            Value::Parameter(_) | Value::Subquery(_) => !bound,
            _ => false,
        };
        match self.conditions.iter().find(|c| !accepted(&c.value)) {
            Some(condition) => Err(ValidationError::new(
                ValidationErrorKind::InvalidHaving,
                None,
                format!("{:?}", condition.value),
                "a HAVING threshold must be a number: an aggregate compared with \
                 anything else matches no group",
            )),
            None => Ok(()),
        }
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
    agg.argument.column()
}

fn select_aggregations(columns: &SelectColumns) -> &[AggregateFunction] {
    match columns {
        SelectColumns::Aggregations(aggregations) | SelectColumns::Mixed { aggregations, .. } => {
            aggregations
        }
        _ => &[],
    }
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

/// The hash of a group key. Numbers hash by value, which [`normalise`] made
/// one representation per number.
fn hash_key(key: &[serde_json::Value]) -> u64 {
    let mut hasher = FxHasher::default();
    for value in key {
        hash_value(value, &mut hasher);
    }
    hasher.finish()
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
