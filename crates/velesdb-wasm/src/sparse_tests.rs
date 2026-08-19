use super::*;

#[test]
fn test_sparse_index_insert_search_basic() {
    let mut index = SparseIndex::new();
    // Insert 5 documents
    index.insert(1, &[10, 20, 30], &[1.0, 0.5, 0.3]).unwrap();
    index.insert(2, &[10, 40], &[0.8, 1.2]).unwrap();
    index.insert(3, &[20, 30, 50], &[0.9, 0.7, 0.4]).unwrap();
    index.insert(4, &[10, 20], &[0.3, 1.5]).unwrap();
    index.insert(5, &[30, 40, 50], &[1.0, 0.6, 0.2]).unwrap();

    assert_eq!(index.doc_count(), 5);

    // Manually test accumulation for query = {10: 1.0, 20: 1.0}
    // Doc 1: 1.0*1.0 + 0.5*1.0 = 1.5
    // Doc 2: 0.8*1.0 = 0.8
    // Doc 3: 0.9*1.0 = 0.9
    // Doc 4: 0.3*1.0 + 1.5*1.0 = 1.8
    let mut accum: std::collections::HashMap<u64, f32> = std::collections::HashMap::new();
    let query_terms: &[(u32, f32)] = &[(10, 1.0), (20, 1.0)];
    for &(term_id, q_w) in query_terms {
        if let Some(list) = index.postings.get(&term_id) {
            for &(doc_id, d_w) in list {
                *accum.entry(doc_id).or_insert(0.0) += q_w * d_w;
            }
        }
    }
    let mut results: Vec<(u64, f32)> = accum.into_iter().collect();
    results.sort_by(|a, b| b.1.total_cmp(&a.1));

    assert_eq!(results[0].0, 4); // Doc 4 = 1.8
    assert_eq!(results[1].0, 1); // Doc 1 = 1.5
}

#[test]
fn test_sparse_index_empty() {
    let index = SparseIndex::new();
    assert_eq!(index.doc_count(), 0);
}

#[test]
fn test_sparse_index_insert_works() {
    let mut index = SparseIndex::new();
    // Verify correct insert works with matching lengths.
    assert!(index.insert(1, &[10, 20], &[1.0, 2.0]).is_ok());
    assert_eq!(index.doc_count(), 1);
}

#[test]
fn test_sparse_index_upsert_does_not_increment_doc_count() {
    let mut index = SparseIndex::new();
    // First insert: new doc → count becomes 1.
    index.insert(42, &[1, 2], &[1.0, 0.5]).unwrap();
    assert_eq!(
        index.doc_count(),
        1,
        "first insert should increment doc_count"
    );

    // Second insert of the same doc_id with overlapping terms → still 1.
    index.insert(42, &[1, 3], &[2.0, 0.3]).unwrap();
    assert_eq!(
        index.doc_count(),
        1,
        "re-insert of existing doc_id must not increment doc_count"
    );

    // Third insert of the same doc_id with a completely disjoint term set → still 1.
    index.insert(42, &[99], &[0.7]).unwrap();
    assert_eq!(
        index.doc_count(),
        1,
        "re-insert with disjoint terms must not increment doc_count"
    );

    // A different doc_id → count becomes 2.
    index.insert(99, &[1], &[1.0]).unwrap();
    assert_eq!(
        index.doc_count(),
        2,
        "new doc_id should increment doc_count"
    );
}

#[test]
fn test_rrf_fusion_basic() {
    // Test RRF logic manually (can't call wasm function in native tests).
    let k_f32 = 60.0_f32;
    let dense: &[(u64, f32)] = &[(1_u64, 0.9_f32), (2, 0.8), (3, 0.7)];
    let sparse: &[(u64, f32)] = &[(2_u64, 5.0_f32), (3, 4.0), (4, 3.0)];

    let mut scores: std::collections::HashMap<u64, f32> = std::collections::HashMap::new();
    for (rank, &(doc_id, _)) in dense.iter().enumerate() {
        *scores.entry(doc_id).or_insert(0.0) += 1.0 / (k_f32 + (rank as f32) + 1.0);
    }
    for (rank, &(doc_id, _)) in sparse.iter().enumerate() {
        *scores.entry(doc_id).or_insert(0.0) += 1.0 / (k_f32 + (rank as f32) + 1.0);
    }

    let mut results: Vec<(u64, f32)> = scores.into_iter().collect();
    results.sort_by(|a, b| b.1.total_cmp(&a.1));

    // Doc 2 appears in both lists (rank 1 in dense, rank 0 in sparse) -> highest RRF
    assert_eq!(results[0].0, 2);
    // Doc 3 also in both
    assert_eq!(results[1].0, 3);
}
