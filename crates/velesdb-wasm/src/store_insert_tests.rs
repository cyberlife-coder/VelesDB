//! Regression tests for `store_insert::remove_at_index` (Devin Review PR
//! #594 finding #2).
//!
//! Before the fix, ids/payloads used `swap_remove` (O(1)) while the
//! per-mode data buffers used `drain` (O(n) shift-left). Removing a
//! non-last index desynchronised id[idx] from the vector chunk at idx.
//! These tests pin the corrected swap-remove semantics in every storage
//! mode (all of which keep the f32 vectors), plus the metadata-only
//! (dimension = 0) edge case.

use crate::store_insert::{insert_batch_raw, insert_vector, insert_with_payload, remove_at_index};
use crate::store_new::create_store;
use crate::{DistanceMetric, StorageMode};

fn mk_full_store(dim: usize) -> crate::VectorStore {
    create_store(dim, DistanceMetric::Euclidean, StorageMode::Full)
}

// -------------------------------------------------------------------------
// Full mode: exact float recovery
// -------------------------------------------------------------------------

#[test]
fn test_remove_middle_index_keeps_ids_and_vectors_synced_full() {
    // Dimension 3 so the shift-vs-swap difference is obvious.
    let mut store = mk_full_store(3);
    // Insert 5 vectors with distinctive values so we can check alignment.
    for i in 0..5u64 {
        let f = i as f32;
        insert_vector(&mut store, 100 + i, &[f, f + 0.1, f + 0.2]);
    }
    assert_eq!(store.ids.len(), 5);
    assert_eq!(store.data.len(), 15);

    // Remove the element at index 1 (id = 101). With correct swap-remove
    // semantics: the last element (id = 104) takes its slot for BOTH the
    // id array and the data buffer.
    remove_at_index(&mut store, 1);

    assert_eq!(store.ids.len(), 4);
    assert_eq!(store.data.len(), 12);

    // After swap-remove: ids = [100, 104, 102, 103]
    assert_eq!(store.ids, vec![100, 104, 102, 103]);

    // Vectors must line up with ids. id=104 should be at index 1.
    // Before the fix, ids[1] would be 104 but data[3..6] would hold the
    // vector of the previous id=102.
    assert_eq!(&store.data[0..3], &[0.0, 0.1, 0.2]); // id=100
    assert_eq!(&store.data[3..6], &[4.0, 4.1, 4.2]); // id=104 (swapped in)
    assert_eq!(&store.data[6..9], &[2.0, 2.1, 2.2]); // id=102
    assert_eq!(&store.data[9..12], &[3.0, 3.1, 3.2]); // id=103
}

#[test]
fn test_remove_last_index_truncates_full() {
    let mut store = mk_full_store(2);
    for i in 0..3u64 {
        let f = i as f32;
        insert_vector(&mut store, i, &[f, f]);
    }
    remove_at_index(&mut store, 2);
    assert_eq!(store.ids, vec![0, 1]);
    assert_eq!(store.data, vec![0.0, 0.0, 1.0, 1.0]);
}

#[test]
fn test_remove_only_element_empties_buffers_full() {
    let mut store = mk_full_store(4);
    insert_vector(&mut store, 42, &[1.0, 2.0, 3.0, 4.0]);
    remove_at_index(&mut store, 0);
    assert!(store.ids.is_empty());
    assert!(store.data.is_empty());
    assert!(store.payloads.is_empty());
}

// -------------------------------------------------------------------------
// Quantized modes: the f32 vectors stay aligned with the ids after remove
// -------------------------------------------------------------------------

#[test]
fn test_remove_middle_index_keeps_vectors_synced_in_every_quantized_mode() {
    for mode in [
        StorageMode::SQ8,
        StorageMode::Binary,
        StorageMode::ProductQuantization,
        StorageMode::RaBitQ,
    ] {
        let mut store = create_store(3, DistanceMetric::Euclidean, mode);
        for i in 0..4u64 {
            let f = i as f32 + 0.5;
            insert_vector(&mut store, 10 + i, &[f, -f * 2.0, f * 3.0]);
        }

        // Remove index 1 (id = 11): the last row (id = 13) swaps into slot 1.
        remove_at_index(&mut store, 1);

        assert_eq!(store.ids, vec![10, 13, 12], "{mode:?}");
        assert_eq!(
            store.data,
            vec![0.5, -1.0, 1.5, 3.5, -7.0, 10.5, 2.5, -5.0, 7.5],
            "{mode:?}: the inserted f32 values follow their ids"
        );
    }
}

