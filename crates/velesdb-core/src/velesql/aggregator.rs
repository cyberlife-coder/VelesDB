//! Streaming aggregation for VelesQL (EPIC-017 US-002).
//!
//! Implements O(1) memory aggregation using single-pass streaming algorithm.
//! Based on state-of-art practices from DuckDB and DataFusion (arXiv 2024).

// Reason: Numeric casts in aggregation are intentional:
// - u64->f64 for count-to-double conversion: precision loss acceptable for averages
// - Count values are bounded by result set size
#![allow(clippy::cast_precision_loss)]

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashMap;

use serde_json::Number;

use super::ast::{AggregateFunction, AggregateType};
use super::json_order::compare_numbers;

/// Result of aggregation operations.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AggregateResult {
    /// COUNT(*) result.
    pub count: u64,
    /// COUNT(column) results by column name (non-null value counts).
    pub counts: HashMap<String, u64>,
    /// SUM results by column name.
    pub sums: HashMap<String, f64>,
    /// AVG results by column name (computed from sum/count).
    pub avgs: HashMap<String, f64>,
    /// MIN results by column name.
    pub mins: HashMap<String, f64>,
    /// MAX results by column name.
    pub maxs: HashMap<String, f64>,
    /// MIN results by column name, exactly: an integer stays an integer,
    /// beyond 2^53 included, where `mins` holds its `f64`.
    #[serde(default)]
    pub min_values: HashMap<String, Number>,
    /// MAX results by column name, exactly (see `min_values`).
    #[serde(default)]
    pub max_values: HashMap<String, Number>,
}

impl AggregateResult {
    /// The value `agg` computed, as an output row holds it: a count as an
    /// integer, a sum or mean as a float, a minimum or maximum as the value
    /// it is (an integer exactly), `null` where the group had no numeric
    /// value to aggregate. `FIRST` is not a running
    /// aggregate and reads `null` here; its value is the group's best row's
    /// (`velesql::grouping`). `score` names the column of that name, which a
    /// vector GROUP BY fills with each row's search score.
    #[must_use]
    pub(crate) fn value_of(&self, agg: &AggregateFunction) -> serde_json::Value {
        let column = agg.argument.column();
        let float = |values: &HashMap<String, f64>, column: &str| {
            values
                .get(column)
                .map_or(serde_json::Value::Null, |v| serde_json::json!(v))
        };
        match (agg.function_type, column) {
            (AggregateType::Count, None) => serde_json::json!(self.count),
            (AggregateType::Count, Some(column)) => {
                serde_json::json!(self.counts.get(column).copied().unwrap_or(0))
            }
            (AggregateType::Sum, Some(column)) => float(&self.sums, column),
            (AggregateType::Avg, Some(column)) => float(&self.avgs, column),
            (AggregateType::Min, Some(column)) => extreme(&self.min_values, &self.mins, column),
            (AggregateType::Max, Some(column)) => extreme(&self.max_values, &self.maxs, column),
            _ => serde_json::Value::Null,
        }
    }
}

/// A minimum or maximum as the value it is, from `exact`, else as the `f64`
/// `floats` holds (a result deserialized from before `exact` existed).
fn extreme(
    exact: &HashMap<String, Number>,
    floats: &HashMap<String, f64>,
    column: &str,
) -> serde_json::Value {
    match exact.get(column) {
        Some(n) => serde_json::Value::Number(n.clone()),
        None => floats
            .get(column)
            .map_or(serde_json::Value::Null, |v| serde_json::json!(v)),
    }
}

/// Running aggregate state for a single column.
///
/// Collocates sum, count, min, and max together so that the `Aggregator`
/// can maintain them with a single `HashMap` lookup per value instead of four
/// separate lookups across four maps. This also eliminates the cross-map
/// synchronisation invariant that previously required `debug_assert` guards.
///
/// The extremes are kept as the numbers they are, compared exactly
/// ([`compare_numbers`]): an integer past 2^53 stays itself, where an `f64`
/// would round it onto its neighbour.
#[derive(Debug, Clone)]
struct ColumnAgg {
    sum: f64,
    count: u64,
    min: Extreme,
    max: Extreme,
}

impl ColumnAgg {
    fn new(value: &Number) -> Self {
        Self {
            sum: value.as_f64().unwrap_or(0.0),
            count: 1,
            min: Extreme::Finite(value.clone()),
            max: Extreme::Finite(value.clone()),
        }
    }

    fn new_batch(sum: f64, count: u64, min: f64, max: f64) -> Self {
        Self {
            sum,
            count,
            min: Extreme::of_f64(min),
            max: Extreme::of_f64(max),
        }
    }

