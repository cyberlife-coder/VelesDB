//! Tests for `bm25` module

use super::bm25::*;

// =========================================================================
// Basic functionality tests
// =========================================================================

#[test]
fn test_bm25_index_creation() {
    let index = Bm25Index::new();
    assert!(index.is_empty());
    assert_eq!(index.len(), 0);
    assert_eq!(index.term_count(), 0);
}

#[test]
fn test_bm25_index_with_custom_params() {
    // Identical corpus indexed with default vs. custom params.
    // A long doc and a short doc share the term "rust"; b controls
    // length-normalization, so changing b/k1 must change the scores.
    let make = |idx: &Bm25Index| {
        idx.add_document(1, "rust");
        idx.add_document(2, "rust is a systems programming language that runs fast");
    };

    let default_idx = Bm25Index::new(); // k1=1.2, b=0.75
    make(&default_idx);
    let default_scores = default_idx.search("rust", 10);

    let custom_idx = Bm25Index::with_params(Bm25Params { k1: 1.5, b: 0.5 });
    make(&custom_idx);
    let custom_scores = custom_idx.search("rust", 10);

    // Same docs match under both parameterizations.
    assert_eq!(default_scores.len(), 2);
    assert_eq!(custom_scores.len(), 2);
    // Scores must be finite and the custom params must actually reach the
    // scoring path: at least one matching doc's score differs from default.
    for (_, s) in &custom_scores {
        assert!(s.is_finite() && *s > 0.0);
    }
    let changed = default_scores
        .iter()
        .zip(&custom_scores)
        .any(|((_, d), (_, c))| (d - c).abs() > f32::EPSILON);
    assert!(changed, "custom k1/b must alter BM25 scores vs. defaults");
}

#[test]
fn test_add_single_document() {
    let index = Bm25Index::new();
    index.add_document(1, "hello world");

    assert_eq!(index.len(), 1);
    assert!(!index.is_empty());
    assert!(index.term_count() >= 2); // "hello" and "world"
}

#[test]
fn test_add_multiple_documents() {
    let index = Bm25Index::new();
    index.add_document(1, "rust programming language");
    index.add_document(2, "python programming language");
    index.add_document(3, "java programming");

    assert_eq!(index.len(), 3);
}

#[test]
fn test_remove_document() {
    let index = Bm25Index::new();
    index.add_document(1, "hello world");
    index.add_document(2, "goodbye world");

    assert_eq!(index.len(), 2);

    let removed = index.remove_document(1);
    assert!(removed);
    assert_eq!(index.len(), 1);

    // Removing again should return false
    let removed_again = index.remove_document(1);
    assert!(!removed_again);
}

#[test]
fn test_update_document() {
    let index = Bm25Index::new();
    index.add_document(1, "original text");
    index.add_document(1, "updated text"); // Same ID

    assert_eq!(index.len(), 1); // Still one document
}

// =========================================================================
// Tokenization tests
// =========================================================================

#[test]
fn test_tokenize_basic() {
    let tokens = Bm25Index::tokenize("Hello World");
    assert_eq!(tokens, vec!["hello", "world"]);
}

#[test]
fn test_tokenize_punctuation() {
    let tokens = Bm25Index::tokenize("Hello, World! How are you?");
    assert_eq!(tokens, vec!["hello", "world", "how", "are", "you"]);
}

#[test]
fn test_tokenize_single_chars_filtered() {
    let tokens = Bm25Index::tokenize("I am a test");
    // Single characters should be filtered out
    assert!(!tokens.contains(&"i".to_string()));
    assert!(!tokens.contains(&"a".to_string()));
    assert!(tokens.contains(&"am".to_string()));
    assert!(tokens.contains(&"test".to_string()));
}

#[test]
fn test_tokenize_empty() {
    let tokens = Bm25Index::tokenize("");
    assert!(tokens.is_empty());
}

// =========================================================================
// Search tests
// =========================================================================

#[test]
fn test_search_single_term() {
    let index = Bm25Index::new();
    index.add_document(1, "rust programming language");
    index.add_document(2, "python programming language");
    index.add_document(3, "rust is fast");

    let results = index.search("rust", 10);

    // Documents 1 and 3 should match
    assert_eq!(results.len(), 2);
    let ids: Vec<u64> = results.iter().map(|(id, _)| *id).collect();
    assert!(ids.contains(&1));
    assert!(ids.contains(&3));
}

