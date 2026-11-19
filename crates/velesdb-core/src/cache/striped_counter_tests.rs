//! Tests for `striped_counter`.

use super::striped_counter::StripedCounter;
use std::sync::Arc;
use std::thread;

#[test]
fn test_a_new_counter_reads_zero() {
    assert_eq!(StripedCounter::new().get(), 0);
}

#[test]
fn test_increments_from_one_thread_add_up() {
    let counter = StripedCounter::new();
    for _ in 0..1_000 {
        counter.increment();
    }
    assert_eq!(counter.get(), 1_000);
}

/// Every increment is counted exactly once, whichever stripe each thread
/// lands on: the sum over the stripes is exact.
#[test]
fn test_concurrent_increments_are_all_counted() {
    const THREADS: u64 = 8;
    const PER_THREAD: u64 = 10_000;
    let counter = Arc::new(StripedCounter::new());

    let handles: Vec<_> = (0..THREADS)
        .map(|_| {
            let counter = Arc::clone(&counter);
            thread::spawn(move || (0..PER_THREAD).for_each(|_| counter.increment()))
        })
        .collect();
    for handle in handles {
        handle.join().expect("incrementing thread panicked");
    }

    assert_eq!(counter.get(), THREADS * PER_THREAD);
}

/// Stripes sit on separate cache lines (128 bytes covers the adjacent-line
/// prefetch of x86 and the 128-byte lines of Apple silicon).
#[test]
fn test_each_stripe_fills_its_own_cache_line() {
    assert!(StripedCounter::stripe_size() >= 128);
    assert_eq!(StripedCounter::stripe_size() % 128, 0);
}
