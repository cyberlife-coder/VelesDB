//! Lock-free histogram implementation for latency tracking.
//!
//! # PERF-001: Lock-Free Histogram
//!
//! Uses atomic counters in logarithmic buckets to avoid mutex contention.
//! Provides approximate percentiles without allocations in the hot path.
//!
//! # Design
//!
//! - 64 buckets covering 1µs to ~18 hours (log2 scale)
//! - Each bucket is an `AtomicU64` counter
//! - `record()` is wait-free (single atomic increment)
//! - `percentile()` requires reading all buckets (still lock-free)

use std::sync::atomic::{AtomicU64, Ordering};

/// Number of histogram buckets (covers 1µs to ~18h with log2 scale).
const NUM_BUCKETS: usize = 64;

/// Lock-free histogram for latency measurements.
///
/// Uses logarithmic bucketing for memory efficiency while maintaining
/// accuracy across a wide range of latencies (1µs to hours).
#[derive(Debug)]
pub struct LockFreeHistogram {
    /// Bucket counters (log2 scale, each bucket is 2x the previous)
    buckets: [AtomicU64; NUM_BUCKETS],
    /// Total count of all recorded values
    count: AtomicU64,
    /// Sum of all recorded values (for mean calculation)
    sum: AtomicU64,
    /// Minimum value seen
    min: AtomicU64,
    /// Maximum value seen
    max: AtomicU64,
}

impl Default for LockFreeHistogram {
    fn default() -> Self {
        Self::new()
    }
}

impl LockFreeHistogram {
    /// Creates a new empty histogram.
    #[must_use]
    pub fn new() -> Self {
        Self {
            buckets: std::array::from_fn(|_| AtomicU64::new(0)),
            count: AtomicU64::new(0),
            sum: AtomicU64::new(0),
            min: AtomicU64::new(u64::MAX),
            max: AtomicU64::new(0),
        }
    }

    /// Records a value in microseconds. Wait-free operation.
    #[inline]
    pub fn record(&self, value_us: u64) {
        let bucket = Self::bucket_for(value_us);
        self.buckets[bucket].fetch_add(1, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
        self.sum.fetch_add(value_us, Ordering::Relaxed);

        // Update min (CAS loop)
        let mut current_min = self.min.load(Ordering::Relaxed);
        while value_us < current_min {
            match self.min.compare_exchange_weak(
                current_min,
                value_us,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current_min = actual,
            }
        }

        // Update max (CAS loop)
        let mut current_max = self.max.load(Ordering::Relaxed);
        while value_us > current_max {
            match self.max.compare_exchange_weak(
                current_max,
                value_us,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current_max = actual,
            }
        }
    }

    /// Returns the bucket index for a value (log2 scale).
    #[inline]
    fn bucket_for(value_us: u64) -> usize {
        if value_us == 0 {
            0
        } else {
            // log2(value) clamped to bucket range
            (64 - value_us.leading_zeros()) as usize - 1
        }
        .min(NUM_BUCKETS - 1)
    }

    /// Returns the approximate value for a bucket (midpoint).
    #[inline]
    fn value_for_bucket(bucket: usize) -> u64 {
        if bucket == 0 {
            1
        } else {
            // Midpoint of bucket range: 2^bucket + 2^(bucket-1)
            (1u64 << bucket) + (1u64 << (bucket.saturating_sub(1)))
        }
    }

    /// Returns the total count of recorded values.
    #[must_use]
    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    /// Returns true if no values have been recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count.load(Ordering::Relaxed) == 0
    }

    /// Returns the minimum recorded value.
    #[must_use]
    pub fn min(&self) -> u64 {
        let min = self.min.load(Ordering::Relaxed);
        if min == u64::MAX {
            0
        } else {
            min
        }
    }

    /// Returns the maximum recorded value.
    #[must_use]
    pub fn max(&self) -> u64 {
        self.max.load(Ordering::Relaxed)
    }

    /// Returns the mean of recorded values.
    #[must_use]
    pub fn mean(&self) -> u64 {
        let count = self.count.load(Ordering::Relaxed);
        if count == 0 {
            return 0;
        }
        self.sum.load(Ordering::Relaxed) / count
    }

    /// Returns an approximate percentile value (0-100).
    ///
    /// Uses linear interpolation within buckets for accuracy.
    /// Result is capped by the actual max value recorded.
    #[must_use]
    pub fn percentile(&self, p: u8) -> u64 {
        let total = self.count.load(Ordering::Relaxed);
        if total == 0 {
            return 0;
        }

        let max_val = self.max();

        #[allow(clippy::cast_possible_truncation)]
        let target = (u128::from(total) * u128::from(p.min(100)) / 100) as u64;
        let mut cumulative = 0u64;

        for (i, bucket) in self.buckets.iter().enumerate() {
            let bucket_count = bucket.load(Ordering::Relaxed);
            cumulative += bucket_count;

            if cumulative >= target {
                // Cap by actual max to avoid bucket approximation exceeding real max
                return Self::value_for_bucket(i).min(max_val);
            }
        }

        max_val
    }

    /// Resets all counters to zero.
    pub fn reset(&self) {
        for bucket in &self.buckets {
            bucket.store(0, Ordering::Relaxed);
        }
        self.count.store(0, Ordering::Relaxed);
        self.sum.store(0, Ordering::Relaxed);
        self.min.store(u64::MAX, Ordering::Relaxed);
        self.max.store(0, Ordering::Relaxed);
    }
}