#[test]
fn test_search_multiple_terms() {
    let index = Bm25Index::new();
    index.add_document(1, "rust programming language fast");
    index.add_document(2, "python programming language");
    index.add_document(3, "rust systems programming");

    let results = index.search("rust programming", 10);

    // All docs match "programming", docs 1 and 3 also match "rust"
    assert!(!results.is_empty());

    let ids: Vec<u64> = results.iter().map(|(id, _)| *id).collect();
    assert!(ids.contains(&1));
    assert!(ids.contains(&3));

    let score = |id: u64| {
        results
            .iter()
            .find(|(i, _)| *i == id)
            .map_or(0.0, |(_, s)| *s)
    };
    // Docs matching BOTH "rust" and "programming" must outscore the doc matching only "programming".
    assert!(
        score(1) > score(2),
        "dual-match doc 1 must outscore single-match doc 2"
    );
    assert!(
        score(3) > score(2),
        "dual-match doc 3 must outscore single-match doc 2"
    );
}

#[test]
fn test_search_no_match() {
    let index = Bm25Index::new();
    index.add_document(1, "rust programming");
    index.add_document(2, "python programming");

    let results = index.search("javascript", 10);
    assert!(results.is_empty());
}

#[test]
fn test_search_empty_query() {
    let index = Bm25Index::new();
    index.add_document(1, "rust programming");

    let results = index.search("", 10);
    assert!(results.is_empty());
}

#[test]
fn test_search_empty_index() {
    let index = Bm25Index::new();
    let results = index.search("rust", 10);
    assert!(results.is_empty());
}

#[test]
fn test_search_limit_k() {
    let index = Bm25Index::new();
    for i in 1..=100 {
        index.add_document(i, &format!("document number {i} about rust"));
    }

    let results = index.search("rust", 5);
    assert_eq!(results.len(), 5);
}

#[test]
fn test_search_scores_sorted_descending() {
    let index = Bm25Index::new();
    index.add_document(1, "rust");
    index.add_document(2, "rust rust"); // Higher TF
    index.add_document(3, "rust rust rust");

    let results = index.search("rust", 10);

    // Scores should be sorted descending
    for window in results.windows(2) {
        assert!(window[0].1 >= window[1].1);
    }
}

// =========================================================================
// BM25 scoring tests
// =========================================================================

#[test]
fn test_idf_common_term() {
    let index = Bm25Index::new();
    // "programming" appears in all documents
    index.add_document(1, "rust programming");
    index.add_document(2, "python programming");
    index.add_document(3, "java programming");

    // "rust" appears in 1 document
    let results = index.search("rust", 10);
    assert_eq!(results.len(), 1);

    // "programming" appears in all - should have lower IDF but still return results
    let results = index.search("programming", 10);
    assert_eq!(results.len(), 3);
}

#[test]
fn test_longer_documents_normalized() {
    let index = Bm25Index::new();
    // Short document with "rust"
    index.add_document(1, "rust");
    // Long document with "rust" once among many other words
    index.add_document(
        2,
        "rust is a systems programming language that runs blazingly fast",
    );

    let results = index.search("rust", 10);

    // Both should match
    assert_eq!(results.len(), 2);
    // The short document should score higher (more concentrated term)
    assert_eq!(results[0].0, 1);
}

// =========================================================================
// Edge cases
// =========================================================================

#[test]
fn test_special_characters() {
    let index = Bm25Index::new();
    index.add_document(1, "hello@world.com is an email");

    let results = index.search("hello", 10);
    assert_eq!(results.len(), 1);

    let results = index.search("world", 10);
    assert_eq!(results.len(), 1);
}

#[test]
fn test_numbers_in_text() {
    let index = Bm25Index::new();
    index.add_document(1, "version 2.0 released in 2024");

    let results = index.search("2024", 10);
    assert_eq!(results.len(), 1);
}

#[test]
fn test_unicode_text() {
    let index = Bm25Index::new();
    index.add_document(1, "café résumé naïve");

    let results = index.search("café", 10);
    assert_eq!(results.len(), 1);
}

#[test]
fn test_duplicate_terms_in_query() {
    let index = Bm25Index::new();
    index.add_document(1, "rust programming");

    // Query with duplicate terms
    let results = index.search("rust rust rust", 10);
    assert_eq!(results.len(), 1);
}

// =========================================================================
// Thread safety tests
// =========================================================================

