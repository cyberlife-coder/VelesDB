use super::super::super::distance::CpuDistance;
use super::*;
use crate::distance::DistanceMetric;

fn empty_hnsw() -> NativeHnsw<CpuDistance> {
    let dist = CpuDistance::new(DistanceMetric::Euclidean);
    NativeHnsw::new(dist, 16, 200, 0)
}

/// Exactly 10 000 vectors must use 1 probe (boundary fix, issue #377).
///
/// Before the fix `count < 10_000` excluded the 10K case, causing the
/// Balanced preset (ef=160, k=10) benchmark to execute 2 probes.
#[test]
fn single_probe_at_exactly_10k() {
    let hnsw = empty_hnsw();
    assert_eq!(hnsw.adaptive_num_probes(10_000, 160, 10), 1);
}

#[test]
fn single_probe_below_10k() {
    let hnsw = empty_hnsw();
    assert_eq!(hnsw.adaptive_num_probes(9_999, 160, 10), 1);
}

#[test]
fn two_probes_above_10k_balanced() {
    let hnsw = empty_hnsw();
    assert_eq!(hnsw.adaptive_num_probes(10_001, 160, 10), 2);
}

#[test]
fn single_probe_for_small_ef() {
    let hnsw = empty_hnsw();
    // ef_search=40 <= max(k*4=40, 64)=64 → single probe at any scale
    assert_eq!(hnsw.adaptive_num_probes(100_000, 40, 10), 1);
}

#[test]
fn four_probes_for_large_ef_at_scale() {
    let hnsw = empty_hnsw();
    assert_eq!(hnsw.adaptive_num_probes(50_000, 1024, 10), 4);
}

// =========================================================================
// Thread-local probe RNG (issue #967)
// =========================================================================

/// `next_probe_rng` must never return 0 (XORshift64 invariant) and must
/// produce at least 64 distinct values over 64 consecutive calls on the
/// same thread (i.e. no short cycle in any reachable range).
#[test]
fn probe_rng_no_zero_and_no_short_cycle() {
    let mut seen = std::collections::HashSet::new();
    for _ in 0..64 {
        let v = NativeHnsw::<CpuDistance>::next_probe_rng();
        assert_ne!(v, 0, "XORshift64 must never produce 0");
        seen.insert(v);
    }
    assert_eq!(
        seen.len(),
        64,
        "64 consecutive calls should all be distinct"
    );
}

/// Two threads seeded from the same global counter must diverge immediately.
#[test]
fn probe_rng_threads_diverge() {
    use std::sync::{Arc, Barrier};
    use std::thread;

    let barrier = Arc::new(Barrier::new(2));
    let b1 = Arc::clone(&barrier);
    let b2 = Arc::clone(&barrier);

    let t1 = thread::spawn(move || {
        b1.wait();
        NativeHnsw::<CpuDistance>::next_probe_rng()
    });
    let t2 = thread::spawn(move || {
        b2.wait();
        NativeHnsw::<CpuDistance>::next_probe_rng()
    });

    let v1 = t1.join().expect("thread 1 panicked");
    let v2 = t2.join().expect("thread 2 panicked");
    assert_ne!(
        v1, v2,
        "different threads must start with different RNG values"
    );
}
