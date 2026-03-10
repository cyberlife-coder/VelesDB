use super::{search::sparse_search, SparseInvertedIndex, SparseVector};

fn make_vector(pairs: &[(u32, f32)]) -> SparseVector {
    SparseVector::new(pairs.to_vec())
}

fn posting_signature(index: &SparseInvertedIndex, term_id: u32) -> Vec<(u64, u32)> {
    index
        .get_all_postings(term_id)
        .into_iter()
        .map(|entry| (entry.doc_id, entry.weight.to_bits()))
        .collect()
}

fn sample_docs() -> Vec<(u64, SparseVector)> {
    vec![
        (1, make_vector(&[(1, 0.5), (3, 1.0), (7, 0.3)])),
        (2, make_vector(&[(1, 0.8), (4, 0.5)])),
        (3, make_vector(&[(2, 1.1), (7, 0.9)])),
        (4, make_vector(&[(1, 1.2), (2, 0.2), (8, 0.4)])),
    ]
}

#[test]
fn test_insert_batch_chunk_matches_sequential_index_shape() {
    let sequential = SparseInvertedIndex::new();
    let batched = SparseInvertedIndex::new();
    let docs = sample_docs();

    for (doc_id, vector) in &docs {
        sequential.insert(*doc_id, vector);
    }
    batched.insert_batch_chunk(&docs);

    assert_eq!(batched.doc_count(), sequential.doc_count());
    for term_id in [1_u32, 2, 3, 4, 7, 8] {
        assert_eq!(
            posting_signature(&batched, term_id),
            posting_signature(&sequential, term_id),
            "term_id={term_id}"
        );
    }
}

#[test]
fn test_insert_batch_chunk_preserves_upsert_last_write_wins() {
    let index = SparseInvertedIndex::new();
    let docs = vec![
        (42_u64, make_vector(&[(1, 0.5), (2, 0.1)])),
        (42_u64, make_vector(&[(1, 1.5), (2, 0.3)])),
        (7_u64, make_vector(&[(1, 0.7)])),
    ];

    index.insert_batch_chunk(&docs);

    assert_eq!(index.doc_count(), 2);
    let postings = index.get_all_postings(1);
    let updated = postings
        .iter()
        .find(|entry| entry.doc_id == 42)
        .expect("doc 42 should be present");
    assert!((updated.weight - 1.5).abs() < f32::EPSILON);
}

#[test]
fn test_insert_batch_chunk_keeps_search_results_equivalent() {
    let sequential = SparseInvertedIndex::new();
    let batched = SparseInvertedIndex::new();
    let docs = sample_docs();
    let query = make_vector(&[(1, 1.0), (7, 0.8)]);

    for (doc_id, vector) in &docs {
        sequential.insert(*doc_id, vector);
    }
    batched.insert_batch_chunk(&docs);

    assert_eq!(
        sparse_search(&batched, &query, 3),
        sparse_search(&sequential, &query, 3)
    );
}
