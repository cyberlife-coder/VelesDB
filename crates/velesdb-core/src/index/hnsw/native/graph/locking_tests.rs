use super::*;

#[test]
fn holds_lock_reports_currently_held_rank() {
    // Empty stack — nothing held.
    assert!(!holds_lock(LockRank::Layers));
    assert!(!holds_lock(LockRank::Vectors));

    record_lock_acquire(LockRank::Layers);
    assert!(holds_lock(LockRank::Layers));
    assert!(!holds_lock(LockRank::Vectors));

    record_lock_release(LockRank::Layers);
    assert!(!holds_lock(LockRank::Layers));
}

#[test]
fn gpu_vectors_snapshot_rank_sorts_before_vectors() {
    // Monotone rank check — the core invariant of the enum.
    assert!(LockRank::GpuVectorsSnapshot < LockRank::Vectors);
    assert!(LockRank::Layers < LockRank::Neighbors);
}

#[test]
fn nested_acquire_in_declared_order_reports_both_held() {
    // Simulate `get_or_refresh_vector_snapshot`: snapshot then vectors.
    record_lock_acquire(LockRank::GpuVectorsSnapshot);
    record_lock_acquire(LockRank::Vectors);

    assert!(holds_lock(LockRank::GpuVectorsSnapshot));
    assert!(holds_lock(LockRank::Vectors));

    record_lock_release(LockRank::Vectors);
    record_lock_release(LockRank::GpuVectorsSnapshot);

    // Stack back to empty.
    assert!(!holds_lock(LockRank::GpuVectorsSnapshot));
    assert!(!holds_lock(LockRank::Vectors));
}

#[test]
fn vectors_after_layers_counts_a_violation() {
    let before = HNSW_COUNTERS.snapshot().invariant_violation_total;
    record_lock_acquire(LockRank::Layers);
    record_lock_acquire(LockRank::Vectors); // wrong order: 10 while holding 20
    record_lock_release(LockRank::Vectors);
    record_lock_release(LockRank::Layers);
    assert!(HNSW_COUNTERS.snapshot().invariant_violation_total > before,);
}
