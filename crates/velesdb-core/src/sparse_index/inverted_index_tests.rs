use super::*;

fn make_vector(pairs: Vec<(u32, f32)>) -> SparseVector {
    SparseVector::new(pairs)
}

#[test]
fn test_insert_creates_posting_lists() {
    let index = SparseInvertedIndex::new();
    let v = make_vector(vec![(1, 0.5), (3, 1.0), (7, 0.3)]);
    index.insert(100, &v);

    assert_eq!(index.doc_count(), 1);

    let postings_1 = index.get_all_postings(1);
    assert_eq!(postings_1.len(), 1);
    assert_eq!(postings_1[0].doc_id, 100);
    assert!((postings_1[0].weight - 0.5).abs() < f32::EPSILON);

    let postings_3 = index.get_all_postings(3);
    assert_eq!(postings_3.len(), 1);
    assert_eq!(postings_3[0].doc_id, 100);

    let postings_7 = index.get_all_postings(7);
    assert_eq!(postings_7.len(), 1);
}

#[test]
fn test_insert_updates_max_weight() {
    let index = SparseInvertedIndex::new();
    let v1 = make_vector(vec![(1, 0.5)]);
    let v2 = make_vector(vec![(1, 2.0)]);
    index.insert(1, &v1);
    index.insert(2, &v2);

    assert!((index.get_global_max_weight(1) - 2.0).abs() < f32::EPSILON);
}

#[test]
fn test_freeze_at_threshold() {
    let index = SparseInvertedIndex::new();
    for i in 0..=FREEZE_THRESHOLD {
        let v = make_vector(vec![(1, 1.0)]);
        index.insert(i as u64, &v);
    }

    assert_eq!(index.frozen_count(), 1);
    assert_eq!(index.mutable_doc_count(), 1);
    assert_eq!(index.doc_count(), (FREEZE_THRESHOLD + 1) as u64);
}

#[test]
fn test_read_across_segments() {
    let index = SparseInvertedIndex::new();

    // Fill up to freeze
    for i in 0..FREEZE_THRESHOLD {
        let v = make_vector(vec![(1, 1.0)]);
        index.insert(i as u64, &v);
    }
    assert_eq!(index.frozen_count(), 1);

    // Insert into new mutable segment
    let v = make_vector(vec![(1, 2.0)]);
    index.insert(99_999, &v);

    let postings = index.get_all_postings(1);
    // FREEZE_THRESHOLD from frozen + 1 from mutable
    assert_eq!(postings.len(), FREEZE_THRESHOLD + 1);
}

#[test]
fn test_delete_from_mutable() {
    let index = SparseInvertedIndex::new();
    let v = make_vector(vec![(1, 1.0), (2, 2.0)]);
    index.insert(42, &v);

    let postings = index.get_all_postings(1);
    assert_eq!(postings.len(), 1);

    index.delete(42);

    let postings = index.get_all_postings(1);
    assert!(postings.is_empty());
    let postings = index.get_all_postings(2);
    assert!(postings.is_empty());
}

#[test]
fn test_delete_from_frozen_uses_tombstone() {
    let index = SparseInvertedIndex::new();

    // Fill to freeze
    for i in 0..FREEZE_THRESHOLD {
        let v = make_vector(vec![(1, 1.0)]);
        index.insert(i as u64, &v);
    }
    assert_eq!(index.frozen_count(), 1);

    // Delete doc 0 from frozen segment
    index.delete(0);

    let postings = index.get_all_postings(1);
    assert_eq!(postings.len(), FREEZE_THRESHOLD - 1);
    assert!(!postings.iter().any(|e| e.doc_id == 0));
}

#[test]
fn test_get_max_weight_across_segments() {
    let index = SparseInvertedIndex::new();

    // Insert a vector with weight 5.0 for term 1, fill to freeze
    let v = make_vector(vec![(1, 5.0)]);
    index.insert(0, &v);
    for i in 1..FREEZE_THRESHOLD {
        let v = make_vector(vec![(1, 1.0)]);
        index.insert(i as u64, &v);
    }
    assert_eq!(index.frozen_count(), 1);

    // Insert into mutable with weight 3.0
    let v = make_vector(vec![(1, 3.0)]);
    index.insert(99_999, &v);

    // Max should be 5.0 from frozen segment
    assert!((index.get_global_max_weight(1) - 5.0).abs() < f32::EPSILON);
}

#[test]
fn test_term_count() {
    let index = SparseInvertedIndex::new();
    let v1 = make_vector(vec![(1, 1.0), (2, 2.0)]);
    let v2 = make_vector(vec![(2, 1.0), (3, 3.0)]);
    index.insert(1, &v1);
    index.insert(2, &v2);

    assert_eq!(index.term_count(), 3); // terms 1, 2, 3
}

