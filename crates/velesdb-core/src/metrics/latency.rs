//! Latency measurement and percentile statistics.
//!
//! Provides tools for computing latency percentiles (p50, p95, p99)
//! from duration samples, useful for performance monitoring.

use std::time::Duration;

/// Statistics for latency measurements including percentiles.
///
/// Percentiles are more useful than mean for understanding real-world
/// performance, especially p99 which shows worst-case latency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LatencyStats {
    /// Minimum latency observed
    pub min: Duration,
    /// Maximum latency observed
    pub max: Duration,
    /// Mean (average) latency
    pub mean: Duration,
    /// 50th percentile (median)
    pub p50: Duration,
    /// 95th percentile
    pub p95: Duration,
    /// 99th percentile
    pub p99: Duration,
}

impl Default for LatencyStats {
    fn default() -> Self {
        Self {
            min: Duration::ZERO,
            max: Duration::ZERO,
            mean: Duration::ZERO,
            p50: Duration::ZERO,
            p95: Duration::ZERO,
            p99: Duration::ZERO,
        }
    }
}

/// Computes latency percentiles from a list of duration samples.
///
/// # Arguments
///
/// * `samples` - List of latency measurements
///
/// # Returns
///
/// A `LatencyStats` struct with min, max, mean, p50, p95, and p99.
///
/// # Example
///
/// ```rust
/// use std::time::Duration;
/// use velesdb_core::metrics::compute_latency_percentiles;
///
/// let samples: Vec<Duration> = (1..=100)
///     .map(|i| Duration::from_micros(i * 10))
///     .collect();
///
/// let stats = compute_latency_percentiles(&samples);
/// println!("p50: {:?}, p99: {:?}", stats.p50, stats.p99);
/// ```
#[must_use]
pub fn compute_latency_percentiles(samples: &[Duration]) -> LatencyStats {
    if samples.is_empty() {
        return LatencyStats::default();
    }

    let mut sorted: Vec<Duration> = samples.to_vec();
    sorted.sort();

    let n = sorted.len();
    let sum: Duration = sorted.iter().sum();

    // SAFETY: Division result is bounded by sum.as_nanos() which came from Duration values.
    // The mean of durations cannot exceed the maximum duration, which fits in u64 nanoseconds.
    #[allow(clippy::cast_possible_truncation)]
    let mean = if n > 0 {
        Duration::from_nanos((sum.as_nanos() / n as u128) as u64)
    } else {
        Duration::ZERO
    };

    LatencyStats {
        min: sorted[0],
        max: sorted[n - 1],
        mean,
        p50: percentile(&sorted, 50),
        p95: percentile(&sorted, 95),
        p99: percentile(&sorted, 99),
    }
}

/// Computes a percentile from a sorted list of durations.
fn percentile(sorted: &[Duration], p: usize) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }

    let n = sorted.len();
    // SAFETY: Percentile calculation produces index in [0, n-1] range.
    // - p is in [0, 100], so p/100.0 is in [0.0, 1.0]
    // - Multiplied by (n-1), result is in [0.0, n-1.0]
    // - After round(), result is non-negative and <= n-1, fitting in usize
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let idx = ((p as f64 / 100.0) * (n - 1) as f64).round() as usize;
    sorted[idx.min(n - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_latency_stats_empty() {
        let samples: Vec<Duration> = vec![];
        let stats = compute_latency_percentiles(&samples);
        assert_eq!(stats.min, Duration::ZERO);
        assert_eq!(stats.max, Duration::ZERO);
    }

    #[test]
    fn test_latency_stats_single() {
        let samples = vec![Duration::from_micros(100)];
        let stats = compute_latency_percentiles(&samples);
        assert_eq!(stats.min, Duration::from_micros(100));
        assert_eq!(stats.max, Duration::from_micros(100));
    }

    #[test]
    fn test_latency_stats_multiple() {
        let samples: Vec<Duration> = (1..=100).map(|i| Duration::from_micros(i * 10)).collect();
        let stats = compute_latency_percentiles(&samples);
        assert_eq!(stats.min, Duration::from_micros(10));
        assert_eq!(stats.max, Duration::from_micros(1000));
        assert!(stats.p50 > Duration::ZERO);
        assert!(stats.p99 > stats.p50);
    }

    #[test]
    fn test_latency_stats_default() {
        let stats = LatencyStats::default();
        assert_eq!(stats.min, Duration::ZERO);
        assert_eq!(stats.max, Duration::ZERO);
        assert_eq!(stats.mean, Duration::ZERO);
    }
}
