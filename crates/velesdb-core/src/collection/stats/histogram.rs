//! Histogram data structures for equi-depth column value distribution estimation.
//!
//! Provides [`Histogram`] and [`HistogramBucket`] used by the CBO to estimate
//! predicate selectivity via binary search on bucket boundaries.

// Reason: u64→f64 casts are intentional for selectivity ratio computation.
// Values are bounded by collection size; precision loss is acceptable for statistics.
#![allow(clippy::cast_precision_loss)]

use serde::{Deserialize, Serialize};

/// Returns the next representable `f64` above `val`.
///
/// Unlike `val + f64::EPSILON`, this works correctly for all magnitudes.
/// `f64::EPSILON` is only the ULP at 1.0; for values ≥ 2.0, adding EPSILON
/// is a no-op because EPSILON is smaller than the unit-in-last-place.
///
/// Uses IEEE 754 bit manipulation: incrementing (or decrementing for negative
/// values) the integer representation of a float yields the next float.
pub(crate) fn next_after(v: f64) -> f64 {
    if v.is_nan() || v == f64::INFINITY {
        return v;
    }
    if v == 0.0 {
        return f64::from_bits(1);
    }
    let bits = v.to_bits();
    let next_bits = if v > 0.0 { bits + 1 } else { bits - 1 };
    f64::from_bits(next_bits)
}

/// A single bucket in an equi-depth histogram.
///
/// Represents a contiguous range `[lower_bound, upper_bound)` of column values
/// with associated row count and distinct value count. Bucket boundaries use
/// `f64` to unify Int, Float, and String (ordinal rank) columns.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct HistogramBucket {
    /// Inclusive lower bound for the bucket.
    pub lower_bound: f64,
    /// Exclusive upper bound for the bucket.
    pub upper_bound: f64,
    /// Number of sampled rows in the bucket.
    pub count: u64,
    /// Number of distinct values in the bucket.
    #[serde(default)]
    pub distinct_count: u64,
}

/// Equi-depth histogram for column value distribution estimation.
///
/// Buckets are sorted by `lower_bound` and non-overlapping. The CBO uses
/// binary search (`O(log B)`) on bucket boundaries for all selectivity lookups.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Histogram {
    /// Ordered, non-overlapping histogram buckets.
    pub buckets: Vec<HistogramBucket>,
    /// Total number of rows represented by this histogram (sum of all bucket counts).
    #[serde(default)]
    pub total_count: u64,
    /// Cumulative number of incremental updates since last full ANALYZE.
    #[serde(default)]
    pub incremental_updates: u64,
    /// Whether the histogram is considered stale (updates > 20% of total_count).
    #[serde(default)]
    pub stale: bool,
}

impl Histogram {
    /// Finds the bucket index containing `value` via binary search.
    ///
    /// Returns the index of the bucket whose range `[lower_bound, upper_bound)`
    /// contains `value`. Returns `None` if `value` is outside all bucket ranges.
    ///
    /// Complexity: O(log B) where B = number of buckets. No allocations.
    #[must_use]
    pub fn find_bucket(&self, value: f64) -> Option<usize> {
        let buckets = &self.buckets;
        if buckets.is_empty() {
            return None;
        }
        // Binary search: find the rightmost bucket whose lower_bound <= value
        let idx = buckets.partition_point(|b| b.lower_bound <= value);
        if idx == 0 {
            return None;
        }
        let candidate = idx - 1;
        if value < buckets[candidate].upper_bound {
            Some(candidate)
        } else {
            None
        }
    }

    /// Estimates equality selectivity for a given value.
    ///
    /// If the value falls within a bucket with `distinct_count > 0`, returns
    /// `bucket.count / (bucket.distinct_count × total_count)`.
    /// If `distinct_count == 0` or value is outside all buckets, returns
    /// `1 / total_count`. Returns `0.0` when `total_count == 0`.
    /// Result is clamped to `[0.0, 1.0]`.
    #[must_use]
    pub fn estimate_eq_selectivity(&self, value: f64) -> f64 {
        let total = self.bucket_sum();
        if total == 0 {
            return 0.0;
        }
        let sel = if let Some(idx) = self.find_bucket(value) {
            let bucket = &self.buckets[idx];
            if bucket.distinct_count > 0 {
                bucket.count as f64 / (bucket.distinct_count as f64 * total as f64)
            } else {
                1.0 / total.max(1) as f64
            }
        } else {
            1.0 / total.max(1) as f64
        };
        sel.clamp(0.0, 1.0)
    }

