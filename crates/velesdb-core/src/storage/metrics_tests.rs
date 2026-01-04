//! Tests for storage metrics module.
//!
//! Note: `LockFreeHistogram` tests are in `histogram.rs`.

use super::metrics::{LatencyStats, StorageMetrics};
use parking_lot::Mutex;
use std::thread;
use std::time::Duration;

// =============================================================================
// StorageMetrics Basic Tests
// =============================================================================

#[test]
fn test_metrics_new_is_empty() {
    // Arrange & Act
    let metrics = StorageMetrics::new();

    // Assert
    assert_eq!(metrics.ensure_capacity_count(), 0);
    assert_eq!(metrics.resize_count(), 0);
    assert_eq!(metrics.total_bytes_resized(), 0);
}

#[test]
fn test_metrics_record_ensure_capacity_no_resize() {
    // Arrange
    let metrics = StorageMetrics::new();
    let latency = Duration::from_micros(100);

    // Act
    metrics.record_ensure_capacity(latency, false, 0);

    // Assert
    assert_eq!(metrics.ensure_capacity_count(), 1);
    assert_eq!(metrics.resize_count(), 0);
    assert_eq!(metrics.total_bytes_resized(), 0);
}

#[test]
fn test_metrics_record_ensure_capacity_with_resize() {
    // Arrange
    let metrics = StorageMetrics::new();
    let latency = Duration::from_millis(50);
    let bytes_resized = 64 * 1024 * 1024; // 64MB

    // Act
    metrics.record_ensure_capacity(latency, true, bytes_resized);

    // Assert
    assert_eq!(metrics.ensure_capacity_count(), 1);
    assert_eq!(metrics.resize_count(), 1);
    assert_eq!(metrics.total_bytes_resized(), bytes_resized);
}

#[test]
fn test_metrics_multiple_operations() {
    // Arrange
    let metrics = StorageMetrics::new();

    // Act - 10 operations, 3 with resize
    for i in 0..10 {
        let did_resize = i % 3 == 0; // 0, 3, 6, 9 = 4 resizes
        let bytes = if did_resize { 1024 } else { 0 };
        metrics.record_ensure_capacity(Duration::from_micros(100), did_resize, bytes);
    }

    // Assert
    assert_eq!(metrics.ensure_capacity_count(), 10);
    assert_eq!(metrics.resize_count(), 4); // 0, 3, 6, 9
    assert_eq!(metrics.total_bytes_resized(), 4 * 1024);
}

#[test]
fn test_metrics_reset() {
    // Arrange
    let metrics = StorageMetrics::new();
    metrics.record_ensure_capacity(Duration::from_micros(100), true, 1024);

    // Act
    metrics.reset();

    // Assert
    assert_eq!(metrics.ensure_capacity_count(), 0);
    assert_eq!(metrics.resize_count(), 0);
    assert_eq!(metrics.total_bytes_resized(), 0);
    assert_eq!(metrics.ensure_capacity_latency_stats().count, 0);
}

// =============================================================================
// Latency Statistics Tests
// =============================================================================

#[test]
fn test_latency_stats_empty() {
    // Arrange
    let metrics = StorageMetrics::new();

    // Act
    let stats = metrics.ensure_capacity_latency_stats();

    // Assert
    assert_eq!(stats.count, 0);
    assert_eq!(stats.min_us, 0);
    assert_eq!(stats.max_us, 0);
    assert_eq!(stats.p99_us, 0);
}

#[test]
fn test_latency_stats_single_sample() {
    // Arrange
    let metrics = StorageMetrics::new();
    metrics.record_ensure_capacity(Duration::from_micros(500), false, 0);

    // Act
    let stats = metrics.ensure_capacity_latency_stats();

    // Assert
    assert_eq!(stats.count, 1);
    assert_eq!(stats.min_us, 500);
    assert_eq!(stats.max_us, 500);
    // PERF-001: LockFreeHistogram uses log2 buckets, percentiles are approximate
    // 500µs falls in bucket 8 (256-512), midpoint ~384
    assert!(stats.p50_us > 0, "P50 should be non-zero");
    assert!(stats.p99_us > 0, "P99 should be non-zero");
}

#[test]
fn test_latency_stats_percentiles() {
    // Arrange
    let metrics = StorageMetrics::new();

    // Add 100 samples: 1, 2, 3, ..., 100 microseconds
    for i in 1..=100 {
        metrics.record_ensure_capacity(Duration::from_micros(i), false, 0);
    }

    // Act
    let stats = metrics.ensure_capacity_latency_stats();

    // Assert
    assert_eq!(stats.count, 100);
    assert_eq!(stats.min_us, 1);
    assert_eq!(stats.max_us, 100);
    // PERF-001: LockFreeHistogram uses log2 buckets, percentiles are approximate
    // P50 should be in low range (values 1-100 mostly in buckets 0-6)
    assert!(
        stats.p50_us > 0 && stats.p50_us <= 100,
        "p50 should be reasonable, got {}",
        stats.p50_us
    );
    // P95/P99 should be higher than P50
    assert!(stats.p95_us >= stats.p50_us, "p95 should be >= p50");
    assert!(stats.p99_us >= stats.p95_us, "p99 should be >= p95");
}