// -------------------------------------------------------------------------
// Metadata-only edge case (dimension == 0)
// -------------------------------------------------------------------------

#[test]
fn test_remove_at_index_metadata_only_noop_on_data() {
    // dimension=0 is the metadata-only branch. data/sq8/binary are empty
    // and must stay empty after remove. Only ids/payloads shrink.
    let mut store = crate::store_new::create_metadata_only();
    insert_with_payload(&mut store, 1, &[], Some(serde_json::json!({"a": 1})));
    insert_with_payload(&mut store, 2, &[], Some(serde_json::json!({"a": 2})));
    insert_with_payload(&mut store, 3, &[], Some(serde_json::json!({"a": 3})));
    assert_eq!(store.ids.len(), 3);
    assert!(store.data.is_empty());

    remove_at_index(&mut store, 1);

    assert_eq!(store.ids, vec![1, 3]);
    assert_eq!(store.payloads.len(), 2);
    assert!(store.data.is_empty());
}

// -------------------------------------------------------------------------
// Payload alignment after remove (covers all modes via Full)
// -------------------------------------------------------------------------

#[test]
fn test_remove_middle_index_keeps_payload_aligned() {
    let mut store = mk_full_store(2);
    insert_with_payload(&mut store, 1, &[1.0, 1.0], Some(serde_json::json!("p1")));
    insert_with_payload(&mut store, 2, &[2.0, 2.0], Some(serde_json::json!("p2")));
    insert_with_payload(&mut store, 3, &[3.0, 3.0], Some(serde_json::json!("p3")));
    insert_with_payload(&mut store, 4, &[4.0, 4.0], Some(serde_json::json!("p4")));

    remove_at_index(&mut store, 1);

    // After swap-remove: ids = [1, 4, 3]; payloads = [p1, p4, p3]; data
    // chunks follow the same permutation.
    assert_eq!(store.ids, vec![1, 4, 3]);
    assert_eq!(store.payloads[0], Some(serde_json::json!("p1")));
    assert_eq!(store.payloads[1], Some(serde_json::json!("p4")));
    assert_eq!(store.payloads[2], Some(serde_json::json!("p3")));
    assert_eq!(&store.data[2..4], &[4.0, 4.0]);
}

// -------------------------------------------------------------------------
// Idempotent re-insert (which uses remove_at_index internally) stays sane
// -------------------------------------------------------------------------

#[test]
fn test_reinsert_same_id_overwrites_and_keeps_alignment() {
    let mut store = mk_full_store(2);
    insert_vector(&mut store, 1, &[1.0, 1.0]);
    insert_vector(&mut store, 2, &[2.0, 2.0]);
    insert_vector(&mut store, 3, &[3.0, 3.0]);
    // Re-insert id=2 with different data; this triggers
    // remove_at_index(store, 1) followed by append.
    insert_vector(&mut store, 2, &[20.0, 20.0]);

    assert_eq!(store.ids.len(), 3);
    // After swap-remove + push: ids = [1, 3, 2]; data in the same order.
    assert_eq!(store.ids, vec![1, 3, 2]);
    assert_eq!(&store.data[0..2], &[1.0, 1.0]);
    assert_eq!(&store.data[2..4], &[3.0, 3.0]);
    assert_eq!(&store.data[4..6], &[20.0, 20.0]);
}

// -------------------------------------------------------------------------
// insert_batch_raw: flat raw-bulk insert (VRB1 contract)
// -------------------------------------------------------------------------