    /// Estimates less-than selectivity for a given value.
    ///
    /// Sums counts of all buckets fully below `value`, plus linear interpolation
    /// of the partial bucket containing `value`. Divides by `total_count`.
    /// Returns `0.0` if value ≤ first bucket lower bound, `1.0` if value ≥ last
    /// bucket upper bound. Result is clamped to `[0.0, 1.0]`.
    #[must_use]
    pub fn estimate_lt_selectivity(&self, value: f64) -> f64 {
        let total = self.bucket_sum();
        if self.buckets.is_empty() || total == 0 {
            return 0.0;
        }
        if value <= self.buckets[0].lower_bound {
            return 0.0;
        }
        if value >= self.buckets[self.buckets.len() - 1].upper_bound {
            return 1.0;
        }
        let count_below = accumulate_lt_count(&self.buckets, value);
        (count_below / total as f64).clamp(0.0, 1.0)
    }

    /// Estimates range selectivity for `[low, high]`.
    ///
    /// Sums full buckets within the range plus interpolates boundary buckets.
    /// Returns `0.0` if `low > high` or range is outside the histogram.
    /// Returns `1.0` if range encompasses the entire histogram.
    /// Result is clamped to `[0.0, 1.0]`.
    #[must_use]
    pub fn estimate_range_selectivity(&self, low: f64, high: f64) -> f64 {
        if let Some(shortcut) = self.range_selectivity_shortcut(low, high) {
            return shortcut;
        }
        let total = self.bucket_sum();
        let mut count_in_range: f64 = 0.0;
        for bucket in &self.buckets {
            if let Some(fraction) = bucket_range_fraction(bucket, low, high) {
                count_in_range += bucket.count as f64 * fraction;
            }
        }
        (count_in_range / total as f64).clamp(0.0, 1.0)
    }

    /// Returns a short-circuit selectivity if the range can be resolved without
    /// iterating buckets (empty histogram, out-of-bounds, or full coverage).
    fn range_selectivity_shortcut(&self, low: f64, high: f64) -> Option<f64> {
        if low > high || self.buckets.is_empty() || self.bucket_sum() == 0 {
            return Some(0.0);
        }
        let first_lower = self.buckets[0].lower_bound;
        let last_upper = self.buckets[self.buckets.len() - 1].upper_bound;
        if low >= last_upper || high <= first_lower {
            return Some(0.0);
        }
        if low <= first_lower && high >= last_upper {
            return Some(1.0);
        }
        None
    }

    /// Increments the count of the bucket containing `value`.
    ///
    /// Finds the bucket via binary search and increments its count by 1.
    /// Increments `incremental_updates` by 1. If `incremental_updates`
    /// exceeds 20% of `total_count`, marks the histogram as stale.
    /// No-op if `value` is outside all bucket ranges.
    pub fn increment_bucket(&mut self, value: f64) {
        if let Some(idx) = self.find_bucket(value) {
            self.buckets[idx].count += 1;
            self.incremental_updates += 1;
            self.check_staleness();
        }
    }

    /// Decrements the count of the bucket containing `value`, floored at zero.
    ///
    /// Finds the bucket via binary search and decrements its count by 1
    /// (minimum 0). Increments `incremental_updates` by 1. Checks staleness.
    /// No-op if `value` is outside all bucket ranges.
    pub fn decrement_bucket(&mut self, value: f64) {
        if let Some(idx) = self.find_bucket(value) {
            self.buckets[idx].count = self.buckets[idx].count.saturating_sub(1);
            self.incremental_updates += 1;
            self.check_staleness();
        }
    }

    /// Returns the sum of all bucket counts — the effective total for selectivity.
    ///
    /// `total_count` captures the ANALYZE-time snapshot and is used only for
    /// staleness detection. After incremental updates the actual denominator
    /// is the live sum of bucket counts, keeping estimates accurate.
    fn bucket_sum(&self) -> u64 {
        self.buckets.iter().map(|b| b.count).sum()
    }

    /// Checks if incremental updates exceed the 20% staleness threshold.
    fn check_staleness(&mut self) {
        if self.total_count > 0 && self.incremental_updates > self.total_count / 5 {
            self.stale = true;
        }
    }
}

