//! Regression tests for `store_insert::remove_at_index` (Devin Review PR
//! #594 finding #2).
//!
//! Before the fix, ids/payloads used `swap_remove` (O(1)) while the
//! per-mode data buffers used `drain` (O(n) shift-left). Removing a
//! non-last index desynchronised id[idx] from the vector chunk at idx.
//! These tests pin the corrected swap-remove semantics across Full, SQ8
//! and Binary modes, plus the metadata-only (dimension = 0) edge case.

use crate::store_insert::{insert_vector, insert_with_payload, remove_at_index};
use crate::store_new::create_store;
use crate::{DistanceMetric, StorageMode};

fn mk_full_store(dim: usize) -> crate::VectorStore {
    create_store(dim, DistanceMetric::Euclidean, StorageMode::Full)
}

fn mk_sq8_store(dim: usize) -> crate::VectorStore {
    create_store(dim, DistanceMetric::Euclidean, StorageMode::SQ8)
}

fn mk_binary_store(dim: usize) -> crate::VectorStore {
    create_store(dim, DistanceMetric::Hamming, StorageMode::Binary)
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
// SQ8 mode: sq8_mins / sq8_scales stay aligned with data_sq8 after remove
// -------------------------------------------------------------------------

#[test]
fn test_remove_middle_index_keeps_sq8_buffers_synced() {
    let mut store = mk_sq8_store(3);
    // 4 distinctive vectors.
    for i in 0..4u64 {
        let f = i as f32 + 0.5;
        insert_vector(&mut store, 10 + i, &[f, f * 2.0, f * 3.0]);
    }
    assert_eq!(store.sq8_mins.len(), 4);
    assert_eq!(store.sq8_scales.len(), 4);
    assert_eq!(store.data_sq8.len(), 12);

    let mins_before = store.sq8_mins.clone();
    let scales_before = store.sq8_scales.clone();

    // Remove index 1 (id = 11). Last (id = 13) should swap in for ids,
    // mins, scales, AND the 3-byte data chunk.
    remove_at_index(&mut store, 1);

    assert_eq!(store.ids, vec![10, 13, 12]);
    assert_eq!(store.sq8_mins.len(), 3);
    assert_eq!(store.sq8_scales.len(), 3);
    assert_eq!(store.data_sq8.len(), 9);

    // mins[1] should now be the original mins[3] (id=13).
    assert!((store.sq8_mins[1] - mins_before[3]).abs() < 1e-6);
    assert!((store.sq8_scales[1] - scales_before[3]).abs() < 1e-6);
    // Unchanged: mins[0] == original mins[0], mins[2] == original mins[2].
    assert!((store.sq8_mins[0] - mins_before[0]).abs() < 1e-6);
    assert!((store.sq8_mins[2] - mins_before[2]).abs() < 1e-6);
}

#[test]
fn test_remove_last_index_sq8_truncates() {
    let mut store = mk_sq8_store(2);
    for i in 0..3u64 {
        let f = i as f32;
        insert_vector(&mut store, i, &[f, f + 1.0]);
    }
    remove_at_index(&mut store, 2);
    assert_eq!(store.ids, vec![0, 1]);
    assert_eq!(store.sq8_mins.len(), 2);
    assert_eq!(store.data_sq8.len(), 4);
}

// -------------------------------------------------------------------------
// Binary mode: byte chunks stay aligned
// -------------------------------------------------------------------------

#[test]
fn test_remove_middle_index_keeps_binary_buffer_synced() {
    // Dim 9 so each vector takes ceil(9/8) = 2 bytes — makes the chunk
    // arithmetic non-trivial.
    let mut store = mk_binary_store(9);
    let v0 = vec![1.0; 9];
    let v1 = vec![0.5; 9];
    let v2 = vec![-1.0; 9];
    let v3 = vec![1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0];
    insert_vector(&mut store, 1, &v0);
    insert_vector(&mut store, 2, &v1);
    insert_vector(&mut store, 3, &v2);
    insert_vector(&mut store, 4, &v3);
    assert_eq!(store.data_binary.len(), 8);

    let bytes_3 = store.data_binary[6..8].to_vec();

    // Remove index 1 (id=2). With swap-remove: id=4 bytes move to slot 1.
    remove_at_index(&mut store, 1);

    assert_eq!(store.ids, vec![1, 4, 3]);
    assert_eq!(store.data_binary.len(), 6);

    // Slot 1 (bytes 2..4) should now equal the original bytes of id=4.
    assert_eq!(store.data_binary[2..4], bytes_3[..]);
}

#[test]
fn test_remove_last_index_binary_truncates() {
    let mut store = mk_binary_store(8);
    for i in 0..3u64 {
        insert_vector(&mut store, i, &[1.0; 8]);
    }
    remove_at_index(&mut store, 2);
    assert_eq!(store.ids, vec![0, 1]);
    assert_eq!(store.data_binary.len(), 2);
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