#[test]
fn test_latency_stats_p99_exceeds() {
    // Arrange
    let stats = LatencyStats {
        count: 100,
        min_us: 10,
        max_us: 10000,
        mean_us: 500,
        p50_us: 100,
        p95_us: 1000,
        p99_us: 5000, // 5ms P99
    };

    // Assert
    assert!(stats.p99_exceeds(Duration::from_millis(1))); // 5ms > 1ms
    assert!(!stats.p99_exceeds(Duration::from_millis(10))); // 5ms < 10ms
}

// =============================================================================
// Thread Safety Tests (PERF-001: Lock-Free)
// =============================================================================

#[test]
fn test_metrics_thread_safety() {
    // Arrange
    let metrics = std::sync::Arc::new(StorageMetrics::new());
    let num_threads: u64 = 4;
    let ops_per_thread: u64 = 1000;

    // Act - Concurrent writes from multiple threads
    let handles: Vec<_> = (0..num_threads)
        .map(|_| {
            let m = metrics.clone();
            thread::spawn(move || {
                for i in 0..ops_per_thread {
                    m.record_ensure_capacity(Duration::from_micros(i), i % 10 == 0, 1024);
                }
            })
        })
        .collect();

    for h in handles {
        h.join().expect("Thread panicked");
    }

    // Assert
    let expected_total = num_threads * ops_per_thread;
    assert_eq!(metrics.ensure_capacity_count(), expected_total);

    // Each thread does ops_per_thread/10 resizes
    let expected_resizes = num_threads * (ops_per_thread / 10);
    assert_eq!(metrics.resize_count(), expected_resizes);
}

// =============================================================================
// TimingGuard Tests
// =============================================================================

#[test]
fn test_timing_guard_records_elapsed() {
    use super::metrics::TimingGuard;

    // Arrange
    let recorded = std::sync::Arc::new(Mutex::new(Duration::ZERO));
    let recorded_clone = recorded.clone();

    // Act
    {
        let _guard = TimingGuard::new(move |d| {
            *recorded_clone.lock() = d;
        });
        thread::sleep(Duration::from_millis(10));
    }

    // Assert - Should have recorded ~10ms
    let elapsed = *recorded.lock();
    assert!(elapsed >= Duration::from_millis(5)); // Allow some slack
    assert!(elapsed < Duration::from_millis(100)); // But not too much
}

// =============================================================================
// P0 Audit: ensure_capacity Monitoring Tests
// =============================================================================

#[test]
fn test_detect_slow_ensure_capacity() {
    // Arrange - Simulate a slow resize operation
    let metrics = StorageMetrics::new();

    // Normal operations (fast)
    for _ in 0..98 {
        metrics.record_ensure_capacity(Duration::from_micros(10), false, 0);
    }

    // Two slow operations (simulated STW during resize) to ensure P99 captures them
    metrics.record_ensure_capacity(Duration::from_millis(200), true, 1024 * 1024 * 1024);
    metrics.record_ensure_capacity(Duration::from_millis(500), true, 2 * 1024 * 1024 * 1024);

    // Act
    let stats = metrics.ensure_capacity_latency_stats();

    // Assert
    assert_eq!(stats.count, 100);
    // Max should be the 500ms outlier
    assert_eq!(stats.max_us, 500_000);
    // P99 should be high (at least capturing one of the slow ops)
    assert!(
        stats.p99_us >= 100_000,
        "P99 should capture slow ops: got {} us",
        stats.p99_us
    );
    assert!(stats.p99_exceeds(Duration::from_millis(100)));
}

#[test]
fn test_bytes_resized_tracks_growth() {
    // Arrange
    let metrics = StorageMetrics::new();

    // Simulate typical growth pattern: 16MB -> 32MB -> 64MB -> 128MB
    let growth_sizes = [16 * 1024 * 1024u64, 32 * 1024 * 1024, 64 * 1024 * 1024];

    // Act
    for size in growth_sizes {
        metrics.record_ensure_capacity(Duration::from_millis(50), true, size);
    }

    // Assert
    let expected_total: u64 = growth_sizes.iter().sum();
    assert_eq!(metrics.total_bytes_resized(), expected_total);
    assert_eq!(metrics.resize_count(), 3);
}
