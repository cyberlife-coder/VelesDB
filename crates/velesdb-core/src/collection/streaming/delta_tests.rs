//! Extended tests for the [`DeltaBuffer`] state machine and concurrent
//! safety. How searches score and merge its points is tested at the
//! collection's entry points (`search/pending_tests.rs`).

#![allow(clippy::cast_precision_loss)]

use super::delta::DeltaBuffer;
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
fn delta_ids_names_each_point_once_sorted() {
    let buf = DeltaBuffer::new();
    buf.activate();
    for i in [5, 1, 5, 3] {
        buf.push(i, vec![i as f32; 2]);
    }
    assert_eq!(buf.ids(), vec![1, 3, 5]);
}

#[test]
fn delta_ids_empty_when_inactive() {
    let buf = DeltaBuffer::new();
    assert!(buf.ids().is_empty());
}

/// A sealed buffer takes no new write and is still read by searches, until
/// the drain empties it.
#[test]
fn delta_seal_stops_writes_and_keeps_points_readable() {
    let buf = DeltaBuffer::new();
    buf.activate();
    buf.push(1, vec![1.0]);
    assert_eq!(buf.seal(), vec![1]);
    buf.push(2, vec![2.0]);
    assert_eq!(buf.ids(), vec![1], "sealed: still read, no new write");
    let _ = buf.deactivate_and_drain();
    assert!(buf.ids().is_empty());
}

#[test]
fn delta_seal_of_an_inactive_buffer_holds_nothing() {
    let buf = DeltaBuffer::new();
    assert!(buf.seal().is_empty());
    assert!(!buf.is_searchable());
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
// Concurrent push + read safety
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn delta_concurrent_push_and_read() {
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
                    let _ = b.ids();
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
