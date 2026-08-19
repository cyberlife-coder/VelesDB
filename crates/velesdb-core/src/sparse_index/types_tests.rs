use super::*;

#[test]
fn test_sparse_vector_sort_merge_dedup() {
    // (3,1.0),(1,2.0),(3,0.5) -> sorted: 1->2.0, 3->1.5
    let sv = SparseVector::new(vec![(3, 1.0), (1, 2.0), (3, 0.5)]);
    assert_eq!(sv.indices, vec![1, 3]);
    assert_eq!(sv.values, vec![2.0, 1.5]);
}

#[test]
fn test_sparse_vector_zero_filtered() {
    // (1,0.0),(2,1.0) -> only 2->1.0
    let sv = SparseVector::new(vec![(1, 0.0), (2, 1.0)]);
    assert_eq!(sv.indices, vec![2]);
    assert_eq!(sv.values, vec![1.0]);
}

#[test]
fn test_sparse_vector_negatives_allowed() {
    let sv = SparseVector::new(vec![(5, -0.3), (2, 1.0)]);
    assert_eq!(sv.indices, vec![2, 5]);
    assert_eq!(sv.values, vec![1.0, -0.3]);
}

#[test]
fn test_sparse_vector_empty_input() {
    let sv = SparseVector::new(vec![]);
    assert!(sv.is_empty());
    assert_eq!(sv.nnz(), 0);
}

#[test]
fn test_sparse_vector_from_sorted_unchecked() {
    let sv = SparseVector::from_sorted_unchecked(vec![1, 3, 5], vec![0.5, 1.0, 2.0]);
    assert_eq!(sv.indices, vec![1, 3, 5]);
    assert_eq!(sv.values, vec![0.5, 1.0, 2.0]);
}

#[test]
fn test_sparse_vector_nnz() {
    let sv = SparseVector::new(vec![(1, 1.0), (2, 2.0), (3, 3.0)]);
    assert_eq!(sv.nnz(), 3);
}

#[test]
fn test_sparse_vector_dot_product() {
    let a = SparseVector::new(vec![(1, 2.0), (3, 1.0)]);
    let b = SparseVector::new(vec![(1, 0.5), (2, 1.0), (3, 3.0)]);
    let result = a.dot(&b);
    assert!((result - 4.0).abs() < f32::EPSILON);
}

#[test]
fn test_sparse_vector_dot_disjoint() {
    let a = SparseVector::new(vec![(1, 1.0), (2, 2.0)]);
    let b = SparseVector::new(vec![(3, 3.0), (4, 4.0)]);
    assert!((a.dot(&b)).abs() < f32::EPSILON);
}

#[test]
fn test_sparse_vector_dot_empty() {
    let a = SparseVector::new(vec![(1, 1.0)]);
    let b = SparseVector::new(vec![]);
    assert!((a.dot(&b)).abs() < f32::EPSILON);
}

#[test]
fn test_posting_entry_size() {
    // PostingEntry is #[repr(C)] with u64 (8 bytes) + f32 (4 bytes) + 4 bytes alignment
    // padding = 16 bytes total. The on-disk packed layout uses 12 bytes (POSTING_DISK_SIZE).
    assert_eq!(std::mem::size_of::<PostingEntry>(), 16);
}

#[test]
fn test_scored_doc_ordering() {
    let high = ScoredDoc {
        score: 5.0,
        doc_id: 1,
    };
    let low = ScoredDoc {
        score: 2.0,
        doc_id: 2,
    };
    assert!(high > low);
}

#[test]
fn test_scored_doc_tiebreak_by_doc_id() {
    let a = ScoredDoc {
        score: 3.0,
        doc_id: 1,
    };
    let b = ScoredDoc {
        score: 3.0,
        doc_id: 2,
    };
    assert!(a < b); // Same score, lower doc_id is "less"
}

#[test]
fn test_sparse_vector_merge_cancellation() {
    // Duplicate indices that sum to zero should be filtered
    let sv = SparseVector::new(vec![(1, 1.0), (1, -1.0), (2, 3.0)]);
    assert_eq!(sv.indices, vec![2]);
    assert_eq!(sv.values, vec![3.0]);
}
