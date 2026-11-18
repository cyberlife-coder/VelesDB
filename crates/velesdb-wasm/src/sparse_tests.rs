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

    // Query {10: 1.0, 20: 1.0}: doc 4 = 0.3 + 1.5 = 1.8, doc 1 = 1.0 + 0.5 = 1.5,
    // doc 3 = 0.9, doc 2 = 0.8.
    let results: Vec<u64> = index
        .search_scored(&[10, 20], &[1.0, 1.0], 10)
        .unwrap()
        .iter()
        .map(SparseSearchResult::doc_id)
        .collect();
    assert_eq!(results, vec![4, 1, 3, 2]);
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
fn test_sparse_index_remove_deletes_doc_postings() {
    let mut index = SparseIndex::new();
    index.insert(1, &[10, 20], &[1.0, 0.5]).unwrap();
    index.insert(2, &[10, 30], &[0.8, 1.2]).unwrap();

    assert!(index.remove(1), "removing a present doc returns true");
    assert_eq!(index.doc_count(), 1, "remove decrements doc_count");

    // Doc 1's postings are gone from every term it touched; doc 2 still matches.
    let results = index
        .search_scored(&[10, 20, 30], &[1.0, 1.0, 1.0], 10)
        .unwrap();
    let ids: Vec<u64> = results.iter().map(|r| r.doc_id()).collect();
    assert_eq!(ids, vec![2], "only the surviving doc may match");
}

#[test]
fn test_sparse_index_remove_unknown_doc_is_noop() {
    let mut index = SparseIndex::new();
    index.insert(1, &[10], &[1.0]).unwrap();

    assert!(!index.remove(99), "removing an absent doc returns false");
    assert_eq!(index.doc_count(), 1, "doc_count must not underflow");
}

#[test]
fn test_sparse_index_remove_is_idempotent() {
    let mut index = SparseIndex::new();
    index.insert(1, &[10], &[1.0]).unwrap();

    assert!(index.remove(1));
    assert!(!index.remove(1), "a second remove reports the doc is gone");
    assert_eq!(index.doc_count(), 0);
    assert!(
        index.search_scored(&[10], &[1.0], 10).unwrap().is_empty(),
        "an emptied posting list matches nothing"
    );
}

#[test]
fn test_sparse_index_remove_all_then_reinsert() {
    let mut index = SparseIndex::new();
    index.insert(1, &[10], &[1.0]).unwrap();
    index.insert(2, &[10], &[0.5]).unwrap();
    index.remove(1);
    index.remove(2);

    assert_eq!(index.doc_count(), 0);
    assert_eq!(index.inner.term_count(), 0, "empty terms are dropped");

    index.insert(1, &[10], &[2.0]).unwrap();
    assert_eq!(index.doc_count(), 1);
    let results = index.search_scored(&[10], &[1.0], 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].doc_id(), 1);
}

#[test]
fn test_sparse_index_reinsert_replaces_dropped_terms() {
    let mut index = SparseIndex::new();
    index.insert(1, &[10, 20], &[1.0, 0.5]).unwrap();

    // Re-inserting the same doc replaces its vector: terms the new vector
    // leaves out must stop matching — insert is an upsert, not an
    // accumulation (#2287).
    index.insert(1, &[99], &[0.7]).unwrap();
    assert_eq!(index.doc_count(), 1);
    assert!(
        index
            .search_scored(&[10, 20], &[1.0, 1.0], 10)
            .unwrap()
            .is_empty(),
        "terms the replacement dropped must not match"
    );
    let results = index.search_scored(&[99], &[1.0], 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].doc_id(), 1);
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

#[test]
fn test_sparse_index_keeps_a_small_nonzero_weight() {
    // The binding dropped any weight under f32::EPSILON; core keeps every
    // nonzero weight (sub-unit TF-IDF and SPLADE scores are legitimate).
    let mut index = SparseIndex::new();
    index.insert(1, &[10], &[1e-9]).unwrap();
    let results = index.search_scored(&[10], &[1.0], 10).unwrap();
    assert_eq!(results.len(), 1, "a 1e-9 weight still matches");
}

#[test]
fn test_sparse_index_refuses_a_non_finite_weight() {
    let mut index = SparseIndex::new();
    for weight in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(
            sparse_vector("", &[10], &[weight]).is_err(),
            "weight {weight} is refused"
        );
    }
    assert_eq!(index.doc_count(), 0);
    index.insert(1, &[10], &[1.0]).unwrap();
    let err = index
        .search_scored(&[10], &[f32::NAN], 10)
        .expect_err("a NaN query weight is refused");
    assert!(err.contains("VELES-005"), "{err}");
}

#[test]
fn the_hybrid_branch_depth_is_cores() {
    // A JS client fusing with `hybrid_search_fuse` reads its per-branch depth
    // here: one rule, core's, as VelesQL's hybrid search reads it.
    for limit in [0, 1, 5, 10, 11, 100, usize::MAX] {
        assert_eq!(
            dense_sparse_candidate_k(limit),
            velesdb_core::fusion::dense_sparse_candidate_k(limit),
            "limit {limit}"
        );
    }
}