    #[inline]
    fn update(&mut self, value: &Number) {
        self.sum += value.as_f64().unwrap_or(0.0);
        self.count += 1;
        let extreme = Extreme::Finite(value.clone());
        self.widen(&extreme, &extreme);
    }

    #[inline]
    fn update_batch(&mut self, batch_sum: f64, batch_count: u64, batch_min: f64, batch_max: f64) {
        self.sum += batch_sum;
        self.count += batch_count;
        self.widen(&Extreme::of_f64(batch_min), &Extreme::of_f64(batch_max));
    }

    #[inline]
    fn merge_from(&mut self, other: &Self) {
        self.sum += other.sum;
        self.count += other.count;
        self.widen(&other.min, &other.max);
    }

    /// Takes `min` and `max` as the extremes where they extend them, or
    /// where they carry the better representation of an equal value.
    #[inline]
    fn widen(&mut self, min: &Extreme, max: &Extreme) {
        if replaces(min, &self.min, Ordering::is_lt) {
            self.min = min.clone();
        }
        if replaces(max, &self.max, Ordering::is_gt) {
            self.max = max.clone();
        }
    }
}

/// A column extreme: an exact JSON number, or an infinity a
/// [`Aggregator::process_batch`] batch carried — a value `mins` / `maxs`
/// report but no JSON `Number` (and so `min_values` / `max_values`) holds.
#[derive(Debug, Clone)]
enum Extreme {
    /// `-inf`: no finite value is below it.
    NegInf,
    /// An exact JSON number.
    Finite(Number),
    /// `+inf`: no finite value is above it.
    PosInf,
}

impl Extreme {
    /// An `f64` batch extreme as an `Extreme`: the finite ones are JSON
    /// numbers, an infinity has no JSON form. A NaN is no value —
    /// `process_batch` filters it before this is ever called.
    fn of_f64(value: f64) -> Self {
        if let Some(n) = Number::from_f64(value) {
            Self::Finite(n)
        } else if value.is_sign_negative() {
            Self::NegInf
        } else {
            Self::PosInf
        }
    }

    /// The extreme as `f64`, what `mins` / `maxs` hold.
    fn as_f64(&self) -> f64 {
        match self {
            Self::NegInf => f64::NEG_INFINITY,
            Self::Finite(n) => n.as_f64().unwrap_or(0.0),
            Self::PosInf => f64::INFINITY,
        }
    }

    /// The extreme as the JSON number it is, where it has one.
    fn number(&self) -> Option<&Number> {
        match self {
            Self::Finite(n) => Some(n),
            Self::NegInf | Self::PosInf => None,
        }
    }
}

/// Two extremes in value order: `-inf` below every number, `+inf` above.
fn compare_extremes(a: &Extreme, b: &Extreme) -> Ordering {
    match (a, b) {
        (Extreme::NegInf, Extreme::NegInf) | (Extreme::PosInf, Extreme::PosInf) => Ordering::Equal,
        (Extreme::NegInf, _) | (_, Extreme::PosInf) => Ordering::Less,
        (Extreme::PosInf, _) | (_, Extreme::NegInf) => Ordering::Greater,
        (Extreme::Finite(x), Extreme::Finite(y)) => compare_numbers(x, y),
    }
}

/// Whether `candidate` takes `current`'s place: strictly further in the
/// `strict` direction, or equal by value and the better representation —
/// an integer over a float (`3` over `3.0`, `0` over `-0.0`), `0.0` over
/// `-0.0`, else the incumbent stays.
fn replaces(candidate: &Extreme, current: &Extreme, strict: fn(Ordering) -> bool) -> bool {
    let ordering = compare_extremes(candidate, current);
    strict(ordering) || (ordering.is_eq() && preferred(candidate, current))
}

/// Of two equal extremes, whether `candidate`'s representation wins.
fn preferred(candidate: &Extreme, current: &Extreme) -> bool {
    match (candidate, current) {
        (Extreme::Finite(a), Extreme::Finite(b)) => representation_rank(a) < representation_rank(b),
        _ => false,
    }
}

/// A number's preference as a reported extreme among equal values: an
/// integer first, `0.0` before `-0.0`, any other float keeps the incumbent.
fn representation_rank(n: &Number) -> u8 {
    if n.is_i64() || n.is_u64() {
        0
    } else if n
        .as_f64()
        .is_some_and(|v| v.to_bits() == (-0.0f64).to_bits())
    {
        2
    } else {
        1
    }
}