#[test]
fn test_concurrent_reads() {
    use std::sync::Arc;
    use std::thread;

    let index = Arc::new(Bm25Index::new());

    // Add documents
    for i in 1..=100 {
        index.add_document(i, &format!("document {i} about rust programming"));
    }

    // Spawn multiple reader threads
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let idx = Arc::clone(&index);
            thread::spawn(move || {
                for _ in 0..100 {
                    let results = idx.search("rust", 10);
                    assert!(!results.is_empty());
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("Thread panicked");
    }
}

#[test]
fn test_concurrent_add_same_point_id_keeps_single_mapping() {
    use std::sync::{Arc, Barrier};
    use std::thread;

    let index = Arc::new(Bm25Index::new());
    let barrier = Arc::new(Barrier::new(8));

    let handles: Vec<_> = (0..8)
        .map(|i| {
            let idx = Arc::clone(&index);
            let sync = Arc::clone(&barrier);
            thread::spawn(move || {
                sync.wait();
                idx.add_document(42, &format!("thread-{i} document"));
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("Thread panicked");
    }

    assert_eq!(index.len(), 1);
    let results = index.search("document", 10);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, 42);
}

// =========================================================================
// ID mapping tests (PointId u64 -> BM25 DocId u32)
// =========================================================================

#[test]
fn test_add_document_id_exceeds_u32_max_is_supported() {
    let index = Bm25Index::new();
    let large_id = u64::from(u32::MAX) + 42;
    index.add_document(large_id, "test document");

    let results = index.search("test", 10);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, large_id);
}

#[test]
fn test_remove_document_id_exceeds_u32_max_is_supported() {
    let index = Bm25Index::new();
    let large_id = u64::from(u32::MAX) + 7;
    index.add_document(large_id, "remove me");

    assert!(index.remove_document(large_id));
    assert!(!index.remove_document(large_id));
}

#[test]
fn test_update_document_removes_old_terms() {
    let index = Bm25Index::new();
    index.add_document(1, "alpha beta");
    index.add_document(1, "gamma delta");

    let old_term_results = index.search("alpha", 10);
    assert!(old_term_results.is_empty());

    let new_term_results = index.search("gamma", 10);
    assert_eq!(new_term_results.len(), 1);
    assert_eq!(new_term_results[0].0, 1);
}

// =========================================================================
// #897 — snapshot validation on load (untrusted on-disk state)
// =========================================================================

#[test]
fn test_from_snapshot_valid_round_trips() {
    let index = Bm25Index::new();
    index.add_document(1, "rust systems programming");
    index.add_document(2, "python data science");

    let snapshot = index.to_snapshot();
    let restored = Bm25Index::from_snapshot(snapshot).expect("valid snapshot must load");

    assert_eq!(restored.len(), 2);
    assert_eq!(restored.search("rust", 10).len(), 1);
    assert_eq!(restored.search("python", 10).len(), 1);
}

#[test]
fn test_from_snapshot_rejects_version_mismatch() {
    let index = Bm25Index::new();
    index.add_document(1, "hello world");

    let mut snapshot = index.to_snapshot();
    snapshot.version = BM25_SNAPSHOT_VERSION + 1;

    match Bm25Index::from_snapshot(snapshot) {
        Err(e) => assert!(
            e.to_string().contains("version mismatch"),
            "expected version-mismatch error, got: {e}"
        ),
        Ok(_) => panic!("version mismatch must be rejected"),
    }
}

#[test]
fn test_from_snapshot_recomputes_inconsistent_counters() {
    let index = Bm25Index::new();
    index.add_document(1, "alpha beta gamma");
    index.add_document(2, "delta epsilon");

    // Tamper the counters: zero total_doc_length while doc_count stays positive.
    // The pre-fix code installed these verbatim → avgdl = 0 → inf/NaN scores.
    let mut snapshot = index.to_snapshot();
    snapshot.doc_count = 999;
    snapshot.total_doc_length = 0;

    // documents are intact, so validation recomputes consistent counters.
    let restored = Bm25Index::from_snapshot(snapshot).expect("counters recomputed from documents");
    assert_eq!(restored.len(), 2, "doc_count recomputed from documents");

    // Scores must be finite (no inf/NaN from a zero avgdl).
    for (_, score) in restored.search("alpha", 10) {
        assert!(score.is_finite(), "BM25 score must be finite");
    }
}

/// #897 follow-up: `from_snapshot` must recompute `doc_count`/`avgdl` from the
/// SCORABLE corpus (docs present in `point_to_doc`), not all of `documents`.
/// A doc in `documents` but missing from `point_to_doc` is skipped during
/// inverted-index reconstruction, so counting it would inflate `N`/`avgdl` and
/// skew every IDF and length-normalization.
#[test]
fn test_from_snapshot_counts_only_scorable_corpus() {
    let index = Bm25Index::new();
    index.add_document(1, "rust programming language");
    index.add_document(2, "python programming language");

    let mut snap = index.to_snapshot();
    assert_eq!(snap.documents.len(), 2);
    // Divergence: point 2 stays in `documents` but is no longer scorable.
    snap.point_to_doc.remove(&2);

    let restored = Bm25Index::from_snapshot(snap).expect("valid snapshot");
    assert_eq!(
        restored.len(),
        1,
        "doc_count must reflect only the scorable corpus, not all documents"
    );
}