/// Accumulates the count of rows below `value` across sorted buckets.
///
/// For each bucket: if entirely below `value`, adds its full count;
/// if partially overlapping, adds a linearly interpolated fraction;
/// stops at the first bucket beyond `value`.
fn accumulate_lt_count(buckets: &[HistogramBucket], value: f64) -> f64 {
    let mut count_below: f64 = 0.0;
    for bucket in buckets {
        if bucket.upper_bound <= value {
            count_below += bucket.count as f64;
        } else if bucket.lower_bound < value {
            let width = bucket.upper_bound - bucket.lower_bound;
            if width > 0.0 {
                count_below += bucket.count as f64 * ((value - bucket.lower_bound) / width);
            }
            break;
        } else {
            break;
        }
    }
    count_below
}

/// Returns the fraction of `bucket` that overlaps the range `[low, high]`.
///
/// Returns `None` if the bucket is entirely outside the range or has zero width.
/// Otherwise returns `Some((eff_high - eff_low) / width)` where the effective
/// bounds are clamped to the bucket boundaries.
fn bucket_range_fraction(bucket: &HistogramBucket, low: f64, high: f64) -> Option<f64> {
    if bucket.upper_bound <= low || bucket.lower_bound >= high {
        return None;
    }
    let width = bucket.upper_bound - bucket.lower_bound;
    if width <= 0.0 {
        return None;
    }
    let eff_low = low.max(bucket.lower_bound);
    let eff_high = high.min(bucket.upper_bound);
    Some((eff_high - eff_low) / width)
}

/// Default number of histogram buckets.
const DEFAULT_NUM_BUCKETS: usize = 64;

/// Builder for constructing equi-depth histograms from sampled column values.
///
/// Sorts the input values, splits them into approximately equal-sized buckets,
/// and computes per-bucket distinct counts. No allocations occur after the
/// initial sort — bucket construction operates on slices.
pub(crate) struct HistogramBuilder {
    /// Target number of buckets.
    num_buckets: usize,
}

impl HistogramBuilder {
    /// Creates a builder with the specified bucket count.
    ///
    /// If `num_buckets` is 0, defaults to 64.
    #[must_use]
    pub fn new(num_buckets: usize) -> Self {
        Self {
            num_buckets: if num_buckets == 0 {
                DEFAULT_NUM_BUCKETS
            } else {
                num_buckets
            },
        }
    }

    /// Builds an equi-depth histogram from a mutable slice of `f64` values.
    ///
    /// NaN values are filtered out. Empty input produces an empty histogram.
    /// Sets `total_count` to the number of non-NaN values processed.
    #[must_use]
    pub fn build(&self, values: &mut [f64]) -> Histogram {
        let valid_len = partition_nan(values);
        let valid = &mut values[..valid_len];
        if valid.is_empty() {
            return Histogram::default();
        }
        valid.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let distinct = count_distinct(valid);
        let buckets = if distinct == 1 {
            build_single_value_buckets(valid)
        } else if distinct < self.num_buckets {
            build_per_distinct_buckets(valid, distinct)
        } else {
            build_equidepth_buckets(valid, self.num_buckets)
        };
        Histogram {
            buckets,
            total_count: valid_len as u64,
            incremental_updates: 0,
            stale: false,
        }
    }
}

/// Partitions NaN values to the end, returns the count of non-NaN values.
fn partition_nan(values: &mut [f64]) -> usize {
    let mut valid = 0;
    for i in 0..values.len() {
        if !values[i].is_nan() {
            values.swap(valid, i);
            valid += 1;
        }
    }
    valid
}

/// Counts distinct values in a sorted slice.
#[allow(clippy::float_cmp)]
fn count_distinct(sorted: &[f64]) -> usize {
    if sorted.is_empty() {
        return 0;
    }
    // Reason: exact equality is intentional — values come from the same sorted
    // input, so bit-identical duplicates must be grouped together.
    1 + sorted.windows(2).filter(|w| w[0] != w[1]).count()
}

/// Counts distinct values in a sorted sub-slice.
fn slice_distinct_count(sorted: &[f64]) -> u64 {
    count_distinct(sorted) as u64
}

/// Builds a single bucket for a column with exactly one distinct value.
fn build_single_value_buckets(sorted: &[f64]) -> Vec<HistogramBucket> {
    let val = sorted[0];
    vec![HistogramBucket {
        lower_bound: val,
        upper_bound: next_after(val),
        count: sorted.len() as u64,
        distinct_count: 1,
    }]
}

