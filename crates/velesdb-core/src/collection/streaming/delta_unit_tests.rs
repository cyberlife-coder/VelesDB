use super::*;

#[test]
fn test_stream_delta_buffer_compiles_and_defaults_inactive() {
    let buf = DeltaBuffer::new();
    assert!(
        !buf.is_active(),
        "new DeltaBuffer should be inactive by default"
    );
}

#[test]
fn test_stream_delta_buffer_default_trait() {
    let buf = DeltaBuffer::default();
    assert!(!buf.is_active());
}

#[test]
fn test_stream_delta_push_and_read() {
    let buf = DeltaBuffer::new();
    buf.activate();
    buf.push(1, vec![1.0, 0.0, 0.0]);
    buf.push(2, vec![0.0, 1.0, 0.0]);
    assert_eq!(buf.ids(), vec![1, 2]);
}

#[test]
fn test_stream_delta_ids_empty_when_inactive() {
    let buf = DeltaBuffer::new();
    buf.push(1, vec![1.0, 0.0, 0.0]);
    // buffer is NOT active — push() is a no-op when inactive
    assert!(buf.ids().is_empty(), "inactive delta should hold no ids");
}

#[test]
fn test_stream_delta_push_noop_when_inactive() {
    let buf = DeltaBuffer::new();
    // push and extend are no-ops when inactive (C-1 guard)
    buf.push(1, vec![1.0, 0.0]);
    buf.extend(vec![(2, vec![0.0, 1.0])]);
    assert_eq!(buf.len(), 0, "push/extend should be no-ops when inactive");
}

#[test]
fn test_stream_delta_activate_deactivate_drain() {
    let buf = DeltaBuffer::new();
    assert!(!buf.is_active());

    buf.activate();
    assert!(buf.is_active());

    buf.push(1, vec![1.0]);
    buf.push(2, vec![2.0]);
    assert_eq!(buf.len(), 2);

    let drained = buf.deactivate_and_drain();
    assert!(!buf.is_active());
    assert!(buf.is_empty());
    assert_eq!(drained.len(), 2);
    assert_eq!(drained[0].0, 1);
    assert_eq!(drained[1].0, 2);
}

// ── STREAM-9: try_activate CAS detects double-activation ───────────

#[test]
fn test_delta_activate_cas_detects_double() {
    let buf = DeltaBuffer::new();
    // First activation on an INACTIVE buffer succeeds.
    assert!(
        buf.try_activate().is_ok(),
        "first try_activate must succeed"
    );
    assert!(buf.is_active());
    // Second activation must be rejected (double-activation detected).
    assert_eq!(
        buf.try_activate(),
        Err(ActivateError::AlreadyActive),
        "re-entrant try_activate must report AlreadyActive"
    );
    // After draining back to INACTIVE, try_activate succeeds again.
    let _ = buf.deactivate_and_drain();
    assert!(
        buf.try_activate().is_ok(),
        "try_activate must succeed again once buffer is INACTIVE"
    );
}

#[test]
fn test_delta_try_activate_pushes_after_cas() {
    let buf = DeltaBuffer::new();
    buf.try_activate().expect("activation should succeed");
    buf.push(1, vec![1.0, 0.0]);
    assert_eq!(buf.len(), 1, "push after CAS activation must accumulate");
}

#[test]
fn test_stream_delta_extend() {
    let buf = DeltaBuffer::new();
    buf.activate();
    buf.extend(vec![(1, vec![1.0]), (2, vec![2.0]), (3, vec![3.0])]);
    assert_eq!(buf.len(), 3);
}

#[test]
fn test_stream_delta_stats() {
    let buf = DeltaBuffer::new();
    buf.activate();
    buf.push(1, vec![1.0]);
    let (len, is_empty) = buf.stats();
    assert_eq!(len, 1);
    assert!(!is_empty);
}

// ── Bug B0.1: remove() drops deleted points ──────────

#[test]
fn test_delta_remove_filters_deleted_point() {
    let buf = DeltaBuffer::new();
    buf.activate();
    buf.push(1, vec![1.0, 2.0, 3.0]);
    buf.push(2, vec![4.0, 5.0, 6.0]);
    buf.remove(1);
    assert_eq!(buf.ids(), vec![2], "Only point 2 should remain");
}

#[test]
fn test_delta_remove_nonexistent_id_is_noop() {
    let buf = DeltaBuffer::new();
    buf.activate();
    buf.push(1, vec![1.0, 2.0]);
    buf.remove(999);
    assert_eq!(buf.len(), 1, "Removing absent ID should not change length");
}

#[test]
fn test_delta_remove_works_in_draining_state() {
    let buf = DeltaBuffer::new();
    buf.activate();
    buf.push(1, vec![1.0]);
    buf.push(2, vec![2.0]);
    // remove() works unconditionally (any state) — a delete must always
    // purge stale data regardless of buffer lifecycle.
    buf.remove(1);
    assert_eq!(buf.len(), 1);
}

// ── Bug B0.4: push() deduplicates on same ID (upsert semantics) ───

#[test]
fn test_delta_push_deduplicates_on_same_id() {
    let buf = DeltaBuffer::new();
    buf.activate();
    buf.push(1, vec![1.0, 2.0, 3.0]);
    buf.push(1, vec![4.0, 5.0, 6.0]); // Same ID, different vector
    assert_eq!(buf.len(), 1, "Should have deduplicated");
    assert_eq!(
        buf.entries(),
        vec![(1, vec![4.0, 5.0, 6.0])],
        "the newer vector"
    );
}

#[test]
fn test_delta_extend_deduplicates_on_same_id() {
    let buf = DeltaBuffer::new();
    buf.activate();
    buf.push(1, vec![1.0, 0.0]);
    buf.push(2, vec![0.0, 1.0]);
    // Extend with updates for id=1 and a new id=3
    buf.extend(vec![(1, vec![0.5, 0.5]), (3, vec![0.0, 0.0])]);
    assert_eq!(buf.len(), 3, "Should have ids 1, 2, 3");
    let entries = buf.entries();
    let one = entries
        .iter()
        .find(|(id, _)| *id == 1)
        .map(|(_, v)| v.as_slice());
    assert_eq!(
        one,
        Some([0.5, 0.5].as_slice()),
        "ID 1 has its updated vector"
    );
}

#[test]
fn test_delta_retire_keeps_an_entry_written_again() {
    let buf = DeltaBuffer::new();
    buf.activate();
    buf.push(1, vec![1.0]);
    buf.push(2, vec![2.0]);
    buf.push(2, vec![9.0]); // written again after a merge copied [2.0]
    buf.retire(&[(1, [1.0].as_slice()), (2, [2.0].as_slice())]);
    assert_eq!(buf.entries(), vec![(2, vec![9.0])]);
}
