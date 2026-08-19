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
    assert_eq!(stats.max, Duration::from_millis(1));
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