/// Builds one bucket per distinct value when distinct < num_buckets.
#[allow(clippy::float_cmp)]
fn build_per_distinct_buckets(sorted: &[f64], distinct: usize) -> Vec<HistogramBucket> {
    let mut buckets = Vec::with_capacity(distinct);
    let mut i = 0;
    while i < sorted.len() {
        let val = sorted[i];
        let start = i;
        // Reason: exact equality is intentional — grouping bit-identical values.
        while i < sorted.len() && sorted[i] == val {
            i += 1;
        }
        let next_bound = if i < sorted.len() {
            sorted[i]
        } else {
            next_after(val)
        };
        buckets.push(HistogramBucket {
            lower_bound: val,
            upper_bound: next_bound,
            count: (i - start) as u64,
            distinct_count: 1,
        });
    }
    buckets
}

/// Builds equi-depth buckets by splitting sorted values into equal-sized chunks.
///
/// After chunking, merges any zero-width buckets (`lower_bound == upper_bound`)
/// into adjacent buckets. Zero-width buckets arise from duplicate-heavy data where
/// a chunk boundary falls inside a run of identical values — their counts inflate
/// `bucket_sum()` without contributing to any selectivity lookup.
fn build_equidepth_buckets(sorted: &[f64], num_buckets: usize) -> Vec<HistogramBucket> {
    let chunk_size = sorted.len().div_ceil(num_buckets);
    let mut buckets = Vec::with_capacity(num_buckets);
    for chunk in sorted.chunks(chunk_size) {
        let lower = chunk[0];
        let upper = upper_bound_for_chunk(chunk, sorted, &buckets);
        buckets.push(HistogramBucket {
            lower_bound: lower,
            upper_bound: upper,
            count: chunk.len() as u64,
            distinct_count: slice_distinct_count(chunk),
        });
    }
    merge_zero_width_buckets(buckets)
}

/// Merges zero-width buckets (`lower_bound == upper_bound`) into adjacent buckets.
///
/// Zero-width buckets are absorbed into the nearest **non-zero-width** neighbor.
/// Leading zero-width buckets are merged forward into the first non-zero-width
/// bucket; trailing ones are merged backward into the last non-zero-width bucket.
///
/// Counts and distinct counts are summed into the absorbing bucket. If every
/// bucket is zero-width (all values identical), the input is returned unchanged —
/// this case is already handled by `build_single_value_buckets` upstream.
#[allow(clippy::float_cmp)]
pub(crate) fn merge_zero_width_buckets(buckets: Vec<HistogramBucket>) -> Vec<HistogramBucket> {
    if buckets.is_empty() {
        return buckets;
    }
    // Accumulator for leading/pending zero-width bucket counts.
    let mut pending_count: u64 = 0;
    let mut pending_distinct: u64 = 0;
    let mut result: Vec<HistogramBucket> = Vec::with_capacity(buckets.len());
    for bucket in buckets {
        // Reason: exact equality is intentional — zero-width means the chunk
        // contained only identical values whose upper bound equals the next
        // chunk's lower bound.
        if bucket.lower_bound == bucket.upper_bound {
            pending_count += bucket.count;
            pending_distinct += bucket.distinct_count;
        } else {
            // Absorb any pending zero-width counts into this non-zero-width bucket.
            let mut merged = bucket;
            merged.count += pending_count;
            merged.distinct_count += pending_distinct;
            pending_count = 0;
            pending_distinct = 0;
            result.push(merged);
        }
    }
    // Trailing zero-width buckets: merge into the last non-zero-width bucket.
    if pending_count > 0 {
        if let Some(last) = result.last_mut() {
            last.count += pending_count;
            last.distinct_count += pending_distinct;
        }
        // else: all buckets were zero-width — handled by build_single_value_buckets
    }
    result
}

/// Computes the upper bound for an equi-depth chunk.
///
/// Uses the next chunk's first value if available, otherwise last value + epsilon.
fn upper_bound_for_chunk(chunk: &[f64], sorted: &[f64], existing: &[HistogramBucket]) -> f64 {
    let chunk_end_offset = existing.iter().map(|b| b.count as usize).sum::<usize>() + chunk.len();
    if chunk_end_offset < sorted.len() {
        sorted[chunk_end_offset]
    } else {
        next_after(chunk[chunk.len() - 1])
    }
}
