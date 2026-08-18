use super::super::delta_merge::merge_with_delta;
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
fn test_stream_delta_push_and_search() {
    let buf = DeltaBuffer::new();
    buf.activate();
    buf.push(1, vec![1.0, 0.0, 0.0]);
    buf.push(2, vec![0.0, 1.0, 0.0]);
    buf.push(3, vec![0.5, 0.5, 0.0]);

    let query = &[1.0, 0.0, 0.0];
    let results = buf.search(query, 2, DistanceMetric::Cosine);
    assert_eq!(results.len(), 2, "should return at most k=2 results");
    // Cosine: higher is better; [1,0,0] is identical to query -> highest score
    assert_eq!(
        results[0].0, 1,
        "closest match should be id=1 (identical vector)"
    );
}

#[test]
fn test_stream_delta_search_returns_empty_when_inactive() {
    let buf = DeltaBuffer::new();
    buf.push(1, vec![1.0, 0.0, 0.0]);
    // buffer is NOT active — push() is a no-op when inactive
    let results = buf.search(&[1.0, 0.0, 0.0], 10, DistanceMetric::Cosine);
    assert!(
        results.is_empty(),
        "inactive delta should return no results"
    );
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
fn test_stream_delta_search_cosine_ordering() {
    let buf = DeltaBuffer::new();
    buf.activate();
    // Vec pointing along x-axis
    buf.push(10, vec![1.0, 0.0]);
    // Vec pointing along y-axis (orthogonal)
    buf.push(20, vec![0.0, 1.0]);
    // Vec at 45 degrees
    buf.push(30, vec![1.0, 1.0]);

    let query = &[1.0, 0.0];
    let results = buf.search(query, 3, DistanceMetric::Cosine);
    // Cosine: higher is better. id=10 should be first (similarity ~1.0)
    assert_eq!(results[0].0, 10);
    // id=30 at 45 deg should be next (similarity ~0.707)
    assert_eq!(results[1].0, 30);
    // id=20 orthogonal should be last (similarity ~0.0)
    assert_eq!(results[2].0, 20);
}

#[test]
fn test_stream_delta_search_euclidean_ordering() {
    let buf = DeltaBuffer::new();
    buf.activate();
    buf.push(1, vec![0.0, 0.0]);
    buf.push(2, vec![1.0, 0.0]);
    buf.push(3, vec![3.0, 4.0]);

    let query = &[0.0, 0.0];
    let results = buf.search(query, 3, DistanceMetric::Euclidean);
    // Euclidean: lower is better. id=1 (dist=0) should be first
    assert_eq!(results[0].0, 1);
    assert_eq!(results[1].0, 2);
    assert_eq!(results[2].0, 3);
}

#[test]
fn test_stream_delta_merge_with_delta_inactive() {
    let buf = DeltaBuffer::new();
    // NOT active
    let hnsw = vec![(1, 0.9), (2, 0.8)];
    let merged = merge_with_delta(hnsw.clone(), &buf, &[1.0, 0.0], 5, DistanceMetric::Cosine);
    assert_eq!(merged, hnsw, "inactive delta should return HNSW unchanged");
}

#[test]
fn test_stream_delta_merge_dedup_and_truncate() {
    let buf = DeltaBuffer::new();
    buf.activate();
    // Delta has id=1 with a different score and id=3 (new)
    buf.push(1, vec![0.9, 0.1]);
    buf.push(3, vec![0.8, 0.2]);

    // HNSW results (cosine scores, higher is better)
    let hnsw = vec![(1, 0.95), (2, 0.80)];

    let query = &[1.0, 0.0];
    let merged = merge_with_delta(hnsw, &buf, query, 2, DistanceMetric::Cosine);

    // Should have at most k=2 results
    assert_eq!(merged.len(), 2);

    // Delta wins for id=1 — its score should come from delta's brute-force
    // Check no duplicate ids
    let ids: Vec<u64> = merged.iter().map(|(id, _)| *id).collect();
    let unique: HashSet<u64> = ids.iter().copied().collect();
    assert_eq!(
        ids.len(),
        unique.len(),
        "no duplicate IDs in merged results"
    );
}

#[test]
fn test_stream_delta_merge_empty_delta() {
    let buf = DeltaBuffer::new();
    buf.activate();
    // Delta is active but empty
    let hnsw = vec![(1, 0.9), (2, 0.8)];
    let merged = merge_with_delta(hnsw.clone(), &buf, &[1.0, 0.0], 5, DistanceMetric::Cosine);
    assert_eq!(
        merged, hnsw,
        "empty active delta should return HNSW unchanged"
    );
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

// ── Bug B0.1: remove() filters deleted points from search ──────────

#[test]
fn test_delta_remove_filters_deleted_point() {
    let buf = DeltaBuffer::new();
    buf.activate();
    buf.push(1, vec![1.0, 2.0, 3.0]);
    buf.push(2, vec![4.0, 5.0, 6.0]);
    buf.remove(1);
    let results = buf.search(&[1.0, 2.0, 3.0], 10, DistanceMetric::Euclidean);
    assert!(
        results.iter().all(|(id, _)| *id != 1),
        "Deleted point should not appear in search results"
    );
    assert_eq!(results.len(), 1, "Only point 2 should remain");
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
    let results = buf.search(&[4.0, 5.0, 6.0], 1, DistanceMetric::Euclidean);
    assert_eq!(results[0].0, 1);
    // Distance should be ~0 since query matches the updated vector
    assert!(
        results[0].1 < 0.01,
        "Updated vector should match query closely"
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
    let results = buf.search(&[0.5, 0.5], 1, DistanceMetric::Euclidean);
    assert_eq!(
        results[0].0, 1,
        "ID 1 should have updated vector [0.5, 0.5]"
    );
    assert!(results[0].1 < 0.01, "Updated vector should match query");
}