#[test]
fn test_concurrent_insert() {
    use std::sync::Arc;

    let index = Arc::new(SparseInvertedIndex::new());
    let mut handles = Vec::new();

    for thread_id in 0..4u64 {
        let idx = Arc::clone(&index);
        handles.push(std::thread::spawn(move || {
            for i in 0..100u64 {
                let point_id = thread_id * 1000 + i;
                let v = SparseVector::new(vec![(1, 1.0), (2, 0.5)]);
                idx.insert(point_id, &v);
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    assert_eq!(index.doc_count(), 400);
}

#[test]
fn test_empty_index() {
    let index = SparseInvertedIndex::new();
    assert_eq!(index.doc_count(), 0);
    assert_eq!(index.term_count(), 0);
    assert!(index.get_all_postings(1).is_empty());
    assert!((index.get_global_max_weight(1)).abs() < f32::EPSILON);
}

// --- Bug-fix regression tests ---

#[test]
fn test_double_delete_no_underflow() {
    let index = SparseInvertedIndex::new();
    index.insert(42, &make_vector(vec![(1, 1.0)]));
    assert_eq!(index.doc_count(), 1);

    index.delete(42);
    assert_eq!(index.doc_count(), 0);

    // Second delete of the same point must not wrap to u64::MAX.
    index.delete(42);
    assert_eq!(
        index.doc_count(),
        0,
        "doc_count must not underflow on double-delete"
    );
}

#[test]
fn test_delete_nonexistent_no_underflow() {
    let index = SparseInvertedIndex::new();
    assert_eq!(index.doc_count(), 0);

    // Deleting a point that was never inserted must leave count at 0.
    index.delete(999);
    assert_eq!(
        index.doc_count(),
        0,
        "doc_count must not underflow on delete of non-existent id"
    );
}

#[test]
fn test_upsert_same_id_does_not_increment_doc_count() {
    // H-3 regression: inserting the same point_id twice must not double-count.
    let index = SparseInvertedIndex::new();
    let v1 = make_vector(vec![(1, 1.0)]);
    let v2 = make_vector(vec![(1, 2.0)]);

    index.insert(42, &v1);
    assert_eq!(index.doc_count(), 1, "first insert must set doc_count to 1");

    // Upsert same ID with updated weight — doc_count must stay at 1.
    index.insert(42, &v2);
    assert_eq!(
        index.doc_count(),
        1,
        "upsert of existing ID must not increment doc_count"
    );

    // Weight must reflect the latest insert (upsert semantics).
    let postings = index.get_all_postings(1);
    assert_eq!(postings.len(), 1);
    assert!(
        (postings[0].weight - 2.0).abs() < f32::EPSILON,
        "upsert must update the stored weight"
    );
}

#[test]
fn test_upsert_different_terms_does_not_increment_doc_count() {
    // Upsert where the new vector uses different terms than the first insert.
    let index = SparseInvertedIndex::new();

    index.insert(99, &make_vector(vec![(10, 1.0)]));
    assert_eq!(index.doc_count(), 1);

    // Same point_id, completely different term set.
    index.insert(99, &make_vector(vec![(20, 0.5)]));
    assert_eq!(
        index.doc_count(),
        1,
        "upsert with different terms must not increment doc_count"
    );
}

#[test]
fn test_dedup_last_write_wins_within_mutable() {
    // Insert point 1 twice; second insert (upsert) updates in-place via
    // binary_search. Compaction must see only the newer weight.
    let index = SparseInvertedIndex::new();
    index.insert(1, &make_vector(vec![(5, 0.1)]));
    index.insert(1, &make_vector(vec![(5, 9.9)]));

    let compacted = index.get_merged_postings_for_compaction();
    let (entries, _) = compacted.get(&5).expect("term 5 must be present");
    let entry = entries
        .iter()
        .find(|e| e.doc_id == 1)
        .expect("doc 1 must be present");
    assert!(
        (entry.weight - 9.9).abs() < 1e-5,
        "compaction must keep newest weight; got {}",
        entry.weight
    );
}

#[test]
fn test_dedup_last_write_wins_across_segments() {
    // Force doc 0 into a frozen segment, then re-insert it with a different
    // weight in the mutable segment. Compaction must pick the mutable weight.
    let index = SparseInvertedIndex::new();

    for i in 0..FREEZE_THRESHOLD {
        index.insert(i as u64, &make_vector(vec![(7, 1.0)]));
    }
    assert_eq!(index.frozen_count(), 1, "segment must have frozen");

    // Re-insert doc 0 into the mutable segment with an updated weight.
    index.insert(0, &make_vector(vec![(7, 5.5)]));

    let compacted = index.get_merged_postings_for_compaction();
    let (entries, _) = compacted.get(&7).expect("term 7 must be present");
    let entry = entries
        .iter()
        .find(|e| e.doc_id == 0)
        .expect("doc 0 must be present");
    assert!(
        (entry.weight - 5.5).abs() < 1e-5,
        "mutable (newer) weight 5.5 must win over frozen weight 1.0; got {}",
        entry.weight
    );
}
