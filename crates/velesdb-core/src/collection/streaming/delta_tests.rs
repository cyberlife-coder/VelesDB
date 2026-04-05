//! Extended tests for [`DeltaBuffer`] state machine, concurrent safety,
//! and `merge_with_delta` deduplication.

#![allow(clippy::cast_precision_loss)]

use super::delta::DeltaBuffer;
use super::delta_merge::merge_with_delta;
use crate::distance::DistanceMetric;
use std::sync::Arc;

// ─────────────────────────────────────────────────────────────────────────────
// Push + search basic flow
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn delta_push_increments_len() {
    let buf = DeltaBuffer::new();
    buf.activate();
    for i in 0..5 {
        buf.push(i, vec![i as f32; 3]);
    }
    assert_eq!(buf.len(), 5);
}

#[test]
fn delta_search_respects_k_limit() {
    let buf = DeltaBuffer::new();
    buf.activate();
    for i in 0..20 {
        buf.push(i, vec![i as f32; 2]);
    }
    let results = buf.search(&[0.0, 0.0], 5, DistanceMetric::Euclidean);
    assert_eq!(results.len(), 5, "search must truncate to k");
}

#[test]
fn delta_search_empty_buffer_returns_empty() {
    let buf = DeltaBuffer::new();
    buf.activate();
    let results = buf.search(&[1.0], 10, DistanceMetric::Cosine);
    assert!(results.is_empty(), "empty active buffer returns no results");
}

// ─────────────────────────────────────────────────────────────────────────────
// State transitions: activate / deactivate_and_drain
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn delta_double_activate_is_idempotent() {
    let buf = DeltaBuffer::new();
    buf.activate();
    buf.activate();
    assert!(buf.is_active());
}

#[test]
fn delta_deactivate_returns_all_pushed_entries() {
    let buf = DeltaBuffer::new();
    buf.activate();
    buf.push(1, vec![1.0]);
    buf.push(2, vec![2.0]);
    buf.push(3, vec![3.0]);

    let drained = buf.deactivate_and_drain();
    assert_eq!(drained.len(), 3);
    assert!(!buf.is_active());
    assert!(buf.is_empty());
}

#[test]
fn delta_deactivate_when_inactive_returns_empty() {
    let buf = DeltaBuffer::new();
    let drained = buf.deactivate_and_drain();
    assert!(drained.is_empty());
    assert!(!buf.is_active());
}

#[test]
fn delta_push_after_deactivate_is_noop() {
    let buf = DeltaBuffer::new();
    buf.activate();
    buf.push(1, vec![1.0]);
    let _ = buf.deactivate_and_drain();

    buf.push(99, vec![99.0]);
    assert!(buf.is_empty(), "push after drain should be noop");
}

#[test]
fn delta_reactivate_after_drain_accepts_new_entries() {
    let buf = DeltaBuffer::new();
    buf.activate();
    buf.push(1, vec![1.0]);
    let _ = buf.deactivate_and_drain();

    buf.activate();
    buf.push(10, vec![10.0]);
    assert_eq!(buf.len(), 1);
    assert!(buf.is_active());
}

// ─────────────────────────────────────────────────────────────────────────────
// Concurrent push + search safety
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn delta_concurrent_push_and_search() {
    let buf = Arc::new(DeltaBuffer::new());
    buf.activate();

    let writers: Vec<_> = (0u64..4)
        .map(|t| {
            let b = Arc::clone(&buf);
            std::thread::spawn(move || {
                for i in 0..50 {
                    let id = t * 100 + i;
                    b.push(id, vec![id as f32; 3]);
                }
            })
        })
        .collect();

    let readers: Vec<_> = (0..2)
        .map(|_| {
            let b = Arc::clone(&buf);
            std::thread::spawn(move || {
                for _ in 0..20 {
                    let _ = b.search(&[1.0, 0.0, 0.0], 5, DistanceMetric::Cosine);
                }
            })
        })
        .collect();

    for w in writers {
        w.join().expect("writer thread panicked");
    }
    for r in readers {
        r.join().expect("reader thread panicked");
    }

    assert_eq!(buf.len(), 200, "all 4x50 pushes should land");
}

// ─────────────────────────────────────────────────────────────────────────────
// merge_with_delta
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn merge_delta_dedup_prefers_delta() {
    let buf = DeltaBuffer::new();
    buf.activate();
    // Delta has id=1 with vector very close to query
    buf.push(1, vec![1.0, 0.0]);

    let hnsw = vec![(1, 0.5_f32), (2, 0.4)];
    let merged = merge_with_delta(hnsw, &buf, &[1.0, 0.0], 10, DistanceMetric::Cosine);

    let ids: Vec<u64> = merged.iter().map(|(id, _)| *id).collect();
    // id=1 must appear exactly once (from delta, not HNSW)
    assert_eq!(ids.iter().filter(|&&id| id == 1).count(), 1);
}

#[test]
fn merge_delta_truncates_to_k() {
    let buf = DeltaBuffer::new();
    buf.activate();
    for i in 0..10 {
        buf.push(100 + i, vec![i as f32; 2]);
    }

    let hnsw: Vec<(u64, f32)> = (0..10).map(|i| (i, 0.5)).collect();
    let merged = merge_with_delta(hnsw, &buf, &[0.0, 0.0], 3, DistanceMetric::Euclidean);
    assert_eq!(merged.len(), 3);
}