#[test]
fn test_insert_batch_raw_happy_path_roundtrip() {
    let mut store = mk_full_store(4);
    let ids = [10u64, 11, 12];
    // 3 vectors, dim 4, row-major.
    let vectors = [
        1.0, 0.0, 0.0, 0.0, // id=10
        0.0, 1.0, 0.0, 0.0, // id=11
        0.0, 0.0, 1.0, 0.0, // id=12
    ];
    insert_batch_raw(&mut store, &ids, &vectors, 4).unwrap();

    assert_eq!(store.ids, vec![10, 11, 12]);
    assert_eq!(store.data.len(), 12);
    // Each row lands contiguously in insertion order.
    assert_eq!(&store.data[0..4], &[1.0, 0.0, 0.0, 0.0]);
    assert_eq!(&store.data[4..8], &[0.0, 1.0, 0.0, 0.0]);
    assert_eq!(&store.data[8..12], &[0.0, 0.0, 1.0, 0.0]);
}

#[test]
fn test_insert_batch_raw_upserts_existing_id() {
    let mut store = mk_full_store(2);
    insert_vector(&mut store, 1, &[9.0, 9.0]);
    // id=1 already present — raw batch must dedup via insert_vector.
    insert_batch_raw(&mut store, &[1u64, 2], &[1.0, 1.0, 2.0, 2.0], 2).unwrap();
    assert_eq!(store.ids.len(), 2);
    // id=1 is the only element, so its swap-remove truncates, then id=1 is
    // re-appended with the new data, followed by id=2: ids = [1, 2].
    assert_eq!(store.ids, vec![1, 2]);
    // New data overwrote the old [9.0, 9.0] for id=1.
    assert_eq!(&store.data[0..2], &[1.0, 1.0]);
    assert_eq!(&store.data[2..4], &[2.0, 2.0]);
}

#[test]
fn test_insert_batch_raw_length_mismatch_errors() {
    let mut store = mk_full_store(4);
    // 2 ids but only 4 floats (need 8). Must error, leaving the store empty.
    let err = insert_batch_raw(&mut store, &[1u64, 2], &[1.0, 2.0, 3.0, 4.0], 4).unwrap_err();
    assert!(err.contains("size mismatch"), "unexpected error: {err}");
    assert!(store.ids.is_empty());
    assert!(store.data.is_empty());
}

#[test]
fn test_insert_batch_raw_dimension_mismatch_errors() {
    let mut store = mk_full_store(4);
    let err = insert_batch_raw(&mut store, &[1u64], &[1.0, 2.0, 3.0], 3).unwrap_err();
    assert!(
        err.contains("dimension mismatch"),
        "unexpected error: {err}"
    );
    assert!(store.ids.is_empty());
}

#[test]
fn test_insert_batch_raw_overflow_errors() {
    let mut store = mk_full_store(usize::MAX);
    // ids.len() * dimension overflows usize -> checked_mul guard rejects.
    let err = insert_batch_raw(&mut store, &[1u64, 2], &[], usize::MAX).unwrap_err();
    assert!(err.contains("overflow"), "unexpected error: {err}");
    assert!(store.ids.is_empty());
}

// -------------------------------------------------------------------------
// RaBitQ mode: the first search must not abort.
// -------------------------------------------------------------------------

#[test]
fn test_rabitq_search_does_not_panic() {
    // Regression: the old per-mode dequantization scratch buffer was allocated
    // only for SQ8/Binary/PQ, so a `rabitq` store — reachable from JS via
    // `new_with_mode(.., "rabitq")` — indexed into an empty `Vec` on its first
    // search, a module abort under `panic = "abort"`.
    let mut store = create_store(4, DistanceMetric::Euclidean, StorageMode::RaBitQ);
    insert_vector(&mut store, 1, &[1.0, 2.0, 3.0, 4.0]);
    insert_vector(&mut store, 2, &[5.0, 6.0, 7.0, 8.0]);

    let scores = store.search_scored(&[1.0, 2.0, 3.0, 4.0], 10);

    assert_eq!(scores.len(), 2, "every stored id must be scored");
    assert_eq!(scores[0], (1, 0.0));
    assert_eq!(scores[1].0, 2);
}
