//! Storage operation metrics for monitoring and debugging.
//!
//! Provides latency tracking for critical storage operations:
//! - `ensure_capacity`: mmap resize operations (P0 - critical for P99 latency)
//!
//! # P0 Audit Recommendation
//!
//! The `ensure_capacity` operation can cause "stop-the-world" pauses during
//! large resizes (e.g., 2GB → 4GB). Monitoring P99 latency is essential.
//!
//! # PERF-001: Lock-Free Implementation
//!
//! Uses `LockFreeHistogram` for wait-free latency recording in the hot path.
//! No mutex contention even under high concurrency.

use super::histogram::LockFreeHistogram;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Storage operation metrics collector.
///
/// Thread-safe metrics collection using lock-free data structures.
/// Designed for minimal overhead in the hot path (wait-free recording).
#[derive(Debug)]
pub struct StorageMetrics {
    /// Total number of actual resize operations
    resize_count: AtomicU64,
    /// Total bytes resized
    total_bytes_resized: AtomicU64,
    /// PERF-001: Lock-free histogram for latency tracking
    latency_histogram: LockFreeHistogram,
}

impl Default for StorageMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl StorageMetrics {
    /// Creates a new metrics collector.
    #[must_use]
    pub fn new() -> Self {
        Self {
            resize_count: AtomicU64::new(0),
            total_bytes_resized: AtomicU64::new(0),
            latency_histogram: LockFreeHistogram::new(),
        }
    }

    /// Records an `ensure_capacity` operation. Wait-free operation.
    ///
    /// # Arguments
    ///
    /// * `latency` - Duration of the operation
    /// * `did_resize` - Whether an actual resize occurred
    /// * `bytes_resized` - Number of bytes added (0 if no resize)
    #[inline]
    pub fn record_ensure_capacity(&self, latency: Duration, did_resize: bool, bytes_resized: u64) {
        // PERF-001: Wait-free latency recording
        #[allow(clippy::cast_possible_truncation)]
        let micros = latency.as_micros().min(u128::from(u64::MAX)) as u64;
        self.latency_histogram.record(micros);

        if did_resize {
            self.resize_count.fetch_add(1, Ordering::Relaxed);
            self.total_bytes_resized
                .fetch_add(bytes_resized, Ordering::Relaxed);
        }
    }

    /// Returns the total number of `ensure_capacity` calls.
    #[must_use]
    pub fn ensure_capacity_count(&self) -> u64 {
        self.latency_histogram.count()
    }

    /// Returns true if no metrics have been recorded yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.latency_histogram.is_empty()
    }

    /// Returns the number of actual resize operations.
    #[must_use]
    pub fn resize_count(&self) -> u64 {
        self.resize_count.load(Ordering::Relaxed)
    }

    /// Returns the total bytes resized.
    #[must_use]
    pub fn total_bytes_resized(&self) -> u64 {
        self.total_bytes_resized.load(Ordering::Relaxed)
    }

    /// Returns latency statistics for `ensure_capacity` operations.
    #[must_use]
    pub fn ensure_capacity_latency_stats(&self) -> LatencyStats {
        LatencyStats {
            count: self.latency_histogram.count(),
            min_us: self.latency_histogram.min(),
            max_us: self.latency_histogram.max(),
            mean_us: self.latency_histogram.mean(),
            p50_us: self.latency_histogram.percentile(50),
            p95_us: self.latency_histogram.percentile(95),
            p99_us: self.latency_histogram.percentile(99),
        }
    }

    /// Resets all metrics to zero.
    pub fn reset(&self) {
        self.resize_count.store(0, Ordering::Relaxed);
        self.total_bytes_resized.store(0, Ordering::Relaxed);
        self.latency_histogram.reset();
    }
}

/// Latency statistics with percentiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LatencyStats {
    /// Number of samples
    pub count: u64,
    /// Minimum latency in microseconds
    pub min_us: u64,
    /// Maximum latency in microseconds
    pub max_us: u64,
    /// Mean latency in microseconds
    pub mean_us: u64,
    /// 50th percentile (median) in microseconds
    pub p50_us: u64,
    /// 95th percentile in microseconds
    pub p95_us: u64,
    /// 99th percentile in microseconds
    pub p99_us: u64,
}

impl LatencyStats {
    /// Returns the P99 latency as a Duration.
    #[must_use]
    pub fn p99(&self) -> Duration {
        Duration::from_micros(self.p99_us)
    }

    /// Returns the P95 latency as a Duration.
    #[must_use]
    pub fn p95(&self) -> Duration {
        Duration::from_micros(self.p95_us)
    }

    /// Returns the P50 (median) latency as a Duration.
    #[must_use]
    pub fn p50(&self) -> Duration {
        Duration::from_micros(self.p50_us)
    }

    /// Returns the mean latency as a Duration.
    #[must_use]
    pub fn mean(&self) -> Duration {
        Duration::from_micros(self.mean_us)
    }

    /// Returns true if P99 latency exceeds the threshold.
    ///
    /// # Arguments
    ///
    /// * `threshold` - Maximum acceptable P99 latency
    #[must_use]
    pub fn p99_exceeds(&self, threshold: Duration) -> bool {
        self.p99() > threshold
    }
}

/// RAII guard for timing operations.
///
/// Automatically records the elapsed time when dropped.
pub struct TimingGuard<'a, F>
where
    F: FnOnce(Duration),
{
    start: Instant,
    callback: Option<F>,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<F> TimingGuard<'_, F>
where
    F: FnOnce(Duration),
{
    /// Creates a new timing guard that will call the callback with elapsed time on drop.
    pub fn new(callback: F) -> Self {
        Self {
            start: Instant::now(),
            callback: Some(callback),
            _marker: std::marker::PhantomData,
        }
    }

    /// Returns the elapsed time since creation.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }
}

impl<F> Drop for TimingGuard<'_, F>
where
    F: FnOnce(Duration),
{
    fn drop(&mut self) {
        if let Some(cb) = self.callback.take() {
            cb(self.start.elapsed());
        }
    }
}