/// Streaming aggregator - O(1) memory, single-pass.
///
/// Based on online algorithms for computing aggregates without
/// storing all values in memory.
#[derive(Debug, Default)]
pub struct Aggregator {
    /// Running count for COUNT(*).
    count: u64,
    /// Per-column running aggregates over numeric values (sum, count, min,
    /// max in one entry).
    columns: HashMap<String, ColumnAgg>,
    /// Per-column count of non-null values of any type, for COUNT(column).
    present: HashMap<String, u64>,
}

impl Aggregator {
    /// Create a new aggregator.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment the row count (for COUNT(*)).
    pub fn process_count(&mut self) {
        self.count += 1;
    }

    /// Process a value for a specific column's aggregation.
    ///
    /// Updates SUM, MIN, MAX, and count for AVG calculation in a single
    /// HashMap lookup (fast path) or one allocation (slow path on first
    /// occurrence of the column).
    pub fn process_value(&mut self, column: &str, value: &serde_json::Value) {
        if !value.is_null() {
            Self::count_present(&mut self.present, column, 1);
        }
        if let serde_json::Value::Number(num) = value {
            match self.columns.get_mut(column) {
                Some(agg) => agg.update(num),
                None => {
                    self.columns.insert(column.to_string(), ColumnAgg::new(num));
                }
            }
        }
    }

    /// Adds `n` non-null values to `column`'s COUNT(column).
    fn count_present(present: &mut HashMap<String, u64>, column: &str, n: u64) {
        match present.get_mut(column) {
            Some(count) => *count += n,
            None => {
                present.insert(column.to_string(), n);
            }
        }
    }

    /// Process a batch of numeric values for SIMD-friendly aggregation.
    ///
    /// This method processes values in batches, allowing the compiler to
    /// auto-vectorize the loops using SIMD instructions for better performance.
    /// A NaN is no value: it is skipped, counted nowhere, and a batch of
    /// NaNs alone leaves the column untouched. An infinity is a value: it
    /// sums, counts, and stays an infinite extreme in `mins` / `maxs`.
    ///
    /// # Arguments
    /// * `column` - Column name for the aggregation
    /// * `values` - Slice of f64 values to aggregate
    pub fn process_batch(&mut self, column: &str, values: &[f64]) {
        let mut batch_sum = 0.0f64;
        let mut batch_count = 0u64;
        let mut batch_min = f64::INFINITY;
        let mut batch_max = f64::NEG_INFINITY;
        for &value in values {
            if value.is_nan() {
                continue;
            }
            batch_sum += value;
            batch_count += 1;
            batch_min = batch_min.min(value);
            batch_max = batch_max.max(value);
        }
        if batch_count == 0 {
            return;
        }

        Self::count_present(&mut self.present, column, batch_count);

        match self.columns.get_mut(column) {
            Some(agg) => agg.update_batch(batch_sum, batch_count, batch_min, batch_max),
            None => {
                self.columns.insert(
                    column.to_string(),
                    ColumnAgg::new_batch(batch_sum, batch_count, batch_min, batch_max),
                );
            }
        }
    }

    /// Merge another aggregator into this one (for parallel aggregation).
    ///
    /// Combines counts, sums, mins, maxs from the other aggregator.
    /// Used in map-reduce pattern for parallel processing.
    pub fn merge(&mut self, other: Self) {
        self.count += other.count;
        for (col, n) in other.present {
            Self::count_present(&mut self.present, &col, n);
        }
        for (col, other_agg) in other.columns {
            match self.columns.get_mut(&col) {
                Some(agg) => agg.merge_from(&other_agg),
                None => {
                    self.columns.insert(col, other_agg);
                }
            }
        }
    }

    /// Finalize aggregation and return results.
    #[must_use]
    pub fn finalize(self) -> AggregateResult {
        let cap = self.columns.len();
        let mut sums = HashMap::with_capacity(cap);
        let mut avgs = HashMap::with_capacity(cap);
        let mut mins = HashMap::with_capacity(cap);
        let mut maxs = HashMap::with_capacity(cap);
        let mut min_values = HashMap::with_capacity(cap);
        let mut max_values = HashMap::with_capacity(cap);

        for (col, agg) in self.columns {
            if agg.count > 0 {
                avgs.insert(col.clone(), agg.sum / agg.count as f64);
            }
            sums.insert(col.clone(), agg.sum);
            mins.insert(col.clone(), agg.min.as_f64());
            maxs.insert(col.clone(), agg.max.as_f64());
            if let Some(n) = agg.min.number() {
                min_values.insert(col.clone(), n.clone());
            }
            if let Some(n) = agg.max.number() {
                max_values.insert(col, n.clone());
            }
        }

        AggregateResult {
            count: self.count,
            counts: self.present,
            sums,
            avgs,
            mins,
            maxs,
            min_values,
            max_values,
        }
    }
}

// Tests moved to aggregator_tests.rs per project rules
